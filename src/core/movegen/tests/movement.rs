//! 第7条（移動と捕獲）・第8条（王手）のテスト。

use std::collections::BTreeSet;

use super::dir::{B, FL, FR, L, R};
use super::{direct_destinations, generated, msq, mv, step_squares};
use crate::core::board::Square;
use crate::core::piece::{Color, PieceKind};
use crate::core::rules::MoveRules;
use crate::test_util::position;

// ---------------------------------------------------------------------------
// 第7条　移動と捕獲の一般則
// ---------------------------------------------------------------------------

// D1-007-01: 自駒升への到達禁止（第7条2項）。
#[test]
fn article_7_2_own_occupied_square_is_not_a_destination() {
    let board = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::GoldGeneral),
            (msq(6, 5), Color::Black, PieceKind::Pawn),
        ],
    );
    let destinations = direct_destinations(&board, msq(6, 6));
    // 金将の6方向のうち、自歩がふさぐ前方 (6,5) だけが除かれる。
    let expected = step_squares((6, 6), &[FL, FR, L, R, B]);
    assert_eq!(destinations, expected);
}

// D1-007-03: 走り駒の遮蔽と先端捕獲（第7条4項・5項）。
#[test]
fn article_7_4_5_sliders_stop_at_first_piece_and_capture_enemies() {
    let board = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::Rook),
            (msq(6, 3), Color::Black, PieceKind::Pawn),
            (msq(6, 9), Color::White, PieceKind::Pawn),
        ],
    );
    let destinations = direct_destinations(&board, msq(6, 6));
    // 前方は自駒 (6,3) の手前まで、後方は相手駒 (6,9) を取る升まで。
    let mut expected: BTreeSet<Square> = [msq(6, 5), msq(6, 4), msq(6, 7), msq(6, 8), msq(6, 9)]
        .into_iter()
        .collect();
    expected.extend(super::ray_squares((6, 6), &[L, R]));
    assert_eq!(destinations, expected);
}

// D1-007-04: 跳び越しと中間升の不捕獲（第7条6項・7項、第9条の麒麟・鳳凰）。
#[test]
fn article_7_6_7_jumps_ignore_and_preserve_intermediate_pieces() {
    for middle_owner in [Color::Black, Color::White] {
        let board = position(
            Color::Black,
            &[
                (msq(6, 6), Color::Black, PieceKind::Kirin),
                (msq(6, 5), middle_owner, PieceKind::Pawn),
            ],
        );
        let jump = mv(msq(6, 6), None, msq(6, 4), false);
        // 中間升 (6,5) の駒の有無・所有者にかかわらず跳べる。
        assert!(generated(&board).contains(&jump), "owner={middle_owner:?}");
        let mut after = board.clone();
        after.make_move_unchecked(jump, MoveRules::standard());
        // 跳び越した中間升の駒は取らない（第7条7項）。
        assert_eq!(after.piece_at(msq(6, 5)), board.piece_at(msq(6, 5)));
    }

    // 鳳凰の斜め2升跳びも同様（(5,5) に駒があっても (4,4) へ跳べる）。
    let phoenix = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::Phoenix),
            (msq(5, 5), Color::White, PieceKind::Pawn),
        ],
    );
    assert!(generated(&phoenix).contains(&mv(msq(6, 6), None, msq(4, 4), false)));

    // 跳び先が自駒なら不可、相手駒なら捕獲（D1-007-01・第7条3項）。
    let own_target = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::Kirin),
            (msq(6, 4), Color::Black, PieceKind::Pawn),
        ],
    );
    assert!(!generated(&own_target).contains(&mv(msq(6, 6), None, msq(6, 4), false)));
    let enemy_target = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::Kirin),
            (msq(6, 4), Color::White, PieceKind::Pawn),
        ],
    );
    assert!(generated(&enemy_target).contains(&mv(msq(6, 6), None, msq(6, 4), false)));
}

// ---------------------------------------------------------------------------
// 第8条　王手と擬似合法生成
// ---------------------------------------------------------------------------

// D1-008-01: 王駒を相手の利きへ移す着手も生成される（第8条3項・5項）。
#[test]
fn article_8_3_king_may_move_into_enemy_control() {
    let board = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::King),
            (msq(1, 5), Color::White, PieceKind::Rook),
        ],
    );
    // (6,5) は白飛車の利き筋だが、王将の着手として生成される。
    assert!(generated(&board).contains(&mv(msq(6, 6), None, msq(6, 5), false)));
}

// D1-008-02: 王手放置の着手も生成される（第8条1項・2項・4項・5項）。
#[test]
fn article_8_4_5_check_needs_no_declaration_and_may_be_ignored() {
    let board = position(
        Color::Black,
        &[
            (msq(6, 12), Color::Black, PieceKind::King),
            (msq(6, 1), Color::White, PieceKind::Rook),
            (msq(1, 9), Color::Black, PieceKind::Pawn),
        ],
    );
    // 6筋の飛車による王手を受けたまま、無関係な歩兵の手が生成される。
    assert!(generated(&board).contains(&mv(msq(1, 9), None, msq(1, 8), false)));
}
