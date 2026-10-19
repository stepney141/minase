//! `Position::attackers_to_by`の契約テスト。
//!
//! 期待値は[`Position::attackers_to_by`]の契約である、
//! [`piece_control_with_occupancy`]による駒別利きとの同値関係から導く。

use crate::board::Bitboard;
use crate::board::Square;
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

/// 空升から駒別疑似利きを求めるのは呼び出し側の誤りとして拒否する。
#[test]
#[should_panic(expected = "piece control square must contain a piece")]
fn piece_control_rejects_empty_square() {
    let board = position(Color::Black, &[(msq(6, 6), Color::Black, PieceKind::King)]);
    let _ = board.piece_control(msq(5, 5));
}
