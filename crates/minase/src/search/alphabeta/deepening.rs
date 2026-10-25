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
    FallingEvalCoefficients, InstabilityScaling, TimeBudget, TimeEffortCoefficients,
    TimeInstabilityCoefficients, TimeReductionCoefficients, best_move_effort_factor,
    mate_early_stop, scaled_soft_millis, scaled_time_budget, should_increase_depth,
    should_start_next_iteration, stable_signal,
};

/// 完了反復ごとの時間伸縮と安定判定に使う係数。
///
/// 時間伸縮は秒読みが0で`movetime`のない時計だけに適用する。
/// `stable_iterations`による二値の短縮は、時間予算を持つほかの探索にも適用する。
#[derive(Clone, Copy)]
pub(super) struct IterationScalingCoefficients {
    pub(super) falling_eval: FallingEvalCoefficients,
    pub(super) best_move_stability: TimeReductionCoefficients,
    pub(super) best_move_instability: TimeInstabilityCoefficients,
    pub(super) best_move_effort: TimeEffortCoefficients,
    pub(super) stable_iterations: usize,
}

impl IterationScalingCoefficients {
    fn current() -> Self {
        Self {
            falling_eval: FallingEvalCoefficients::current(),
            best_move_stability: TimeReductionCoefficients::current(),
            best_move_instability: TimeInstabilityCoefficients::current(),
            best_move_effort: TimeEffortCoefficients::current(),
            stable_iterations: params::stable_iterations() as usize,
        }
    }
}

/// 主ワーカーが探索開始時の基礎予算と最新の完了反復で伸縮した予算を区別して保持する。
#[derive(Clone, Copy)]
struct IterationBudgets {
    base: Option<TimeBudget>,
    current: Option<TimeBudget>,
}

impl IterationBudgets {
    fn new(base: Option<TimeBudget>) -> Self {
        Self {
            base,
            current: base,
        }
    }

    fn has_budget(self) -> bool {
        self.current.is_some()
    }

    /// 最新の完了反復で伸縮した予算を使い、次の反復を始められるかを返す。
    fn can_start_next_iteration(
        self,
        elapsed: Duration,
        hit: Duration,
        stable: bool,
    ) -> Option<bool> {
        self.current
            .map(|budget| should_start_next_iteration(elapsed, hit, budget, stable))
    }

    /// 基礎softに完了反復の因子を掛け、次の反復へ渡す予算を更新する。
    fn scale(&mut self, factors: &[u128], root_move_count: usize) -> u128 {
        let base = self
            .base
            .expect("instability scaling requires a time budget");
        let time_ms = scaled_soft_millis(base.soft, factors, root_move_count);
        self.current = Some(scaled_time_budget(base, time_ms));
        time_ms
    }

    /// 先読みで始める反復へ、最新の完了反復で伸縮した予算を渡す。
    fn ponder_iteration(
        self,
        started: Duration,
        stable: bool,
        mate_early_stop: bool,
    ) -> Option<PonderIteration> {
        self.current.map(|budget| PonderIteration {
            started,
            stable,
            mate_early_stop,
            checked: false,
            budget,
        })
    }
}

/// 先読み中の最新の完了反復が詰みによる早期停止の条件を満たすかを記録する。
pub(super) fn update_ponder_mate_early_stop(record: &mut bool, enabled: bool, score: i32) -> bool {
    let current = enabled && mate_early_stop(score);
    *record = current;
    current
}

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
/// 時間伸縮を使う探索では、時間で継続すると決めた完了反復の境界でincreaseDepthの共有状態を更新し、
/// 実際に始める反復ごとに1回だけワーカー固有の回数へ反映する。反復の番号と進捗の深さは1ずつ増やす。
/// 詰みによる早期停止後も、より深い補助ワーカーの結果が選ばれ得るため、返す手が詰みである
/// ことは保証しない。
/// 秒読みつき時計の探索と固定時間だけを指定した探索では、[`super::time::uses_partial_results`]に従い、
/// `adopt_partial`で根の順序と途中結果の採用を有効にする。
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
    instability_scaling: Option<InstabilityScaling>,
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
    run_main_worker_with_iteration_hook(
        nnue,
        position,
        rules,
        repetition,
        draw_value,
        root_moves,
        history_keys,
        depth_limit,
        time_budget,
        instability_scaling,
        shared,
        tt,
        history,
        capture_history,
        correction_values,
        continuation_correction,
        continuation_history,
        pawn_history,
        events,
        ponder,
        adopt_partial,
        IterationScalingCoefficients::current(),
        |_depth, _stable| false,
    )
}

/// 指定した時間係数を使い、反復開始直後の検査フックを伴って主ワーカーを実行する。
/// 通常経路は何もしないフックを渡し、テストは先読み的中を決定的な位置へ注入する。
#[allow(clippy::too_many_arguments)]
pub(super) fn run_main_worker_with_iteration_hook(
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
    coefficients: IterationScalingCoefficients,
    mut iteration_hook: impl FnMut(u32, bool) -> bool,
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
    let mut iteration_budgets = IterationBudgets::new(time_budget);
    // 先読み中は最新の完了反復の判定で毎回上書きし、次の反復の開始時に保存する。
    let mut mate_early_stop_on_hit = false;

    for depth in 1..=depth_limit {
        let prev = (result.depth > 0).then_some((result.best_move, result.score));
        if ponder && iteration_budgets.has_budget() {
            let iteration_started = shared.clock.elapsed();
            let stable = stable_signal(&completed_bests, coefficients.stable_iterations);
            let iteration = iteration_budgets
                .ponder_iteration(iteration_started, stable, mate_early_stop_on_hit)
                .expect("a timed ponder search must have an iteration budget");
            // 前の境界の検査後やワーカー生成前に的中した場合も、開始条件を通す。
            // tを先に記録するので、このhの読み取りより後の的中は必ずtより後になる。
            let hit_ns = shared
                .hard_limit
                .expect("a timed search has a hard limit")
                .hit_ns
                .load(AtomicOrdering::Relaxed);
            if hit_ns != u64::MAX
                && (iteration.mate_early_stop
                    || !should_start_next_iteration(
                        shared.clock.elapsed(),
                        Duration::from_nanos(hit_ns),
                        iteration.budget,
                        iteration.stable,
                    ))
            {
                shared.stop(StopReason::SoftLimit);
                break;
            }
            searcher.ponder_iteration = Some(iteration);
        }
        searcher.begin_iteration();
        let ponder_stable = searcher
            .ponder_iteration
            .as_ref()
            .is_some_and(|iteration| iteration.stable);
        if iteration_hook(depth, ponder_stable) && !searcher.check_time() {
            break;
        }
        if let Some(scaling) = &mut instability_scaling {
            scaling.begin_iteration();
        }
        let Some((best_move, score)) = searcher.search_iteration(position, root_moves, depth, prev)
        else {
            debug_assert!(searcher.stop_reason.is_some());
            break;
        };
        completed_bests.push(best_move);
        let stable = stable_signal(&completed_bests, coefficients.stable_iterations);
        let mut scaled_time_ms = None;
        if let Some(scaling) = &mut instability_scaling {
            let falling_eval =
                scaling.update_falling_eval(score, nnue.pawn_value(), coefficients.falling_eval);
            let best_move_stability = scaling
                .update_best_move_stability(&completed_bests, coefficients.best_move_stability);
            let best_move_instability = scaling.update_best_move_instability(
                shared.take_best_move_changes(),
                coefficients.best_move_instability,
            );
            let best_move_nodes = searcher
                .root_move_stats
                .iter()
                .find(|(mv, _stats)| *mv == best_move)
                .map(|(_mv, stats)| stats.effort)
                .expect("a completed best move must have root move statistics");
            let best_move_effort = best_move_effort_factor(
                best_move_nodes,
                searcher.nodes,
                coefficients.best_move_effort,
            );
            let time_ms = iteration_budgets.scale(
                &[
                    falling_eval as u128,
                    best_move_stability as u128,
                    best_move_instability,
                    best_move_effort as u128,
                ],
                root_moves.len(),
            );
            scaled_time_ms = Some(time_ms);
        }
        let mate_early_stop = update_ponder_mate_early_stop(
            &mut mate_early_stop_on_hit,
            instability_scaling.is_some(),
            score,
        );
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
        let mut iteration_boundary = None;
        let stop_for_time = if iteration_budgets.has_budget() {
            let hit_ns = shared
                .hard_limit
                .expect("a timed search has a hard limit")
                .hit_ns
                .load(AtomicOrdering::Relaxed);
            let boundary_elapsed = shared.clock.elapsed();
            iteration_boundary = Some((boundary_elapsed, hit_ns));
            hit_ns != u64::MAX
                && (mate_early_stop
                    || !iteration_budgets
                        .can_start_next_iteration(
                            boundary_elapsed,
                            Duration::from_nanos(hit_ns),
                            stable,
                        )
                        .expect("a timed search must have an iteration budget"))
        } else {
            false
        };
        if stop_for_time {
            shared.stop(StopReason::SoftLimit);
            break;
        }
        if depth == depth_limit {
            shared.stop(StopReason::DepthCompleted);
            break;
        }
        if let (Some(time_ms), Some((boundary_elapsed, hit_ns))) =
            (scaled_time_ms, iteration_boundary)
        {
            let hit = (hit_ns != u64::MAX).then(|| Duration::from_nanos(hit_ns));
            shared.increase_depth.store(
                should_increase_depth(boundary_elapsed, hit, time_ms),
                AtomicOrdering::Relaxed,
            );
        }
    }
    let time_reduction = instability_scaling.map_or(1000, |scaling| scaling.time_reduction);
    main_worker_outcome(&searcher, result, completed_pv, time_reduction)
}

/// 秒読みつき時計の探索と固定時間だけを指定した探索では、
/// [`RootResults::partial_result`]の途中結果があれば、その着手と主変化を採る。
/// 完了反復の深さと評価値は保ち、途中結果の値は`partial_score`として別に返す。
pub(super) fn main_worker_outcome(
    searcher: &Searcher<'_>,
    mut result: SearchResult,
    mut completed_pv: Vec<Move>,
    time_reduction: i32,
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
        time_reduction: Some(time_reduction),
        #[cfg(test)]
        search_again_counter: searcher.search_again_counter,
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
/// 深さを飛ばしても、実際に始める反復ごとにincreaseDepthの共有状態を1回だけ回数へ反映する。
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
        searcher.begin_iteration();
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
        time_reduction: None,
        #[cfg(test)]
        search_again_counter: searcher.search_again_counter,
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
