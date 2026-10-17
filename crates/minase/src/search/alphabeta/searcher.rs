//! ワーカーごとの探索状態と停止判定。

use std::sync::atomic::Ordering as AtomicOrdering;
use std::time::Duration;

use crate::eval::Nnue;
use crate::eval::nnue::accumulator::NnueAccumulator;
use crate::search::events::StopReason;
use crate::search::{DRAW_SCORE, MAX_PLY, REPETITION_WIN, SearchRepetition, TranspositionTable};
use minase_core::board::Square;
use minase_core::mv::Move;
use minase_core::position::Position;
use minase_core::rules::MoveRules;
use minase_core::{Color, MoveGenerator, move_was_attacking, repetition_key};

use super::capture_history::CaptureHistory;
use super::correction::{CorrectionTable, material_key};
use super::history::HistoryTable;
use super::ordering::MovePicker;
use super::params;
use super::quiesce::{CaptureRanks, QsearchBuffers};
use super::root::RootResults;
use super::team::SharedSearch;
use super::time::{TimeBudget, iteration_prediction_fits};

/// 時計を検査するワーカーごとのノード数間隔。
/// 外部停止とチーム停止は[`Searcher::enter_node`]を呼ぶたびに読み、4096ノードごとなのは時計の確認だけである。
pub(super) const STOP_CHECK_INTERVAL: u64 = 4096;

/// 1つのplyに記録するkiller手の数。
pub(super) const KILLER_COUNT: usize = 2;

/// plyごとに新しい順で保持するkiller表。
type KillerTable = [[Option<Move>; KILLER_COUNT]; MAX_PLY as usize + 1];

/// ワーカー固有の探索状態を構築する。
#[allow(clippy::too_many_arguments)]
pub(super) fn new_searcher<'a>(
    nnue: &'a Nnue,
    position: &Position,
    rules: MoveRules,
    repetition: SearchRepetition,
    draw_value: i32,
    history_keys: &'a [u64],
    shared: &'a SharedSearch<'a>,
    tt: &'a TranspositionTable,
    history: &'a mut HistoryTable,
    capture_history: &'a mut CaptureHistory,
) -> Searcher<'a> {
    let root_accumulator = nnue.refresh_accumulator(position);
    Searcher {
        root_results: None,
        completed_root_nodes: Vec::new(),
        current_root_nodes: Vec::new(),
        #[cfg(test)]
        interrupt_at: None,
        #[cfg(feature = "search-stats")]
        stats: crate::search::SearchStats::default(),
        nnue,
        rules,
        repetition,
        draw_value,
        root_position: position.clone(),
        generator: MoveGenerator::new(rules),
        history_keys,
        path_keys: vec![repetition_key(repetition.rule(), position)],
        path_moves: Vec::new(),
        null_move_ply: None,
        previous_capture: [None; MAX_PLY as usize + 1],
        null_move_boundary: None,
        nodes: 0,
        shared,
        stop_reason: None,
        ponder_iteration: None,
        tt_pv: [false; MAX_PLY as usize + 1],
        pv: (0..=MAX_PLY)
            .map(|ply| Vec::with_capacity((MAX_PLY - ply) as usize))
            .collect(),
        capture_ranks: CaptureRanks::new(nnue),
        move_pickers: (0..=MAX_PLY)
            .map(|_| MovePicker::new(None, [None; KILLER_COUNT]))
            .collect(),
        qsearch: (0..=MAX_PLY).map(|_| QsearchBuffers::default()).collect(),
        accumulators: vec![root_accumulator; MAX_PLY as usize + 1],
        material_keys: [material_key(position); MAX_PLY as usize + 1],
        static_evals: [0; MAX_PLY as usize + 1],
        correction: CorrectionTable::new(nnue.pawn_value()),
        delta_margin: nnue.pawn_value() * params::delta_margin() / 100,
        history,
        capture_history,
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

/// 探索経路上の1着手。再現を検出したときの攻撃側の判定に使う。
pub(super) struct PathMove {
    /// 実着手。null moveは`None`。
    pub(super) mv: Option<Move>,
    /// 再現時に初めて計算する攻撃的着手の判定結果。着手を取り消すときに捨てる。
    pub(super) attacking: Option<bool>,
}

/// 再現した局面に最も近い、同一局面の出現位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RepetitionMatch {
    /// 探索経路の局面キーの添字。
    Path(usize),
    /// 対局履歴の局面キーの添字。
    History(usize),
}

/// 1回の探索実行の可変状態。
pub(super) struct Searcher<'a> {
    /// 対象の探索の主ワーカーだけが保持する途中結果の採用材料。
    pub(super) root_results: Option<RootResults>,
    /// 直前に完了した反復の、根の手ごとの部分木ノード数。
    pub(super) completed_root_nodes: Vec<(Move, u64)>,
    /// 進行中の反復で、窓の読み直しを含めて累積する部分木ノード数。
    pub(super) current_root_nodes: Vec<(Move, u64)>,
    /// 「途中結果の採用」の停止契約を実着手の境界で再現する検査用の停止点。
    #[cfg(test)]
    pub(super) interrupt_at: Option<(u64, StopReason)>,
    /// このワーカーだけが更新する探索統計。
    #[cfg(feature = "search-stats")]
    pub(super) stats: crate::search::SearchStats,
    /// 探索中に使う検証済み学習NNUE。
    pub(super) nnue: &'a Nnue,
    /// 探索内の着手適用に使う規則。
    pub(super) rules: MoveRules,
    /// 採用中の反復規則と、規則が要する根の時点の状態。
    pub(super) repetition: SearchRepetition,
    /// 根の手番側から見た、R1の引き分けの再現に返す値。
    pub(super) draw_value: i32,
    /// 攻撃的着手の判定で探索経路を再生するための、根の局面の写し。
    pub(super) root_position: Position,
    /// 探索ノードでの合法手生成器。
    pub(super) generator: MoveGenerator,
    /// 対局開始から現局面までの反復規則のキー。R2・R3では成り権保留状態を含めない。
    pub(super) history_keys: &'a [u64],
    /// 探索経路上の反復規則のキー。R2・R3では成り権保留状態を含めない。
    pub(super) path_keys: Vec<u64>,
    /// `path_keys[i + 1]`の局面へ至る着手を`i`番目に持つスタック。
    /// 根は着手を持たないので、長さは`path_keys.len() - 1`となる。
    pub(super) path_moves: Vec<PathMove>,
    /// null moveで到達した直後のノードのply。
    pub(super) null_move_ply: Option<u32>,
    /// 直前の捕獲手の到達升。居喰いでも捕獲した駒の最終位置を使う。
    /// 根とnull moveの直後は取り返しの対象を持たない。
    pub(super) previous_capture: [Option<Square>; MAX_PLY as usize + 1],
    /// 直近のnull move直後の局面キーが探索経路に占める位置。
    /// 部分木内の反復判定はこの境界以降だけを参照する。
    pub(super) null_move_boundary: Option<usize>,
    /// このワーカーが[`Self::enter_node`]で数えた実着手の適用回数。
    pub(super) nodes: u64,
    /// 探索チームで共有する停止状態と予算。
    pub(super) shared: &'a SharedSearch<'a>,
    /// 中断時に記録する停止条件。
    pub(super) stop_reason: Option<StopReason>,
    /// 先読みで始めた主ワーカーだけが記録する反復。
    pub(super) ponder_iteration: Option<PonderIteration>,
    /// plyごとの主変化。行plyは、その深さ以降の最善応手列を保持する。
    pub(super) pv: Vec<Vec<Move>>,
    /// plyごとのttPv。PVノードまたは照合した印で立て、α以下なら親の印も継ぐ。
    pub(super) tt_pv: [bool; MAX_PLY as usize + 1],
    /// 静止探索の捕獲生成・整列用バッファをplyごとに再利用する。
    pub(super) qsearch: Vec<QsearchBuffers>,
    /// 通常探索の候補と整列領域をplyごとに再利用する。
    pub(super) move_pickers: Vec<MovePicker>,
    /// ワーカー内で共有する捕獲価値の順位。
    pub(super) capture_ranks: CaptureRanks,
    /// plyごとのNNUE累算値。
    pub(super) accumulators: Vec<NnueAccumulator>,
    /// plyごとの駒種別枚数のハッシュ。null moveでは変化しない。
    pub(super) material_keys: [u64; MAX_PLY as usize + 1],
    /// plyごとの補正後の静的評価。通常探索と根で子を読む前に書き込む。
    pub(super) static_evals: [i32; MAX_PLY as usize + 1],
    /// 反復深化の間で共有する、このワーカー専用の補正表。
    pub(super) correction: CorrectionTable,
    /// 歩兵価値×`DeltaMargin`%で求める、静的評価と同じ単位のdelta pruning余裕値。
    /// `stand_pat + 取る駒の価値 + 余裕値 ≤ α` の捕獲手を飛ばすために使う。
    pub(super) delta_margin: i32,
    /// βカットを起こした非捕獲手の手番側・移動元・移動先別スコア。
    pub(super) history: &'a mut HistoryTable,
    /// ワーカー番号ごとに探索間で持ち越す捕獲履歴。
    pub(super) capture_history: &'a mut CaptureHistory,
    /// βカットを起こした非捕獲手をplyごとに新しい順で保持する表。
    pub(super) killers: KillerTable,
    /// 置換表。
    pub(super) tt: &'a TranspositionTable,
}

impl Searcher<'_> {
    /// R2またはR3の下で、この局面からの着手が禁止手（RULES.md第31条R2・R3、第26条第11項）かを返す。R1では常に`false`。
    ///
    /// 着手を適用して反復規則のキーを求め、R2では対局履歴または探索経路に1回でも現れていれば、
    /// R3では対局履歴と探索経路を通じた出現回数が既に3回なら禁止手とする。根の局面は履歴の末尾と経路の先頭の両方にあるので二重に数えない。
    /// null moveの部分木では、直近のnull move直後以降の探索経路だけを照合し、対局履歴とそれより前の経路は照合しない。
    pub(super) fn move_is_forbidden(&self, position: &mut Position, mv: Move) -> bool {
        let limit = match self.repetition {
            SearchRepetition::R1 { .. } => return false,
            SearchRepetition::R2 => 1,
            SearchRepetition::R3 => 3,
        };
        let undo = position.make_move_unchecked(mv, self.rules);
        let key = repetition_key(self.repetition.rule(), position);
        position.unmake_move(undo);

        let (history, path) = if let Some(boundary) = self.null_move_boundary {
            (&[][..], &self.path_keys[boundary..])
        } else {
            // 履歴があるときは末尾の根を除き、経路の先頭で1回だけ数える。
            let history = &self.history_keys[..self.history_keys.len().saturating_sub(1)];
            (history, self.path_keys.as_slice())
        };
        history
            .iter()
            .chain(path)
            .filter(|&&past| past == key)
            .take(limit)
            .count()
            == limit
    }

    /// 同一局面の最も近い出現を、経路、対局履歴の順に新しい方から探す。
    /// null moveの部分木では、直近のnull move直後の局面以降だけを照合する。
    ///
    /// 呼び出しの時点、反復時の返り値、置換表への保存範囲は[`Self::search_move`]に従う。
    /// null moveをまたぐ反復を除く理由は`docs/adr/detect-repetition-only-after-latest-null-move.md`にある。
    pub(super) fn find_repetition(&self, key: u64) -> Option<RepetitionMatch> {
        if let Some(boundary) = self.null_move_boundary {
            return self.path_keys[boundary..]
                .iter()
                .rposition(|&past| past == key)
                .map(|index| RepetitionMatch::Path(boundary + index));
        }
        self.path_keys
            .iter()
            .rposition(|&past| past == key)
            .map(RepetitionMatch::Path)
            .or_else(|| {
                self.history_keys
                    .iter()
                    .rposition(|&past| past == key)
                    .map(RepetitionMatch::History)
            })
    }

    /// 第31条R1の攻撃側を最も近い出現から現在の着手までの1周で判定する。
    ///
    /// 一方の着手だけがすべて攻撃的なら、その側の負けを専用の帯で返す。
    /// 双方が該当する場合とどちらも該当しない場合は、引き分けの値を返す。
    /// 現在の着手側が根の手番と同じなら`draw_value`、異なれば`-draw_value`とする。
    /// R2・R3の再現には、周回の損得を表す近似値として設定にかかわらず`DRAW_SCORE`を返す。
    /// 可逆手12手の条件と4回目の出現は調べない探索上の近似であり、値は現在の着手側から見る。
    /// 履歴の部分は根の攻撃連続数で判定し、経路の部分は根から再生して結果を着手ごとに記録する。
    /// null moveは区間より前の局面の再生にだけ使い、双方の非攻撃的着手が見つかれば打ち切る。
    pub(super) fn repetition_score(
        &mut self,
        repeated: RepetitionMatch,
        position: &Position,
        mv: Move,
        ply: u32,
    ) -> i32 {
        let SearchRepetition::R1 {
            consecutive_attacking_moves,
        } = self.repetition
        else {
            return DRAW_SCORE;
        };
        debug_assert_eq!(self.path_moves.len() + 1, self.path_keys.len());
        let mut attackers = [true; 2];
        let start = match repeated {
            RepetitionMatch::Path(index) => index,
            RepetitionMatch::History(index) => {
                let plies = self.history_keys.len() - 1 - index;
                let last_mover = self.root_position.side_to_move().opposite();
                for color in Color::ALL {
                    let moves = plies / 2 + usize::from(plies % 2 == 1 && color == last_mover);
                    attackers[color.index()] =
                        moves <= consecutive_attacking_moves[color.index()] as usize;
                }
                0
            }
        };
        debug_assert!(
            self.path_moves[start..]
                .iter()
                .all(|entry| entry.mv.is_some())
        );
        if attackers == [false; 2] {
            return self.r1_draw_score(position.side_to_move().opposite());
        }

        let mut replay = self.root_position.clone();
        for (index, entry) in self.path_moves.iter_mut().enumerate() {
            let mover = replay.side_to_move();
            if let Some(played) = entry.mv {
                replay.make_move_unchecked(played, self.rules);
                if index >= start {
                    let attacking = *entry.attacking.get_or_insert_with(|| {
                        move_was_attacking(&replay, &self.generator, mover, played)
                    });
                    attackers[mover.index()] &= attacking;
                    if attackers == [false; 2] {
                        return self.r1_draw_score(position.side_to_move().opposite());
                    }
                }
            } else {
                replay.make_null_move();
            }
        }

        let mover = position.side_to_move().opposite();
        attackers[mover.index()] &= move_was_attacking(position, &self.generator, mover, mv);
        match (
            attackers[mover.index()],
            attackers[mover.opposite().index()],
        ) {
            (true, false) => -(REPETITION_WIN - (ply + 1) as i32),
            (false, true) => REPETITION_WIN - (ply + 1) as i32,
            (true, true) | (false, false) => self.r1_draw_score(mover),
        }
    }

    /// RULES.md第31条R1の引き分けの値を、現在の着手側から見た符号で返す。
    fn r1_draw_score(&self, mover: Color) -> i32 {
        if mover == self.root_position.side_to_move() {
            self.draw_value
        } else {
            -self.draw_value
        }
    }

    /// 実着手の適用直前に停止条件を検査し、続行可能なら共有カウンタと局所カウンタに1ノードを数える。
    ///
    /// [`Self::search_move`]と[`Self::quiesce`]の着手適用直前だけで呼ぶ。
    /// 数える対象と例外は[`crate::search::SearchResult::nodes`]に従う。
    /// 読み直しで同じ子局面の探索関数を呼ぶだけでは加算しない。
    /// 定義の理由は`docs/adr/count-nodes-per-move-on-shared-counter.md`にある。
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

    /// 先読みの的中時刻を先に読み、hard limitを反復の完了時刻予測より先に検査する。
    ///
    /// 的中前は時間で止めず、的中後は的中からの経過時間でhard limitを判定する。
    /// 先読みから続く主ワーカーの反復は、最初の時計検査で[`iteration_prediction_fits`]を当て直す。
    /// 秒読み期ではこの当て直しを行わず、hard limitだけで中断する。
    pub(super) fn check_time(&mut self) -> bool {
        let Some(limit) = self.shared.hard_limit else {
            return true;
        };
        let hit_ns = limit.hit_ns.load(AtomicOrdering::Relaxed);
        if hit_ns == u64::MAX {
            return true;
        }
        let hit = Duration::from_nanos(hit_ns);
        let elapsed = self.shared.clock.elapsed();
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
