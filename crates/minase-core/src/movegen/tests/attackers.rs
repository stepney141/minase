//! `Position::attackers_to_by`の契約テスト。
//!
//! 期待値は[`Position::attackers_to_by`]の契約である、
//! [`piece_control_with_occupancy`]による駒別利きとの同値関係から導く。

use crate::attacks::{AttackTables, SpecialMovement, movement_profile, movement_profile_data};
use crate::board::Square;
use crate::board::{Bitboard, Direction, step_square};
use crate::movegen::control::piece_control_with_occupancy;
use crate::piece::{Color, PieceKind};
use crate::position::Position;
use crate::rules::MoveRules;
use crate::test_util::{bench_positions, position, sampled_random_positions};

use super::msq;

fn assert_attackers_match_piece_controls(position: &Position, occupied: Bitboard) {
    for target in Square::all() {
        for color in Color::ALL {
            let attackers = position.attackers_to_by(color, target, occupied);
            for from in position.occupied() {
                let piece = position
                    .piece_at(from)
                    .expect("occupied square has a piece");
                let piece_color = piece.color().expect("board piece has a color");
                let expected = piece_color == color
                    && occupied.contains(from)
                    && piece_control_with_occupancy(
                        occupied,
                        piece_color,
                        piece.kind().expect("board piece has a kind"),
                        from,
                    )
                    .contains(target);
                assert_eq!(
                    attackers.contains(from),
                    expected,
                    "color={color:?}, from={from:?}, target={target:?}, occupied={occupied:?}"
                );
            }
        }
    }
}

/// [`Position::attackers_to_by`]の契約に従い、初期局面を含むbenchの15局面と
/// ランダム対局の標本で、各色の逆引き集合と駒別利きの同値を全升で検査する。
#[test]
fn attackers_to_by_matches_piece_control_on_reference_positions() {
    let positions = bench_positions()
        .into_iter()
        .chain(sampled_random_positions(MoveRules::standard()));

    for position in positions {
        let occupied = position.occupied();
        assert_attackers_match_piece_controls(&position, occupied);

        if let Some(removed) = occupied.lsb() {
            let mut without_one = occupied;
            without_one.clear(removed);
            assert_attackers_match_piece_controls(&position, without_one);
        }

        let mut without_several = occupied;
        for (index, removed) in occupied.into_iter().enumerate() {
            if index % 5 == 0 {
                without_several.clear(removed);
            }
        }
        assert_attackers_match_piece_controls(&position, without_several);
    }
}

/// 公開の駒別疑似利きは、現在の占有状態を使う既存の利きの定義と一致する。
#[test]
fn piece_control_covers_fixed_sliding_and_two_stage_destinations() {
    let fixed = position(Color::Black, &[(msq(6, 6), Color::Black, PieceKind::Pawn)]);
    assert_eq!(
        fixed.piece_control(msq(6, 6)),
        Bitboard::from_square(msq(6, 5))
    );

    let sliding = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::Rook),
            (msq(6, 3), Color::Black, PieceKind::Pawn),
            (msq(6, 9), Color::White, PieceKind::Pawn),
        ],
    );
    let sliding_control = sliding.piece_control(msq(6, 6));
    assert!(sliding_control.contains(msq(6, 3)));
    assert!(sliding_control.contains(msq(6, 9)));
    assert!(!sliding_control.contains(msq(6, 2)));
    assert!(!sliding_control.contains(msq(6, 10)));

    let lion = position(Color::Black, &[(msq(6, 6), Color::Black, PieceKind::Lion)]);
    let lion_control = lion.piece_control(msq(6, 6));
    assert!(lion_control.contains(msq(6, 5)));
    assert!(lion_control.contains(msq(6, 4)));
    assert_eq!(
        lion_control,
        piece_control_with_occupancy(lion.occupied(), Color::Black, PieceKind::Lion, msq(6, 6),)
    );
}

/// 占有と仮想の駒を指定する公開関数は、空升からも遮蔽を考慮した疑似利きを返す。
#[test]
fn piece_control_with_occupancy_accepts_a_hypothetical_piece() {
    let board = position(
        Color::Black,
        &[
            (msq(6, 3), Color::Black, PieceKind::Pawn),
            (msq(6, 9), Color::White, PieceKind::Pawn),
        ],
    );
    let control = Position::piece_control_with_occupancy(
        board.occupied(),
        Color::Black,
        PieceKind::Rook,
        msq(6, 6),
    );

    assert!(control.contains(msq(6, 3)));
    assert!(control.contains(msq(6, 9)));
    assert!(!control.contains(msq(6, 2)));
    assert!(!control.contains(msq(6, 10)));
}

/// 空升から駒別疑似利きを求めるのは呼び出し側の誤りとして拒否する。
#[test]
#[should_panic(expected = "piece control square must contain a piece")]
fn piece_control_rejects_empty_square() {
    let board = position(Color::Black, &[(msq(6, 6), Color::Black, PieceKind::King)]);
    let _ = board.piece_control(msq(5, 5));
}

/// 変更前の固定利き表と8方向走査による逆引き。集合の保存を検査する。
fn ordinary_attackers_before(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    square: Square,
    occupied: Bitboard,
) -> Bitboard {
    let mut attackers = Bitboard::EMPTY;
    for from in tables.neighbourhood(square) & position.pieces_of(color) {
        let kind = position
            .piece_at(from)
            .and_then(|piece| piece.kind())
            .expect("own square must contain a piece");
        if tables
            .fixed(color, movement_profile(kind), from)
            .contains(square)
        {
            attackers.set(from);
        }
    }
    let own = position.pieces_of(color) & occupied;
    for direction in Direction::ALL {
        // 相手駒や仮想の占有升を飛び越さないよう、全占有から最初の遮蔽を求める。
        let blockers = tables.ray(square, direction) & occupied;
        let blocker = if direction.increases_raw_index() {
            blockers.lsb()
        } else {
            blockers.msb()
        };
        let Some(blocker) = blocker else {
            continue;
        };
        if !own.contains(blocker) {
            continue;
        }
        let kind = position
            .piece_at(blocker)
            .and_then(|piece| piece.kind())
            .expect("blocker must be a piece");
        let mask = tables.slide_directions(color, movement_profile(kind));
        let reverse = direction.opposite();
        if (mask >> reverse.index()) & 1 != 0 {
            attackers.set(blocker);
        }
    }
    attackers
}

/// 5規則セットの局面と仮想占有で、固定利きと走りの逆引きが変更前と一致する。
#[test]
fn ordinary_attackers_match_reference_in_all_rule_variants() {
    let tables = crate::attacks::attack_tables();
    for rules in super::rule_variants() {
        for board in bench_positions()
            .into_iter()
            .chain(sampled_random_positions(rules))
        {
            for target in Square::all() {
                for color in Color::ALL {
                    let occupied = board.occupied();
                    let reduced = Bitboard::from_squares(
                        occupied
                            .into_iter()
                            .enumerate()
                            .filter_map(|(i, s)| (i % 3 != 0).then_some(s)),
                    );
                    for occupied in [occupied, reduced] {
                        let expected =
                            (ordinary_attackers_before(tables, &board, color, target, occupied)
                                | special_attackers_before(tables, &board, color, target))
                                & occupied;
                        assert_eq!(board.attackers_to_by(color, target, occupied), expected);
                    }
                }
            }
        }
    }
}

fn special_attackers_before(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    square: Square,
) -> Bitboard {
    let mut attackers = Bitboard::EMPTY;
    for kind in [
        PieceKind::Lion,
        PieceKind::HornedFalcon,
        PieceKind::SoaringEagle,
    ] {
        let pieces = position.pieces_of_kind(color, kind);
        if pieces.is_empty() {
            continue;
        }
        match movement_profile_data(movement_profile(kind)).special {
            SpecialMovement::None => {}
            SpecialMovement::Lion => {
                attackers |= (tables.king_steps(square) | tables.lion_jumps(square)) & pieces;
            }
            SpecialMovement::LionLike(profile) => {
                for relative in profile.directions {
                    let reverse = relative.for_color(color).opposite();
                    if let Some(first) = step_square(square, reverse) {
                        if pieces.contains(first) {
                            attackers.set(first);
                        }
                        if let Some(second) = step_square(first, reverse)
                            && pieces.contains(second)
                        {
                            attackers.set(second);
                        }
                    }
                }
            }
        }
    }
    attackers
}
