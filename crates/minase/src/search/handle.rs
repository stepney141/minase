//! 探索の開始と実行中の探索チームの操作。

use core::{fmt, num::NonZeroUsize};
use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, mpsc};
use std::thread;

use crate::eval::Nnue;
use crate::search::alphabeta::team::run_search_team;
use crate::search::clock::{MonotonicClock, SearchClock};
use crate::search::error::SearchError;
use crate::search::events::{SearchEvent, SearchResult};
use crate::search::limits::SearchLimits;
use crate::search::snapshot::SearchSnapshot;
use crate::search::{HistoryTables, Searcher, TranspositionTable};

/// 実行中の探索チームを操作するハンドル。
///
/// 値を破棄すると探索チームへ停止を要求し、全ワーカーを回収するまで待つ。
pub struct SearchHandle {
    /// 探索イベントの受信端。
    pub(super) events: mpsc::Receiver<SearchEvent>,
    /// 探索チームと同じ起点から測る時計。
    pub(super) clock: Arc<dyn SearchClock>,
    /// 的中までのナノ秒数。先読み中はu64::MAX。
    pub(super) hit_ns: Arc<AtomicU64>,
    /// 探索チームと共有する外部停止フラグ。
    pub(super) stop: Arc<AtomicBool>,
    /// 全ワーカーの終了後に探索器を返す調整役のハンドル。
    pub(super) thread: Option<thread::JoinHandle<Result<Searcher, SearchJoinError>>>,
}

impl SearchHandle {
    /// 探索イベントの受信端を返す。
    pub fn events(&self) -> &mpsc::Receiver<SearchEvent> {
        &self.events
    }

    /// 先読みを的中の時点から計時する。重複した通知は無視する。
    pub fn ponderhit(&self) {
        let elapsed = self
            .clock
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX - 1)) as u64;
        let _ = self.hit_ns.compare_exchange(
            u64::MAX,
            elapsed,
            AtomicOrdering::Relaxed,
            AtomicOrdering::Relaxed,
        );
    }

    /// 探索チーム全体へ停止を要求する。
    pub fn request_stop(&self) {
        self.stop.store(true, AtomicOrdering::Relaxed);
    }

    /// 探索チームへ停止を要求して全ワーカーの終了を待ち、探索器を返す。
    ///
    /// パニックはペイロードを、推論の失敗は元のエラーを保持して返す。
    /// いずれの場合も探索イベントの[`SearchEvent::Finished`]は送信されない。
    pub fn join(mut self) -> Result<Searcher, SearchJoinError> {
        self.stop_and_join()
            .expect("a live SearchHandle must own its search thread")
    }

    /// 停止要求と探索スレッドの回収を1回だけ行う。
    fn stop_and_join(&mut self) -> Option<Result<Searcher, SearchJoinError>> {
        self.request_stop();
        self.thread.take().map(|thread| {
            thread
                .join()
                .map_err(SearchJoinError::Panic)
                .and_then(|result| result)
        })
    }

    /// 時計、停止、的中通知、およびスレッドの回収を両方式で共有する。
    pub(super) fn spawn(
        clock: Arc<dyn SearchClock>,
        ponder: bool,
        run: impl FnOnce(
            Arc<dyn SearchClock>,
            Arc<AtomicU64>,
            Arc<AtomicBool>,
            mpsc::Sender<SearchEvent>,
        ) -> Result<Searcher, SearchJoinError>
        + Send
        + 'static,
    ) -> Self {
        let thread_clock = Arc::clone(&clock);
        let hit_ns = Arc::new(AtomicU64::new(if ponder { u64::MAX } else { 0 }));
        let thread_hit_ns = Arc::clone(&hit_ns);
        let (sender, events) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || run(thread_clock, thread_hit_ns, thread_stop, sender));
        Self {
            clock,
            hit_ns,
            events,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for SearchHandle {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}

/// 指定局面を呼び出しスレッド上で反復深化探索する。
/// `histories`は`threads`と同じワーカー数で作成し、次の探索へ持ち越す。
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
    nnue: &Nnue,
    snapshot: &SearchSnapshot,
    limits: &SearchLimits,
    threads: NonZeroUsize,
    tt: &mut TranspositionTable,
    histories: &mut HistoryTables,
) -> Result<SearchResult, SearchError> {
    if limits.is_infinite() {
        return Err(SearchError::InfiniteSynchronousSearch);
    }
    let stop = AtomicBool::new(false);
    Ok(run_search_team(
        nnue,
        &snapshot.position,
        snapshot.rules(),
        snapshot.repetition(),
        snapshot.draw_value(),
        &snapshot.root_moves,
        &snapshot.history_keys,
        limits,
        &stop,
        threads,
        tt,
        histories,
        None,
        &MonotonicClock::new(),
        &AtomicU64::new(0),
        false,
    )
    .result)
}

/// 指定した重みと時計を使い、置換表を確保済みの探索器を開始する。
/// 時計は探索チームと的中通知で共有する。
pub(in crate::search) fn start_search_with_clock(
    mut engine: crate::search::alphabeta::Engine,
    nnue: Arc<Nnue>,
    snapshot: SearchSnapshot,
    limits: SearchLimits,
    search_id: u64,
    ponder: bool,
    clock: Arc<dyn SearchClock>,
) -> SearchHandle {
    SearchHandle::spawn(
        clock,
        ponder,
        move |thread_clock, thread_hit_ns, thread_stop, sender| {
            let outcome = run_search_team(
                &nnue,
                &snapshot.position,
                snapshot.rules(),
                snapshot.repetition(),
                snapshot.draw_value(),
                &snapshot.root_moves,
                &snapshot.history_keys,
                &limits,
                &thread_stop,
                engine.threads,
                engine
                    .transposition_table
                    .as_ref()
                    .expect("a prepared search engine must own its transposition table"),
                &mut engine.histories,
                Some((&sender, search_id)),
                thread_clock.as_ref(),
                &thread_hit_ns,
                ponder,
            );
            let result = outcome.result;
            let _ = sender.send(SearchEvent::Finished {
                partial_score: outcome.partial_score,
                search_id,
                best_move: result.best_move,
                score: result.score,
                depth: result.depth,
                nodes: result.nodes,
                elapsed: outcome.elapsed,
                pv: outcome.pv,
                stop_reason: outcome.stop_reason,
            });
            Ok(Searcher::AlphaBeta(engine))
        },
    )
}

/// 探索スレッドのパニックと、型を保持した探索の失敗。
#[derive(Debug)]
pub enum SearchJoinError {
    /// スレッドのパニックのペイロード。
    Panic(Box<dyn Any + Send + 'static>),
    /// ネットの推論の失敗。
    Inference(crate::nn::NnError),
    /// MCTSの木または駆動部の契約違反。
    Mcts(crate::search::mcts::Error),
}

impl fmt::Display for SearchJoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Panic(payload) => {
                f.write_str("search thread panicked")?;
                if let Some(message) = payload.downcast_ref::<String>() {
                    write!(f, ": {message}")?;
                } else if let Some(message) = payload.downcast_ref::<&str>() {
                    write!(f, ": {message}")?;
                }
                Ok(())
            }
            Self::Inference(error) => write!(f, "search inference failed: {error}"),
            Self::Mcts(error) => write!(f, "MCTS search failed: {error}"),
        }
    }
}

impl std::error::Error for SearchJoinError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Panic(_) => None,
            Self::Inference(error) => Some(error),
            Self::Mcts(error) => Some(error),
        }
    }
}
