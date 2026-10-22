//! 主ワーカーと補助ワーカーの反復深化。

use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::mpsc;
use std::time::Duration;

use crate::eval::Nnue;
use crate::search::events::{SearchEvent, SearchResult, StopReason};
use crate::search::{SearchRepetition, TranspositionTable};
use minase_core::mv::Move;
use minase_core::position::Position;
use minase_core::rules::MoveRules;

use super::capture_history::CaptureHistory;
use super::correction::{ContinuationCorrectionHistory, CorrectionValues};
use super::history::{ContinuationHistory, HistoryTable, PawnHistory};
use super::params;
use super::root::RootResults;
use super::searcher::{PonderIteration, Searcher, new_searcher};
use super::team::{SharedSearch, WorkerOutcome};
use super::time::{
    FallingEvalCoefficients, InstabilityScaling, TimeBudget, scaled_soft_millis,
    scaled_time_budget, should_start_next_iteration, stable_signal,
};

/// 補助ワーカーが探索する深さを昇順に返す。
///
/// 1始まりのワーカー番号kに対し、周期 `2 + ((k − 1) % 4)` ごとに深さ1から反復する。
/// 周期を2〜5の4種類で循環させ、多数のワーカーを使っても深さ1しか読まない割当てを避ける。
/// 深さ上限は周期にかかわらず必ず含める。k=1、上限6なら1、3、5、6である。
/// 判断の理由は`docs/adr/diversify-lazy-smp-workers-by-skipping-depths.md`にある。
pub(super) fn auxiliary_depths(worker_index: usize, depth_limit: u32) -> impl Iterator<Item = u32> {
    assert!(worker_index > 0, "auxiliary worker index must be positive");
    let period = 2 + ((worker_index - 1) % 4) as u32;
    (1..=depth_limit)
        .filter(move |&depth| depth == 1 || (depth - 1) % period == 0 || depth == depth_limit)
}

/// 主ワーカーの反復深化を実行し、窓内で深さを完了するごとに進捗イベントを送る。
/// 深さ1は全窓で読む。
/// 深さ2以上は、直前の完了反復の最善手について記録した評価値の平均を窓の中心に使い、
/// 符号つき2乗の平均を初期半幅に使う。
/// aspiration windowの窓外れは[`Searcher::search_iteration`]内で読み直し、進捗として通知しない。
#[allow(clippy::too_many_arguments)]
pub(super) fn run_main_worker(
    nnue: &Nnue,
    position: &Position,
    rules: MoveRules,
    repetition: SearchRepetition,
    draw_value: i32,
    root_moves: &[Move],
    history_keys: &[u64],
    depth_limit: u32,
    time_budget: Option<TimeBudget>,
    mut instability_scaling: Option<InstabilityScaling>,
    shared: &SharedSearch<'_>,
    tt: &TranspositionTable,
    history: &mut HistoryTable,
    capture_history: &mut CaptureHistory,
    correction_values: &CorrectionValues,
    continuation_correction: &mut ContinuationCorrectionHistory,
    continuation_history: &ContinuationHistory,
    pawn_history: &PawnHistory,
    events: Option<(&mpsc::Sender<SearchEvent>, u64)>,
    ponder: bool,
    adopt_partial: bool,
) -> WorkerOutcome {
    let mut searcher = new_searcher(
        nnue,
        position,
        rules,
        repetition,
        draw_value,
        0,
        history_keys,
        shared,
        tt,
        history,
        capture_history,
        correction_values,
        continuation_correction,
        continuation_history,
        pawn_history,
    );
    searcher.root_results = adopt_partial.then(RootResults::default);
    #[cfg(feature = "invariants")]
    nnue.assert_accumulator(position, &searcher.accumulators[0], 0);
    let mut result = SearchResult {
        #[cfg(feature = "search-stats")]
        stats: crate::search::SearchStats::default(),
        best_move: root_moves[0],
        score: nnue.evaluate_accumulator(&searcher.accumulators[0], position.side_to_move()),
        depth: 0,
        nodes: 0,
    };
    let mut completed_pv = vec![root_moves[0]];
    let mut completed_bests = Vec::new();
    let base_time_budget = time_budget;
    let mut current_time_budget = base_time_budget;

    for depth in 1..=depth_limit {
        let prev = (result.depth > 0).then_some((result.best_move, result.score));
        if ponder && let Some(budget) = current_time_budget {
            let iteration_started = shared.clock.elapsed();
            let stable = stable_signal(&completed_bests, params::stable_iterations() as usize);
            // 前の境界の検査後やワーカー生成前に的中した場合も、開始条件を通す。
            // tを先に記録するので、このhの読み取りより後の的中は必ずtより後になる。
            let hit_ns = shared
                .hard_limit
                .expect("a timed search has a hard limit")
                .hit_ns
                .load(AtomicOrdering::Relaxed);
            if hit_ns != u64::MAX
                && !should_start_next_iteration(
                    shared.clock.elapsed(),
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
        let stable = stable_signal(&completed_bests, params::stable_iterations() as usize);
        if let Some(scaling) = &mut instability_scaling {
            let base = base_time_budget.expect("instability scaling requires a time budget");
            let factor = scaling.update_falling_eval(
                score,
                nnue.pawn_value(),
                FallingEvalCoefficients::current(),
            );
            let time_ms = scaled_soft_millis(base.soft, &[factor as u128]);
            current_time_budget = Some(scaled_time_budget(base, time_ms));
        }
        result.best_move = best_move;
        result.score = score;
        result.depth = depth;
        completed_pv.clone_from(&searcher.pv[0]);
        let elapsed = shared.clock.elapsed();
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
        if current_time_budget.is_some_and(|budget| {
            let hit_ns = shared
                .hard_limit
                .expect("a timed search has a hard limit")
                .hit_ns
                .load(AtomicOrdering::Relaxed);
            hit_ns != u64::MAX
                && !should_start_next_iteration(
                    shared.clock.elapsed(),
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

/// [`RootResults::partial_result`]の途中結果があれば、その着手と主変化を採る。
/// 完了反復の深さと評価値は保ち、途中結果の値は`partial_score`として別に返す。
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
    let returned_move_average = returned_move_average(searcher, result);
    let nodes = searcher.nodes;
    WorkerOutcome {
        partial_score,
        returned_move_average,
        worker_index: 0,
        result,
        pv: completed_pv,
        nodes,
    }
}

/// 補助ワーカーの反復深化を実行し、最後まで完了した反復を返す。
///
/// 進捗を通知せずsoft limitも評価しないが、各ノードでチーム共通の停止条件を検査する。
/// 窓の初期化には自分の直前の完了反復の最善手と、その手について自分が記録した評価値の平均を使う。
/// 初期半幅には、その手について自分が記録した符号つき2乗の平均とワーカー番号も使う。
/// 窓外れは読み直してから同じ反復の最善手、評価値、深さ、主変化を1組で保持する。
/// 深さ上限を完了すると自分の処理を終え、他ワーカーへ深さ完了の停止を通知しない。
#[allow(clippy::too_many_arguments)]
pub(super) fn run_auxiliary_worker(
    nnue: &Nnue,
    position: &Position,
    rules: MoveRules,
    repetition: SearchRepetition,
    draw_value: i32,
    root_moves: &[Move],
    history_keys: &[u64],
    depth_limit: u32,
    worker_index: usize,
    shared: &SharedSearch<'_>,
    tt: &TranspositionTable,
    history: &mut HistoryTable,
    capture_history: &mut CaptureHistory,
    correction_values: &CorrectionValues,
    continuation_correction: &mut ContinuationCorrectionHistory,
    continuation_history: &ContinuationHistory,
    pawn_history: &PawnHistory,
) -> WorkerOutcome {
    let mut searcher = new_searcher(
        nnue,
        position,
        rules,
        repetition,
        draw_value,
        worker_index,
        history_keys,
        shared,
        tt,
        history,
        capture_history,
        correction_values,
        continuation_correction,
        continuation_history,
        pawn_history,
    );
    #[cfg(feature = "invariants")]
    nnue.assert_accumulator(position, &searcher.accumulators[0], 0);
    let mut result = SearchResult {
        #[cfg(feature = "search-stats")]
        stats: crate::search::SearchStats::default(),
        best_move: root_moves[0],
        score: nnue.evaluate_accumulator(&searcher.accumulators[0], position.side_to_move()),
        depth: 0,
        nodes: 0,
    };
    let mut completed_pv = vec![root_moves[0]];
    for depth in auxiliary_depths(worker_index, depth_limit) {
        let prev = (result.depth > 0).then_some((result.best_move, result.score));
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
    let returned_move_average = returned_move_average(&searcher, result);
    let nodes = searcher.nodes;
    WorkerOutcome {
        partial_score: None,
        returned_move_average,
        worker_index,
        result,
        pv: completed_pv,
        nodes,
    }
}

/// 深さ1以上を完了したワーカーが返す手の根の手ごとの記録の平均値を返す。
fn returned_move_average(searcher: &Searcher<'_>, result: SearchResult) -> Option<i32> {
    (result.depth > 0).then(|| {
        searcher
            .root_move_stats
            .iter()
            .find(|(mv, _stats)| *mv == result.best_move)
            .and_then(|(_mv, stats)| stats.averages)
            .map(|(average, _signed_square_average)| average)
            .expect("a completed root move must have an average score")
    })
}
