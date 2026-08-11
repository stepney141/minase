use super::*;
use crate::core::piece::{Color, PieceKind};
use crate::test_util::{position, sq};

use super::tt::{Bound, pack_move, unpack_move};

fn engine_rules() -> Rules {
    Rules::engine_default()
}

fn root_moves(position: &Position) -> Vec<Move> {
    let mut moves = Vec::new();
    MoveGenerator::new(engine_rules()).generate_moves(position, &mut moves);
    moves
}

fn depth(depth: u32) -> SearchLimits {
    SearchLimits {
        depth: Some(depth),
        nodes: None,
        movetime_ms: None,
        clock: None,
        infinite: false,
    }
}

fn tt() -> TranspositionTable {
    TranspositionTable::new(1)
}

fn mv(from: (u8, u8), to: (u8, u8)) -> Move {
    Move {
        from: sq(from.0, from.1),
        mid: None,
        to: sq(to.0, to.1),
        promote: false,
    }
}

fn snapshot() -> SearchSnapshot {
    let position = Position::initial();
    SearchSnapshot {
        root_moves: root_moves(&position),
        history_keys: vec![search_key(&position)],
        position,
        rules: engine_rules(),
    }
}

fn receive_finished(handle: &SearchHandle) -> SearchEvent {
    loop {
        let event = handle
            .events()
            .recv_timeout(Duration::from_secs(2))
            .expect("search must finish within two seconds");
        if matches!(event, SearchEvent::Finished { .. }) {
            return event;
        }
    }
}

#[test]
fn depth_one_finds_capture_of_the_last_royal() {
    let position = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Rook),
            (sq(5, 4), Color::White, PieceKind::King),
        ],
    );
    let moves = root_moves(&position);
    let expected = mv((5, 5), (5, 4));
    let history = [search_key(&position)];

    let result = search(
        &position,
        engine_rules(),
        &moves,
        &history,
        &depth(1),
        &mut tt(),
    );

    assert_eq!(result.best_move, expected);
    assert_eq!(result.score, MATE);
}

#[test]
fn depth_one_prefers_a_free_high_value_capture() {
    let position = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Rook),
            (sq(5, 4), Color::White, PieceKind::Lion),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    let moves = root_moves(&position);
    let expected = mv((5, 5), (5, 4));

    let result = search(&position, engine_rules(), &moves, &[], &depth(1), &mut tt());

    assert_eq!(result.best_move, expected);
    assert!(result.score > 0);
    assert!(result.score < MATE_THRESHOLD);
}

#[test]
fn first_of_two_royals_is_material_not_mate() {
    let position = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Rook),
            (sq(5, 4), Color::White, PieceKind::CrownPrince),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    let moves = root_moves(&position);
    let expected = mv((5, 5), (5, 4));

    let result = search(&position, engine_rules(), &moves, &[], &depth(1), &mut tt());

    assert_eq!(result.best_move, expected);
    assert!(result.score > 0);
    assert!(result.score < MATE_THRESHOLD);
}

#[test]
fn repeated_child_position_scores_as_draw() {
    let position = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    let moves = root_moves(&position);
    let mut history = vec![search_key(&position)];
    for &candidate in &moves {
        let mut child = position.clone();
        child.make_move_unchecked(candidate, engine_rules());
        history.push(search_key(&child));
    }

    let result = search(
        &position,
        engine_rules(),
        &moves,
        &history,
        &depth(1),
        &mut tt(),
    );

    assert_eq!(result.score, DRAW_SCORE);
}

#[test]
fn same_input_produces_the_same_result() {
    let position = Position::initial();
    let moves = root_moves(&position);
    let history = [search_key(&position)];
    let config = depth(2);

    let first = search(
        &position,
        engine_rules(),
        &moves,
        &history,
        &config,
        &mut tt(),
    );
    let second = search(
        &position,
        engine_rules(),
        &moves,
        &history,
        &config,
        &mut tt(),
    );

    assert_eq!(first, second);
}

#[test]
fn node_limit_before_depth_one_returns_first_root_move() {
    let position = Position::initial();
    let moves = root_moves(&position);
    let limits = SearchLimits {
        depth: Some(2),
        nodes: Some(0),
        movetime_ms: None,
        clock: None,
        infinite: false,
    };

    let result = search(&position, engine_rules(), &moves, &[], &limits, &mut tt());

    assert_eq!(result.best_move, moves[0]);
    assert_eq!(result.depth, 0);
    assert_eq!(result.nodes, 0);
}

#[test]
fn limits_reject_an_unconstrained_search() {
    let limits = SearchLimits {
        depth: None,
        nodes: None,
        movetime_ms: None,
        clock: None,
        infinite: false,
    };

    assert_eq!(
        limits.validate(),
        Err("search limits must contain at least one constraint")
    );
    assert!(
        SearchLimits {
            infinite: true,
            ..limits
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn clock_budget_uses_integer_coefficients_and_safety_bounds() {
    let ordinary = clock_budget(ClockLimits {
        remaining_ms: 10_000,
        increment_ms: 1_000,
        byoyomi_ms: 500,
    });
    assert_eq!(ordinary.soft, Duration::from_millis(1_300));
    assert_eq!(ordinary.hard, Duration::from_millis(2_900));

    let safety_margin = clock_budget(ClockLimits {
        remaining_ms: 0,
        increment_ms: 0,
        byoyomi_ms: 100,
    });
    assert_eq!(safety_margin.soft, Duration::from_millis(80));
    assert_eq!(safety_margin.hard, Duration::from_millis(70));

    let minimum = clock_budget(ClockLimits {
        remaining_ms: 0,
        increment_ms: 0,
        byoyomi_ms: 0,
    });
    assert_eq!(minimum.soft, Duration::ZERO);
    assert_eq!(minimum.hard, Duration::from_millis(1));
}

#[test]
fn movetime_is_the_soft_and_hard_budget() {
    let limits = SearchLimits {
        depth: None,
        nodes: None,
        movetime_ms: Some(123),
        clock: None,
        infinite: false,
    };

    assert_eq!(
        time_budget(&limits),
        Some(TimeBudget {
            soft: Duration::from_millis(123),
            hard: Duration::from_millis(123),
        })
    );

    let clock = ClockLimits {
        remaining_ms: 10_000,
        increment_ms: 1_000,
        byoyomi_ms: 500,
    };
    assert_eq!(
        time_budget(&SearchLimits {
            clock: Some(clock),
            ..limits
        }),
        time_budget(&limits)
    );
    assert_eq!(
        time_budget(&SearchLimits {
            movetime_ms: Some(5_000),
            clock: Some(clock),
            ..limits
        }),
        Some(clock_budget(clock))
    );
}

#[test]
fn movetime_search_stops_near_the_fixed_budget() {
    let limits = SearchLimits {
        depth: None,
        nodes: None,
        movetime_ms: Some(100),
        clock: None,
        infinite: false,
    };
    let started = Instant::now();
    let handle = start_search(snapshot(), limits, 7, tt());
    let event = receive_finished(&handle);
    let wall_elapsed = started.elapsed();

    let SearchEvent::Finished {
        elapsed,
        stop_reason,
        ..
    } = event
    else {
        unreachable!();
    };
    assert!(elapsed >= Duration::from_millis(90));
    assert!(wall_elapsed < Duration::from_secs(2));
    assert!(matches!(
        stop_reason,
        StopReason::SoftLimit | StopReason::HardLimit
    ));
    handle.join().expect("search thread must not panic");
}

#[test]
fn infinite_search_stops_only_after_an_external_request() {
    let limits = SearchLimits {
        depth: Some(1),
        nodes: Some(0),
        movetime_ms: Some(0),
        clock: Some(ClockLimits {
            remaining_ms: 0,
            increment_ms: 0,
            byoyomi_ms: 0,
        }),
        infinite: true,
    };
    let handle = start_search(snapshot(), limits, 8, tt());
    handle.request_stop();
    let event = receive_finished(&handle);

    assert!(matches!(
        event,
        SearchEvent::Finished {
            search_id: 8,
            stop_reason: StopReason::ExternalStop,
            ..
        }
    ));
    handle.join().expect("search thread must not panic");
}

#[test]
fn stop_before_depth_one_returns_the_first_legal_move() {
    let snapshot = snapshot();
    let first_move = snapshot.root_moves[0];
    let limits = SearchLimits {
        depth: Some(2),
        nodes: Some(0),
        movetime_ms: None,
        clock: None,
        infinite: false,
    };
    let handle = start_search(snapshot, limits, 9, tt());
    let event = receive_finished(&handle);

    assert!(matches!(
        event,
        SearchEvent::Finished {
            best_move,
            depth: 0,
            nodes: 0,
            stop_reason: StopReason::NodeLimit,
            ..
        } if best_move == first_move
    ));
    handle.join().expect("search thread must not panic");
}

#[test]
fn caller_discards_a_delayed_event_with_a_different_search_id() {
    let limits = SearchLimits {
        depth: None,
        nodes: Some(0),
        movetime_ms: None,
        clock: None,
        infinite: false,
    };
    let stale_handle = start_search(snapshot(), limits, 10, tt());
    let stale = receive_finished(&stale_handle);
    let current_handle = start_search(snapshot(), limits, 11, tt());
    let current = receive_finished(&current_handle);

    let accepted: Vec<_> = [stale, current]
        .into_iter()
        .filter(|event| event.search_id() == 11)
        .collect();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].search_id(), 11);
    stale_handle.join().expect("search thread must not panic");
    current_handle.join().expect("search thread must not panic");
}

#[test]
fn progress_depths_increase_and_join_returns_a_reusable_table() {
    let limits = depth(3);
    let handle = start_search(snapshot(), limits, 12, tt());
    let mut progress_depths = Vec::new();
    let mut progress_elapsed = Vec::new();
    loop {
        match handle
            .events()
            .recv_timeout(Duration::from_secs(2))
            .expect("search must finish within two seconds")
        {
            SearchEvent::Progress {
                search_id,
                depth,
                elapsed,
                pv,
                ..
            } => {
                assert_eq!(search_id, 12);
                assert!(!pv.is_empty());
                progress_depths.push(depth);
                progress_elapsed.push(elapsed);
            }
            SearchEvent::Finished {
                search_id,
                stop_reason,
                ..
            } => {
                assert_eq!(search_id, 12);
                assert_eq!(stop_reason, StopReason::DepthCompleted);
                break;
            }
        }
    }
    assert_eq!(progress_depths, vec![1, 2, 3]);
    assert!(progress_elapsed.windows(2).all(|pair| pair[0] <= pair[1]));

    let mut returned_tt = handle.join().expect("search thread must not panic");
    let snapshot = snapshot();
    let result = search(
        &snapshot.position,
        snapshot.rules,
        &snapshot.root_moves,
        &snapshot.history_keys,
        &depth(1),
        &mut returned_tt,
    );
    assert_eq!(result.depth, 1);
}

#[test]
fn packed_moves_round_trip_for_all_legal_moves_in_multiple_positions() {
    let positions = [
        Position::initial(),
        position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(5, 5), Color::Black, PieceKind::Lion),
                (sq(5, 6), Color::White, PieceKind::Pawn),
                (sq(6, 6), Color::White, PieceKind::Rook),
                (sq(11, 11), Color::White, PieceKind::King),
            ],
        ),
        position(
            Color::White,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(4, 8), Color::Black, PieceKind::Pawn),
                (sq(6, 6), Color::White, PieceKind::Bishop),
                (sq(11, 11), Color::White, PieceKind::King),
            ],
        ),
    ];

    let mut saw_intermediate = false;
    let mut saw_promotion = false;
    for position in positions {
        let moves = root_moves(&position);
        assert!(!moves.is_empty());
        for mv in moves {
            saw_intermediate |= mv.mid.is_some();
            saw_promotion |= mv.promote;
            assert_eq!(unpack_move(pack_move(mv)), mv);
        }
    }
    assert!(saw_intermediate);
    assert!(saw_promotion);
}

#[test]
fn mate_scores_round_trip_through_the_table() {
    let best_move = mv((0, 0), (0, 1));
    let mut table = tt();
    table.new_search();

    let positive_key = 0x1111_1111_0000_0001;
    table.store(positive_key, 8, MATE - 23, Bound::Exact, best_move, 23);
    assert_eq!(table.probe(positive_key, 23).unwrap().score, MATE - 23);
    assert_eq!(table.probe(positive_key, 7).unwrap().score, MATE - 7);

    let negative_key = 0x2222_2222_0000_0002;
    table.store(negative_key, 8, -MATE + 23, Bound::Exact, best_move, 23);
    assert_eq!(table.probe(negative_key, 23).unwrap().score, -MATE + 23);
    assert_eq!(table.probe(negative_key, 7).unwrap().score, -MATE + 7);

    let ordinary_key = 0x3333_3333_0000_0003;
    table.store(ordinary_key, 8, 12_345, Bound::Exact, best_move, 23);
    assert_eq!(table.probe(ordinary_key, 7).unwrap().score, 12_345);
}

#[test]
fn replacement_rules_cover_same_key_age_and_depth() {
    let best_move = mv((0, 0), (0, 1));
    let key_a = 0x1111_1111_0000_0001;
    let key_b = 0x2222_2222_0000_0001;

    let mut table = tt();
    table.new_search();
    table.store(key_a, 8, 100, Bound::Exact, best_move, 0);
    table.store(key_a, 1, 200, Bound::Upper, best_move, 0);
    let hit = table.probe(key_a, 0).unwrap();
    assert_eq!(hit.score, 200);
    assert_eq!(hit.depth, 1);
    assert_eq!(hit.bound, Bound::Upper);

    table.clear();
    table.new_search();
    table.store(key_a, 8, 100, Bound::Exact, best_move, 0);
    table.store(key_b, 7, 200, Bound::Exact, best_move, 0);
    assert!(table.probe(key_a, 0).is_some());
    assert!(table.probe(key_b, 0).is_none());
    table.store(key_b, 9, 300, Bound::Lower, best_move, 0);
    assert!(table.probe(key_a, 0).is_none());
    assert_eq!(table.probe(key_b, 0).unwrap().score, 300);

    table.clear();
    table.new_search();
    table.store(key_a, 200, 100, Bound::Exact, best_move, 0);
    table.new_search();
    table.store(key_b, 1, 400, Bound::Exact, best_move, 0);
    assert!(table.probe(key_a, 0).is_none());
    assert_eq!(table.probe(key_b, 0).unwrap().score, 400);
}
