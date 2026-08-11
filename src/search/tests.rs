use super::*;
use crate::core::piece::{Color, PieceKind};
use crate::test_util::{position, sq};

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

    let result = search(&position, engine_rules(), &moves, &history, &depth(1));

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

    let result = search(&position, engine_rules(), &moves, &[], &depth(1));

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

    let result = search(&position, engine_rules(), &moves, &[], &depth(1));

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

    let result = search(&position, engine_rules(), &moves, &history, &depth(1));

    assert_eq!(result.score, DRAW_SCORE);
}

#[test]
fn same_input_produces_the_same_result() {
    let position = Position::initial();
    let moves = root_moves(&position);
    let history = [search_key(&position)];
    let config = depth(2);

    let first = search(&position, engine_rules(), &moves, &history, &config);
    let second = search(&position, engine_rules(), &moves, &history, &config);

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

    let result = search(&position, engine_rules(), &moves, &[], &config);

    assert_eq!(result.best_move, moves[0]);
    assert_eq!(result.depth, 0);
    assert_eq!(result.nodes, 0);
}
