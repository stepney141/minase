//! 探索の開始と実行中の探索チームの操作。

use core::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Instant;

use crate::eval::Pst;
use crate::search::TranspositionTable;
use crate::search::alphabeta::team::run_search_team;
use crate::search::error::SearchError;
use crate::search::events::{SearchEvent, SearchResult};
use crate::search::limits::SearchLimits;
use crate::search::snapshot::SearchSnapshot;

/// 実行中の探索チームを操作するハンドル。
///
/// 値を破棄すると探索チームへ停止を要求し、全ワーカーを回収するまで待つ。
pub struct SearchHandle {
    /// 探索イベントの受信端。
    pub(super) events: mpsc::Receiver<SearchEvent>,
    /// スレッド生成前に取得した起点。
    pub(super) started: Instant,
    /// 的中までのナノ秒数。先読み中はu64::MAX。
    pub(super) hit_ns: Arc<AtomicU64>,
    /// 探索チームと共有する外部停止フラグ。
    pub(super) stop: Arc<AtomicBool>,
    /// 全ワーカーの終了後に置換表を返す調整役のハンドル。
    pub(super) thread: Option<thread::JoinHandle<TranspositionTable>>,
}

impl SearchHandle {
    /// 探索イベントの受信端を返す。
    pub fn events(&self) -> &mpsc::Receiver<SearchEvent> {
        &self.events
    }

    /// 先読みを的中の時点から計時する。重複した通知は無視する。
    pub fn ponderhit(&self) {
        let elapsed = self
            .started
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

/// 所有権を移した評価重み、入力および置換表を使い、別スレッドで探索チームを
/// 開始する。`ponder`が真なら時間制限は的中の通知まで無効とする。
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
    ponder: bool,
) -> SearchHandle {
    let started = Instant::now();
    let hit_ns = Arc::new(AtomicU64::new(if ponder { u64::MAX } else { 0 }));
    let thread_hit_ns = Arc::clone(&hit_ns);
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
            started,
            &thread_hit_ns,
            ponder,
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
        started,
        hit_ns,
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
        Instant::now(),
        &AtomicU64::new(0),
        false,
    )
    .result)
}
