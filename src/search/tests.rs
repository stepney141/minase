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

fn depth(depth: u32) -> SearchConfig {
    SearchConfig { depth, nodes: None }
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
    let config = SearchConfig {
        depth: 2,
        nodes: Some(0),
    };

    let result = search(&position, engine_rules(), &moves, &[], &config, &mut tt());

    assert_eq!(result.best_move, moves[0]);
    assert_eq!(result.depth, 0);
    assert_eq!(result.nodes, 0);
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
