//! 評価関数と静止探索を使って着手を選ぶ探索。

#[cfg(test)]
mod tests;
mod tt;

pub use tt::{DEFAULT_SIZE_MB as DEFAULT_TT_SIZE_MB, TranspositionTable, TranspositionTableError};

use core::cmp::Reverse;
use core::fmt;
use core::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::MoveGenerator;
use crate::core::game::{Game, GameStatus};
use crate::core::mv::Move;
use crate::core::piece::{COLOR_COUNT, PieceKind};
use crate::core::position::Position;
use crate::core::rules::MoveRules;
use crate::core::square::BOARD_SQUARE_COUNT;
use crate::eval::Pst;
use crate::eval::handcrafted::piece_value;
use crate::eval::pst::PstAccumulator;

use tt::Bound;

/// 詰みを表す評価値。
pub const MATE: i32 = 30_000;
/// 探索が扱う最大ply。
pub const MAX_PLY: u32 = 256;
/// 詰み手数を含む評価値の下限。
pub const MATE_THRESHOLD: i32 = MATE - MAX_PLY as i32;
/// 引き分けを表す評価値。
pub const DRAW_SCORE: i32 = 0;
/// 探索に使う既定のワーカー数。
pub const DEFAULT_THREADS: NonZeroUsize = NonZeroUsize::new(1).unwrap();

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

/// 1回の有限探索に適用する制限。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct FiniteSearchLimits {
    /// 反復深化で完了を目指す最大深さ。
    depth: Option<NonZeroU32>,
    /// 探索するノード数の上限。
    nodes: Option<NonZeroU64>,
    /// 1手に使う固定時間(ms)。
    movetime_ms: Option<NonZeroU64>,
    /// 持ち時間、加算時間、秒読みによる制限。
    clock: Option<ClockLimits>,
}

/// 1回の探索に適用する検証済み制限。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SearchLimits {
    kind: SearchLimitKind,
}

/// 有限探索と無期限探索を排他的に表す。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SearchLimitKind {
    /// 1個以上の停止条件を持つ有限探索。
    Finite(FiniteSearchLimits),
    /// 外部停止要求だけを停止条件とする探索。
    Infinite,
}

impl SearchLimits {
    /// 有限探索の制限を検証して構築する。
    ///
    /// # Errors
    ///
    /// 制約が1つもない場合、深さが範囲外の場合、ノード数が0の場合、または
    /// 固定時間が0 msの場合は[`SearchError`]を返す。
    pub fn new(
        depth: Option<u32>,
        nodes: Option<u64>,
        movetime_ms: Option<u64>,
        clock: Option<ClockLimits>,
    ) -> Result<Self, SearchError> {
        if depth.is_none() && nodes.is_none() && movetime_ms.is_none() && clock.is_none() {
            return Err(SearchError::MissingLimit);
        }
        let depth = depth
            .map(|depth| {
                NonZeroU32::new(depth)
                    .filter(|depth| depth.get() <= MAX_PLY)
                    .ok_or(SearchError::InvalidDepth { depth })
            })
            .transpose()?;
        let nodes = nodes
            .map(|nodes| NonZeroU64::new(nodes).ok_or(SearchError::ZeroNodeLimit))
            .transpose()?;
        let movetime_ms = movetime_ms
            .map(|milliseconds| NonZeroU64::new(milliseconds).ok_or(SearchError::ZeroMoveTime))
            .transpose()?;
        Ok(Self {
            kind: SearchLimitKind::Finite(FiniteSearchLimits {
                depth,
                nodes,
                movetime_ms,
                clock,
            }),
        })
    }

    /// 外部停止要求だけで停止する無期限探索の制限を返す。
    pub const fn infinite() -> Self {
        Self {
            kind: SearchLimitKind::Infinite,
        }
    }

    /// 外部停止要求だけで停止する無期限探索かを返す。
    pub const fn is_infinite(self) -> bool {
        matches!(self.kind, SearchLimitKind::Infinite)
    }

    /// 有限探索の深さ上限を返す。
    pub const fn depth(self) -> Option<u32> {
        match self.kind {
            SearchLimitKind::Finite(limits) => match limits.depth {
                Some(depth) => Some(depth.get()),
                None => None,
            },
            SearchLimitKind::Infinite => None,
        }
    }

    /// 有限探索のノード数上限を返す。
    pub const fn nodes(self) -> Option<u64> {
        match self.kind {
            SearchLimitKind::Finite(limits) => match limits.nodes {
                Some(nodes) => Some(nodes.get()),
                None => None,
            },
            SearchLimitKind::Infinite => None,
        }
    }

    /// 有限探索の固定時間(ms)を返す。
    pub const fn movetime_ms(self) -> Option<u64> {
        match self.kind {
            SearchLimitKind::Finite(limits) => match limits.movetime_ms {
                Some(milliseconds) => Some(milliseconds.get()),
                None => None,
            },
            SearchLimitKind::Infinite => None,
        }
    }

    /// 有限探索の持ち時間制限を返す。
    pub const fn clock(self) -> Option<ClockLimits> {
        match self.kind {
            SearchLimitKind::Finite(limits) => limits.clock,
            SearchLimitKind::Infinite => None,
        }
    }

    /// 有限探索の制限を返す。
    fn finite(self) -> Option<FiniteSearchLimits> {
        match self.kind {
            SearchLimitKind::Finite(limits) => Some(limits),
            SearchLimitKind::Infinite => None,
        }
    }
}

/// 持ち時間から1手の予算を求めるための制限。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ClockLimits {
    /// 手番開始時の残り時間(ms)。
    remaining_ms: u64,
    /// 1手ごとの加算時間(ms)。
    increment_ms: u64,
    /// 1手ごとの秒読み時間(ms)。
    byoyomi_ms: u64,
    /// 開始局面から現局面までの手数。
    ply: u32,
}

impl ClockLimits {
    /// 持ち時間制の制限を検証して構築する。
    ///
    /// # Errors
    ///
    /// 残り時間、加算時間、秒読み時間がすべて0 msの場合は
    /// [`SearchError::EmptyClock`]を返す。
    pub const fn new(
        remaining_ms: u64,
        increment_ms: u64,
        byoyomi_ms: u64,
        ply: u32,
    ) -> Result<Self, SearchError> {
        if remaining_ms == 0 && increment_ms == 0 && byoyomi_ms == 0 {
            return Err(SearchError::EmptyClock);
        }
        Ok(Self {
            remaining_ms,
            increment_ms,
            byoyomi_ms,
            ply,
        })
    }

    /// 手番開始時の残り時間(ms)を返す。
    pub const fn remaining_ms(self) -> u64 {
        self.remaining_ms
    }

    /// 1手ごとの加算時間(ms)を返す。
    pub const fn increment_ms(self) -> u64 {
        self.increment_ms
    }

    /// 1手ごとの秒読み時間(ms)を返す。
    pub const fn byoyomi_ms(self) -> u64 {
        self.byoyomi_ms
    }

    /// 開始局面から現局面までの手数を返す。
    pub const fn ply(self) -> u32 {
        self.ply
    }
}

/// 探索スレッドへ渡す不変の入力。
#[derive(Clone)]
pub struct SearchSnapshot {
    /// 探索を開始する局面。
    position: Position,
    /// 探索内で着手へ適用する規則。
    rules: MoveRules,
    /// 対局開始から現局面までの探索局面キー。
    history_keys: Vec<u64>,
    /// 対局管理層が確定したルート合法手。
    root_moves: Vec<Move>,
}

impl SearchSnapshot {
    /// 継続中の対局から探索用の不変入力を構築する。
    ///
    /// # Errors
    ///
    /// 対局が終局済みの場合は[`SearchError::FinishedGame`]を返す。
    /// 継続中でもルート合法手がない場合は[`SearchError::NoLegalMoves`]を返す。
    pub fn from_game(game: &Game) -> Result<Self, SearchError> {
        if matches!(game.status(), GameStatus::Finished(_)) {
            return Err(SearchError::FinishedGame);
        }
        Self::from_parts(
            game.position().clone(),
            game.rules().moves,
            game.search_key_history().to_vec(),
            game.legal_moves(),
        )
    }

    /// 検証済みの対局管理層が確定した各入力からスナップショットを構築する。
    fn from_parts(
        position: Position,
        rules: MoveRules,
        history_keys: Vec<u64>,
        root_moves: Vec<Move>,
    ) -> Result<Self, SearchError> {
        if root_moves.is_empty() {
            return Err(SearchError::NoLegalMoves);
        }
        Ok(Self {
            position,
            rules,
            history_keys,
            root_moves,
        })
    }

    /// 探索を開始する局面を返す。
    pub const fn position(&self) -> &Position {
        &self.position
    }

    /// 探索内で着手へ適用する規則を返す。
    pub const fn rules(&self) -> MoveRules {
        self.rules
    }

    /// 対局開始から現局面までの探索局面キーを返す。
    pub fn history_keys(&self) -> &[u64] {
        &self.history_keys
    }

    /// 対局管理層が確定したルート合法手を返す。
    pub fn root_moves(&self) -> &[Move] {
        &self.root_moves
    }
}

/// 探索入力または探索器の初期化に失敗した理由。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SearchError {
    /// 有限探索に停止条件が指定されていない。
    MissingLimit,
    /// 探索深さが1以上[`MAX_PLY`]以下でない。
    InvalidDepth {
        /// 指定された探索深さ。
        depth: u32,
    },
    /// ノード数上限が0である。
    ZeroNodeLimit,
    /// 固定探索時間が0 msである。
    ZeroMoveTime,
    /// 持ち時間、加算時間、秒読み時間がすべて0 msである。
    EmptyClock,
    /// 探索できるルート合法手がない。
    NoLegalMoves,
    /// 終局済みの対局が指定された。
    FinishedGame,
    /// 同期探索に無期限探索が指定された。
    InfiniteSynchronousSearch,
}

impl fmt::Display for SearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingLimit => formatter.write_str("search requires at least one limit"),
            Self::InvalidDepth { depth } => write!(
                formatter,
                "search depth must be between 1 and {MAX_PLY}, got {depth}"
            ),
            Self::ZeroNodeLimit => formatter.write_str("search node limit must be non-zero"),
            Self::ZeroMoveTime => formatter.write_str("search movetime must be non-zero"),
            Self::EmptyClock => formatter.write_str("search clock must contain non-zero time"),
            Self::NoLegalMoves => formatter.write_str("search requires at least one legal move"),
            Self::FinishedGame => formatter.write_str("search requires an ongoing game"),
            Self::InfiniteSynchronousSearch => {
                formatter.write_str("synchronous search cannot use an infinite limit")
            }
        }
    }
}

impl std::error::Error for SearchError {}

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

/// 実行中の探索チームを操作するハンドル。
///
/// 値を破棄すると探索チームへ停止を要求し、全ワーカーを回収するまで待つ。
pub struct SearchHandle {
    /// 探索イベントの受信端。
    events: mpsc::Receiver<SearchEvent>,
    /// 探索チームと共有する外部停止フラグ。
    stop: Arc<AtomicBool>,
    /// 全ワーカーの終了後に置換表を返す調整役のハンドル。
    thread: Option<thread::JoinHandle<TranspositionTable>>,
}

impl SearchHandle {
    /// 探索イベントの受信端を返す。
    pub fn events(&self) -> &mpsc::Receiver<SearchEvent> {
        &self.events
    }

    /// 探索チーム全体へ停止を要求する。
    pub fn request_stop(&self) {
        self.stop.store(true, AtomicOrdering::Relaxed);
    }

    /// 探索チームへ停止を要求して全ワーカーの終了を待ち、共有していた
    /// 置換表を返す。
    ///
    /// 探索ワーカーがパニックした場合は、そのペイロードを`Err`で返す。
    /// この場合、探索イベントの[`SearchEvent::Finished`]は送信されない。
    pub fn join(mut self) -> thread::Result<TranspositionTable> {
        self.stop_and_join()
            .expect("a live SearchHandle must own its search thread")
    }

    /// 停止要求と探索スレッドの回収を1回だけ行う。
    fn stop_and_join(&mut self) -> Option<thread::Result<TranspositionTable>> {
        self.request_stop();
        self.thread.take().map(thread::JoinHandle::join)
    }
}

impl Drop for SearchHandle {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
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

/// 所有権を移した評価重み、入力および置換表を使い、別スレッドで探索チームを
/// 開始する。
///
/// 探索ワーカーがパニックした場合は残るワーカーへ停止を通知する。その後、
/// [`SearchHandle::join`]がパニックのペイロードを返し、
/// [`SearchEvent::Finished`]は送信されない。
pub fn start_search(
    pst: Arc<Pst>,
    snapshot: SearchSnapshot,
    limits: SearchLimits,
    search_id: u64,
    threads: NonZeroUsize,
    tt: TranspositionTable,
) -> SearchHandle {
    let (sender, events) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = thread::spawn(move || {
        let outcome = run_search_team(
            &pst,
            &snapshot.position,
            snapshot.rules,
            &snapshot.root_moves,
            &snapshot.history_keys,
            &limits,
            &thread_stop,
            threads,
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
        thread: Some(thread),
    }
}

/// 指定局面を呼び出しスレッド上で反復深化探索する。
///
/// ノード上限によって反復深化が中断された場合は、直前に完了した深さの
/// 結果を返す。深さ1の完了前に中断された場合も、スナップショットが保持する
/// ルート合法手の先頭を返す。
///
/// # Errors
///
/// 外部停止手段を持たない同期版へ無期限探索を指定した場合は
/// [`SearchError`]を返す。
pub fn search(
    pst: &Pst,
    snapshot: &SearchSnapshot,
    limits: &SearchLimits,
    threads: NonZeroUsize,
    tt: &mut TranspositionTable,
) -> Result<SearchResult, SearchError> {
    if limits.is_infinite() {
        return Err(SearchError::InfiniteSynchronousSearch);
    }
    let stop = AtomicBool::new(false);
    Ok(run_search_team(
        pst,
        &snapshot.position,
        snapshot.rules,
        &snapshot.root_moves,
        &snapshot.history_keys,
        limits,
        &stop,
        threads,
        tt,
        None,
    )
    .result)
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

/// 1ワーカーが最後まで完了した反復と実訪問ノード数。
#[derive(Clone, PartialEq, Eq, Debug)]
struct WorkerOutcome {
    /// 探索チーム内のワーカー番号。主ワーカーは0。
    worker_index: usize,
    /// 最後まで完了した反復の結果。
    result: SearchResult,
    /// 最後まで完了した反復の主変化。
    pv: Vec<Move>,
    /// このワーカーが実際に訪問したノード数。
    nodes: u64,
}

/// 探索チームで共有する停止状態と探索予算。
struct SharedSearch<'a> {
    /// 呼び出し側からの停止要求。
    external_stop: &'a AtomicBool,
    /// 探索チーム内部の停止要求。
    team_stop: AtomicBool,
    /// 優先順位を反映した停止理由。
    stop_reason: AtomicU8,
    /// 全ワーカーが訪問したノード数。
    total_nodes: AtomicU64,
    /// 探索チーム全体のノード数上限。
    node_limit: Option<u64>,
    /// 補助ワーカー生成前に記録した探索開始時刻。
    started: Instant,
    /// 探索途中でも打ち切る時間制限。
    hard_limit: Option<Duration>,
}

impl SharedSearch<'_> {
    /// 停止理由を優先度付きで共有状態へ合成し、チーム停止を要求する。
    fn stop(&self, reason: StopReason) {
        self.stop_reason
            .fetch_max(stop_reason_priority(reason), AtomicOrdering::Relaxed);
        self.team_stop.store(true, AtomicOrdering::Release);
    }

    /// 外部停止が成立していれば最優先の停止理由として記録する。
    fn observe_external_stop(&self) -> bool {
        if self.external_stop.load(AtomicOrdering::Relaxed) {
            self.stop(StopReason::ExternalStop);
            true
        } else {
            false
        }
    }

    /// 現在までに探索チームが訪問した総ノード数を返す。
    fn nodes(&self) -> u64 {
        self.total_nodes.load(AtomicOrdering::Relaxed)
    }

    /// 共有状態に記録された停止理由を返す。
    fn reason(&self) -> StopReason {
        stop_reason_from_priority(self.stop_reason.load(AtomicOrdering::Relaxed))
            .expect("a completed search team must record a stop reason")
    }

    /// ノードを1個予約する。上限を超える予約は拒否する。
    fn reserve_node(&self) -> bool {
        let Some(limit) = self.node_limit else {
            self.total_nodes.fetch_add(1, AtomicOrdering::Relaxed);
            return true;
        };
        let mut current = self.total_nodes.load(AtomicOrdering::Relaxed);
        loop {
            if current >= limit {
                self.stop(StopReason::NodeLimit);
                return false;
            }
            match self.total_nodes.compare_exchange_weak(
                current,
                current + 1,
                AtomicOrdering::Relaxed,
                AtomicOrdering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }
}

/// 停止理由の優先順位を原子値へ写す。
const fn stop_reason_priority(reason: StopReason) -> u8 {
    match reason {
        StopReason::DepthCompleted => 1,
        StopReason::SoftLimit => 2,
        StopReason::NodeLimit => 3,
        StopReason::HardLimit => 4,
        StopReason::ExternalStop => 5,
    }
}

/// 原子値から停止理由を復元する。
const fn stop_reason_from_priority(priority: u8) -> Option<StopReason> {
    match priority {
        1 => Some(StopReason::DepthCompleted),
        2 => Some(StopReason::SoftLimit),
        3 => Some(StopReason::NodeLimit),
        4 => Some(StopReason::HardLimit),
        5 => Some(StopReason::ExternalStop),
        _ => None,
    }
}

/// 調整役として補助ワーカーを生成し、主ワーカー探索と全joinを実行する。
#[allow(clippy::too_many_arguments)]
fn run_search_team(
    pst: &Pst,
    position: &Position,
    rules: MoveRules,
    root_moves: &[Move],
    history_keys: &[u64],
    limits: &SearchLimits,
    external_stop: &AtomicBool,
    threads: NonZeroUsize,
    tt: &TranspositionTable,
    events: Option<(&mpsc::Sender<SearchEvent>, u64)>,
) -> SearchOutcome {
    let started = Instant::now();
    let time_budget = time_budget(limits);
    let finite_limits = limits.finite();
    let depth_limit = finite_limits
        .and_then(|limits| limits.depth)
        .map_or(MAX_PLY, NonZeroU32::get);
    let node_limit = finite_limits
        .and_then(|limits| limits.nodes)
        .map(NonZeroU64::get);
    tt.new_search();
    let shared = SharedSearch {
        external_stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        node_limit,
        started,
        hard_limit: time_budget.map(|budget| budget.hard),
    };

    let worker_outcomes = run_worker_team(threads, &shared, |worker_index| {
        if worker_index == 0 {
            run_main_worker(
                pst,
                position,
                rules,
                root_moves,
                history_keys,
                depth_limit,
                time_budget,
                &shared,
                tt,
                events,
            )
        } else {
            run_auxiliary_worker(
                pst,
                position,
                rules,
                root_moves,
                history_keys,
                depth_limit,
                worker_index,
                &shared,
                tt,
            )
        }
    });
    let total_nodes = shared.nodes();
    debug_assert_eq!(
        total_nodes,
        worker_outcomes
            .iter()
            .map(|outcome| outcome.nodes)
            .sum::<u64>()
    );
    let adopted = select_worker_outcome(&worker_outcomes);
    let mut result = adopted.result;
    result.nodes = total_nodes;
    SearchOutcome {
        result,
        elapsed: started.elapsed(),
        pv: adopted.pv.clone(),
        stop_reason: shared.reason(),
    }
}

/// 主ワーカーと補助ワーカーを実行し、パニック時も全ワーカーを回収する。
fn run_worker_team(
    threads: NonZeroUsize,
    shared: &SharedSearch<'_>,
    worker: impl Fn(usize) -> WorkerOutcome + Sync,
) -> Vec<WorkerOutcome> {
    thread::scope(|scope| {
        let auxiliary_workers: Vec<_> = (1..threads.get())
            .map(|worker_index| {
                let worker = &worker;
                scope.spawn(move || run_worker_guarded(shared, || worker(worker_index)))
            })
            .collect();
        let main_outcome = run_worker_guarded(shared, || worker(0));
        let mut worker_outcomes = Vec::with_capacity(threads.get());
        let mut panic_payload = match main_outcome {
            Ok(outcome) => {
                worker_outcomes.push(outcome);
                None
            }
            Err(payload) => Some(payload),
        };
        for worker in auxiliary_workers {
            let outcome = worker
                .join()
                .expect("guarded search worker must not unwind across its thread boundary");
            match outcome {
                Ok(outcome) => worker_outcomes.push(outcome),
                Err(payload) if panic_payload.is_none() => panic_payload = Some(payload),
                Err(_) => {}
            }
        }
        if let Some(payload) = panic_payload {
            std::panic::resume_unwind(payload);
        }
        worker_outcomes
    })
}

/// ワーカーパニックを捕捉し、残るワーカーへ停止を通知してから呼び出し側へ返す。
fn run_worker_guarded<T>(
    shared: &SharedSearch<'_>,
    worker: impl FnOnce() -> T,
) -> thread::Result<T> {
    let outcome = catch_unwind(AssertUnwindSafe(worker));
    if outcome.is_err() {
        shared.team_stop.store(true, AtomicOrdering::Release);
    }
    outcome
}

/// 完了深さが最大のワーカーを選び、同じ深さなら番号が最小のものを選ぶ。
///
/// 深さ0は採用候補から除き、全ワーカーが深さ0なら主ワーカーの既定結果を
/// 返す。
fn select_worker_outcome(worker_outcomes: &[WorkerOutcome]) -> &WorkerOutcome {
    let main_outcome = worker_outcomes
        .iter()
        .find(|outcome| outcome.worker_index == 0)
        .expect("search team must contain the main worker");
    worker_outcomes
        .iter()
        .filter(|outcome| outcome.result.depth > 0)
        .max_by_key(|outcome| (outcome.result.depth, Reverse(outcome.worker_index)))
        .unwrap_or(main_outcome)
}

/// 補助ワーカーが探索する深さを昇順に返す。
fn auxiliary_depths(worker_index: usize, depth_limit: u32) -> impl Iterator<Item = u32> {
    assert!(worker_index > 0, "auxiliary worker index must be positive");
    let period = 2 + ((worker_index - 1) % 4) as u32;
    (1..=depth_limit)
        .filter(move |&depth| depth == 1 || (depth - 1) % period == 0 || depth == depth_limit)
}

/// ワーカー固有の探索状態を構築する。
fn new_searcher<'a>(
    pst: &'a Pst,
    position: &Position,
    rules: MoveRules,
    history_keys: &'a [u64],
    shared: &'a SharedSearch<'a>,
    tt: &'a TranspositionTable,
) -> Searcher<'a> {
    let root_accumulator = pst.refresh_accumulator(position);
    Searcher {
        pst,
        rules,
        generator: MoveGenerator::new(rules),
        history_keys,
        path_keys: vec![search_key(position)],
        null_move_ply: None,
        nodes: 0,
        shared,
        stop_reason: None,
        pv: (0..=MAX_PLY)
            .map(|ply| Vec::with_capacity((MAX_PLY - ply) as usize))
            .collect(),
        accumulators: [root_accumulator; MAX_PLY as usize + 1],
        history: Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]),
        killers: [[None; KILLER_COUNT]; MAX_PLY as usize + 1],
        tt,
    }
}

/// 主ワーカーの反復深化を実行し、深さ完了ごとに進捗イベントを送る。
#[allow(clippy::too_many_arguments)]
fn run_main_worker(
    pst: &Pst,
    position: &Position,
    rules: MoveRules,
    root_moves: &[Move],
    history_keys: &[u64],
    depth_limit: u32,
    time_budget: Option<TimeBudget>,
    shared: &SharedSearch<'_>,
    tt: &TranspositionTable,
    events: Option<(&mpsc::Sender<SearchEvent>, u64)>,
) -> WorkerOutcome {
    let mut searcher = new_searcher(pst, position, rules, history_keys, shared, tt);
    let mut result = SearchResult {
        best_move: root_moves[0],
        score: pst.evaluate_accumulator(searcher.accumulators[0], position.side_to_move()),
        depth: 0,
        nodes: 0,
    };
    let mut completed_pv = vec![root_moves[0]];

    for depth in 1..=depth_limit {
        let Some((best_move, score)) = searcher.search_root(position, root_moves, depth) else {
            debug_assert!(searcher.stop_reason.is_some());
            break;
        };
        result.best_move = best_move;
        result.score = score;
        result.depth = depth;
        completed_pv.clone_from(&searcher.pv[0]);
        let elapsed = shared.started.elapsed();
        if let Some((sender, search_id)) = events {
            let _ = sender.send(SearchEvent::Progress {
                search_id,
                depth,
                score,
                nodes: shared.nodes(),
                elapsed,
                pv: completed_pv.clone(),
            });
        }
        if shared
            .node_limit
            .is_some_and(|limit| shared.nodes() >= limit)
        {
            shared.stop(StopReason::NodeLimit);
            break;
        }
        if time_budget.is_some_and(|budget| elapsed >= budget.soft) {
            shared.stop(StopReason::SoftLimit);
            break;
        }
        if depth == depth_limit {
            shared.stop(StopReason::DepthCompleted);
            break;
        }
    }
    let nodes = searcher.nodes;
    WorkerOutcome {
        worker_index: 0,
        result,
        pv: completed_pv,
        nodes,
    }
}

/// 補助ワーカーの反復深化を実行し、最後まで完了した反復を返す。
#[allow(clippy::too_many_arguments)]
fn run_auxiliary_worker(
    pst: &Pst,
    position: &Position,
    rules: MoveRules,
    root_moves: &[Move],
    history_keys: &[u64],
    depth_limit: u32,
    worker_index: usize,
    shared: &SharedSearch<'_>,
    tt: &TranspositionTable,
) -> WorkerOutcome {
    let mut searcher = new_searcher(pst, position, rules, history_keys, shared, tt);
    let mut result = SearchResult {
        best_move: root_moves[0],
        score: pst.evaluate_accumulator(searcher.accumulators[0], position.side_to_move()),
        depth: 0,
        nodes: 0,
    };
    let mut completed_pv = vec![root_moves[0]];
    for depth in auxiliary_depths(worker_index, depth_limit) {
        let Some((best_move, score)) = searcher.search_root(position, root_moves, depth) else {
            break;
        };
        result.best_move = best_move;
        result.score = score;
        result.depth = depth;
        completed_pv.clone_from(&searcher.pv[0]);
        if depth == depth_limit {
            break;
        }
    }
    let nodes = searcher.nodes;
    WorkerOutcome {
        worker_index,
        result,
        pv: completed_pv,
        nodes,
    }
}

/// 1回の探索実行の可変状態。
struct Searcher<'a> {
    /// 探索中に使う検証済み学習PST。
    pst: &'a Pst,
    /// 探索内の着手適用に使う規則。
    rules: MoveRules,
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
    /// 探索チームで共有する停止状態と予算。
    shared: &'a SharedSearch<'a>,
    /// 中断時に記録する停止条件。
    stop_reason: Option<StopReason>,
    /// plyごとの主変化。行plyは、その深さ以降の最善応手列を保持する。
    pv: Vec<Vec<Move>>,
    /// plyごとのPST生重み和。
    accumulators: [PstAccumulator; MAX_PLY as usize + 1],
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
        let tt_move = self.tt.probe(key, 0).and_then(|hit| hit.best_move);
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
            .store(key, depth, best_score, Bound::Exact, Some(best_move), 0);
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
            tt_move = hit.best_move;
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
            let lion_before = position
                .lion_taken_by_non_lion()
                .map(|trigger| trigger.square);
            let undo = position.make_null_move();
            self.accumulators[(ply + 1) as usize] = self
                .pst
                .update_accumulator_after_null(self.accumulators[ply as usize], lion_before);
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

        let mut picker = MovePicker::new(tt_move, self.killers[ply as usize]);
        let mut best_move = None;
        let mut best_score = -INFINITY;
        let mut beta_cutoff = false;
        let mut index = 0;
        while let Some(mv) = picker.next(position, &self.generator, &self.history) {
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
                best_move = Some(mv);
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
            index += 1;
        }
        let Some(best_move) = best_move else {
            return Some(-MATE + ply as i32);
        };
        let bound = if best_score <= original_alpha {
            Bound::Upper
        } else if beta_cutoff {
            Bound::Lower
        } else {
            Bound::Exact
        };
        self.tt
            .store(key, depth, best_score, bound, Some(best_move), ply);
        Some(best_score)
    }

    /// stand-patと捕獲手だけを使う静止探索で局面を評価する。
    ///
    /// 静止探索内の手は主変化へ含めず、反復検出は行わない。
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
            return Some(
                self.pst
                    .evaluate_accumulator(self.accumulators[ply as usize], position.side_to_move()),
            );
        }

        let original_alpha = alpha;
        let key = search_key(position);
        let mut tt_move = None;
        if let Some(hit) = self.tt.probe(key, ply) {
            tt_move = hit.best_move;
            let cutoff = match hit.bound {
                Bound::Exact => true,
                Bound::Lower => hit.score >= beta,
                Bound::Upper => hit.score <= alpha,
            };
            if cutoff {
                return Some(hit.score);
            }
        }

        let stand_pat = self
            .pst
            .evaluate_accumulator(self.accumulators[ply as usize], position.side_to_move());
        if stand_pat >= beta {
            self.tt.store(key, 0, stand_pat, Bound::Lower, None, ply);
            return Some(stand_pat);
        }
        let mut best = stand_pat;
        let mut best_move = None;
        alpha = alpha.max(stand_pat);

        let mut captures = Vec::new();
        self.generator.generate_captures(position, &mut captures);
        let mut captures: Vec<_> = captures
            .into_iter()
            .map(|mv| {
                let key = move_order_key(position, mv)
                    .expect("capture generator must not return a quiet move");
                (mv, key)
            })
            .collect();
        order_captures(&mut captures);
        if let Some(index) =
            tt_move.and_then(|tt_move| captures.iter().position(|&(mv, _)| mv == tt_move))
        {
            captures.swap(0, index);
        }

        for (mv, key) in captures {
            let is_last_royal_capture = captures_last_royal(position, mv);
            if !is_last_royal_capture && stand_pat + key.captured_value + DELTA_MARGIN <= alpha {
                continue;
            }
            let score = if is_last_royal_capture {
                self.enter_node().then_some(MATE - ply as i32)?
            } else {
                let undo = position.make_move_unchecked(mv, self.rules);
                self.accumulators[(ply + 1) as usize] = self.pst.update_accumulator_after_move(
                    self.accumulators[ply as usize],
                    position,
                    &undo,
                );
                let score = self
                    .quiesce(position, -beta, -alpha, ply + 1)
                    .map(|value| -value);
                position.unmake_move(undo);
                score?
            };
            if score > best {
                best = score;
                best_move = Some(mv);
            }
            alpha = alpha.max(score);
            if alpha >= beta {
                break;
            }
        }
        let bound = if best >= beta {
            Bound::Lower
        } else if best <= original_alpha {
            Bound::Upper
        } else {
            Bound::Exact
        };
        self.tt.store(key, 0, best, bound, best_move, ply);
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

        self.accumulators[(ply + 1) as usize] = self.pst.update_accumulator_after_move(
            self.accumulators[ply as usize],
            position,
            &undo,
        );
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
        if self.shared.observe_external_stop() {
            self.stop_reason = Some(StopReason::ExternalStop);
            return false;
        }
        if self.shared.team_stop.load(AtomicOrdering::Acquire) {
            self.stop_reason = Some(self.shared.reason());
            return false;
        }
        if self.nodes.is_multiple_of(STOP_CHECK_INTERVAL)
            && self
                .shared
                .hard_limit
                .is_some_and(|limit| self.shared.started.elapsed() >= limit)
        {
            self.shared.stop(StopReason::HardLimit);
            self.stop_reason = Some(StopReason::HardLimit);
            return false;
        }
        if !self.shared.reserve_node() {
            self.stop_reason = Some(self.shared.reason());
            return false;
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

/// 1局の開始から終局までに見込む手数。
const EXPECTED_PLIES: u32 = 450;
/// 1局面で見込む残り手数の下限。
const MIN_MOVES: u32 = 100;

/// 現在の手数から、手番側が今後指すと見込む手数を返す。
fn moves_to_go(ply: u32) -> u128 {
    u128::from(MIN_MOVES.max(EXPECTED_PLIES.saturating_sub(ply) / 2))
}

/// 持ち時間制の予算式を1箇所に集約する。
///
/// `moves_to_go = max(MIN_MOVES, EXPECTED_PLIES.saturating_sub(ply) / 2)`、
/// `soft_raw = remaining / moves_to_go + 0.7 * increment + 0.8 * byoyomi`、
/// `safe_hard = max(1ms, (remaining + byoyomi).saturating_sub(30ms))`、
/// `hard = max(1ms, min(4 * soft_raw, remaining / 4 + 0.8 * byoyomi, safe_hard))`、
/// `soft = min(soft_raw, hard)`とする。
/// 係数を変更する場合は自己対局で採否を判定する。
fn clock_budget(clock: ClockLimits) -> TimeBudget {
    let remaining = u128::from(clock.remaining_ms);
    let increment = u128::from(clock.increment_ms);
    let byoyomi = u128::from(clock.byoyomi_ms);
    let byoyomi_share = byoyomi * 8 / 10;
    let soft_raw = remaining / moves_to_go(clock.ply) + increment * 7 / 10 + byoyomi_share;
    let safe_hard = remaining.saturating_add(byoyomi).saturating_sub(30).max(1);
    let hard = (soft_raw * 4)
        .min(remaining / 4 + byoyomi_share)
        .min(safe_hard)
        .max(1);
    TimeBudget {
        soft: Duration::from_millis(to_u64_ms(soft_raw.min(hard))),
        hard: Duration::from_millis(to_u64_ms(hard)),
    }
}

/// 探索制限から時間予算を求める。`movetime`と時計の併用時は小さい方を採る。
fn time_budget(limits: &SearchLimits) -> Option<TimeBudget> {
    let limits = limits.finite()?;
    let movetime = limits.movetime_ms.map(|milliseconds| TimeBudget {
        soft: Duration::from_millis(milliseconds.get()),
        hard: Duration::from_millis(milliseconds.get()),
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

/// 通常探索で手を返す段階。
#[derive(Clone, Copy)]
enum MovePickerStage {
    Tt,
    Captures,
    Killer0,
    Killer1,
    Quiets,
    Done,
}

/// TT手、捕獲手、killer手、静かな手の順に合法手を1回ずつ返す。
struct MovePicker {
    stage: MovePickerStage,
    tt_move: Option<Move>,
    killers: [Option<Move>; KILLER_COUNT],
    captures: Vec<Move>,
    capture_index: usize,
    captures_generated: bool,
    quiets: Vec<Move>,
    quiet_index: usize,
    quiets_generated: bool,
}

impl MovePicker {
    /// 助言手を保持した空の手選択器を作る。
    fn new(tt_move: Option<Move>, killers: [Option<Move>; KILLER_COUNT]) -> Self {
        Self {
            stage: MovePickerStage::Tt,
            tt_move,
            killers,
            captures: Vec::new(),
            capture_index: 0,
            captures_generated: false,
            quiets: Vec::new(),
            quiet_index: 0,
            quiets_generated: false,
        }
    }

    /// 現在の段階で次に探索する合法手を返す。
    fn next(
        &mut self,
        position: &Position,
        generator: &MoveGenerator,
        history: &HistoryTable,
    ) -> Option<Move> {
        loop {
            match self.stage {
                MovePickerStage::Tt => {
                    self.stage = MovePickerStage::Captures;
                    if let Some(tt_move) = self.tt_move
                        && generator.is_legal_move(position, tt_move)
                    {
                        return Some(tt_move);
                    }
                }
                MovePickerStage::Captures => {
                    if !self.captures_generated {
                        generator.generate_captures(position, &mut self.captures);
                        self.captures.retain(|&mv| Some(mv) != self.tt_move);
                        self.captures.sort_by_key(|&mv| {
                            let key = move_order_key(position, mv)
                                .expect("capture generator must not return a quiet move");
                            (Reverse(key.captured_value), key.attacker_value)
                        });
                        self.captures_generated = true;
                    }
                    if let Some(&mv) = self.captures.get(self.capture_index) {
                        self.capture_index += 1;
                        return Some(mv);
                    }
                    self.stage = MovePickerStage::Killer0;
                }
                MovePickerStage::Killer0 | MovePickerStage::Killer1 => {
                    if !self.quiets_generated {
                        generator.generate_quiets(position, &mut self.quiets);
                        self.quiets.retain(|&mv| Some(mv) != self.tt_move);
                        self.quiets_generated = true;
                    }
                    let killer_index = usize::from(matches!(self.stage, MovePickerStage::Killer1));
                    self.stage = if killer_index == 0 {
                        MovePickerStage::Killer1
                    } else {
                        MovePickerStage::Quiets
                    };
                    if let Some(killer) = self.killers[killer_index]
                        && let Some(index) = self.quiets.iter().position(|&mv| mv == killer)
                    {
                        return Some(self.quiets.remove(index));
                    }
                }
                MovePickerStage::Quiets => {
                    let color = position.side_to_move().index();
                    self.quiets.sort_by_cached_key(|&mv| {
                        Reverse(history[color][mv.from.dense_index()][mv.to.dense_index()])
                    });
                    self.stage = MovePickerStage::Done;
                }
                MovePickerStage::Done => {
                    if let Some(&mv) = self.quiets.get(self.quiet_index) {
                        self.quiet_index += 1;
                        return Some(mv);
                    }
                    return None;
                }
            }
        }
    }
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
