//! ワーカーごとの探索状態と停止判定。

use std::sync::atomic::Ordering as AtomicOrdering;
use std::time::Duration;

use crate::eval::Pst;
use crate::eval::pst::accumulator::PstAccumulator;
use crate::search::events::StopReason;
use crate::search::snapshot::search_key;
use crate::search::{MAX_PLY, TranspositionTable};
use minase_core::MoveGenerator;
use minase_core::board::Square;
use minase_core::mv::Move;
use minase_core::position::Position;
use minase_core::rules::MoveRules;

use super::capture_history::CaptureHistory;
use super::correction::{CorrectionTable, material_key};
use super::history::HistoryTable;
use super::ordering::MovePicker;
use super::params;
use super::quiesce::{CaptureRanks, QsearchBuffers};
use super::root::RootResults;
use super::team::SharedSearch;
use super::time::{TimeBudget, iteration_prediction_fits};

/// 停止要求と時間切れを検査するノード数間隔。
pub(super) const STOP_CHECK_INTERVAL: u64 = 4096;

/// 1つのplyに記録するkiller手の数。
pub(super) const KILLER_COUNT: usize = 2;

/// plyごとに新しい順で保持するkiller表。
type KillerTable = [[Option<Move>; KILLER_COUNT]; MAX_PLY as usize + 1];

/// ワーカー固有の探索状態を構築する。
pub(super) fn new_searcher<'a>(
    pst: &'a Pst,
    position: &Position,
    rules: MoveRules,
    history_keys: &'a [u64],
    shared: &'a SharedSearch<'a>,
    tt: &'a TranspositionTable,
    history: &'a mut HistoryTable,
) -> Searcher<'a> {
    let root_accumulator = pst.refresh_accumulator(position);
    Searcher {
        root_results: None,
        #[cfg(test)]
        interrupt_at: None,
        #[cfg(feature = "search-stats")]
        stats: crate::search::SearchStats::default(),
        pst,
        rules,
        generator: MoveGenerator::new(rules),
        history_keys,
        path_keys: vec![search_key(position)],
        null_move_ply: None,
        previous_capture: [None; MAX_PLY as usize + 1],
        null_move_boundary: None,
        nodes: 0,
        shared,
        stop_reason: None,
        ponder_iteration: None,
        pv: (0..=MAX_PLY)
            .map(|ply| Vec::with_capacity((MAX_PLY - ply) as usize))
            .collect(),
        capture_ranks: CaptureRanks::new(pst),
        move_pickers: (0..=MAX_PLY)
            .map(|_| MovePicker::new(None, [None; KILLER_COUNT]))
            .collect(),
        qsearch: (0..=MAX_PLY).map(|_| QsearchBuffers::default()).collect(),
        accumulators: [root_accumulator; MAX_PLY as usize + 1],
        material_keys: [material_key(position); MAX_PLY as usize + 1],
        correction: CorrectionTable::new(pst.pawn_value()),
        delta_margin: pst.pawn_value() * params::delta_margin() / 100,
        history,
        capture_history: CaptureHistory::new(),
        killers: [[None; KILLER_COUNT]; MAX_PLY as usize + 1],
        tt,
    }
}

/// 主ワーカーが先読みから継続した反復の判定材料。
pub(super) struct PonderIteration {
    pub(super) started: Duration,
    pub(super) stable: bool,
    pub(super) checked: bool,
    pub(super) budget: TimeBudget,
}

/// 1回の探索実行の可変状態。
pub(super) struct Searcher<'a> {
    /// 対象の探索の主ワーカーだけが保持する途中結果の採用材料。
    pub(super) root_results: Option<RootResults>,
    /// 「途中結果の採用」の停止契約を実着手の境界で再現する検査用の停止点。
    #[cfg(test)]
    pub(super) interrupt_at: Option<(u64, StopReason)>,
    /// このワーカーだけが更新する探索統計。
    #[cfg(feature = "search-stats")]
    pub(super) stats: crate::search::SearchStats,
    /// 探索中に使う検証済み学習PST。
    pub(super) pst: &'a Pst,
    /// 探索内の着手適用に使う規則。
    pub(super) rules: MoveRules,
    /// 探索ノードでの合法手生成器。
    pub(super) generator: MoveGenerator,
    /// 対局開始から現局面までの探索局面キー。反復の検出に使う。
    pub(super) history_keys: &'a [u64],
    /// 探索経路上の局面キー。探索内の反復の検出に使う。
    pub(super) path_keys: Vec<u64>,
    /// null moveで到達した直後のノードのply。
    pub(super) null_move_ply: Option<u32>,
    /// 直前の捕獲手の到達升。居喰いでも捕獲した駒の最終位置を使う。
    /// 根とnull moveの直後は取り返しの対象を持たない。
    pub(super) previous_capture: [Option<Square>; MAX_PLY as usize + 1],
    /// 直近のnull move直後の局面キーが探索経路に占める位置。
    /// 部分木内の反復判定はこの境界以降だけを参照する。
    pub(super) null_move_boundary: Option<usize>,
    /// 実際の着手を盤面へ適用した回数。
    pub(super) nodes: u64,
    /// 探索チームで共有する停止状態と予算。
    pub(super) shared: &'a SharedSearch<'a>,
    /// 中断時に記録する停止条件。
    pub(super) stop_reason: Option<StopReason>,
    /// 先読みで始めた主ワーカーだけが記録する反復。
    pub(super) ponder_iteration: Option<PonderIteration>,
    /// plyごとの主変化。行plyは、その深さ以降の最善応手列を保持する。
    pub(super) pv: Vec<Vec<Move>>,
    /// 静止探索の捕獲生成・整列用バッファをplyごとに再利用する。
    pub(super) qsearch: Vec<QsearchBuffers>,
    /// 「段階6」（movegen-speedup-2.md）の主探索用領域を深さごとに再利用する。
    pub(super) move_pickers: Vec<MovePicker>,
    /// ワーカー内で共有する捕獲価値の順位。
    pub(super) capture_ranks: CaptureRanks,
    /// plyごとのPST生重み和。
    pub(super) accumulators: [PstAccumulator; MAX_PLY as usize + 1],
    /// plyごとの駒種別枚数のハッシュ。null moveでは変化しない。
    pub(super) material_keys: [u64; MAX_PLY as usize + 1],
    /// 反復深化の間で共有する、このワーカー専用の補正表。
    pub(super) correction: CorrectionTable,
    /// 静止探索で小さな捕獲を残すための余裕値。
    pub(super) delta_margin: i32,
    /// βカットを起こした非捕獲手の手番側・移動元・移動先別スコア。
    pub(super) history: &'a mut HistoryTable,
    /// 捕獲履歴は探索ごとに初期化する。
    pub(super) capture_history: CaptureHistory,
    /// βカットを起こした非捕獲手をplyごとに新しい順で保持する表。
    pub(super) killers: KillerTable,
    /// 置換表。
    pub(super) tt: &'a TranspositionTable,
}

impl Searcher<'_> {
    /// null moveの部分木では境界以降だけ、それ以外では対局履歴と全経路を照合する。
    pub(super) fn is_repetition(&self, key: u64) -> bool {
        match self.null_move_boundary {
            Some(boundary) => self.path_keys[boundary..].contains(&key),
            None => self.history_keys.contains(&key) || self.path_keys.contains(&key),
        }
    }

    /// 実着手の適用直前に停止条件を検査し、続行可能なら適用回数を数える。
    pub(super) fn enter_node(&mut self) -> bool {
        #[cfg(test)]
        if let Some((nodes, reason)) = self.interrupt_at
            && self.nodes >= nodes
        {
            self.shared.stop(reason);
            self.stop_reason = Some(reason);
            return false;
        }
        if self.shared.observe_external_stop() {
            self.stop_reason = Some(StopReason::ExternalStop);
            return false;
        }
        if self.shared.team_stop.load(AtomicOrdering::Acquire) {
            self.stop_reason = Some(self.shared.reason());
            return false;
        }
        if self.nodes.is_multiple_of(STOP_CHECK_INTERVAL) && !self.check_time() {
            return false;
        }
        if let Some(limit) = self.shared.node_limit {
            if !self.shared.reserve_node(limit) {
                self.stop_reason = Some(self.shared.reason());
                return false;
            }
        } else {
            self.shared
                .total_nodes
                .fetch_add(1, AtomicOrdering::Relaxed);
        }
        self.nodes += 1;
        true
    }

    /// hを先に読み、hardを当て直しより先に検査する。
    /// 対象の探索の秒読み期では当て直しを行わない
    /// （`docs/plans/byoyomi-time-usage.md`の「秒読み期の締切」）。
    pub(super) fn check_time(&mut self) -> bool {
        let Some(limit) = self.shared.hard_limit else {
            return true;
        };
        let hit_ns = limit.hit_ns.load(AtomicOrdering::Relaxed);
        if hit_ns == u64::MAX {
            return true;
        }
        let hit = Duration::from_nanos(hit_ns);
        let elapsed = self.shared.started.elapsed();
        let reason = if elapsed.saturating_sub(hit) >= limit.duration {
            Some(StopReason::HardLimit)
        } else if let Some(iteration) = &mut self.ponder_iteration {
            if iteration.budget.byoyomi_period || iteration.checked {
                return true;
            }
            iteration.checked = true;
            (iteration.started < hit
                && !iteration_prediction_fits(
                    iteration.started,
                    hit,
                    iteration.budget,
                    iteration.stable,
                ))
            .then_some(StopReason::SoftLimit)
        } else {
            None
        };
        if let Some(reason) = reason {
            self.shared.stop(reason);
            self.stop_reason = Some(reason);
            return false;
        }
        true
    }

    /// 指定plyの主変化を、この手と子plyの主変化の連結で置き換える。
    pub(super) fn update_pv(&mut self, ply: u32, mv: Move) {
        let index = ply as usize;
        let (rows, child_rows) = self.pv.split_at_mut(index + 1);
        let row = &mut rows[index];
        row.clear();
        row.push(mv);
        if let Some(child) = child_rows.first() {
            row.extend_from_slice(child);
        }
    }
}
