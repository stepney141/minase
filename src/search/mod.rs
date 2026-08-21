//! 評価関数と静止探索を使って着手を選ぶ探索。

#[cfg(test)]
mod tests;
mod tt;

pub use tt::{DEFAULT_SIZE_MB as DEFAULT_TT_SIZE_MB, TranspositionTable};

use core::cmp::Reverse;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::{COLOR_COUNT, PieceKind};
use crate::core::position::Position;
use crate::core::rules::Rules;
use crate::core::square::BOARD_SQUARE_COUNT;
use crate::eval::{evaluate, piece_value};

use tt::Bound;

/// 詰みを表す評価値。
pub const MATE: i32 = 30_000;
/// 探索が扱う最大ply。
pub const MAX_PLY: u32 = 256;
/// 詰み手数を含む評価値の下限。
pub const MATE_THRESHOLD: i32 = MATE - MAX_PLY as i32;
/// 引き分けを表す評価値。
pub const DRAW_SCORE: i32 = 0;

/// 探索窓の初期値。全評価値より大きい。
const INFINITY: i32 = MATE + 1;
/// 停止要求と時間切れを検査するノード数間隔。
const STOP_CHECK_INTERVAL: u64 = 4096;
/// History値を全体の半減で抑える上限。
const HISTORY_LIMIT: i32 = 1 << 14;
/// 1つのplyに記録するkiller手の数。
const KILLER_COUNT: usize = 2;
/// 静止探索で小さな捕獲を残すための余裕値。
const DELTA_MARGIN: i32 = 200;

/// 手番側・移動元・移動先で参照するhistory表。
type HistoryTable = [[[i32; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT];
/// plyごとに新しい順で保持するkiller表。
type KillerTable = [[Option<Move>; KILLER_COUNT]; MAX_PLY as usize + 1];

/// 1回の探索に適用する制限。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SearchLimits {
    /// 反復深化で完了を目指す最大深さ。`None`なら深さで制限しない。
    pub depth: Option<u32>,
    /// 探索するノード数の上限。`None`なら上限を設けない。
    pub nodes: Option<u64>,
    /// 1手に使う固定時間(ms)。
    pub movetime_ms: Option<u64>,
    /// 持ち時間、加算時間、秒読みによる制限。
    pub clock: Option<ClockLimits>,
    /// 外部停止要求だけを停止条件とするか。
    pub infinite: bool,
}

impl SearchLimits {
    /// 制限が探索可能な組合せか検査する。
    ///
    /// # Errors
    ///
    /// 制約が1つもない場合、または深さが1未満か[`MAX_PLY`]を超える場合は
    /// エラーメッセージを返す。
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.infinite
            && self.depth.is_none()
            && self.nodes.is_none()
            && self.movetime_ms.is_none()
            && self.clock.is_none()
        {
            return Err("search limits must contain at least one constraint");
        }
        if self
            .depth
            .is_some_and(|depth| depth == 0 || depth > MAX_PLY)
        {
            return Err("search depth must be between one and MAX_PLY");
        }
        Ok(())
    }
}

/// 持ち時間から1手の予算を求めるための制限。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ClockLimits {
    /// 手番開始時の残り時間(ms)。
    pub remaining_ms: u64,
    /// 1手ごとの加算時間(ms)。
    pub increment_ms: u64,
    /// 1手ごとの秒読み時間(ms)。
    pub byoyomi_ms: u64,
}

/// 探索スレッドへ渡す不変の入力。
#[derive(Clone)]
pub struct SearchSnapshot {
    /// 探索を開始する局面。
    pub position: Position,
    /// 探索内で着手へ適用する規則。
    pub rules: Rules,
    /// 対局開始から現局面までの探索局面キー。
    pub history_keys: Vec<u64>,
    /// 対局管理層が確定したルート合法手。
    pub root_moves: Vec<Move>,
}

/// 探索を停止した条件。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StopReason {
    /// 指定深さを完了した。
    DepthCompleted,
    /// 指定ノード数へ達した。
    NodeLimit,
    /// 完了イテレーションの境界でsoft limitへ達した。
    SoftLimit,
    /// 探索中にhard limitへ達した。
    HardLimit,
    /// 呼び出し側から停止を要求された。
    ExternalStop,
}

/// 探索スレッドから届く進捗または完了通知。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SearchEvent {
    /// 反復深化の1イテレーションが完了した。
    Progress {
        /// 通知元の探索ID。
        search_id: u64,
        /// 完了した深さ。
        depth: u32,
        /// その深さでの評価値。
        score: i32,
        /// 探索開始から訪問したノード数。
        nodes: u64,
        /// 探索開始からの経過時間。
        elapsed: Duration,
        /// その深さでの主変化。
        pv: Vec<Move>,
    },
    /// 探索が停止した。
    Finished {
        /// 通知元の探索ID。
        search_id: u64,
        /// 選んだ着手。
        best_move: Move,
        /// 最後まで完了した深さの評価値。
        score: i32,
        /// 最後まで完了した深さ。
        depth: u32,
        /// 探索開始から訪問したノード数。
        nodes: u64,
        /// 探索開始からの経過時間。
        elapsed: Duration,
        /// 最後まで完了した深さの主変化。
        pv: Vec<Move>,
        /// 探索を停止した条件。
        stop_reason: StopReason,
    },
}

impl SearchEvent {
    /// 通知元の探索IDを返す。
    pub fn search_id(&self) -> u64 {
        match self {
            Self::Progress { search_id, .. } | Self::Finished { search_id, .. } => *search_id,
        }
    }
}

/// 実行中の探索スレッドを操作するハンドル。
pub struct SearchHandle {
    /// 探索イベントの受信端。
    events: mpsc::Receiver<SearchEvent>,
    /// 探索スレッドと共有する停止フラグ。
    stop: Arc<AtomicBool>,
    /// 置換表を返して終了する探索スレッドのハンドル。
    thread: thread::JoinHandle<TranspositionTable>,
}

impl SearchHandle {
    /// 探索イベントの受信端を返す。
    pub fn events(&self) -> &mpsc::Receiver<SearchEvent> {
        &self.events
    }

    /// 探索スレッドへ停止を要求する。
    pub fn request_stop(&self) {
        self.stop.store(true, AtomicOrdering::Relaxed);
    }

    /// 探索スレッドの終了を待ち、所有していた置換表を返す。
    pub fn join(self) -> thread::Result<TranspositionTable> {
        self.thread.join()
    }
}

/// 完了した探索の結果。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SearchResult {
    /// 選んだ着手。
    pub best_move: Move,
    /// 選んだ着手の評価値。
    pub score: i32,
    /// 最後まで完了した反復深化の深さ。
    pub depth: u32,
    /// 探索開始から訪問したノード数。
    pub nodes: u64,
}

/// 所有権を移した入力と置換表を使い、別スレッドで探索を開始する。
///
/// # Panics
///
/// `snapshot.root_moves`が空、または`limits`が不正な場合はpanicする。
pub fn start_search(
    snapshot: SearchSnapshot,
    limits: SearchLimits,
    search_id: u64,
    tt: TranspositionTable,
) -> SearchHandle {
    validate_input(&snapshot.root_moves, &limits);
    let (sender, events) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = thread::spawn(move || {
        let outcome = run_search(
            &snapshot.position,
            snapshot.rules,
            &snapshot.root_moves,
            &snapshot.history_keys,
            &limits,
            &thread_stop,
            &tt,
            Some((&sender, search_id)),
        );
        let result = outcome.result;
        let _ = sender.send(SearchEvent::Finished {
            search_id,
            best_move: result.best_move,
            score: result.score,
            depth: result.depth,
            nodes: result.nodes,
            elapsed: outcome.elapsed,
            pv: outcome.pv,
            stop_reason: outcome.stop_reason,
        });
        tt
    });
    SearchHandle {
        events,
        stop,
        thread,
    }
}

/// 指定局面を呼び出しスレッド上で反復深化探索する。
///
/// `root_moves`には、対局管理層がR2・R3を含めて検査したルート合法手を
/// 渡す。`history_keys`の各要素は、対局開始から現局面までの
/// `Position::zobrist() ^ Position::rights_zobrist()`でなければならない。
///
/// ノード上限によって反復深化が中断された場合は、直前に完了した深さの
/// 結果を返す。深さ1の完了前に中断された場合も、`root_moves`の先頭を返す。
///
/// # Panics
///
/// `root_moves`が空、`limits`が不正、または外部停止手段を持たない同期版へ
/// `infinite`を指定した場合はpanicする。
pub fn search(
    position: &Position,
    rules: Rules,
    root_moves: &[Move],
    history_keys: &[u64],
    limits: &SearchLimits,
    tt: &mut TranspositionTable,
) -> SearchResult {
    validate_input(root_moves, limits);
    assert!(
        !limits.infinite,
        "synchronous search cannot use an infinite limit"
    );
    let stop = AtomicBool::new(false);
    run_search(
        position,
        rules,
        root_moves,
        history_keys,
        limits,
        &stop,
        tt,
        None,
    )
    .result
}

/// ルート合法手と制限の妥当性を検査する。
fn validate_input(root_moves: &[Move], limits: &SearchLimits) {
    assert!(!root_moves.is_empty(), "root move list must not be empty");
    assert!(limits.validate().is_ok(), "invalid search limits");
}

/// 探索の内部実行が返す結果一式。
struct SearchOutcome {
    /// 完了した探索の結果。
    result: SearchResult,
    /// 探索開始からの経過時間。
    elapsed: Duration,
    /// 最後まで完了した深さの主変化。
    pv: Vec<Move>,
    /// 探索を停止した条件。
    stop_reason: StopReason,
}

/// 反復深化のループを実行し、深さ完了ごとに進捗イベントを送る。
///
/// soft limitの判定は完了イテレーションの境界だけで行い、探索途中の
/// 打ち切りはhard limitと停止要求が担う。
#[allow(clippy::too_many_arguments)]
fn run_search(
    position: &Position,
    rules: Rules,
    root_moves: &[Move],
    history_keys: &[u64],
    limits: &SearchLimits,
    stop: &AtomicBool,
    tt: &TranspositionTable,
    events: Option<(&mpsc::Sender<SearchEvent>, u64)>,
) -> SearchOutcome {
    let started = Instant::now();
    let time_budget = time_budget(limits);
    let depth_limit = if limits.infinite {
        MAX_PLY
    } else {
        limits.depth.unwrap_or(MAX_PLY)
    };
    let node_limit = (!limits.infinite).then_some(limits.nodes).flatten();

    tt.new_search();
    let mut searcher = Searcher {
        rules,
        generator: MoveGenerator::new(rules),
        history_keys,
        path_keys: vec![search_key(position)],
        null_move_ply: None,
        nodes: 0,
        node_limit,
        started,
        hard_limit: time_budget.map(|budget| budget.hard),
        stop,
        stop_reason: None,
        pv: (0..=MAX_PLY)
            .map(|ply| Vec::with_capacity((MAX_PLY - ply) as usize))
            .collect(),
        history: Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]),
        killers: [[None; KILLER_COUNT]; MAX_PLY as usize + 1],
        tt,
    };
    let mut result = SearchResult {
        best_move: root_moves[0],
        score: evaluate(position),
        depth: 0,
        nodes: 0,
    };
    let mut completed_pv = vec![root_moves[0]];
    let mut stop_reason = StopReason::DepthCompleted;

    for depth in 1..=depth_limit {
        let Some((best_move, score)) = searcher.search_root(position, root_moves, depth) else {
            stop_reason = searcher
                .stop_reason
                .expect("interrupted search must record a stop reason");
            break;
        };
        result.best_move = best_move;
        result.score = score;
        result.depth = depth;
        completed_pv.clone_from(&searcher.pv[0]);
        let elapsed = started.elapsed();
        if let Some((sender, search_id)) = events {
            let _ = sender.send(SearchEvent::Progress {
                search_id,
                depth,
                score,
                nodes: searcher.nodes,
                elapsed,
                pv: completed_pv.clone(),
            });
        }
        if node_limit.is_some_and(|limit| searcher.nodes >= limit) {
            stop_reason = StopReason::NodeLimit;
            break;
        }
        if time_budget.is_some_and(|budget| elapsed >= budget.soft) {
            stop_reason = StopReason::SoftLimit;
            break;
        }
    }
    result.nodes = searcher.nodes;
    SearchOutcome {
        result,
        elapsed: started.elapsed(),
        pv: completed_pv,
        stop_reason,
    }
}

/// 1回の探索実行の可変状態。
struct Searcher<'a> {
    /// 探索内の着手適用に使う規則。
    rules: Rules,
    /// 探索ノードでの合法手生成器。
    generator: MoveGenerator,
    /// 対局開始から現局面までの探索局面キー。反復の検出に使う。
    history_keys: &'a [u64],
    /// 探索経路上の局面キー。探索内の反復の検出に使う。
    path_keys: Vec<u64>,
    /// null moveで到達した直後のノードのply。
    null_move_ply: Option<u32>,
    /// 訪問したノード数。
    nodes: u64,
    /// ノード数の上限。
    node_limit: Option<u64>,
    /// 探索の開始時刻。
    started: Instant,
    /// 探索途中でも打ち切る時間制限。
    hard_limit: Option<Duration>,
    /// 外部からの停止要求フラグ。
    stop: &'a AtomicBool,
    /// 中断時に記録する停止条件。
    stop_reason: Option<StopReason>,
    /// plyごとの主変化。行plyは、その深さ以降の最善応手列を保持する。
    pv: Vec<Vec<Move>>,
    /// βカットを起こした非捕獲手の手番側・移動元・移動先別スコア。
    history: Box<HistoryTable>,
    /// βカットを起こした非捕獲手をplyごとに新しい順で保持する表。
    killers: KillerTable,
    /// 置換表。
    tt: &'a TranspositionTable,
}

impl Searcher<'_> {
    /// ルート局面を指定深さで探索し、最善手と評価値を返す。
    ///
    /// 中断された場合は`None`を返し、停止条件を記録する。
    fn search_root(
        &mut self,
        position: &Position,
        root_moves: &[Move],
        depth: u32,
    ) -> Option<(Move, i32)> {
        if !self.enter_node() {
            return None;
        }
        self.pv[0].clear();

        let mut position = position.clone();
        let mut moves = root_moves.to_vec();
        let key = search_key(&position);
        let tt_move = self.tt.probe(key, 0).map(|hit| hit.best_move);
        self.order_moves(&position, &mut moves, tt_move, 0);
        let mut alpha = -INFINITY;
        let beta = INFINITY;
        let mut best_move = moves[0];
        let mut best_score = -INFINITY;

        for (index, mv) in moves.into_iter().enumerate() {
            let score =
                self.search_move(&mut position, mv, depth, alpha, beta, 0, index == 0, 0)?;
            if score > best_score {
                best_score = score;
                best_move = mv;
                self.update_pv(0, mv);
            }
            alpha = alpha.max(score);
        }
        self.tt
            .store(key, depth, best_score, Bound::Exact, best_move, 0);
        Some((best_move, best_score))
    }

    /// ネガマックス形式のアルファベータ探索で局面を評価する。
    ///
    /// 深さ0では静止探索へ移り、合法手のない局面は詰みとして
    /// `-MATE + ply`を返す。中断された場合は`None`を返す。
    fn negamax(
        &mut self,
        position: &mut Position,
        depth: u32,
        mut alpha: i32,
        beta: i32,
        ply: u32,
    ) -> Option<i32> {
        if !self.enter_node() {
            return None;
        }
        self.pv[ply as usize].clear();

        if depth == 0 {
            return self.quiesce(position, alpha, beta, ply);
        }

        let key = search_key(position);
        let original_alpha = alpha;
        let mut tt_move = None;
        // 深さが足りるヒットは即時カットオフだけに使い、探索窓は狭めない。
        // 窓を狭めると、格納時のバウンド分類が実際に探索した窓と食い違う。
        if let Some(hit) = self.tt.probe(key, ply) {
            tt_move = Some(hit.best_move);
            if u32::from(hit.depth) >= depth {
                let cutoff = match hit.bound {
                    Bound::Exact => true,
                    Bound::Lower => hit.score >= beta,
                    Bound::Upper => hit.score <= alpha,
                };
                if cutoff {
                    return Some(hit.score);
                }
            }
        }

        let side = position.side_to_move();
        let has_non_royal_piece =
            !(position.pieces_of(side) & !position.royal_pieces(side)).is_empty();
        if depth >= 3
            && self.null_move_ply != Some(ply)
            && beta.abs() < MATE_THRESHOLD
            && has_non_royal_piece
        {
            let reduction = 2 + depth / 6;
            let undo = position.make_null_move();
            let previous_null_move_ply = self.null_move_ply.replace(ply + 1);
            let score = self
                .negamax(
                    position,
                    depth.saturating_sub(1 + reduction),
                    -beta,
                    -beta + 1,
                    ply + 1,
                )
                .map(|value| -value);
            self.null_move_ply = previous_null_move_ply;
            position.unmake_null_move(undo);
            let score = score?;
            if score >= beta {
                return Some(if score.abs() >= MATE_THRESHOLD {
                    beta
                } else {
                    score
                });
            }
        }

        let mut moves = Vec::new();
        self.generator.generate_moves(position, &mut moves);
        if moves.is_empty() {
            return Some(-MATE + ply as i32);
        }

        self.order_moves(position, &mut moves, tt_move, ply);
        let mut best_move = moves[0];
        let mut best_score = -INFINITY;
        let mut beta_cutoff = false;
        for (index, mv) in moves.into_iter().enumerate() {
            let reduction = u32::from(
                depth >= 3
                    && index >= 3
                    && Some(mv) != tt_move
                    && move_order_key(position, mv).is_none()
                    && !self.killers[ply as usize][..].contains(&Some(mv)),
            );
            let score =
                self.search_move(position, mv, depth, alpha, beta, ply, index == 0, reduction)?;
            if score > best_score {
                best_score = score;
                best_move = mv;
                self.update_pv(ply, mv);
            }
            alpha = alpha.max(score);
            if alpha >= beta {
                beta_cutoff = true;
                if move_order_key(position, mv).is_none() {
                    self.record_quiet_beta_cutoff(position, mv, depth, ply);
                }
                break;
            }
        }
        let bound = if best_score <= original_alpha {
            Bound::Upper
        } else if beta_cutoff {
            Bound::Lower
        } else {
            Bound::Exact
        };
        self.tt.store(key, depth, best_score, bound, best_move, ply);
        Some(best_score)
    }

    /// stand-patと捕獲手だけを使う静止探索で局面を評価する。
    ///
    /// 静止探索内の手は主変化へ含めず、反復検出と置換表も使用しない。
    /// 中断された場合は`None`を返す。
    fn quiesce(
        &mut self,
        position: &mut Position,
        mut alpha: i32,
        beta: i32,
        ply: u32,
    ) -> Option<i32> {
        if !self.enter_node() {
            return None;
        }
        self.pv[ply as usize].clear();

        if ply >= MAX_PLY {
            return Some(evaluate(position));
        }

        let stand_pat = evaluate(position);
        if stand_pat >= beta {
            return Some(stand_pat);
        }
        let mut best = stand_pat;
        alpha = alpha.max(stand_pat);

        let mut captures = Vec::new();
        self.generator.generate_moves(position, &mut captures);
        let mut captures: Vec<_> = captures
            .into_iter()
            .filter_map(|mv| move_order_key(position, mv).map(|key| (mv, key)))
            .collect();
        order_captures(&mut captures);

        for (mv, key) in captures {
            let is_last_royal_capture = captures_last_royal(position, mv);
            if !is_last_royal_capture && stand_pat + key.captured_value + DELTA_MARGIN <= alpha {
                continue;
            }
            let score = if is_last_royal_capture {
                self.enter_node().then_some(MATE - ply as i32)?
            } else {
                let undo = position.make_move_unchecked(mv, self.rules);
                let score = self
                    .quiesce(position, -beta, -alpha, ply + 1)
                    .map(|value| -value);
                position.unmake_move(undo);
                score?
            };
            best = best.max(score);
            alpha = alpha.max(score);
            if alpha >= beta {
                break;
            }
        }
        Some(best)
    }

    /// 1手を適用して子局面を探索し、この局面から見た評価値を返す。
    ///
    /// 王駒をすべて取る手は即詰みの値を返す。対局履歴または探索経路と
    /// 同一の局面は引き分け値とする。2手目以降は零窓で探索する。減深した
    /// 探索がαを超えた場合は通常深さの零窓、さらに窓内なら全窓で再探索する。
    #[allow(clippy::too_many_arguments)]
    fn search_move(
        &mut self,
        position: &mut Position,
        mv: Move,
        depth: u32,
        alpha: i32,
        beta: i32,
        ply: u32,
        first: bool,
        reduction: u32,
    ) -> Option<i32> {
        self.pv[(ply + 1) as usize].clear();
        if captures_last_royal(position, mv) {
            return self.enter_node().then_some(MATE - ply as i32);
        }

        let undo = position.make_move_unchecked(mv, self.rules);
        let key = search_key(position);
        let repeated = self.history_keys.contains(&key) || self.path_keys.contains(&key);
        if repeated {
            let score = self.enter_node().then_some(DRAW_SCORE);
            position.unmake_move(undo);
            return score;
        }

        self.path_keys.push(key);
        let mut score = if first {
            self.negamax(position, depth - 1, -beta, -alpha, ply + 1)
                .map(|value| -value)
        } else {
            self.negamax(position, depth - 1 - reduction, -alpha - 1, -alpha, ply + 1)
                .map(|value| -value)
        };
        if !first && reduction > 0 && score.is_some_and(|value| value > alpha) {
            score = self
                .negamax(position, depth - 1, -alpha - 1, -alpha, ply + 1)
                .map(|value| -value);
        }
        if !first && score.is_some_and(|value| value > alpha && value < beta) {
            score = self
                .negamax(position, depth - 1, -beta, -alpha, ply + 1)
                .map(|value| -value);
        }
        self.path_keys.pop();
        position.unmake_move(undo);
        score
    }

    /// ノードへ入る前に停止条件を検査し、続行可能ならノード数を数える。
    fn enter_node(&mut self) -> bool {
        if self.node_limit.is_some_and(|limit| self.nodes >= limit) {
            self.stop_reason = Some(StopReason::NodeLimit);
            return false;
        }
        if self.nodes.is_multiple_of(STOP_CHECK_INTERVAL) {
            if self.stop.load(AtomicOrdering::Relaxed) {
                self.stop_reason = Some(StopReason::ExternalStop);
                return false;
            }
            if self
                .hard_limit
                .is_some_and(|limit| self.started.elapsed() >= limit)
            {
                self.stop_reason = Some(StopReason::HardLimit);
                return false;
            }
        }
        self.nodes += 1;
        true
    }

    /// 指定plyの主変化を、この手と子plyの主変化の連結で置き換える。
    fn update_pv(&mut self, ply: u32, mv: Move) {
        let index = ply as usize;
        let (rows, child_rows) = self.pv.split_at_mut(index + 1);
        let row = &mut rows[index];
        row.clear();
        row.push(mv);
        if let Some(child) = child_rows.first() {
            row.extend_from_slice(child);
        }
    }

    /// 捕獲手、killer手、history値の順で着手を整列し、置換表の手を先頭へ置く。
    fn order_moves(
        &self,
        position: &Position,
        moves: &mut [Move],
        tt_move: Option<Move>,
        ply: u32,
    ) {
        let killers = self.killers[ply as usize];
        let color = position.side_to_move().index();
        moves.sort_by_cached_key(|&mv| {
            if let Some(key) = move_order_key(position, mv) {
                OrderedMoveKey::Capture {
                    captured_value: Reverse(key.captured_value),
                    attacker_value: key.attacker_value,
                }
            } else if Some(mv) == killers[0] {
                OrderedMoveKey::Killer(0)
            } else if Some(mv) == killers[1] {
                OrderedMoveKey::Killer(1)
            } else {
                OrderedMoveKey::Quiet(Reverse(
                    self.history[color][mv.from.dense_index()][mv.to.dense_index()],
                ))
            }
        });
        if let Some(index) = tt_move.and_then(|tt_move| moves.iter().position(|&mv| mv == tt_move))
        {
            moves.swap(0, index);
        }
    }

    /// βカットを起こした非捕獲手をkiller表とhistory表へ記録する。
    fn record_quiet_beta_cutoff(&mut self, position: &Position, mv: Move, depth: u32, ply: u32) {
        let killers = &mut self.killers[ply as usize];
        if killers[0] != Some(mv) {
            killers[1] = killers[0];
            killers[0] = Some(mv);
        }

        let color = position.side_to_move().index();
        let from = mv.from.dense_index();
        let to = mv.to.dense_index();
        self.history[color][from][to] += (depth * depth) as i32;
        if self.history[color][from][to] > HISTORY_LIMIT {
            for color_history in self.history.iter_mut() {
                for from_history in color_history.iter_mut() {
                    for value in from_history.iter_mut() {
                        *value /= 2;
                    }
                }
            }
        }
    }
}

/// 1手に使う時間の予算。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct TimeBudget {
    /// 完了イテレーションの境界で停止する目安時間。
    soft: Duration,
    /// 探索途中でも打ち切る上限時間。
    hard: Duration,
}

/// 持ち時間制の初期予算式を1箇所に集約する。
///
/// softは`remaining / 50 + increment * 0.7 + byoyomi * 0.8`、hardは
/// `min(soft * 4, remaining / 4 + byoyomi * 0.8)`とする。hardは1ms以上とし、
/// 時計合計が30ms以下の場合を除いて`remaining + byoyomi - 30ms`を超えない。
/// 係数を変更する場合は自己対局で採否を判定する。
fn clock_budget(clock: ClockLimits) -> TimeBudget {
    let remaining = u128::from(clock.remaining_ms);
    let increment = u128::from(clock.increment_ms);
    let byoyomi = u128::from(clock.byoyomi_ms);
    let byoyomi_share = byoyomi * 8 / 10;
    let soft = remaining / 50 + increment * 7 / 10 + byoyomi_share;
    let raw_hard = (soft * 4).min(remaining / 4 + byoyomi_share);
    let safe_hard = remaining.saturating_add(byoyomi).saturating_sub(30).max(1);
    TimeBudget {
        soft: Duration::from_millis(to_u64_ms(soft)),
        hard: Duration::from_millis(to_u64_ms(raw_hard.max(1).min(safe_hard))),
    }
}

/// 探索制限から時間予算を求める。`movetime`と時計の併用時は小さい方を採る。
fn time_budget(limits: &SearchLimits) -> Option<TimeBudget> {
    if limits.infinite {
        return None;
    }
    let movetime = limits.movetime_ms.map(|milliseconds| TimeBudget {
        soft: Duration::from_millis(milliseconds),
        hard: Duration::from_millis(milliseconds),
    });
    match (movetime, limits.clock.map(clock_budget)) {
        (Some(fixed), Some(clock)) => Some(TimeBudget {
            soft: fixed.soft.min(clock.soft),
            hard: fixed.hard.min(clock.hard),
        }),
        (Some(budget), None) | (None, Some(budget)) => Some(budget),
        (None, None) => None,
    }
}

/// ミリ秒をu64へ飽和変換する。
fn to_u64_ms(milliseconds: u128) -> u64 {
    milliseconds.min(u128::from(u64::MAX)) as u64
}

/// 反復検出に使う探索局面キー(第24条第1項)を計算する。
fn search_key(position: &Position) -> u64 {
    position.zobrist() ^ position.rights_zobrist()
}

/// 着手が相手の残存王駒をすべて取るかを返す(第21条第1項)。
fn captures_last_royal(position: &Position, mv: Move) -> bool {
    let opponent = position.side_to_move().opposite();
    let royals = position.royal_pieces(opponent);
    let royal_count = royals.popcount();
    royal_count > 0
        && position
            .captured_squares(mv)
            .into_iter()
            .flatten()
            .filter(|&square| royals.contains(square))
            .count()
            == royal_count as usize
}

/// 捕獲手と整列キーのペアをMVV-LVA順で安定に整列する。
fn order_captures(captures: &mut [(Move, MoveOrderKey)]) {
    captures.sort_by_key(|&(_, key)| (Reverse(key.captured_value), key.attacker_value));
}

/// 通常探索で使う着手の整列キー。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum OrderedMoveKey {
    /// 捕獲価値の降順、攻撃駒価値の昇順で並べる捕獲手。
    Capture {
        /// 取る駒の駒価値の合計。
        captured_value: Reverse<i32>,
        /// 動かす駒の駒価値。
        attacker_value: i32,
    },
    /// 添字が小さいほど新しいkiller手。
    Killer(usize),
    /// History値の降順で並べる残りの非捕獲手。
    Quiet(Reverse<i32>),
}

/// 捕獲手の整列キー。
#[derive(Clone, Copy)]
struct MoveOrderKey {
    /// 取る駒の駒価値の合計。
    captured_value: i32,
    /// 動かす駒の駒価値。
    attacker_value: i32,
}

/// 捕獲手なら整列キーを返す。非捕獲手は`None`を返す。
fn move_order_key(position: &Position, mv: Move) -> Option<MoveOrderKey> {
    let captured_value: i32 = position
        .captured_squares(mv)
        .into_iter()
        .flatten()
        .map(|square| piece_value(piece_kind_at(position, square)))
        .sum();
    (captured_value > 0).then(|| MoveOrderKey {
        captured_value,
        attacker_value: piece_value(piece_kind_at(position, mv.from)),
    })
}

/// 指定升の駒種を返す。
///
/// # Panics
///
/// 升に駒がない場合にパニックする。
fn piece_kind_at(position: &Position, square: crate::Square) -> PieceKind {
    position
        .piece_at(square)
        .and_then(crate::PieceCode::kind)
        .expect("move ordering square must contain a piece")
}
