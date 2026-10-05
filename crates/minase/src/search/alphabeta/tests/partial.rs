//! byoyomi-time-usage.md「途中結果の採用」の契約を検査する。

use super::*;
use crate::search::alphabeta::deepening::main_worker_outcome;
use crate::search::alphabeta::root::RootResults;
use crate::search::alphabeta::time::byoyomi_clock;

/// 「対象の探索」の境界を列挙する。予算の長短には依存しない。
fn clock_cases() -> Vec<(SearchLimits, bool)> {
    [
        (0, 1000, None, None, None, true),
        (1000, 1000, None, None, None, true),
        (1000, 0, None, None, None, false),
        (0, 1000, Some(5), None, None, false),
        (0, 1000, None, Some(10000), None, false),
        (0, 1000, None, None, Some(1000), false),
    ]
    .into_iter()
    .map(|(remaining, byoyomi, depth, nodes, movetime, eligible)| {
        (
            SearchLimits::new(
                depth,
                nodes,
                movetime,
                Some(ClockLimits::new(remaining, 0, byoyomi, 0).unwrap()),
            )
            .unwrap(),
            eligible,
        )
    })
    .chain([
        (depth_limits(5), false),
        (nodes_limits(10000), false),
        (movetime_limits(1000), false),
        (infinite_limits(), false),
    ])
    .collect()
}

#[test]
fn partial_requires_previous_best_and_non_upper_maximum_in_the_current_window() {
    let moves = legal_moves(&Position::initial());
    let mut results = RootResults::default();
    results.begin_window(0);
    results.record(moves[1], 10, 0, &[]);
    assert!(
        results
            .partial_result(Some(StopReason::HardLimit))
            .is_none()
    );
    results.previous_best = Some(moves[0]);
    assert!(
        results
            .partial_result(Some(StopReason::HardLimit))
            .is_none()
    );
    results.record(moves[0], -10, 0, &[]);
    assert_eq!(
        results
            .partial_result(Some(StopReason::HardLimit))
            .unwrap()
            .pv,
        [moves[1]]
    );

    // 最大値が窓の下限以下なら採用しない。等号も上界として除く。
    for score in [-1, 0] {
        results.begin_window(0);
        results.record(moves[0], score, 0, &[]);
        assert!(
            results
                .partial_result(Some(StopReason::HardLimit))
                .is_none()
        );
    }
    results.begin_window(-100);
    results.record(moves[0], -50, -100, &[]);
    // 数値が大きくても、その手を読んだ下限以下なら上界であり候補にしない。
    results.record(moves[1], 100, 100, &[]);
    results.record(moves[2], 10, -50, &[moves[3]]);
    results.record(moves[4], 10, 0, &[]);
    let partial = results.partial_result(Some(StopReason::HardLimit)).unwrap();
    assert_eq!(partial.score, 10);
    assert_eq!(partial.pv, [moves[2], moves[3]]); // 同値なら探索順を保つ。
    for reason in [
        None,
        Some(StopReason::ExternalStop),
        Some(StopReason::SoftLimit),
        Some(StopReason::NodeLimit),
        Some(StopReason::DepthCompleted),
    ] {
        assert!(results.partial_result(reason).is_none());
    }
    results.begin_window(-200);
    results.record(moves[2], 20, -200, &[]);
    assert!(
        results
            .partial_result(Some(StopReason::HardLimit))
            .is_none()
    );
}

#[test]
fn byoyomi_root_order_moves_only_previous_best_ahead_of_existing_order() {
    let position = Position::initial();
    let roots = legal_moves(&position);
    for (limits, eligible) in clock_cases() {
        with_root_searcher(&position, &[], |searcher| {
            let tt_move = *roots.last().unwrap();
            searcher
                .tt
                .store(search_key(&position), 1, 0, Bound::Exact, Some(tt_move), 0);
            let mut expected = roots.clone();
            searcher.order_moves(&position, &mut expected, Some(tt_move), 0);
            searcher.root_results = byoyomi_clock(&limits).map(|_| RootResults::default());
            if let Some(results) = &mut searcher.root_results {
                results.previous_best = Some(roots[1]);
            }
            let mut actual = roots.clone();
            searcher.order_root_moves(&position, &mut actual);
            if eligible {
                expected.retain(|&mv| mv != roots[1]);
                expected.insert(0, roots[1]);
            }
            assert_eq!(actual, expected, "{limits:?}");
        });
    }
}

#[test]
fn hard_partial_preserves_completed_values_and_root_tt_only_for_eligible_searches() {
    let position = Position::initial();
    let roots = legal_moves(&position);
    let history = repeated_root_children(&position, &roots);
    for (limits, eligible) in clock_cases() {
        for reason in [
            StopReason::HardLimit,
            StopReason::ExternalStop,
            StopReason::SoftLimit,
            StopReason::NodeLimit,
            StopReason::DepthCompleted,
        ] {
            with_root_searcher(&position, &history, |searcher| {
                searcher.root_results = byoyomi_clock(&limits).map(|_| RootResults::default());
                let (best_move, score) = searcher
                    .search_iteration(&position, &roots, 1, None)
                    .unwrap();
                let completed = SearchResult {
                    #[cfg(feature = "search-stats")]
                    stats: crate::search::SearchStats::default(),
                    best_move,
                    score,
                    depth: 1,
                    nodes: 0,
                };
                let completed_pv = searcher.pv[0].clone();
                let key = search_key(&position);
                let before = searcher.tt.probe(key, 0).unwrap();
                // 反復局面なので各根の手は1ノードで完了し、2手目の直前で止められる。
                searcher.interrupt_at = Some((searcher.nodes + 1, reason));
                assert!(
                    searcher
                        .search_iteration(&position, &roots, 2, Some(score))
                        .is_none()
                );
                let outcome = main_worker_outcome(searcher, completed, completed_pv.clone());
                assert_eq!((outcome.result.depth, outcome.result.score), (1, score));
                assert_eq!(
                    outcome.partial_score.is_some(),
                    eligible && reason == StopReason::HardLimit
                );
                assert_eq!(outcome.result.best_move, best_move);
                assert_eq!(outcome.pv, completed_pv);
                let after = searcher.tt.probe(key, 0).unwrap();
                assert_eq!(
                    (after.depth, after.score, after.bound, after.best_move),
                    (before.depth, before.score, before.bound, before.best_move)
                );
            });
        }
    }
}

#[test]
fn hard_interruption_before_previous_best_and_in_research_has_no_partial() {
    let position = Position::initial();
    let roots = legal_moves(&position);
    let history = repeated_root_children(&position, &roots);
    for (complete_first, retry) in [(false, false), (true, false), (true, true)] {
        with_root_searcher(&position, &history, |searcher| {
            searcher.root_results = Some(RootResults::default());
            if complete_first {
                searcher
                    .search_iteration(&position, &roots, 1, None)
                    .unwrap();
            }
            // 初回なら1手だけ完了。読み直しなら最初のfail-high窓の1手だけ完了。
            let nodes = searcher.nodes + u64::from(!complete_first || retry);
            searcher.interrupt_at = Some((nodes, StopReason::HardLimit));
            let prev = retry.then_some(-1000);
            assert!(
                searcher
                    .search_iteration(&position, &roots, if retry { 5 } else { 2 }, prev)
                    .is_none()
            );
            assert!(
                searcher
                    .root_results
                    .as_ref()
                    .unwrap()
                    .partial_result(searcher.stop_reason)
                    .is_none()
            );
            if retry {
                // 最初の窓は下界として記録されたが、新しい窓では未完了である。
                assert_eq!(
                    searcher.tt.probe(search_key(&position), 0).unwrap().bound,
                    Bound::Lower
                );
            }
        });
    }
}

#[test]
fn partial_changes_move_and_pv_without_changing_completed_score_or_depth() {
    let position = Position::initial();
    let roots = legal_moves(&position);
    with_root_searcher(&position, &[], |searcher| {
        let mut results = RootResults::default();
        results.previous_best = Some(roots[0]);
        results.begin_window(-INFINITY);
        results.record(roots[0], -2000, -INFINITY, &[]);
        results.record(roots[1], -1000, -2000, &[roots[2]]);
        searcher.root_results = Some(results);
        searcher.stop_reason = Some(StopReason::HardLimit);
        let completed = SearchResult {
            #[cfg(feature = "search-stats")]
            stats: crate::search::SearchStats::default(),
            best_move: roots[0],
            score: 100,
            depth: 3,
            nodes: 0,
        };
        let outcome = main_worker_outcome(searcher, completed, vec![roots[0]]);
        assert_eq!(outcome.result.best_move, roots[1]);
        assert_eq!(outcome.pv, [roots[1], roots[2]]);
        assert_eq!(outcome.partial_score, Some(-1000));
        assert_eq!((outcome.result.score, outcome.result.depth), (100, 3));
    });
}
