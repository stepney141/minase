//! 第6条（手番）・第7条（移動と捕獲）・第8条（王手）のテスト。

use std::collections::BTreeSet;

use super::dir::{B, BL, BR, F, FL, FR, L, R};
use super::{direct_destinations, generated, jitto_moves, msq, mv, same_board, step_squares};
use crate::core::piece::{Color, PieceKind};
use crate::core::position::Position;
use crate::core::rules::Rules;
use crate::core::square::Square;
use crate::test_util::position;

/// 着手適用の前後で「動いた自駒はfromの1枚だけ・相手駒は捕獲分だけ減る」を検査する
/// （第6条2項・3項、第4条4項）。
fn assert_single_mover(before: &Position, mv: crate::core::mv::Move) {
    let mover = before.side_to_move();
    let mut after = before.clone();
    after.make_move_unchecked(mv, Rules::standard());

    // 期待捕獲升: mid（第1段階の捕獲）と、相手駒がいる到達升（第7条3項）。
    let mut expected_captures: BTreeSet<Square> = BTreeSet::new();
    if let Some(mid) = mv.mid {
        expected_captures.insert(mid);
    }
    if mv.to != mv.from && before.pieces_of(mover.opposite()).contains(mv.to) {
        expected_captures.insert(mv.to);
    }

    for square in Square::all() {
        let was = before.piece_at(square);
        let now = after.piece_at(square);
        if was == now {
            continue;
        }
        // 変化してよいのは from・to（動いた1枚）と捕獲升だけ（第6条2項）。
        assert!(
            square == mv.from || square == mv.to || expected_captures.contains(&square),
            "move={mv:?}, square={square:?}"
        );
    }
    assert_eq!(
        before.pieces_of(mover).popcount(),
        after.pieces_of(mover).popcount(),
        "move={mv:?}"
    );
    assert_eq!(
        before.pieces_of(mover.opposite()).popcount(),
        after.pieces_of(mover.opposite()).popcount() + expected_captures.len() as u32,
        "move={mv:?}"
    );
}

// ---------------------------------------------------------------------------
// 第6条　手番
// ---------------------------------------------------------------------------

// D1-006-01: 先手の初手と交互着手（第6条1項）。
#[test]
fn article_6_1_initial_position_alternates_turns_starting_with_black() {
    let mut position = Position::initial();
    let moves = generated(&position);
    assert!(!moves.is_empty());
    // 初期局面の全着手は先手（Black）の駒を from とする。
    for m in &moves {
        assert_eq!(
            position.piece_at(m.from).and_then(|piece| piece.color()),
            Some(Color::Black),
            "move={m:?}"
        );
    }

    position.make_move_unchecked(moves[0], Rules::standard());
    // 先手の1手を適用した局面では、全着手が後手（White）の駒を from とする。
    for m in generated(&position) {
        assert_eq!(
            position.piece_at(m.from).and_then(|piece| piece.color()),
            Some(Color::White),
            "move={m:?}"
        );
    }
}

// D1-006-02: 1着手1駒（第6条2項・3項）。2段階移動の2枚取りを含めて検査する。
#[test]
fn article_6_2_each_move_moves_exactly_one_own_piece() {
    let initial = Position::initial();
    for m in generated(&initial) {
        assert_single_mover(&initial, m);
    }

    // 獅子の2段階移動（2枚取り・居喰い）でも動く自駒は1枚である（第6条3項）。
    let lion_board = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::Lion),
            (msq(6, 5), Color::White, PieceKind::Pawn),
            (msq(5, 4), Color::White, PieceKind::CopperGeneral),
        ],
    );
    for m in generated(&lion_board) {
        assert_single_mover(&lion_board, m);
    }
}

// D1-006-03: じっとは手番放棄ではなく1手（第6条4項、第3条13号、第12条11項）。
#[test]
fn article_6_4_lion_jitto_passes_turn_without_board_change() {
    let before = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::Lion),
            (msq(12, 1), Color::White, PieceKind::Pawn),
        ],
    );
    let jitto = mv(msq(6, 6), None, msq(6, 6), false);
    assert!(generated(&before).contains(&jitto));

    let mut after = before.clone();
    after.make_move_unchecked(jitto, Rules::standard());
    // 盤面成分は不変で、手番だけが相手へ移る。
    assert!(same_board(&before, &after));
    assert_eq!(after.side_to_move(), Color::White);
}

// D1-006-04: パスの不存在（第6条5項）。to=from は獅子・角鷹・飛鷲に限る。
#[test]
fn article_6_5_no_pass_move_exists_for_ordinary_pieces() {
    let board = position(
        Color::Black,
        &[
            (msq(3, 6), Color::Black, PieceKind::GoldGeneral),
            (msq(6, 6), Color::Black, PieceKind::Lion),
            (msq(9, 6), Color::Black, PieceKind::FreeKing),
        ],
    );
    let moves = generated(&board);
    // 金将・奔王には to=from の着手がなく、獅子のじっとだけが存在する。
    for m in &moves {
        if m.to == m.from {
            assert_eq!(m.from, msq(6, 6), "move={m:?}");
        }
    }
    assert_eq!(jitto_moves(&moves, msq(6, 6)).len(), 1);
    // 全域の性質（任意局面で to=from ⇒ 獅子・角鷹・飛鷲）は properties.rs の
    // mc_canonical_move_invariants_hold_in_seeded_playouts が検査する。
}

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
    assert!(!destinations.contains(&msq(6, 5)));
    let expected = step_squares((6, 6), &[FL, FR, L, R, B]);
    assert_eq!(destinations, expected);
}

// D1-007-02: 相手駒の捕獲と除去（第7条3項、第4条4項・5項）。
#[test]
fn article_7_3_capture_removes_the_enemy_piece_permanently() {
    let mut board = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::GoldGeneral),
            (msq(6, 5), Color::White, PieceKind::Pawn),
        ],
    );
    let capture = mv(msq(6, 6), None, msq(6, 5), false);
    assert!(generated(&board).contains(&capture));

    board.make_move_unchecked(capture, Rules::standard());
    // 到達升に金将が移り、取られた歩兵は盤上から消える（持ち駒にならない）。
    assert_eq!(
        board.piece_at(msq(6, 5)).and_then(|piece| piece.kind()),
        Some(PieceKind::GoldGeneral)
    );
    assert!(board.pieces_of(Color::White).is_empty());
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
    assert!(!destinations.contains(&msq(6, 3)));
    assert!(!destinations.contains(&msq(6, 2)));
    assert!(!destinations.contains(&msq(6, 10)));
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
        after.make_move_unchecked(jump, Rules::standard());
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

// D1-007-05: 2段階移動の逐次判定（第7条8項）。第2段階の到達可否は mid の隣接判定。
#[test]
fn article_7_8_second_stage_is_judged_after_first_capture() {
    let board = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::Lion),
            (msq(6, 5), Color::White, PieceKind::Pawn),
            (msq(5, 4), Color::White, PieceKind::CopperGeneral),
        ],
    );
    let moves = generated(&board);
    let mid = msq(6, 5);
    let second_stage: BTreeSet<Square> = moves
        .iter()
        .filter(|m| m.from == msq(6, 6) && m.mid == Some(mid))
        .map(|m| m.to)
        .collect();
    // mid=(6,5) の第2段階は (6,5) の周囲8升すべて（from=(6,6) は駒が離れた後
    // なので居喰いとして含む。第12条12項）。
    let expected = step_squares((6, 5), &[F, B, L, R, FL, FR, BL, BR]);
    assert_eq!(second_stage, expected);
    // to=(5,4) は2枚取りとして適用できる（第12条4項）。
    let mut after = board.clone();
    after.make_move_unchecked(
        mv(msq(6, 6), Some(mid), msq(5, 4), false),
        Rules::standard(),
    );
    assert!(after.pieces_of(Color::White).is_empty());
    // (6,8) は mid に隣接しないので到達できない。
    assert!(!second_stage.contains(&msq(6, 8)));
}

// D1-007-06: 同一駒の2回捕獲禁止（第7条9項）。正準形では mid≠to かつ mid≠from。
#[test]
fn article_7_9_no_move_captures_the_same_piece_twice() {
    let board = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::Lion),
            (msq(6, 5), Color::White, PieceKind::Pawn),
            (msq(7, 6), Color::White, PieceKind::Pawn),
        ],
    );
    for m in generated(&board) {
        if let Some(mid) = m.mid {
            assert_ne!(mid, m.to, "move={m:?}");
            assert_ne!(mid, m.from, "move={m:?}");
        }
    }
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
