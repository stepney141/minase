//! 根の探索と探索窓を検査する。

use super::*;
use crate::search::REPETITION_BAND_START;
use crate::search::alphabeta::params;
use crate::search::alphabeta::root::{
    RootMoveStats, grow_aspiration_delta, initial_aspiration_delta, root_move_weight,
};

fn set_root_move_averages(searcher: &mut Searcher<'_>, mv: Move, averages: Option<(i32, i64)>) {
    if let Some((_, stats)) = searcher
        .root_move_stats
        .iter_mut()
        .find(|(recorded, _)| *recorded == mv)
    {
        stats.averages = averages;
    } else {
        searcher.root_move_stats.push((
            mv,
            RootMoveStats {
                effort: 0,
                averages,
            },
        ));
    }
}

fn current_initial_delta(pawn: i32, signed_square_average: i64, worker_index: usize) -> i32 {
    initial_aspiration_delta(
        pawn,
        params::aspiration_delta(),
        params::aspiration_score_width(),
        signed_square_average,
        worker_index,
    )
}

#[test]
fn first_root_move_stats_update_uses_the_score_and_its_signed_square() {
    for (value, signed_square) in [(3, 9), (-3, -9)] {
        let mut stats = RootMoveStats::default();
        stats.update(7, value);
        assert_eq!(stats.effort, 7);
        assert_eq!(stats.averages, Some((value, signed_square)));
    }
}

#[test]
fn root_move_stats_weight_has_a_lower_bound_of_twelve() {
    let mut stats = RootMoveStats {
        effort: 100,
        averages: Some((8, 64)),
    };
    stats.update(1, 40);
    assert_eq!(stats.effort, 101);
    assert_eq!(stats.averages, Some((20, 640)));
}

#[test]
fn root_move_stats_weight_has_an_upper_bound_of_twenty_four() {
    let mut stats = RootMoveStats {
        effort: 0,
        averages: Some((8, 64)),
    };
    stats.update(100, 40);
    assert_eq!(stats.effort, 100);
    assert_eq!(stats.averages, Some((32, 832)));
}

#[test]
fn root_move_signed_square_weight_has_an_upper_bound_of_sixteen() {
    let mut stats = RootMoveStats {
        effort: 1,
        averages: Some((0, 0)),
    };
    stats.update(2, 32);
    assert_eq!(stats.effort, 3);
    assert_eq!(stats.averages, Some((18, 512)));
}

#[test]
fn zero_node_root_move_update_initializes_averages_without_effort() {
    assert_eq!(root_move_weight(0, 0), 12);
    let mut stats = RootMoveStats::default();
    stats.update(0, -2);
    assert_eq!(stats.effort, 0);
    assert_eq!(stats.averages, Some((-2, -4)));
}

#[test]
fn root_move_stats_round_negative_averages_down() {
    let mut stats = RootMoveStats {
        effort: 100,
        averages: Some((0, 0)),
    };
    stats.update(1, -1);
    assert_eq!(stats.effort, 101);
    assert_eq!(stats.averages, Some((-1, -1)));
}

#[test]
fn root_move_weight_accepts_near_limit_node_counts() {
    assert_eq!(root_move_weight(u64::MAX, u64::MAX), 12);
}

#[test]
fn iteration_sets_seek_mate_from_previous_score_before_window_research() {
    let position = bare_kings();
    let moves = legal_moves(&position);
    with_root_searcher(&position, &[], |searcher| {
        assert!(!searcher.seek_mate);
        assert_eq!(searcher.nnue.pawn_value(), 100);
        // D=2の閾値は26803、D=5は105769/25を切り捨てて4591。
        // 窓の中心となる平均は0に固定し、組の値と反復の深さだけを変える。
        for (depth, previous_score, expected) in [
            (1, None, false),
            (2, Some(4591), false),
            (5, Some(4591), true),
            (5, Some(4590), false),
            (5, Some(-4591), true),
            (1, None, false),
        ] {
            let previous = previous_score.map(|score| {
                set_root_move_averages(searcher, moves[0], Some((0, 0)));
                (moves[0], score)
            });
            let (_, score) = searcher
                .search_iteration(&position, &moves, depth, previous)
                .unwrap();
            assert!(score.abs() < 4590);
            assert_eq!(
                searcher.seek_mate, expected,
                "depth={depth}, prev={previous_score:?}"
            );
        }
    });
}

// 部分木ノード数は完了反復だけを保存し、次の反復では先頭手を保って残りをノード数の降順に安定整列する。
// 判断の理由は docs/adr/order-root-moves-by-previous-subtree-nodes.md にある。
#[test]
fn completed_root_subtrees_order_the_next_iteration() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    with_root_searcher(&position, &[], |searcher| {
        assert!(searcher.completed_root_nodes.is_empty());
        assert!(searcher.current_root_nodes.is_empty());
        let mut prev = None;
        for depth in 1..=2 {
            let before = searcher.nodes;
            let (best, score) = searcher
                .search_iteration(&position, &moves, depth, prev)
                .unwrap();
            prev = Some((best, score));
            assert!(searcher.current_root_nodes.is_empty());
            let counts = &searcher.completed_root_nodes;
            assert_eq!(counts.len(), moves.len());
            assert_eq!(
                counts.iter().map(|(_, nodes)| nodes).sum::<u64>(),
                searcher.nodes - before
            );
            for mv in &moves {
                assert_eq!(counts.iter().filter(|(m, _)| m == mv).count(), 1);
            }
            let count = |mv: Move| counts.iter().find(|(m, _)| *m == mv).unwrap().1;
            let mut baseline = moves.clone();
            searcher.order_moves(&position, &mut baseline, Some(best), 0);
            let mut ordered = moves.clone();
            searcher.order_root_moves(&position, &mut ordered);
            assert_eq!(ordered[0], best);
            assert_eq!(ordered.len(), baseline.len());
            for pair in ordered[1..].windows(2) {
                assert!(count(pair[0]) >= count(pair[1]));
                if count(pair[0]) == count(pair[1]) {
                    assert!(
                        baseline.iter().position(|&mv| mv == pair[0])
                            < baseline.iter().position(|&mv| mv == pair[1])
                    );
                }
            }
            if depth == 2 {
                assert!(counts.iter().any(|(_, nodes)| *nodes > 1));
                assert!(counts.iter().any(|(_, nodes)| *nodes != counts[0].1));
            }
        }
    });
}

// 同点と未記録の手は通常の整列順を保ち、previous_bestは置換表の手より優先する。
#[test]
fn root_node_order_preserves_ties_missing_moves_and_the_preferred_first_move() {
    let position = Position::initial();
    let moves = legal_moves(&position);
    for previous_best in [None, Some(moves[1])] {
        with_root_searcher(&position, &[], |searcher| {
            let tt_move = *moves.last().unwrap();
            searcher.tt.store(
                search_key(&position),
                1,
                0,
                None,
                Bound::Exact,
                Some(tt_move),
                0,
                false,
            );
            if let Some(previous) = previous_best {
                let mut results = crate::search::alphabeta::root::RootResults::default();
                results.previous_best = Some(previous);
                searcher.root_results = Some(results);
            }
            let mut baseline = moves.clone();
            searcher.order_root_moves(&position, &mut baseline);
            assert_eq!(baseline[0], previous_best.unwrap_or(tt_move));
            searcher.completed_root_nodes = vec![
                (baseline[0], 1),
                (baseline[2], 0),
                (baseline[3], 10),
                (baseline[4], 20),
                (baseline[5], 10),
            ];
            let mut expected = vec![baseline[0], baseline[4], baseline[3], baseline[5]];
            expected.extend([baseline[1], baseline[2]]);
            expected.extend_from_slice(&baseline[6..]);
            let mut actual = moves.clone();
            searcher.order_root_moves(&position, &mut actual);
            assert_eq!(actual, expected);
        });
    }
}

// 根も置換表の補正前の静的評価に、現在の補正を加えて保持する。
#[test]
fn root_uses_tt_static_evaluation_before_current_correction() {
    let position = bare_kings();
    let moves = legal_moves(&position);
    with_root_searcher(&position, &[], |searcher| {
        let evaluated = evaluate(searcher.nnue, &position);
        let tt_static_eval = if evaluated == 1_000 { -1_000 } else { 1_000 };
        let correction = position_correction(searcher, position.side_to_move(), 0);
        searcher.tt.store(
            search_key(&position),
            0,
            0,
            Some(tt_static_eval),
            Bound::Exact,
            None,
            0,
            false,
        );

        searcher
            .search_root(&position, &moves, 1, -INFINITY, INFINITY)
            .unwrap();

        assert_eq!(searcher.static_evals[0], tt_static_eval + correction);
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.tt_static_eval_uses, 1);
            assert_eq!(searcher.stats.tt_static_eval_stores, 0);
        }
    });
}

// 根の照会ミスでも、補正前の静的評価をPVの探索値なし記録として保存する。
#[test]
fn root_stores_static_eval_only_on_a_tt_miss() {
    let position = bare_kings();
    let moves = legal_moves(&position);
    with_root_searcher(&position, &[], |searcher| {
        let raw = evaluate(searcher.nnue, &position);
        searcher
            .shared
            .external_stop
            .store(true, AtomicOrdering::Relaxed);

        assert_eq!(
            searcher.search_root(&position, &moves, 1, -INFINITY, INFINITY),
            None
        );
        let hit = searcher.tt.probe(search_key(&position), 0).unwrap();
        assert_eq!(hit.value, None);
        assert_eq!(hit.static_eval, Some(raw));
        assert_eq!(hit.best_move, None);
        assert!(hit.is_pv);
        #[cfg(feature = "search-stats")]
        assert_eq!(searcher.stats.tt_static_eval_stores, 1);
    });
}

// 読み直し中は前回完了表を参照し、同じ手へ各窓の探索量を加算する。
#[test]
fn root_research_keeps_completed_counts_and_accumulates_each_move() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    let history = repeated_root_children(&position, &moves);
    with_root_searcher(&position, &history, |searcher| {
        let mut baseline = moves.clone();
        searcher.order_root_moves(&position, &mut baseline);
        let promoted = *baseline.last().unwrap();
        let completed = vec![(promoted, 100)];
        searcher.completed_root_nodes.clone_from(&completed);
        // 全ての手が反復による引き分けとなり、各窓で正確に1ノードを使う。
        for expected_count in 1..=2 {
            assert_eq!(
                searcher.search_root(&position, &moves, 5, 0, 1).unwrap().1,
                DRAW_SCORE
            );
            assert_eq!(searcher.completed_root_nodes, completed);
            assert_eq!(searcher.current_root_nodes.len(), moves.len());
            assert!(
                searcher
                    .current_root_nodes
                    .iter()
                    .all(|(_, nodes)| *nodes == expected_count)
            );
            let mut ordered = moves.clone();
            searcher.order_root_moves(&position, &mut ordered);
            assert_eq!(ordered[0], baseline[0]);
            assert_eq!(ordered[1], promoted);
        }
    });
}

// 同じfail-low窓を読み直しても、並べ替え後の添字ではなく手の等値で記録を累積する。
#[test]
fn root_move_stats_accumulate_by_move_across_window_research() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    let history = repeated_root_children(&position, &moves);
    with_root_searcher(&position, &history, |searcher| {
        assert!(searcher.root_move_stats.is_empty());
        // 全ての手が反復による引き分けとなり、各窓で正確に1ノードを使う。
        for expected_effort in 1..=2 {
            assert_eq!(
                searcher.search_root(&position, &moves, 5, 0, 1).unwrap().1,
                DRAW_SCORE
            );
            assert_eq!(searcher.root_move_stats.len(), moves.len());
            for mv in &moves {
                let mut matching = searcher
                    .root_move_stats
                    .iter()
                    .filter(|(recorded, _)| recorded == mv);
                let stats = &matching.next().unwrap().1;
                assert_eq!(stats.effort, expected_effort);
                assert_eq!(stats.averages, Some((DRAW_SCORE, 0)));
                assert!(matching.next().is_none());
            }
        }
    });
}

#[test]
fn interrupted_root_iteration_discards_only_current_counts() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    let history = repeated_root_children(&position, &moves);
    with_root_searcher(&position, &history, |searcher| {
        let (best_move, score) = searcher
            .search_iteration(&position, &moves, 2, None)
            .unwrap();
        let completed = searcher.completed_root_nodes.clone();
        // 最初のfail-high窓を完了し、読み直しの途中で止める。
        searcher.interrupt_at = Some((searcher.nodes + 2, StopReason::NodeLimit));
        let delta = current_initial_delta(searcher.nnue.pawn_value(), 0, searcher.worker_index);
        set_root_move_averages(searcher, best_move, Some((-delta, 0)));
        assert!(
            searcher
                .search_iteration(&position, &moves, 5, Some((best_move, score)))
                .is_none()
        );
        assert_eq!(searcher.completed_root_nodes, completed);
        assert!(searcher.current_root_nodes.is_empty());
    });
}

#[test]
fn initial_aspiration_delta_adds_constant_and_absolute_score_terms() {
    for signed_square_average in [1_000_000, -1_000_000] {
        // 100 × 2 / 100 + 204 × 1,000,000 / (10,000 × 100) + 0 = 206。
        assert_eq!(
            initial_aspiration_delta(100, 2, 204, signed_square_average, 0),
            206
        );
    }
}

#[test]
fn initial_aspiration_delta_worker_term_repeats_every_eight_workers() {
    let base = initial_aspiration_delta(100, 2, 0, 0, 0);
    for (worker_index, expected_worker_term) in [
        (0, 0),
        (1, 0),
        (2, 0),
        (3, 1),
        (4, 1),
        (5, 2),
        (6, 2),
        (7, 3),
        (8, 0),
        (11, 1),
    ] {
        assert_eq!(
            initial_aspiration_delta(100, 2, 0, 0, worker_index) - base,
            expected_worker_term,
            "worker={worker_index}"
        );
    }
}

#[test]
fn initial_aspiration_delta_truncates_each_term_before_adding() {
    // 定数項は150/100 = 1、評価値の項は250,000/500,000 = 0。
    // 端数を足してから切り捨てれば2になるが、項ごとの切り捨てでは1になる。
    assert_eq!(initial_aspiration_delta(50, 3, 100, 2_500, 0), 1);
}

#[test]
fn initial_aspiration_delta_has_a_lower_bound_of_one() {
    // 10 × 2 / 100 + 204 × 0 / (10,000 × 10) + 0 = 0。
    assert_eq!(initial_aspiration_delta(10, 2, 204, 0, 0), 1);
}

#[test]
fn initial_aspiration_delta_uses_i64_for_largest_valid_products() {
    // 57,998 + 1,241 + 975 = 60,214。
    assert_eq!(
        initial_aspiration_delta(28_999, 200, 400, 900_060_001, 7),
        60_214
    );
}

#[test]
fn aspiration_growth_increases_small_deltas() {
    for delta in 1..=3 {
        assert!(grow_aspiration_delta(delta) > delta);
    }
}

#[test]
fn aspiration_growth_saturates_at_i32_max() {
    assert_eq!(grow_aspiration_delta(i32::MAX), i32::MAX);
}

// 通常範囲の平均だけを狭い窓の中心に使う。
#[test]
fn aspiration_initial_window_uses_only_eligible_averages() {
    for (case, average, expected, expected_delta) in [
        ("no_average", None, (-INFINITY, INFINITY), None),
        (
            "mate_threshold",
            Some(MATE_THRESHOLD),
            (-INFINITY, INFINITY),
            None,
        ),
        (
            "negative_mate_threshold",
            Some(-MATE_THRESHOLD),
            (-INFINITY, INFINITY),
            None,
        ),
        (
            "repetition_band_start",
            Some(REPETITION_BAND_START),
            (-INFINITY, INFINITY),
            None,
        ),
        (
            "negative_repetition_band_start",
            Some(-REPETITION_BAND_START),
            (-INFINITY, INFINITY),
            None,
        ),
        (
            "just_below_repetition_band",
            Some(REPETITION_BAND_START - 1),
            (REPETITION_BAND_START - 51, INFINITY),
            Some(50),
        ),
        (
            "just_above_negative_repetition_band",
            Some(-REPETITION_BAND_START + 1),
            (-INFINITY, -REPETITION_BAND_START + 51),
            Some(50),
        ),
        ("ordinary", Some(100), (50, 150), Some(50)),
    ] {
        let window = AspirationWindow::initial(average.map(|score| (score, 50)));
        assert_eq!((window.alpha, window.beta), expected, "{case}");
        assert_eq!(window.delta, expected_delta, "{case}");
    }
}

#[test]
fn iteration_uses_full_window_without_a_previous_best_average() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    let history = repeated_root_children(&position, &moves);
    for previous_best in [None, Some(moves[0])] {
        with_root_searcher(&position, &history, |searcher| {
            let previous = previous_best.map(|best_move| {
                set_root_move_averages(searcher, best_move, None);
                (best_move, DRAW_SCORE)
            });
            let depth = if previous.is_some() { 2 } else { 1 };
            let (_, score) = searcher
                .search_iteration(&position, &moves, depth, previous)
                .unwrap();
            assert_eq!(score, DRAW_SCORE);
            assert_eq!(searcher.nodes, moves.len() as u64);
        });
    }
}

#[test]
fn iteration_centers_the_window_on_the_previous_best_average() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    let history = repeated_root_children(&position, &moves);
    with_root_searcher(&position, &history, |searcher| {
        let delta = current_initial_delta(searcher.nnue.pawn_value(), 0, searcher.worker_index);
        let best_move = moves[0];
        set_root_move_averages(searcher, best_move, Some((delta, 0)));
        let (_, score) = searcher
            .search_iteration(&position, &moves, 2, Some((best_move, 10_000)))
            .unwrap();
        assert_eq!(score, DRAW_SCORE);
        assert_eq!(searcher.nodes, 2 * moves.len() as u64 + 1);
    });
}

#[test]
fn iteration_uses_signed_square_average_to_set_initial_window_width() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    let history = repeated_root_children(&position, &moves);
    let pawn = test_nnue().pawn_value();
    let narrow_q = 0;
    let wide_q = 1_000_000;
    let narrow_delta = current_initial_delta(pawn, narrow_q, 0);
    let wide_delta = current_initial_delta(pawn, wide_q, 0);
    assert!(wide_delta > narrow_delta);

    for (signed_square_average, expected_nodes) in [
        (narrow_q, 2 * moves.len() as u64 + 1),
        (wide_q, moves.len() as u64),
    ] {
        with_root_searcher(&position, &history, |searcher| {
            let best_move = moves[0];
            set_root_move_averages(
                searcher,
                best_move,
                Some((narrow_delta, signed_square_average)),
            );
            let (_, score) = searcher
                .search_iteration(&position, &moves, 2, Some((best_move, DRAW_SCORE)))
                .unwrap();
            assert_eq!(score, DRAW_SCORE);
            assert_eq!(searcher.nodes, expected_nodes);
        });
    }
}

#[test]
fn aspiration_window_endpoints_use_i64_before_normalization() {
    let window = AspirationWindow::initial(Some((1, i32::MAX)));
    assert_eq!((window.alpha, window.beta), (-INFINITY, INFINITY));

    let mut low = AspirationWindow::initial(Some((0, 1)));
    low.delta = Some(i32::MAX);
    low.widen_low(-1);
    assert_eq!((low.alpha, low.beta), (-INFINITY, -1));
    assert_eq!(low.delta, Some(i32::MAX));

    let mut high = AspirationWindow::initial(Some((0, 1)));
    high.delta = Some(i32::MAX);
    high.widen_high(1);
    assert_eq!((high.alpha, high.beta), (-1, INFINITY));
    assert_eq!(high.delta, Some(i32::MAX));
}

#[test]
fn aspiration_moves_edges_from_failed_score_before_growing_delta() {
    let mut low = AspirationWindow::initial(Some((100, 50)));
    low.widen_low(40);
    assert_eq!((low.alpha, low.beta), (-10, 50));
    assert_eq!(low.delta, Some(grow_aspiration_delta(50)));

    let mut high = AspirationWindow::initial(Some((100, 50)));
    high.widen_high(170);
    assert_eq!((high.alpha, high.beta), (100, 220));
    assert_eq!(high.delta, Some(grow_aspiration_delta(50)));
}

#[test]
fn aspiration_fail_high_keeps_the_larger_original_alpha() {
    for (delta, expected_alpha) in [(50, 100), (200, 50)] {
        let mut window = AspirationWindow::initial(Some((100, 50)));
        window.delta = Some(delta);
        window.widen_high(170);
        assert_eq!(window.alpha, expected_alpha, "delta={delta}");
        assert_eq!(window.beta, 170 + delta, "delta={delta}");
        assert_eq!(
            window.delta,
            Some(grow_aspiration_delta(delta)),
            "delta={delta}"
        );
    }
}

#[test]
fn aspiration_alternating_edge_scores_eventually_fall_inside_the_window() {
    let mut window = AspirationWindow::initial(Some((1, 1)));
    let mut failures = 0;
    loop {
        if 0 <= window.alpha {
            window.widen_low(0);
        } else if 0 >= window.beta {
            window.widen_high(0);
        } else {
            break;
        }
        failures += 1;
        assert!(failures <= 3, "alternating edge failures must terminate");
    }
    assert_eq!(failures, 2);
    assert!(window.alpha < 0 && 0 < window.beta);
    assert_eq!(
        window.delta,
        Some(grow_aspiration_delta(grow_aspiration_delta(1)))
    );
}

#[test]
fn aspiration_large_delta_reaches_full_window_within_three_failures() {
    let mut window = AspirationWindow::initial(Some((0, 10)));
    window.delta = Some(2 * REPETITION_BAND_START + 1);

    window.widen_high(10);
    assert_eq!((window.alpha, window.beta), (-10, INFINITY));
    window.widen_low(-10);
    assert_eq!((window.alpha, window.beta), (-INFINITY, -10));
    window.widen_high(-10);
    assert_eq!((window.alpha, window.beta), (-INFINITY, INFINITY));
}

// 引き分け値0を境界に置き、等号とβ打ち切りを検査する。
// 反復までに適用する着手は、全手探索なら合法手数、β打ち切りなら最初の1手となる。
#[test]
fn root_window_classifies_bounds_and_cuts_off_remaining_moves() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    assert!(moves.len() > 1);
    let history = repeated_root_children(&position, &moves);
    for (alpha, beta, bound, expected_nodes) in [
        (0, 1, Bound::Upper, moves.len() as u64),
        (-1, 0, Bound::Lower, 1),
        (-1, 1, Bound::Exact, moves.len() as u64),
    ] {
        with_root_searcher(&position, &history, |searcher| {
            let (best_move, score) = searcher
                .search_root(&position, &moves, 1, alpha, beta)
                .unwrap();
            assert_eq!(score, DRAW_SCORE);
            assert_eq!(searcher.nodes, expected_nodes);
            let hit = searcher.tt.probe(search_key(&position), 0).unwrap();
            assert_eq!(hit.value.unwrap().bound, bound);
            assert_eq!(hit.value.unwrap().score, score);
            assert_eq!(hit.best_move, Some(best_move));
            assert_eq!(hit.static_eval, Some(evaluate(searcher.nnue, &position)));
        });
    }
}

// s == α、s == βのどちらも再探索を要する。
// αから始める場合はfail low後のβにも等しくなり、3回目の窓で完了する。
#[test]
fn aspiration_researches_scores_equal_to_either_edge() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    let history = repeated_root_children(&position, &moves);
    let delta = current_initial_delta(test_nnue().pawn_value(), 0, 0);
    let move_count = moves.len() as u64;
    for (average, expected_nodes, first_move_nodes, other_move_nodes) in [
        (delta, 2 * move_count + 1, 3, 2),
        (-delta, move_count + 1, 2, 1),
    ] {
        with_root_searcher(&position, &history, |searcher| {
            let best_move = moves[0];
            set_root_move_averages(searcher, best_move, Some((average, 0)));
            let (_, score) = searcher
                .search_iteration(&position, &moves, 5, Some((best_move, 10_000)))
                .unwrap();
            assert_eq!(score, DRAW_SCORE);
            assert_eq!(searcher.nodes, expected_nodes);
            assert!(searcher.current_root_nodes.is_empty());
            assert_eq!(searcher.completed_root_nodes.len(), moves.len());
            for (index, &(_, nodes)) in searcher.completed_root_nodes.iter().enumerate() {
                assert_eq!(
                    nodes,
                    if index == 0 {
                        first_move_nodes
                    } else {
                        other_move_nodes
                    }
                );
            }
            assert_eq!(
                searcher
                    .tt
                    .probe(search_key(&position), 0)
                    .unwrap()
                    .value
                    .unwrap()
                    .bound,
                Bound::Exact
            );
        });
    }
}

// fail-lowでもαの更新と独立に最善手を保存し、
// 読み直しではその記録手を先頭に使う。王駒捕獲は適用しないので0ノードとなる。
#[test]
fn root_fail_low_keeps_the_best_bound_move_for_research() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::Rook),
            (fs(6, 12), Color::Black, PieceKind::King),
        ],
    );
    let moves = legal_moves(&position);
    let quiet = *moves
        .iter()
        .find(|&&mv| !captures_last_royal(&position, mv))
        .unwrap();
    let history = repeated_root_children(&position, &moves);
    with_root_searcher(&position, &history, |searcher| {
        let key = search_key(&position);
        searcher.tt.store(
            key,
            0,
            DRAW_SCORE,
            None,
            Bound::Exact,
            Some(quiet),
            0,
            false,
        );
        let (best_move, score) = searcher
            .search_root(&position, &moves, 1, MATE, INFINITY)
            .unwrap();
        assert_eq!(score, MATE - 1);
        assert!(captures_last_royal(&position, best_move));
        let hit = searcher.tt.probe(key, 0).unwrap();
        assert_eq!(hit.value.unwrap().bound, Bound::Upper);
        assert_eq!(hit.best_move, Some(best_move));
        let before = searcher.nodes;
        assert_eq!(
            searcher.search_root(&position, &moves, 1, -1, 0),
            Some((best_move, MATE - 1))
        );
        assert_eq!(searcher.nodes - before, 0);
        assert_eq!(
            searcher.tt.probe(key, 0).unwrap().value.unwrap().bound,
            Bound::Lower
        );
    });
}

// 全ルート手が反復引き分けとなる局面では、各深さで1手につき1ノードを使う。
// 深さ4の完了後に1手だけ読む固定予算で、次のaspiration反復を中断する。
#[test]
fn aspiration_interruption_preserves_the_last_completed_iteration() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    let completed_depth = 4;
    let completed_nodes = u64::from(completed_depth) * moves.len() as u64;
    let node_limit = completed_nodes + 1;
    let snapshot = SearchSnapshot::from_parts(
        position.clone(),
        engine_rules(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        repeated_root_children(&position, &moves),
        moves,
    )
    .unwrap();

    let handle = start(
        snapshot,
        nodes_limits(node_limit),
        85,
        worker_count(1),
        small_tt(),
    );
    let (progress, finished) = event_reports(drain_raw(&handle));
    handle.join().expect("search thread must not panic");
    assert_eq!(
        progress.iter().map(|entry| entry.0).collect::<Vec<_>>(),
        (1..=completed_depth).collect::<Vec<_>>()
    );
    assert_eq!(finished.depth, completed_depth);
    assert_eq!(finished.nodes, node_limit);
    assert_eq!(finished.stop_reason, StopReason::NodeLimit);
    let last = progress.last().unwrap();
    assert_eq!(finished.best_move, last.4[0]);
    assert_eq!(last.1, finished.score);
    assert_eq!(last.4, finished.pv);
    assert_eq!(last.2, completed_nodes);
    assert!(last.2 < finished.nodes);
}

#[test]
fn iteration_snapshots_previous_pv_before_window_research() {
    let position = bare_kings();
    let moves = legal_moves(&position);
    with_root_searcher(&position, &[], |searcher| {
        let (best_move, score) = searcher
            .search_iteration(&position, &moves, 1, None)
            .unwrap();
        assert!(searcher.previous_pv.is_empty());
        assert!(searcher.follow_pv[0]);
        let previous = searcher.pv[0].clone();
        assert!(!previous.is_empty());
        // 王将だけの局面では到達しない平均を与え、窓の読み直しを起こす。
        set_root_move_averages(searcher, best_move, Some((10_000, 0)));
        searcher
            .search_iteration(&position, &moves, 5, Some((best_move, score)))
            .unwrap();
        assert_eq!(searcher.previous_pv, previous);
        assert!(searcher.follow_pv[0]);
    });
}

// 窓外れと再探索中断は、反復間の評価変動に頼らず反復引き分けで作る。
#[test]
fn aspiration_retry_interruption_keeps_fail_high_bound() {
    let board = quiet_midgame();
    let moves = legal_moves(&board);
    let history = repeated_root_children(&board, &moves);
    with_root_searcher(&board, &history, |searcher| {
        let delta = current_initial_delta(searcher.nnue.pawn_value(), 0, searcher.worker_index);
        let best_move = moves[0];
        set_root_move_averages(searcher, best_move, Some((-delta - 1, 0)));
        searcher.interrupt_at = Some((1, StopReason::NodeLimit));
        assert!(
            searcher
                .search_iteration(&board, &moves, 5, Some((best_move, DRAW_SCORE)))
                .is_none()
        );
        assert_eq!(searcher.nodes, 1);
        assert_eq!(searcher.stop_reason, Some(StopReason::NodeLimit));
        let hit = searcher.tt.probe(search_key(&board), 0).unwrap();
        assert_eq!(hit.value.unwrap().bound, Bound::Lower);
        assert_eq!(hit.value.unwrap().score, DRAW_SCORE);
    });
}

/// 固定したhistory係数で、根の最善手と探索済みの手の更新を検査する。
/// 調整係数の変更を他のテストへ漏らさないよう、`tuning`の子プロセスから呼ぶ。
#[cfg(feature = "tuning")]
pub(super) fn root_history_updates_best_and_only_searched_non_improving_quiets() {
    let board = Position::initial();
    let moves: Vec<_> = legal_moves(&board)
        .into_iter()
        .filter(|&mv| move_order_key(&board, test_nnue(), mv).is_none())
        .take(4)
        .collect();
    assert_eq!(moves.len(), 4);

    with_root_searcher(&board, &[], |searcher| {
        let first = moves[0];
        let mut root_results = crate::search::alphabeta::root::RootResults::default();
        root_results.previous_best = Some(first);
        searcher.root_results = Some(root_results);

        let mut ordered = moves.clone();
        searcher.order_root_moves(&board, &mut ordered);
        assert_eq!(ordered[0], first);
        let scores = [0, 10, 20, -5];
        for (&mv, score) in ordered.iter().zip(scores) {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            searcher.tt.store(
                search_key(&child),
                MAX_PLY,
                -score,
                None,
                Bound::Exact,
                None,
                1,
                false,
            );
        }
        for (&mv, score) in ordered.iter().zip(scores) {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            let hit = searcher.tt.probe(search_key(&child), 1).unwrap();
            assert_eq!(hit.value.unwrap().score, -score, "child entry for {mv:?}");
        }

        assert_eq!(
            searcher.search_root(&board, &moves, 8, -10, 100),
            Some((ordered[2], scores[2]))
        );
        let history = |mv: Move| {
            searcher.history[Color::Black.index()][mv.from.dense_index()][mv.to.dense_index()]
        };
        assert!(history(ordered[2]) > 0, "根の静かな最善手を加点する");
        assert!(
            history(ordered[3]) < 0,
            "αを上回らなかった探索済みの手を減点する"
        );
        assert_eq!(history(ordered[0]), 0, "αを上回った先頭手は減点しない");
        assert_eq!(
            history(ordered[1]),
            0,
            "後で最善手でなくなってもαを上回った手は減点しない"
        );
    });
}

#[test]
fn interrupted_root_search_does_not_record_the_interrupted_move_or_update_history() {
    let board = bare_kings();
    let moves = legal_moves(&board);
    let history = repeated_root_children(&board, &moves);
    with_root_searcher(&board, &history, |searcher| {
        searcher.interrupt_at = Some((1, StopReason::NodeLimit));
        assert!(searcher.search_root(&board, &moves, 1, -1, 1).is_none());
        assert_eq!(searcher.nodes, 1);
        let interrupted = searcher
            .current_root_nodes
            .iter()
            .find(|(_, nodes)| *nodes == 0)
            .unwrap()
            .0;
        assert_eq!(searcher.root_move_stats.len(), 1);
        assert!(
            searcher
                .root_move_stats
                .iter()
                .all(|(mv, _)| *mv != interrupted)
        );
        assert!(
            searcher
                .history
                .iter()
                .flatten()
                .flatten()
                .all(|&value| value == 0),
            "中断した根の局所的な探索済み列は更新に使わない"
        );
    });
}

/// 固定したLMR係数で、根の後順手の減深と再探索を検査する。
/// 調整係数の変更を他のテストへ漏らさないよう、`tuning`の子プロセスから呼ぶ。
#[cfg(feature = "tuning")]
pub(super) fn root_lmr_research_records_only_the_final_score() {
    let board = Position::initial();
    let moves: Vec<_> = legal_moves(&board)
        .into_iter()
        .filter(|&mv| move_order_key(&board, test_nnue(), mv).is_none() && !mv.promote)
        .take(11)
        .collect();
    assert_eq!(moves.len(), 11);
    let first = moves[0];

    let mut ordered = Vec::new();
    with_root_searcher(&board, &[], |searcher| {
        let mut root_results = crate::search::alphabeta::root::RootResults::default();
        root_results.previous_best = Some(first);
        searcher.root_results = Some(root_results);
        ordered.clone_from(&moves);
        searcher.order_root_moves(&board, &mut ordered);
    });
    assert_eq!(ordered[0], first);
    let target = ordered[10];
    let mut target_child = board.clone();
    target_child.make_move_unchecked(target, engine_rules());
    let mut history: Vec<_> = legal_moves(&target_child)
        .into_iter()
        .map(|reply| {
            let mut grandchild = target_child.clone();
            grandchild.make_move_unchecked(reply, engine_rules());
            search_key(&grandchild)
        })
        .collect();
    history.extend([search_key(&board); 2]);

    with_root_searcher(&board, &history, |searcher| {
        let mut root_results = crate::search::alphabeta::root::RootResults::default();
        root_results.previous_best = Some(first);
        searcher.root_results = Some(root_results);
        for &mv in &ordered {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            let (depth, root_score) = if mv == target { (6, 100) } else { (MAX_PLY, 0) };
            searcher.tt.store(
                search_key(&child),
                depth,
                -root_score,
                None,
                Bound::Exact,
                None,
                1,
                false,
            );
        }
        assert_eq!(
            searcher
                .tt
                .probe(search_key(&target_child), 1)
                .unwrap()
                .value
                .unwrap()
                .depth,
            6
        );

        assert_eq!(
            searcher.search_root(&board, &moves, 8, -1, 200),
            Some((first, 0))
        );
        let target_nodes = searcher
            .current_root_nodes
            .iter()
            .find(|(mv, _)| *mv == target)
            .unwrap()
            .1;
        assert_eq!(target_nodes, 2, "減深探索と通常深さの再探索の全体を数える");
        let hit = searcher.tt.probe(search_key(&target_child), 1).unwrap();
        assert_eq!(
            (hit.value.unwrap().depth, hit.value.unwrap().score),
            (7, 0),
            "深さ6の値を超えた後に再探索する"
        );

        let partial = searcher
            .root_results
            .as_ref()
            .unwrap()
            .partial_result(Some(StopReason::HardLimit))
            .unwrap();
        assert_eq!((partial.score, partial.pv.as_slice()), (0, &[first][..]));
        assert_ne!(
            partial.score, 100,
            "減深探索の途中値はRootResultsへ記録しない"
        );
    });
}
