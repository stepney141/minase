//! time-management-efficiency.md第1段階の採用条件と停止条件。
use super::root_moves_tests::with_root_results;
use super::*;
use crate::core::piece::{Color, PieceKind};
use crate::eval::weights;
use crate::test_util::{position, sq};

#[test]
fn partial_requires_previous_best_complete_and_a_score_above_original_alpha() {
    with_root_results(true, None, |_, moves, searcher| {
        let results = searcher.root_results.as_mut().unwrap();
        results.previous_best = Some(moves[0]);
        results.record(moves[1], Some(20), (0, 100), &[], 1);
        assert!(results.partial_result(0).is_none());
        // None also represents interruption during the full-width re-search.
        results.record(moves[0], None, (0, 100), &[], 2);
        assert!(results.partial_result(0).is_none());
        results.record(moves[0], Some(10), (0, 100), &[], 1);
        assert!(results.partial_result(20).is_none());
        assert!(results.partial_result(21).is_none());
        let partial = results.partial_result(0).unwrap();
        assert_eq!(partial.mv, moves[1]);
        assert_eq!(partial.score, Some((20, Bound::Exact)));
        assert_eq!(partial.pv, [moves[1]]);
        results.begin_window();
        assert!(results.partial_result(0).is_none());
    });
}

#[test]
fn root_prioritizes_previous_best_over_tt_and_only_adopts_hard_interruptions() {
    for reason in [
        StopReason::HardLimit,
        StopReason::NodeLimit,
        StopReason::ExternalStop,
    ] {
        with_root_results(true, None, |position, moves, searcher| {
            let completed = searcher.search_iteration(position, moves, 1, None).unwrap();
            let previous = completed.value.0;
            let other = *moves.iter().find(|&&mv| mv != previous).unwrap();
            searcher
                .tt
                .store(search_key(position), 1, 0, Bound::Exact, Some(other), 0);
            // The root and first child finish; the second child is interrupted.
            searcher.interrupt_at = Some((searcher.nodes + 2, reason));
            let result = searcher.search_iteration(position, moves, 2, Some(0));
            if reason == StopReason::HardLimit {
                assert_eq!(
                    result,
                    Some(RootResult {
                        value: (previous, DRAW_SCORE),
                        partial: true
                    })
                );
                assert_eq!(searcher.pv[0], [previous]);
            } else {
                assert_eq!(result, None);
            }
            assert_eq!(searcher.stop_reason, Some(reason));
            // An interrupted root must not overwrite the existing TT entry.
            assert_eq!(
                searcher
                    .tt
                    .probe(search_key(position), 0)
                    .unwrap()
                    .best_move,
                Some(other)
            );
        });
    }
}

#[test]
fn interrupted_aspiration_window_does_not_reuse_previous_window_lower_bound() {
    with_root_results(true, None, |position, moves, searcher| {
        searcher.search_iteration(position, moves, 1, None).unwrap();
        // First window fails high at zero; the widened window stops before its first child.
        searcher.interrupt_at = Some((searcher.nodes + 3, StopReason::HardLimit));
        let delta = searcher.pst.pawn_value() / 2;
        assert_eq!(
            searcher.search_iteration(position, moves, 5, Some(-delta)),
            None
        );
        assert!(
            searcher
                .root_results
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .all(|entry| entry.score.is_none())
        );
    });
}

#[test]
fn worker_selection_prefers_depth_then_partial_then_lowest_index() {
    with_root_results(true, None, |_, moves, _| {
        let make = |worker_index, depth, partial| WorkerOutcome {
            worker_index,
            partial,
            result: SearchResult {
                best_move: moves[worker_index],
                score: 10,
                depth,
                nodes: 0,
            },
            pv: vec![moves[worker_index]],
            nodes: 0,
        };
        let mut outcomes = vec![make(2, 3, false), make(0, 3, true), make(1, 3, false)];
        assert_eq!(select_worker_outcome(&outcomes), &outcomes[1]);
        outcomes[0].result.depth = 4;
        assert_eq!(select_worker_outcome(&outcomes), &outcomes[0]);
        outcomes[1].result.depth = 2;
        outcomes[0].result.depth = 3;
        assert_eq!(select_worker_outcome(&outcomes), &outcomes[2]);
        // Directly exercise the second key independently of main-worker numbering.
        outcomes[1].result.depth = 3;
        outcomes[1].worker_index = 3;
        outcomes.push(make(0, 0, false));
        assert_eq!(select_worker_outcome(&outcomes), &outcomes[1]);
    });
}

/// A king boxed in by immobile pawns has only the move to (0, 10).
fn forced_position() -> Position {
    position(
        Color::Black,
        &[
            (sq(0, 11), Color::Black, PieceKind::King),
            (sq(1, 11), Color::Black, PieceKind::Pawn),
            (sq(1, 10), Color::Black, PieceKind::Pawn),
            (sq(5, 0), Color::White, PieceKind::King),
        ],
    )
}

#[test]
fn forced_move_stops_at_depth_one_only_with_a_time_budget() {
    let position = forced_position();
    let rules = MoveRules::standard();
    let mut moves = Vec::new();
    MoveGenerator::new(rules).generate_moves(&position, &mut moves);
    assert_eq!(moves.len(), 1);
    let pst = weights().unwrap();
    for threads in [1, 4] {
        for limits in [
            SearchLimits::new(None, None, Some(10_000), None).unwrap(),
            SearchLimits::new(
                None,
                None,
                None,
                Some(ClockLimits::new(300_000, 0, 10_000, 0).unwrap()),
            )
            .unwrap(),
            SearchLimits::new(Some(3), None, None, None).unwrap(),
            SearchLimits::new(None, Some(100), None, None).unwrap(),
            SearchLimits::infinite(),
        ] {
            let external_stop = AtomicBool::new(limits.is_infinite());
            let outcome = run_search_team(
                &pst,
                &position,
                rules,
                &moves,
                &[search_key(&position)],
                &limits,
                &external_stop,
                NonZeroUsize::new(threads).unwrap(),
                &TranspositionTable::new(1).unwrap(),
                None,
            );
            assert_eq!(outcome.result.best_move, moves[0]);
            assert!(!outcome.partial);
            if time_budget(&limits).is_some() {
                assert_eq!(outcome.result.depth, 1);
                assert_eq!(outcome.stop_reason, StopReason::SoftLimit);
                assert!(outcome.forced);
            } else {
                assert!(!outcome.forced);
                assert_ne!(outcome.stop_reason, StopReason::SoftLimit);
                if !limits.is_infinite() {
                    assert!(outcome.result.depth > 1);
                }
            }
        }
    }
}

#[test]
fn partial_root_rejects_an_unfinished_previous_best_and_fail_low() {
    for (completed_nodes, alpha) in [(1, -1), (2, 0), (2, 1)] {
        with_root_results(true, None, |position, moves, searcher| {
            searcher.search_iteration(position, moves, 1, None).unwrap();
            searcher.root_results.as_mut().unwrap().begin_iteration();
            searcher.interrupt_at = Some((searcher.nodes + completed_nodes, StopReason::HardLimit));
            assert_eq!(searcher.search_root(position, moves, 2, alpha, 2), None);
        });
    }
}

#[test]
fn partial_root_can_adopt_a_different_best_move() {
    let position = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Rook),
            (sq(5, 11), Color::White, PieceKind::King),
        ],
    );
    let rules = MoveRules::standard();
    let mut moves = Vec::new();
    MoveGenerator::new(rules).generate_moves(&position, &mut moves);
    let previous = *moves
        .iter()
        .find(|&&mv| !captures_last_royal(&position, mv))
        .unwrap();
    let winning = *moves
        .iter()
        .find(|&&mv| captures_last_royal(&position, mv))
        .unwrap();
    let history: Vec<_> = moves
        .iter()
        .map(|&mv| {
            let mut child = position.clone();
            child.make_move_unchecked(mv, rules);
            search_key(&child)
        })
        .collect();
    let external_stop = AtomicBool::new(false);
    let shared = SharedSearch {
        external_stop: &external_stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        node_limit: None,
        started: Instant::now(),
        hard_limit: None,
    };
    let pst = weights().unwrap();
    let tt = TranspositionTable::new(1).unwrap();
    let mut results = RootMoves::new(&moves);
    results.record(previous, Some(0), (-1, 1), &[], 1);
    let mut searcher = new_searcher(&pst, &position, rules, &history, &shared, &tt);
    searcher.root_results = Some(&mut results);
    searcher.interrupt_at = Some((3, StopReason::HardLimit));
    assert_eq!(
        searcher.search_iteration(&position, &moves, 2, Some(0)),
        Some(RootResult {
            value: (winning, MATE),
            partial: true
        })
    );
    assert_eq!(searcher.pv[0], [winning]);
    assert!(tt.probe(search_key(&position), 0).is_none());
}

#[test]
fn full_width_research_interruption_leaves_the_move_unfinished() {
    with_root_results(false, None, |position, moves, searcher| {
        let mv = moves[0];
        let alpha = -20_000;
        let beta = 20_000;
        // Count the scout alone, then interrupt at entry to the full-width search.
        let mut child = position.clone();
        let undo = child.make_move_unchecked(mv, searcher.rules);
        searcher.accumulators[1] =
            searcher
                .pst
                .update_accumulator_after_move(searcher.accumulators[0], &child, &undo);
        searcher.path_keys.push(search_key(&child));
        let scout = -searcher
            .negamax(&mut child, 0, -alpha - 1, -alpha, 1)
            .unwrap();
        searcher.path_keys.pop();
        assert!(scout > alpha && scout < beta);
        let scout_nodes = searcher.nodes;
        searcher.interrupt_at = Some((searcher.nodes + scout_nodes, StopReason::HardLimit));
        let score = searcher.search_move(&mut position.clone(), mv, 1, alpha, beta, 0, false, 0);
        assert_eq!(score, None);
        assert_eq!(searcher.stop_reason, Some(StopReason::HardLimit));
        let results = searcher.root_results.as_mut().unwrap();
        results.previous_best = Some(mv);
        results.record(moves[1], Some(0), (alpha, beta), &[], 1);
        results.record(mv, score, (alpha, beta), &searcher.pv[1], scout_nodes);
        assert!(results.partial_result(alpha).is_none());
    });
}

#[test]
fn equal_upper_bound_does_not_replace_the_completed_best() {
    with_root_results(true, None, |_, moves, searcher| {
        let results = searcher.root_results.as_mut().unwrap();
        results.previous_best = Some(moves[1]);
        results.record(moves[1], Some(20), (0, 100), &[], 1);
        results.record(moves[0], Some(20), (20, 100), &[], 1);
        assert_eq!(results.partial_result(0).unwrap().mv, moves[1]);
        results.begin_iteration();
        assert_eq!(results.previous_best(), Some(moves[1]));
    });
}

#[test]
fn infinite_forced_position_continues_past_depth_one_until_external_stop() {
    let position = forced_position();
    let rules = MoveRules::standard();
    let mut moves = Vec::new();
    MoveGenerator::new(rules).generate_moves(&position, &mut moves);
    let pst = weights().unwrap();
    let external_stop = AtomicBool::new(false);
    let (sender, receiver) = mpsc::channel();
    thread::scope(|scope| {
        let stop = &external_stop;
        scope.spawn(move || {
            loop {
                let event = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
                if matches!(event, SearchEvent::Progress { depth: 2, .. }) {
                    stop.store(true, AtomicOrdering::Release);
                    break;
                }
            }
        });
        let outcome = run_search_team(
            &pst,
            &position,
            rules,
            &moves,
            &[search_key(&position)],
            &SearchLimits::infinite(),
            &external_stop,
            NonZeroUsize::new(1).unwrap(),
            &TranspositionTable::new(1).unwrap(),
            Some((&sender, 1)),
        );
        assert!(outcome.result.depth >= 2);
        assert_eq!(outcome.stop_reason, StopReason::ExternalStop);
        assert!(!outcome.forced);
    });
}

/// 時間超過の実測に使う15局面について、根からの静止探索の規模を記録する。
#[test]
#[ignore = "releaseで実行する時間管理の診断"]
fn report_quiescence_nodes_for_clock_fixtures() {
    let pst = weights().unwrap();
    for (index, position) in crate::test_util::bench_positions().iter().enumerate() {
        let external_stop = AtomicBool::new(false);
        let shared = SharedSearch {
            external_stop: &external_stop,
            team_stop: AtomicBool::new(false),
            stop_reason: AtomicU8::new(0),
            total_nodes: AtomicU64::new(0),
            node_limit: None,
            started: Instant::now(),
            hard_limit: None,
        };
        let tt = TranspositionTable::new(1).unwrap();
        let history = [search_key(position)];
        let mut searcher = new_searcher(
            &pst,
            position,
            MoveRules::standard(),
            &history,
            &shared,
            &tt,
        );
        assert!(
            searcher
                .quiesce(&mut position.clone(), -INFINITY, INFINITY, 0)
                .is_some()
        );
        eprintln!(
            "fixture={index} qnodes={} elapsed_ms={:.3}",
            searcher.nodes,
            shared.started.elapsed().as_secs_f64() * 1000.0
        );
    }
}
