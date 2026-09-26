//! 探索チームの共有状態とワーカーの実行。

use core::cmp::Reverse;
use core::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering as AtomicOrdering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::core::mv::Move;
use crate::core::position::Position;
use crate::core::rules::MoveRules;
use crate::eval::Pst;
use crate::search::events::{SearchEvent, SearchResult, StopReason};
use crate::search::limits::SearchLimits;
use crate::search::{MAX_PLY, TranspositionTable};

use super::deepening::{run_auxiliary_worker, run_main_worker};
use super::time::time_budget;

/// 探索の内部実行が返す結果一式。
pub(in crate::search) struct SearchOutcome {
    /// 完了した探索の結果。
    pub(in crate::search) result: SearchResult,
    /// 探索開始からの経過時間。
    pub(in crate::search) elapsed: Duration,
    /// 最後まで完了した深さの主変化。
    pub(in crate::search) pv: Vec<Move>,
    /// 探索を停止した条件。
    pub(in crate::search) stop_reason: StopReason,
}

/// 1ワーカーが最後まで完了した反復と実着手の適用回数。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct WorkerOutcome {
    /// 探索チーム内のワーカー番号。主ワーカーは0。
    pub(super) worker_index: usize,
    /// 最後まで完了した反復の結果。
    pub(super) result: SearchResult,
    /// 最後まで完了した反復の主変化。
    pub(super) pv: Vec<Move>,
    /// このワーカーが実際の着手を盤面へ適用した回数。
    pub(super) nodes: u64,
}

/// 探索チームで共有する停止状態と探索予算。
pub(super) struct SharedSearch<'a> {
    /// 呼び出し側からの停止要求。
    pub(super) external_stop: &'a AtomicBool,
    /// 探索チーム内部の停止要求。
    pub(super) team_stop: AtomicBool,
    /// 優先順位を反映した停止理由。
    pub(super) stop_reason: AtomicU8,
    /// 全ワーカーが実際の着手を盤面へ適用した合計回数。
    pub(super) total_nodes: AtomicU64,
    /// 探索チーム全体で実着手を適用する回数の上限。
    pub(super) node_limit: Option<u64>,
    /// 補助ワーカー生成前に記録した探索開始時刻。
    pub(super) started: Instant,
    /// 探索途中でも打ち切る時間制限。
    pub(super) hard_limit: Option<HardLimit<'a>>,
}

/// 時間の上限と、同じ起点から測った的中時刻。
#[derive(Clone, Copy)]
pub(super) struct HardLimit<'a> {
    pub(super) duration: Duration,
    pub(super) hit_ns: &'a AtomicU64,
}

impl SharedSearch<'_> {
    /// 停止理由を優先度付きで共有状態へ合成し、チーム停止を要求する。
    pub(super) fn stop(&self, reason: StopReason) {
        self.stop_reason
            .fetch_max(stop_reason_priority(reason), AtomicOrdering::Relaxed);
        self.team_stop.store(true, AtomicOrdering::Release);
    }

    /// 外部停止が成立していれば最優先の停止理由として記録する。
    pub(super) fn observe_external_stop(&self) -> bool {
        if self.external_stop.load(AtomicOrdering::Relaxed) {
            self.stop(StopReason::ExternalStop);
            true
        } else {
            false
        }
    }

    /// 現在までに探索チームが実際の着手を盤面へ適用した合計回数を返す。
    pub(super) fn nodes(&self) -> u64 {
        self.total_nodes.load(AtomicOrdering::Relaxed)
    }

    /// 共有状態に記録された停止理由を返す。
    pub(super) fn reason(&self) -> StopReason {
        stop_reason_from_priority(self.stop_reason.load(AtomicOrdering::Relaxed))
            .expect("a completed search team must record a stop reason")
    }

    /// 実着手の適用1回分を予約する。上限を超える予約は拒否する。
    pub(super) fn reserve_node(&self, limit: u64) -> bool {
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
pub(in crate::search) fn run_search_team(
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
    started: Instant,
    hit_ns: &AtomicU64,
    ponder: bool,
) -> SearchOutcome {
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
        hard_limit: time_budget.map(|budget| HardLimit {
            duration: budget.hard,
            hit_ns,
        }),
    };

    let history_keys: Vec<u64> = history_keys.to_vec();
    let worker_outcomes = run_worker_team(threads, &shared, |worker_index| {
        if worker_index == 0 {
            run_main_worker(
                pst,
                position,
                rules,
                root_moves,
                &history_keys,
                depth_limit,
                time_budget,
                &shared,
                tt,
                events,
                ponder,
            )
        } else {
            run_auxiliary_worker(
                pst,
                position,
                rules,
                root_moves,
                &history_keys,
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
pub(super) fn run_worker_team(
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
pub(super) fn select_worker_outcome(worker_outcomes: &[WorkerOutcome]) -> &WorkerOutcome {
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
