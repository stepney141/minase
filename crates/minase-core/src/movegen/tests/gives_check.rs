//! `Position::gives_check`の契約テスト。

use super::{generated_with, mv};
use crate::board::Square;
use crate::movegen::MoveGenerator;
use crate::mv::Move;
use crate::piece::{Color, PieceCode, PieceKind};
use crate::position::Position;
use crate::test_util::{
    bench_positions, capture_test_rules, position, position_from_codes,
    reflect_ranks_and_swap_colors, sampled_random_positions, sq,
};

fn piece(color: Color, kind: PieceKind) -> PieceCode {
    PieceCode::new(color, kind).expect("fixture uses an unpromoted-capable kind")
}

fn promoted(color: Color, kind: PieceKind) -> PieceCode {
    PieceCode::new_promoted(color, kind).expect("fixture uses a promoted-only kind")
}

fn assert_generated_check(
    position: &Position,
    generator: &MoveGenerator,
    mv: Move,
    expected: bool,
) {
    assert!(
        generated_with(generator, position).contains(&mv),
        "test move must be legal: {mv:?}"
    );
    let before = position.clone();
    assert_eq!(position.gives_check(generator, mv), expected, "mv={mv:?}");
    assert_eq!(
        position, &before,
        "gives_check must not change the position"
    );
}

/// 着手後の局面で、着手側に相手の残存王駒をすべて取る合法手があるかを調べる。
fn gives_check_reference(position: &Position, generator: &MoveGenerator, mv: Move) -> bool {
    let mover = position.side_to_move();
    let opponent = mover.opposite();
    let mut after = position.clone();
    after.make_move_unchecked(mv, generator.rules());
    let remaining_royals = after.royal_pieces(opponent);
    if remaining_royals.is_empty() {
        return false;
    }

    let checking_position = after.clone_with_side_to_move(mover);
    generated_with(generator, &checking_position)
        .into_iter()
        .any(|candidate| {
            let captured = checking_position.captured_squares(candidate);
            remaining_royals
                .into_iter()
                .all(|royal| captured.contains(&Some(royal)))
        })
}

#[test]
fn article_8_gives_check_matches_post_move_legal_capture_reference() {
    let mut compared = 0_usize;
    let mut checks = 0_usize;

    for rules in capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        let positions = bench_positions()
            .into_iter()
            .chain(sampled_random_positions(rules));
        for position in positions {
            for mv in generated_with(&generator, &position) {
                let expected = gives_check_reference(&position, &generator, mv);
                let actual = position.gives_check(&generator, mv);
                assert_eq!(actual, expected, "rules={rules:?}, mv={mv:?}");
                compared += 1;
                checks += usize::from(actual);
            }
        }
    }

    eprintln!("gives_check reference comparison: moves={compared}, checks={checks}");
    assert!(compared > 0, "the reference comparison must examine moves");
    assert!(
        checks > 0,
        "the reference comparison must include checking moves"
    );
}

#[test]
fn article_8_moved_piece_gives_direct_check() {
    let generator = MoveGenerator::standard();
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 2), Color::Black, PieceKind::Rook),
            (sq(5, 8), Color::White, PieceKind::King),
        ],
    );

    assert_generated_check(
        &board,
        &generator,
        mv(sq(5, 2), None, sq(5, 4), false),
        true,
    );
}

#[test]
fn article_8_moving_a_blocker_out_of_line_gives_discovered_check() {
    let generator = MoveGenerator::standard();
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 2), Color::Black, PieceKind::Rook),
            (sq(5, 4), Color::Black, PieceKind::SideMover),
            (sq(5, 8), Color::White, PieceKind::King),
        ],
    );

    assert_generated_check(
        &board,
        &generator,
        mv(sq(5, 4), None, sq(6, 4), false),
        true,
    );
}

#[test]
fn article_8_existing_slider_check_continues_unless_the_move_blocks_it() {
    let generator = MoveGenerator::standard();
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 2), Color::Black, PieceKind::Rook),
            (sq(4, 4), Color::Black, PieceKind::SideMover),
            (sq(5, 8), Color::White, PieceKind::King),
        ],
    );

    assert_generated_check(
        &board,
        &generator,
        mv(sq(4, 4), None, sq(3, 4), false),
        true,
    );
    assert_generated_check(
        &board,
        &generator,
        mv(sq(4, 4), None, sq(5, 4), false),
        false,
    );
}

#[test]
fn article_8_attacking_slider_can_move_away_along_the_same_line() {
    let generator = MoveGenerator::standard();
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Rook),
            (sq(5, 8), Color::White, PieceKind::King),
        ],
    );

    assert_generated_check(
        &board,
        &generator,
        mv(sq(5, 5), None, sq(5, 3), false),
        true,
    );
}

#[test]
fn articles_8_and_9_promotion_uses_the_promoted_piece_control() {
    let generator = MoveGenerator::standard();
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 7), Color::Black, PieceKind::Kirin),
            (sq(6, 9), Color::White, PieceKind::King),
        ],
    );

    assert_generated_check(
        &board,
        &generator,
        mv(sq(5, 7), None, sq(5, 9), false),
        false,
    );
    assert_generated_check(&board, &generator, mv(sq(5, 7), None, sq(5, 9), true), true);
}

#[test]
fn article_8_king_and_crown_prince_can_give_check() {
    let generator = MoveGenerator::standard();
    for moving_piece in [
        piece(Color::Black, PieceKind::King),
        promoted(Color::Black, PieceKind::CrownPrince),
    ] {
        let board = position_from_codes(
            Color::Black,
            &[
                (sq(4, 4), moving_piece),
                (sq(6, 6), piece(Color::White, PieceKind::King)),
            ],
        );
        assert_generated_check(
            &board,
            &generator,
            mv(sq(4, 4), None, sq(5, 5), false),
            true,
        );
    }
}

#[test]
fn articles_8_11_3_12_4_mid_capture_can_open_a_slider_check() {
    let generator = MoveGenerator::standard();

    let lion = position_from_codes(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(5, 1), piece(Color::Black, PieceKind::Rook)),
            (sq(4, 3), piece(Color::Black, PieceKind::Lion)),
            (sq(5, 4), piece(Color::White, PieceKind::Pawn)),
            (sq(5, 8), piece(Color::White, PieceKind::King)),
        ],
    );
    assert_generated_check(
        &lion,
        &generator,
        mv(sq(4, 3), Some(sq(5, 4)), sq(4, 5), false),
        true,
    );

    let falcon = position_from_codes(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(2, 2), piece(Color::Black, PieceKind::Bishop)),
            (sq(5, 4), promoted(Color::Black, PieceKind::HornedFalcon)),
            (sq(5, 5), piece(Color::White, PieceKind::Pawn)),
            (sq(8, 8), piece(Color::White, PieceKind::King)),
        ],
    );
    assert_generated_check(
        &falcon,
        &generator,
        mv(sq(5, 4), Some(sq(5, 5)), sq(5, 6), false),
        true,
    );
}

#[test]
fn article_8_7_two_royal_check_requires_adjacent_royals() {
    let generator = MoveGenerator::standard();

    let nonadjacent = position_from_codes(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(6, 5), piece(Color::Black, PieceKind::Lion)),
            (sq(5, 7), piece(Color::White, PieceKind::King)),
            (sq(7, 7), promoted(Color::White, PieceKind::CrownPrince)),
        ],
    );
    assert_generated_check(
        &nonadjacent,
        &generator,
        mv(sq(6, 5), None, sq(6, 6), false),
        false,
    );

    let adjacent = position_from_codes(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(5, 5), piece(Color::Black, PieceKind::Lion)),
            (sq(5, 7), piece(Color::White, PieceKind::King)),
            (sq(6, 7), promoted(Color::White, PieceKind::CrownPrince)),
        ],
    );
    assert_generated_check(
        &adjacent,
        &generator,
        mv(sq(5, 5), None, sq(5, 6), false),
        true,
    );
}

#[test]
fn article_8_capturing_one_royal_and_attacking_the_other_is_check() {
    let generator = MoveGenerator::standard();
    let board = position_from_codes(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(5, 2), piece(Color::Black, PieceKind::Rook)),
            (sq(5, 5), promoted(Color::White, PieceKind::CrownPrince)),
            (sq(5, 8), piece(Color::White, PieceKind::King)),
        ],
    );

    assert_generated_check(
        &board,
        &generator,
        mv(sq(5, 2), None, sq(5, 5), false),
        true,
    );
}

fn reflect_rank(square: Square) -> Square {
    sq(square.file(), 11 - square.rank())
}

fn reflect_move(mv: Move) -> Move {
    Move {
        from: reflect_rank(mv.from),
        mid: mv.mid.map(reflect_rank),
        to: reflect_rank(mv.to),
        promote: mv.promote,
    }
}

#[test]
fn article_8_directional_control_is_correct_for_both_colors() {
    let generator = MoveGenerator::standard();
    let black = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 4), Color::Black, PieceKind::Pawn),
            (sq(5, 6), Color::White, PieceKind::King),
        ],
    );
    let black_move = mv(sq(5, 4), None, sq(5, 5), false);
    assert_generated_check(&black, &generator, black_move, true);

    let white = reflect_ranks_and_swap_colors(&black);
    assert_generated_check(&white, &generator, reflect_move(black_move), true);
}

#[test]
fn articles_8_and_21_capturing_the_last_royal_is_not_check() {
    let generator = MoveGenerator::standard();
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 2), Color::Black, PieceKind::Rook),
            (sq(5, 5), Color::White, PieceKind::King),
        ],
    );

    assert_generated_check(
        &board,
        &generator,
        mv(sq(5, 2), None, sq(5, 5), false),
        false,
    );
}

#[test]
fn article_8_a_move_cannot_check_three_royals_at_once() {
    let generator = MoveGenerator::standard();
    let board = position_from_codes(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(5, 5), piece(Color::Black, PieceKind::Lion)),
            (sq(4, 7), piece(Color::White, PieceKind::King)),
            (sq(5, 7), piece(Color::White, PieceKind::King)),
            (sq(6, 7), promoted(Color::White, PieceKind::CrownPrince)),
        ],
    );

    assert_generated_check(
        &board,
        &generator,
        mv(sq(5, 5), None, sq(5, 6), false),
        false,
    );
}
