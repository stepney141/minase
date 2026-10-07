//! 主ワーカーと補助ワーカーの反復深化。

use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::mpsc;
use std::time::Duration;

use crate::eval::Pst;
use crate::search::TranspositionTable;
use crate::search::events::{SearchEvent, SearchResult, StopReason};
use minase_core::mv::Move;
use minase_core::position::Position;
use minase_core::rules::MoveRules;

use super::history::HistoryTable;
use super::root::RootResults;
use super::searcher::{PonderIteration, Searcher, new_searcher};
use super::team::{SharedSearch, WorkerOutcome};
use super::time::{TimeBudget, should_start_next_iteration, stable_signal};

/// 補助ワーカーが探索する深さを昇順に返す。
pub(super) fn auxiliary_depths(worker_index: usize, depth_limit: u32) -> impl Iterator<Item = u32> {
    assert!(worker_index > 0, "auxiliary worker index must be positive");
    let period = 2 + ((worker_index - 1) % 4) as u32;
    (1..=depth_limit)
        .filter(move |&depth| depth == 1 || (depth - 1) % period == 0 || depth == depth_limit)
}

/// 主ワーカーの反復深化を実行し、深さ完了ごとに進捗イベントを送る。
#[allow(clippy::too_many_arguments)]
pub(super) fn run_main_worker(
    pst: &Pst,
    position: &Position,
    rules: MoveRules,
    root_moves: &[Move],
    history_keys: &[u64],
    depth_limit: u32,
    time_budget: Option<TimeBudget>,
    shared: &SharedSearch<'_>,
    tt: &TranspositionTable,
    history: &mut HistoryTable,
    events: Option<(&mpsc::Sender<SearchEvent>, u64)>,
    ponder: bool,
    adopt_partial: bool,
) -> WorkerOutcome {
    let mut searcher = new_searcher(pst, position, rules, history_keys, shared, tt, history);
    searcher.root_results = adopt_partial.then(RootResults::default);
    #[cfg(feature = "invariants")]
    pst.assert_accumulator(position, searcher.accumulators[0], 0);
    let mut result = SearchResult {
        #[cfg(feature = "search-stats")]
        stats: crate::search::SearchStats::default(),
        best_move: root_moves[0],
        score: pst.evaluate_accumulator(searcher.accumulators[0], position.side_to_move()),
        depth: 0,
        nodes: 0,
    };
    let mut completed_pv = vec![root_moves[0]];
    let mut completed_bests = Vec::new();

    for depth in 1..=depth_limit {
        let prev = (result.depth > 0).then_some(result.score);
        if ponder && let Some(budget) = time_budget {
            let iteration_started = shared.started.elapsed();
            let stable = stable_signal(&completed_bests);
            // 前の境界の検査後やワーカー生成前に的中した場合も、開始条件を通す。
            // tを先に記録するので、このhの読み取りより後の的中は必ずtより後になる。
            let hit_ns = shared
                .hard_limit
                .expect("a timed search has a hard limit")
                .hit_ns
                .load(AtomicOrdering::Relaxed);
            if hit_ns != u64::MAX
                && !should_start_next_iteration(
                    shared.started.elapsed(),
                    Duration::from_nanos(hit_ns),
                    budget,
                    stable,
                )
            {
                shared.stop(StopReason::SoftLimit);
                break;
            }
            searcher.ponder_iteration = Some(PonderIteration {
                started: iteration_started,
                stable,
                checked: false,
                budget,
            });
        }
        let Some((best_move, score)) = searcher.search_iteration(position, root_moves, depth, prev)
        else {
            debug_assert!(searcher.stop_reason.is_some());
            break;
        };
        completed_bests.push(best_move);
        let stable = stable_signal(&completed_bests);
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
        if time_budget.is_some_and(|budget| {
            let hit_ns = shared
                .hard_limit
                .expect("a timed search has a hard limit")
                .hit_ns
                .load(AtomicOrdering::Relaxed);
            hit_ns != u64::MAX
                && !should_start_next_iteration(
                    shared.started.elapsed(),
                    Duration::from_nanos(hit_ns),
                    budget,
                    stable,
                )
        }) {
            shared.stop(StopReason::SoftLimit);
            break;
        }
        if depth == depth_limit {
            shared.stop(StopReason::DepthCompleted);
            break;
        }
    }
    main_worker_outcome(&searcher, result, completed_pv)
}

/// 「途中結果の採用」（byoyomi-time-usage.md）に従い、完了深さと評価値を保って着手を選ぶ。
pub(super) fn main_worker_outcome(
    searcher: &Searcher<'_>,
    mut result: SearchResult,
    mut completed_pv: Vec<Move>,
) -> WorkerOutcome {
    #[cfg(feature = "search-stats")]
    {
        result.stats = searcher.stats;
    }
    let partial = searcher
        .root_results
        .as_ref()
        .and_then(|results| results.partial_result(searcher.stop_reason));
    let partial_score = partial.map(|partial| partial.score);
    if let Some(partial) = partial {
        result.best_move = partial.pv[0];
        completed_pv.clone_from(&partial.pv);
    }
    let nodes = searcher.nodes;
    WorkerOutcome {
        partial_score,
        worker_index: 0,
        result,
        pv: completed_pv,
        nodes,
    }
}

/// 補助ワーカーの反復深化を実行し、最後まで完了した反復を返す。
#[allow(clippy::too_many_arguments)]
pub(super) fn run_auxiliary_worker(
    pst: &Pst,
    position: &Position,
    rules: MoveRules,
    root_moves: &[Move],
    history_keys: &[u64],
    depth_limit: u32,
    worker_index: usize,
    shared: &SharedSearch<'_>,
    tt: &TranspositionTable,
    history: &mut HistoryTable,
) -> WorkerOutcome {
    let mut searcher = new_searcher(pst, position, rules, history_keys, shared, tt, history);
    #[cfg(feature = "invariants")]
    pst.assert_accumulator(position, searcher.accumulators[0], 0);
    let mut result = SearchResult {
        #[cfg(feature = "search-stats")]
        stats: crate::search::SearchStats::default(),
        best_move: root_moves[0],
        score: pst.evaluate_accumulator(searcher.accumulators[0], position.side_to_move()),
        depth: 0,
        nodes: 0,
    };
    let mut completed_pv = vec![root_moves[0]];
    for depth in auxiliary_depths(worker_index, depth_limit) {
        let prev = (result.depth > 0).then_some(result.score);
        let Some((best_move, score)) = searcher.search_iteration(position, root_moves, depth, prev)
        else {
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
    #[cfg(feature = "search-stats")]
    {
        result.stats = searcher.stats;
    }
    let nodes = searcher.nodes;
    WorkerOutcome {
        partial_score: None,
        worker_index,
        result,
        pv: completed_pv,
        nodes,
    }
}
