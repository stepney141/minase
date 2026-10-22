//! 探索チームの共有状態とワーカーの実行。
//!
//! `Threads=1`では同じ入力、表の初期状態、固定深さまたは固定ノード数に対する結果が決定的である。
//! `Threads>1`では共有置換表への書き込み順が変わるため、固定深さや固定ノード数でも再現性を保証しない。

use core::cmp::Reverse;
use core::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering as AtomicOrdering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::eval::Nnue;
use crate::search::events::{SearchEvent, SearchResult, StopReason};
use crate::search::limits::SearchLimits;
use crate::search::{MAX_PLY, SearchRepetition, TranspositionTable};
use minase_core::mv::Move;
use minase_core::position::Position;
use minase_core::rules::MoveRules;

use super::clock::SearchClock;
use super::deepening::{run_auxiliary_worker, run_main_worker};
use super::history::HistoryTables;
use super::params;
use super::time::{InstabilityScaling, byoyomi_clock, time_budget};

/// 探索の内部実行が返す結果一式。
pub(in crate::search) struct SearchOutcome {
    /// 採用した途中結果の下界。完了反復の評価値とは分けて通知する。
    pub(in crate::search) partial_score: Option<i32>,
    /// 合算前の値を検査するためにテスト時だけ保持する各ワーカーの統計。
    #[cfg(all(test, feature = "search-stats"))]
    pub(super) worker_stats: Vec<crate::search::SearchStats>,
    /// 完了した探索の結果。
    pub(in crate::search) result: SearchResult,
    /// 探索開始からの経過時間。
    pub(in crate::search) elapsed: Duration,
    /// 採用した着手の主変化。途中結果があればその手のもの。
    pub(in crate::search) pv: Vec<Move>,
    /// 探索を停止した条件。
    pub(in crate::search) stop_reason: StopReason,
}

/// 1ワーカーが最後まで完了した反復と実着手の適用回数。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct WorkerOutcome {
    /// 対象の探索の主ワーカーが採用した途中結果の下界。
    pub(super) partial_score: Option<i32>,
    /// 返却手について、このワーカーの根の手ごとの記録にある評価値の平均。
    pub(super) returned_move_average: Option<i32>,
    /// 主ワーカーが最後に求めた短縮率。補助ワーカーでは値を持たない。
    pub(super) time_reduction: Option<i32>,
    /// 探索チーム内のワーカー番号。主ワーカーは0。
    pub(super) worker_index: usize,
    /// 最後まで完了した反復の結果。
    pub(super) result: SearchResult,
    /// 採用した着手の主変化。途中結果があればその手のもの。
    pub(super) pv: Vec<Move>,
    /// このワーカーが実際の着手を盤面へ適用した回数。
    pub(super) nodes: u64,
}

/// 探索チームで共有する停止状態と探索予算。
pub(super) struct SharedSearch<'a> {
    /// 呼び出し側からの停止要求。
    /// 他ワーカーの内部停止を`ExternalStop`と誤認しないよう、[`Self::team_stop`]とは別の原子値にする。
    pub(super) external_stop: &'a AtomicBool,
    /// 探索チーム内部の停止要求。外部停止との区別は[`Self::external_stop`]に従う。
    pub(super) team_stop: AtomicBool,
    /// 優先順位を反映した停止理由。
    pub(super) stop_reason: AtomicU8,
    /// 全ワーカーが実際の着手を盤面へ適用した合計回数。
    pub(super) total_nodes: AtomicU64,
    /// 全ワーカーが数えた、根で最善値を更新した2手目以降の手の回数。
    pub(super) best_move_changes: AtomicU64,
    /// 探索チーム全体で実着手を適用する回数の上限。
    pub(super) node_limit: Option<u64>,
    /// 探索開始からの経過時間を測る共有時計。
    pub(super) clock: &'a dyn SearchClock,
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

    /// 根で完了した2手目以降の手が探索前のαを上回ったことを記録する。
    pub(super) fn record_best_move_change(&self) {
        self.best_move_changes.fetch_add(1, AtomicOrdering::Relaxed);
    }

    /// 全ワーカーが記録した最善手の変化回数を回収し、次の区間を0から始める。
    pub(super) fn take_best_move_changes(&self) -> u64 {
        self.best_move_changes.swap(0, AtomicOrdering::Relaxed)
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

/// 外部停止、hard limit、ノード上限、soft limit、深さ完了の順に高い優先度を与える。
/// 近接して成立した条件を同じ規則で合成し、実行順による停止理由の表示の揺れを抑える。
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

/// 調整役として補助ワーカーを生成し、主ワーカーの探索と全ワーカーのjoinを実行する。
///
/// 置換表の世代はワーカー生成前に探索ごとに1回だけ進める。
/// 全ワーカーの終了後に[`select_worker_outcome`]で結果を選び、ノード数をチーム全体の合計へ置き換える。
/// scopeを抜けた後に外部停止を再確認せず、自然完了後の停止要求で記録済みの理由を上書きしない。
#[allow(clippy::too_many_arguments)]
pub(in crate::search) fn run_search_team(
    nnue: &Nnue,
    position: &Position,
    rules: MoveRules,
    repetition: SearchRepetition,
    draw_value: i32,
    root_moves: &[Move],
    history_keys: &[u64],
    limits: &SearchLimits,
    external_stop: &AtomicBool,
    threads: NonZeroUsize,
    tt: &TranspositionTable,
    histories: &mut HistoryTables,
    events: Option<(&mpsc::Sender<SearchEvent>, u64)>,
    clock: &dyn SearchClock,
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
    if histories.workers.len() != threads.get() {
        *histories = HistoryTables::new(threads);
    }
    let instability_scaling = (finite_limits
        .is_some_and(|limits| limits.clock.is_some() && limits.movetime_ms.is_none())
        && time_budget.is_some_and(|budget| !budget.byoyomi_period))
    .then(|| InstabilityScaling::new(histories.previous_search, threads));
    // 新しい根で1回だけ減衰する。新品の表にも適用し、既定値では−5を−3、−742を−452にする。
    let decay = params::history_decay();
    for history in &mut histories.workers {
        for value in history.iter_mut().flatten().flatten() {
            *value = *value * decay / 100;
        }
    }
    for history in &mut histories.capture {
        history.decay(params::capture_history_decay());
    }
    tt.new_search();
    let shared = SharedSearch {
        external_stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        best_move_changes: AtomicU64::new(0),
        node_limit,
        clock,
        hard_limit: time_budget.map(|budget| HardLimit {
            duration: budget.hard,
            hit_ns,
        }),
    };

    let history_keys: Vec<u64> = history_keys.to_vec();
    let correction = &histories.correction;
    let mut worker_histories: Vec<_> = histories
        .workers
        .iter_mut()
        .zip(&mut histories.capture)
        .zip(&mut histories.continuation_correction)
        .map(|((history, capture_history), continuation_correction)| {
            (history, capture_history, continuation_correction)
        })
        .collect();
    let continuation_history = &histories.continuation;
    let pawn_history = &histories.pawn;
    let worker_outcomes = run_worker_team(
        &mut worker_histories,
        &shared,
        |worker_index, (history, capture_history, continuation_correction)| {
            if worker_index == 0 {
                run_main_worker(
                    nnue,
                    position,
                    rules,
                    repetition,
                    draw_value,
                    root_moves,
                    &history_keys,
                    depth_limit,
                    time_budget,
                    instability_scaling,
                    &shared,
                    tt,
                    history,
                    capture_history,
                    correction,
                    continuation_correction,
                    continuation_history,
                    pawn_history,
                    events,
                    ponder,
                    byoyomi_clock(limits).is_some(),
                )
            } else {
                run_auxiliary_worker(
                    nnue,
                    position,
                    rules,
                    repetition,
                    draw_value,
                    root_moves,
                    &history_keys,
                    depth_limit,
                    worker_index,
                    &shared,
                    tt,
                    history,
                    capture_history,
                    correction,
                    continuation_correction,
                    continuation_history,
                    pawn_history,
                )
            }
        },
    );
    let total_nodes = shared.nodes();
    debug_assert_eq!(
        total_nodes,
        worker_outcomes
            .iter()
            .map(|outcome| outcome.nodes)
            .sum::<u64>()
    );
    let main = worker_outcomes
        .iter()
        .find(|outcome| outcome.worker_index == 0)
        .expect("search team must contain the main worker");
    let adopted = select_worker_outcome(&worker_outcomes);
    record_previous_search(histories, adopted, main);
    let mut result = adopted.result;
    result.nodes = total_nodes;
    #[cfg(feature = "search-stats")]
    {
        result.stats = crate::search::SearchStats::default();
        for outcome in &worker_outcomes {
            result.stats += outcome.result.stats;
        }
    }
    SearchOutcome {
        partial_score: adopted.partial_score,
        #[cfg(all(test, feature = "search-stats"))]
        worker_stats: worker_outcomes
            .iter()
            .map(|outcome| outcome.result.stats)
            .collect(),
        result,
        elapsed: clock.elapsed(),
        pv: adopted.pv.clone(),
        stop_reason: shared.reason(),
    }
}

/// 採用したワーカーから評価値と平均値を、主ワーカーから最後の短縮率を記録する。
pub(super) fn record_previous_search(
    histories: &mut HistoryTables,
    adopted: &WorkerOutcome,
    main: &WorkerOutcome,
) {
    assert_eq!(main.worker_index, 0, "main worker index must be zero");
    histories.previous_search.score_and_average = (adopted.result.depth > 0).then(|| {
        let score = adopted.partial_score.unwrap_or(adopted.result.score);
        let average = adopted
            .returned_move_average
            .expect("a completed worker result must have an average score");
        (score, average)
    });
    histories.previous_search.time_reduction = main
        .time_reduction
        .expect("the main worker must return its time reduction");
}

/// 主ワーカーと補助ワーカーを実行し、パニック時も全ワーカーを回収する。
pub(super) fn run_worker_team<T: Send>(
    histories: &mut [T],
    shared: &SharedSearch<'_>,
    worker: impl Fn(usize, &mut T) -> WorkerOutcome + Sync,
) -> Vec<WorkerOutcome> {
    let worker_count = histories.len();
    let (main_history, auxiliary_histories) = histories
        .split_first_mut()
        .expect("search team must have a main worker");
    thread::scope(|scope| {
        let auxiliary_workers: Vec<_> = auxiliary_histories
            .iter_mut()
            .enumerate()
            .map(|(index, history)| {
                let worker_index = index + 1;
                let worker = &worker;
                scope.spawn(move || run_worker_guarded(shared, || worker(worker_index, history)))
            })
            .collect();
        let main_outcome = run_worker_guarded(shared, || worker(0, main_history));
        let mut worker_outcomes = Vec::with_capacity(worker_count);
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

/// 完了深さが大きい結果を選び、同深さなら途中結果を持つ結果、次に番号が小さいワーカーを優先する。
/// [`byoyomi_clock`]の対象外の探索と補助ワーカーには途中結果がない。
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
        .max_by_key(|outcome| {
            (
                outcome.result.depth,
                outcome.partial_score.is_some(),
                Reverse(outcome.worker_index),
            )
        })
        .unwrap_or(main_outcome)
}
