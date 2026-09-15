//! `Position::attackers_to_by`の契約テスト。
//!
//! 期待値は`docs/plans/strength-stage3.md`「升への疑似利き集合」が定める、
//! 既存の駒別利きとの同値関係から導く。

use crate::core::bitboard::Bitboard;
use crate::core::movegen::piece_control_with_occupancy;
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::Position;
use crate::core::rules::MoveRules;
use crate::core::square::Square;
use crate::test_util::{bench_positions, position_from_codes, sampled_random_positions, sq};

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

// strength-stage3.md「升への疑似利き集合」: 初期局面、benchの14局面、および
// 一様ランダム対局の数十局面で、各色の逆引き集合とその色の駒別利きの同値を
// 全升で固定する。反対側の駒は集合に現れない。
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

// strength-stage3.md「升への疑似利き集合」: 仮想盤面から除いた駒は攻撃駒として
// 再出現せず、走り駒の後ろの利きは遮蔽物の除去によって開通する。
#[test]
fn attackers_to_by_respects_removed_pieces_and_opens_xrays() {
    let rook = sq(2, 2);
    let first_blocker = sq(2, 4);
    let second_blocker = sq(2, 6);
    let target = sq(2, 8);
    let position = position_from_codes(
        Color::Black,
        &[
            (rook, PieceCode::new(Color::Black, PieceKind::Rook).unwrap()),
            (
                first_blocker,
                PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
            ),
            (
                second_blocker,
                PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
            ),
        ],
    );

    let mut without_first = position.occupied();
    without_first.clear(first_blocker);
    assert!(
        !position
            .attackers_to_by(Color::Black, target, without_first)
            .contains(rook)
    );
    assert!(
        !position
            .attackers_to_by(Color::Black, target, without_first)
            .contains(first_blocker)
    );

    let mut without_both = without_first;
    without_both.clear(second_blocker);
    assert!(
        position
            .attackers_to_by(Color::Black, target, without_both)
            .contains(rook)
    );
    assert!(
        !position
            .attackers_to_by(Color::Black, target, without_both)
            .contains(second_blocker)
    );

    let lion = sq(7, 7);
    let lion_target = sq(9, 9);
    let special = position_from_codes(
        Color::Black,
        &[(lion, PieceCode::new(Color::Black, PieceKind::Lion).unwrap())],
    );
    let mut without_lion = special.occupied();
    without_lion.clear(lion);
    assert!(
        !special
            .attackers_to_by(Color::Black, lion_target, without_lion)
            .contains(lion)
    );
}
