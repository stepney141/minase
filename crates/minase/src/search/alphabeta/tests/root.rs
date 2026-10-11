//! 根の探索と探索窓を検査する。

use super::*;

// move-ordering.md「候補7」。完了反復だけを保存し、次の反復では先頭以外を整列する。
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
            prev = Some(score);
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
            searcher
                .tt
                .store(search_key(&position), 1, 0, Bound::Exact, Some(tt_move), 0);
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

#[test]
fn interrupted_root_iteration_discards_only_current_counts() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    let history = repeated_root_children(&position, &moves);
    with_root_searcher(&position, &history, |searcher| {
        searcher
            .search_iteration(&position, &moves, 2, None)
            .unwrap();
        let completed = searcher.completed_root_nodes.clone();
        // 最初のfail-high窓を完了し、読み直しの途中で止める。
        searcher.interrupt_at = Some((searcher.nodes + 2, StopReason::NodeLimit));
        let delta = aspiration_delta(searcher.pst.pawn_value());
        assert!(
            searcher
                .search_iteration(&position, &moves, 5, Some(-delta))
                .is_none()
        );
        assert_eq!(searcher.completed_root_nodes, completed);
        assert!(searcher.current_root_nodes.is_empty());
    });
}

// docs/plans/strength-stage6.md「窓の適用条件と拡大」「検証」。
#[test]
fn aspiration_initial_window_uses_only_eligible_previous_scores() {
    for depth in 1..=4 {
        let window = AspirationWindow::initial(depth, Some(100), 50);
        assert_eq!((window.alpha, window.beta), (-INFINITY, INFINITY));
    }
    for prev in [
        None,
        Some(MATE_THRESHOLD),
        Some(-MATE_THRESHOLD),
        Some(MATE),
        Some(-MATE),
    ] {
        let window = AspirationWindow::initial(5, prev, 50);
        assert_eq!((window.alpha, window.beta), (-INFINITY, INFINITY));
    }
    for depth in [5, 6, 8] {
        let window = AspirationWindow::initial(depth, Some(100), 50);
        assert_eq!((window.alpha, window.beta, window.delta), (50, 150, 50));
    }
}

#[test]
fn aspiration_widens_only_the_failed_side() {
    let growth = crate::search::alphabeta::params::aspiration_growth();
    let delta1 = 50 * growth / 100;
    let delta2 = delta1 * growth / 100;
    let delta3 = delta2 * growth / 100;
    let delta4 = delta3 * growth / 100;
    let delta5 = delta4 * growth / 100;
    let mut low = AspirationWindow::initial(5, Some(100), 50);
    low.widen_low();
    assert_eq!((low.alpha, low.beta, low.delta), (0, 150, delta1));
    low.widen_low();
    assert_eq!((low.alpha, low.beta, low.delta), (-delta1, 150, delta2));

    let mut high = AspirationWindow::initial(5, Some(100), 50);
    high.widen_high();
    assert_eq!((high.alpha, high.beta, high.delta), (50, 200, delta1));
    high.widen_high();
    assert_eq!(
        (high.alpha, high.beta, high.delta),
        (50, 200 + delta1, delta2)
    );

    low.widen_high();
    assert_eq!(
        (low.alpha, low.beta, low.delta),
        (-delta1, 150 + delta2, delta3)
    );
    low.widen_low();
    assert_eq!(
        (low.alpha, low.beta, low.delta),
        (-delta1 - delta3, 150 + delta2, delta4)
    );
    low.widen_high();
    assert_eq!(
        (low.alpha, low.beta, low.delta),
        (-delta1 - delta3, 150 + delta2 + delta4, delta5)
    );
    // 最小の拡大率でも詰み帯を越える回数だけ両側を広げる。
    for _ in 0..32 {
        low.widen_low();
        low.widen_high();
    }
    assert_eq!((low.alpha, low.beta), (-INFINITY, INFINITY));
}

#[test]
fn aspiration_normalizes_initial_and_widened_mate_edges() {
    for distance in [49, 50, 51, 100] {
        let mut high = AspirationWindow::initial(5, Some(MATE_THRESHOLD - distance), 50);
        let mut low = AspirationWindow::initial(5, Some(-MATE_THRESHOLD + distance), 50);
        if distance <= 50 {
            assert_eq!(high.beta, INFINITY);
            assert_eq!(low.alpha, -INFINITY);
        } else {
            assert_eq!(high.beta, MATE_THRESHOLD - distance + 50);
            assert_eq!(low.alpha, -MATE_THRESHOLD + distance - 50);
        }
        let unchanged_alpha = high.alpha;
        let unchanged_beta = low.beta;
        for _ in 0..3 {
            high.widen_high();
            low.widen_low();
            assert_eq!((high.alpha, high.beta), (unchanged_alpha, INFINITY));
            assert_eq!((low.alpha, low.beta), (-INFINITY, unchanged_beta));
        }
    }
}

// 同「根の探索の窓化」。引き分け値0を境界に置き、等号とβ打ち切りを検査する。
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
            assert_eq!(hit.bound, bound);
            assert_eq!(hit.score, score);
            assert_eq!(hit.best_move, Some(best_move));
        });
    }
}

// 同「aspiration windows」。s == α、s == βのどちらも再探索を要する。
// 再探索では全合法手を適用するため、初回に適用した着手数へ合法手数を加える。
#[test]
fn aspiration_researches_scores_equal_to_either_edge() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    let history = repeated_root_children(&position, &moves);
    let delta = aspiration_delta(weights().unwrap().pawn_value());
    for (prev, first_nodes) in [(delta, moves.len() as u64), (-delta, 1)] {
        with_root_searcher(&position, &history, |searcher| {
            let (_, score) = searcher
                .search_iteration(&position, &moves, 5, Some(prev))
                .unwrap();
            assert_eq!(score, DRAW_SCORE);
            assert_eq!(searcher.nodes, first_nodes + moves.len() as u64);
            assert!(searcher.current_root_nodes.is_empty());
            assert_eq!(searcher.completed_root_nodes.len(), moves.len());
            for (index, &(_, nodes)) in searcher.completed_root_nodes.iter().enumerate() {
                assert_eq!(nodes, if prev == delta || index == 0 { 2 } else { 1 });
            }
            assert_eq!(
                searcher.tt.probe(search_key(&position), 0).unwrap().bound,
                Bound::Exact
            );
        });
    }
}

// 同「根の探索の窓化」。fail-lowでもαの更新と独立に最善手を保存し、
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
        searcher
            .tt
            .store(key, 0, DRAW_SCORE, Bound::Exact, Some(quiet), 0);
        let (best_move, score) = searcher
            .search_root(&position, &moves, 1, MATE, INFINITY)
            .unwrap();
        assert_eq!(score, MATE - 1);
        assert!(captures_last_royal(&position, best_move));
        let hit = searcher.tt.probe(key, 0).unwrap();
        assert_eq!(hit.bound, Bound::Upper);
        assert_eq!(hit.best_move, Some(best_move));
        let before = searcher.nodes;
        assert_eq!(
            searcher.search_root(&position, &moves, 1, -1, 0),
            Some((best_move, MATE - 1))
        );
        assert_eq!(searcher.nodes - before, 0);
        assert_eq!(searcher.tt.probe(key, 0).unwrap().bound, Bound::Lower);
    });
}

// 同「窓外れの報告」「検証」。深さ5の最初の窓外れを実測してノード予算を
// 決め、読み直しで1手を適用した後に中断させる。経過時間やスケジューリングに依存しない。
#[test]
fn aspiration_interruption_preserves_the_last_completed_iteration() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    let history = [search_key(&position)];
    let mut node_limit = 0;
    let mut completed = None;
    let mut completed_pv = Vec::new();
    with_root_searcher(&position, &history, |searcher| {
        for depth in 1..=4 {
            completed = searcher.search_iteration(
                &position,
                &moves,
                depth,
                completed.map(|(_, score)| score),
            );
            assert!(completed.is_some());
        }
        completed_pv.clone_from(&searcher.pv[0]);
        let (_, prev) = completed.unwrap();
        let delta = searcher.pst.pawn_value() / 2;
        let (_, score) = searcher
            .search_root(&position, &moves, 5, prev - delta, prev + delta)
            .unwrap();
        assert!(
            score >= prev + delta,
            "fixture must fail high before the interruption"
        );
        assert_eq!(
            searcher.tt.probe(search_key(&position), 0).unwrap().bound,
            Bound::Lower
        );
        node_limit = searcher.nodes + 1;
    });

    let handle = start(
        snapshot_for(&position),
        nodes_limits(node_limit),
        85,
        worker_count(1),
        small_tt(),
    );
    let (progress, finished) = event_reports(drain_raw(&handle));
    handle.join().expect("search thread must not panic");
    assert_eq!(
        progress.iter().map(|entry| entry.0).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(finished.depth, 4);
    assert_eq!((finished.best_move, finished.score), completed.unwrap());
    assert_eq!(finished.pv, completed_pv);
    assert_eq!(finished.nodes, node_limit);
    assert_eq!(finished.stop_reason, StopReason::NodeLimit);
    let last = progress.last().unwrap();
    assert_eq!(last.1, finished.score);
    assert_eq!(last.4, finished.pv);
    assert!(last.2 < finished.nodes);
}
