//! `Position::attackers_to_by`の契約テスト。
//!
//! 期待値は[`Position::attackers_to_by`]の契約である、
//! [`piece_control_with_occupancy`]による駒別利きとの同値関係から導く。

use crate::board::Bitboard;
use crate::board::Square;
use crate::movegen::control::piece_control_with_occupancy;
use crate::piece::Color;
use crate::position::Position;
use crate::rules::MoveRules;
use crate::test_util::{bench_positions, sampled_random_positions};

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
