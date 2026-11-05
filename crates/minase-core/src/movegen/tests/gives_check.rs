//! `Position::gives_check`の契約テスト。

use super::{generated_with, mv, rule_variants};
use crate::Bitboard;
use crate::board::Square;
use crate::movegen::CheckContext;
use crate::movegen::MoveGenerator;
use crate::movegen::control::piece_control_with_occupancy;
use crate::mv::Move;
use crate::piece::{Color, PieceCode, PieceKind};
use crate::position::Position;
use crate::test_util::{
    bench_positions, position, position_from_codes, reflect_ranks_and_swap_colors,
    sampled_random_positions, sq,
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
    let mut context = CheckContext::default();
    assert_eq!(context.gives_check(position, generator, mv), expected);
    assert_eq!(context.gives_check(position, generator, mv), expected);
    assert_eq!(gives_check_before(position, generator, mv), expected);
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

    for rules in rule_variants() {
        let generator = MoveGenerator::new(rules);
        let positions = bench_positions()
            .into_iter()
            .chain(sampled_random_positions(rules));
        for position in positions {
            let mut context = CheckContext::default();
            for mv in generated_with(&generator, &position) {
                let expected = gives_check_reference(&position, &generator, mv);
                let actual = context.gives_check(&position, &generator, mv);
                assert_eq!(gives_check_before(&position, &generator, mv), expected);
                assert_eq!(position.gives_check(&generator, mv), expected);
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

/// 単一王駒でも全利きを求める変更前の判定。最適化の集合保存を検査する。
fn gives_check_before(position: &Position, generator: &MoveGenerator, mv: Move) -> bool {
    let mover = position.side_to_move();
    let captured = position.captured_squares(mv);
    let mut remaining_royals = position.royal_pieces(mover.opposite());
    for square in captured.into_iter().flatten() {
        remaining_royals.clear(square);
    }

    match remaining_royals.popcount() {
        0 => false,
        1 => {
            let royal = remaining_royals
                .lsb()
                .expect("one remaining royal must have a square");
            let mut occupied = position.occupied();
            occupied.clear(mv.from);
            if let Some(mid_capture) = captured[0] {
                occupied.clear(mid_capture);
            }
            occupied.set(mv.to);

            let moving_piece = position
                .piece_at(mv.from)
                .expect("move origin must contain a piece");
            let moving_kind = if mv.promote {
                moving_piece
                    .promote()
                    .expect("promoting move must have a promotable piece")
                    .kind()
                    .expect("promoted board piece must have a kind")
            } else {
                moving_piece
                    .kind()
                    .expect("board piece must have a valid kind")
            };
            if piece_control_with_occupancy(occupied, mover, moving_kind, mv.to).contains(royal) {
                return true;
            }

            let mut other_attackers = position.attackers_to_by(mover, royal, occupied);
            other_attackers.clear(mv.from);
            !other_attackers.is_empty()
        }
        2 => {
            let first = remaining_royals
                .lsb()
                .expect("two remaining royals must have a first square");
            let second = remaining_royals
                .msb()
                .expect("two remaining royals must have a second square");
            if !generator.tables().king_steps(first).contains(second) {
                return false;
            }

            let mut after = position.clone();
            after.make_move_unchecked(mv, generator.rules());
            let checking_position = after.clone_with_side_to_move(mover);
            can_capture_all_royals_before(&checking_position, generator, remaining_royals)
        }
        // 1手の最大捕獲数は2枚なので、到達不能な構築局面に3枚以上あっても王手ではない。
        _ => false,
    }
}

/// 手番側に`royals`の全駒を1手で取る合法手があるかを返す。
fn can_capture_all_royals_before(
    position: &Position,
    generator: &MoveGenerator,
    royals: Bitboard,
) -> bool {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    moves.into_iter().any(|mv| {
        let captured = position.captured_squares(mv);
        royals
            .into_iter()
            .all(|royal| captured.contains(&Some(royal)))
    })
}

/// 同じ局面で利きを共有しても、到達升が遮蔽する手には現在の占有の判定を流用しない。
#[test]
fn article_8_check_context_distinguishes_unrelated_moves_from_blockers() {
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
    let mut context = CheckContext::default();
    for (to, expected) in [(sq(3, 4), true), (sq(5, 4), false), (sq(2, 4), true)] {
        let candidate = mv(sq(4, 4), None, to, false);
        assert!(generated_with(&generator, &board).contains(&candidate));
        assert_eq!(context.gives_check(&board, &generator, candidate), expected);
        assert_eq!(gives_check_before(&board, &generator, candidate), expected);
    }
}

/// 第12条の獅子の跳びには、8方向の線に載らない2升先も含まれる。
#[test]
fn article_12_check_context_tracks_lion_jumps_outside_the_rays() {
    let generator = MoveGenerator::standard();
    let board = position(
        Color::Black,
        &[
            (sq(4, 6), Color::Black, PieceKind::Lion),
            (sq(5, 8), Color::White, PieceKind::King),
        ],
    );
    assert!(board.piece_control(sq(4, 6)).contains(sq(5, 8)));
    assert_generated_check(
        &board,
        &generator,
        mv(sq(4, 6), None, sq(3, 5), false),
        false,
    );
}

/// 逆引きの契約から、依存範囲外の占有を反転しても攻撃駒の集合は同じである。
#[test]
fn attack_dependencies_bound_all_occupancy_changes() {
    for board in bench_positions()
        .into_iter()
        .chain(sampled_random_positions(crate::rules::MoveRules::standard()))
    {
        for royal in Square::all() {
            let outside = !crate::attacks::attack_tables().attack_dependencies(royal);
            for color in Color::ALL {
                let occupied = board.occupied();
                assert_eq!(
                    board.attackers_to_by(color, royal, occupied),
                    board.attackers_to_by(color, royal, occupied ^ outside)
                );
            }
        }
    }
}
