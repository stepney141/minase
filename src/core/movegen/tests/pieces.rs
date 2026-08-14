//! 第9条（初期駒の動き）・第10条（成駒としてのみ現れる駒）のテスト。
//!
//! 期待到達升集合は、条文の方向・距離定義（先手基準ベクトル）から
//! step_squares / ray_squares で組み立てる。実装の移動テーブルは参照しない。

use std::collections::BTreeSet;

use super::dir::{B, BL, BR, F, FL, FR, L, R};
use super::{
    direct_destinations, generated, jitto_moves, moves_from, msq, ray_squares, step_squares, union,
};
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::Position;
use crate::core::square::Square;
use crate::test_util::{position, position_from_codes};

/// マトリクスの単駒配置（先手駒を(6,6)へ）。
const ORIGIN: (i16, i16) = (6, 6);

/// 先手の未成駒1枚を (6,6) に置いた局面を作る。
fn lone_piece(kind: PieceKind) -> Position {
    position(Color::Black, &[(msq(6, 6), Color::Black, kind)])
}

/// 先手の成駒1枚を (6,6) に置いた局面を作る。
fn lone_promoted(kind: PieceKind) -> Position {
    position_from_codes(
        Color::Black,
        &[(
            msq(6, 6),
            PieceCode::new_promoted(Color::Black, kind).unwrap(),
        )],
    )
}

/// 到達升集合が期待集合と一致し、要素数が表の値であることを検査する。
fn assert_destinations(
    board: &Position,
    expected: BTreeSet<Square>,
    count: usize,
    excluded: &[(u8, u8)],
) {
    let destinations = direct_destinations(board, msq(6, 6));
    assert_eq!(destinations, expected);
    assert_eq!(destinations.len(), count);
    for &(file, rank) in excluded {
        assert!(!destinations.contains(&msq(file, rank)), "({file},{rank})");
    }
}

// ---------------------------------------------------------------------------
// 第9条　初期駒21種の到達升集合（D1-009-01〜20。獅子はlion_moves.rs）
// ---------------------------------------------------------------------------

// D1-009-01: 王将・玉将は周囲8方向へ1升。
#[test]
fn article_9_king_steps_one_square_in_eight_directions() {
    let expected = step_squares(ORIGIN, &[F, B, L, R, FL, FR, BL, BR]);
    assert_destinations(&lone_piece(PieceKind::King), expected, 8, &[(6, 4)]);
}

// D1-009-02: 金将は前・前斜め・左右・後へ1升。
#[test]
fn article_9_gold_general_steps() {
    let expected = step_squares(ORIGIN, &[F, FL, FR, L, R, B]);
    assert_destinations(&lone_piece(PieceKind::GoldGeneral), expected, 6, &[(5, 7)]);
}

// D1-009-03: 銀将は前および斜め4方向へ1升。
#[test]
fn article_9_silver_general_steps() {
    let expected = step_squares(ORIGIN, &[F, FL, FR, BL, BR]);
    assert_destinations(
        &lone_piece(PieceKind::SilverGeneral),
        expected,
        5,
        &[(5, 6), (6, 7)],
    );
}

// D1-009-04: 銅将は前・前斜め・後へ1升。
#[test]
fn article_9_copper_general_steps() {
    let expected = step_squares(ORIGIN, &[F, FL, FR, B]);
    assert_destinations(
        &lone_piece(PieceKind::CopperGeneral),
        expected,
        4,
        &[(5, 6), (5, 7)],
    );
}

// D1-009-05: 猛豹は前後および斜め4方向へ1升。
#[test]
fn article_9_ferocious_leopard_steps() {
    let expected = step_squares(ORIGIN, &[F, B, FL, FR, BL, BR]);
    assert_destinations(
        &lone_piece(PieceKind::FerociousLeopard),
        expected,
        6,
        &[(5, 6)],
    );
}

// D1-009-06: 盲虎は前以外の7方向へ1升。
#[test]
fn article_9_blind_tiger_steps() {
    let expected = step_squares(ORIGIN, &[B, L, R, FL, FR, BL, BR]);
    assert_destinations(&lone_piece(PieceKind::BlindTiger), expected, 7, &[(6, 5)]);
}

// D1-009-07: 醉象は後以外の7方向へ1升。
#[test]
fn article_9_drunk_elephant_steps() {
    let expected = step_squares(ORIGIN, &[F, L, R, FL, FR, BL, BR]);
    assert_destinations(
        &lone_piece(PieceKind::DrunkElephant),
        expected,
        7,
        &[(6, 7)],
    );
}

// D1-009-08: 歩兵は前へ1升。
#[test]
fn article_9_pawn_steps_forward_only() {
    let expected = step_squares(ORIGIN, &[F]);
    assert_destinations(&lone_piece(PieceKind::Pawn), expected, 1, &[(5, 5), (6, 7)]);
}

// D1-009-09: 仲人は前後へ1升。
#[test]
fn article_9_go_between_steps() {
    let expected = step_squares(ORIGIN, &[F, B]);
    assert_destinations(&lone_piece(PieceKind::GoBetween), expected, 2, &[(5, 5)]);
}

// D1-009-10: 香車は前へ任意の升数。
#[test]
fn article_9_lance_slides_forward() {
    let expected = ray_squares(ORIGIN, &[F]);
    assert_destinations(&lone_piece(PieceKind::Lance), expected, 5, &[(6, 7)]);
}

// D1-009-11: 反車は前後へ任意の升数。
#[test]
fn article_9_reverse_chariot_slides() {
    let expected = ray_squares(ORIGIN, &[F, B]);
    assert_destinations(
        &lone_piece(PieceKind::ReverseChariot),
        expected,
        11,
        &[(5, 6)],
    );
}

// D1-009-12: 横行は左右へ任意の升数、前後へ1升。
#[test]
fn article_9_side_mover_moves() {
    let expected = union(ray_squares(ORIGIN, &[L, R]), step_squares(ORIGIN, &[F, B]));
    assert_destinations(&lone_piece(PieceKind::SideMover), expected, 13, &[(6, 4)]);
}

// D1-009-13: 竪行は前後へ任意の升数、左右へ1升。
#[test]
fn article_9_vertical_mover_moves() {
    let expected = union(ray_squares(ORIGIN, &[F, B]), step_squares(ORIGIN, &[L, R]));
    assert_destinations(
        &lone_piece(PieceKind::VerticalMover),
        expected,
        13,
        &[(4, 6)],
    );
}

// D1-009-14: 角行は斜めへ任意の升数。
#[test]
fn article_9_bishop_slides_diagonally() {
    let expected = ray_squares(ORIGIN, &[FL, FR, BL, BR]);
    assert_destinations(&lone_piece(PieceKind::Bishop), expected, 21, &[(6, 5)]);
}

// D1-009-15: 飛車は縦横へ任意の升数。
#[test]
fn article_9_rook_slides_orthogonally() {
    let expected = ray_squares(ORIGIN, &[F, B, L, R]);
    assert_destinations(&lone_piece(PieceKind::Rook), expected, 22, &[(5, 5)]);
}

// D1-009-16: 龍馬は斜めへ任意の升数、縦横へ1升。
#[test]
fn article_9_dragon_horse_moves() {
    let expected = union(
        ray_squares(ORIGIN, &[FL, FR, BL, BR]),
        step_squares(ORIGIN, &[F, B, L, R]),
    );
    assert_destinations(&lone_piece(PieceKind::DragonHorse), expected, 25, &[(6, 4)]);
}

// D1-009-17: 龍王は縦横へ任意の升数、斜めへ1升。
#[test]
fn article_9_dragon_king_moves() {
    let expected = union(
        ray_squares(ORIGIN, &[F, B, L, R]),
        step_squares(ORIGIN, &[FL, FR, BL, BR]),
    );
    assert_destinations(&lone_piece(PieceKind::DragonKing), expected, 26, &[(4, 4)]);
}

// D1-009-18: 麒麟は斜めへ1升、縦横へ2升跳ぶ。
#[test]
fn article_9_kirin_steps_and_jumps() {
    let expected = union(
        step_squares(ORIGIN, &[FL, FR, BL, BR]),
        step_squares(ORIGIN, &[(0, -2), (0, 2), (-2, 0), (2, 0)]),
    );
    assert_destinations(
        &lone_piece(PieceKind::Kirin),
        expected,
        8,
        &[(6, 5), (5, 4)],
    );
}

// D1-009-19: 鳳凰は縦横へ1升、斜めへ2升跳ぶ。
#[test]
fn article_9_phoenix_steps_and_jumps() {
    let expected = union(
        step_squares(ORIGIN, &[F, B, L, R]),
        step_squares(ORIGIN, &[(-2, -2), (2, -2), (-2, 2), (2, 2)]),
    );
    assert_destinations(
        &lone_piece(PieceKind::Phoenix),
        expected,
        8,
        &[(5, 5), (5, 4)],
    );
}

// D1-009-20: 奔王は8方向へ任意の升数。
#[test]
fn article_9_free_king_slides_in_eight_directions() {
    let expected = ray_squares(ORIGIN, &[F, B, L, R, FL, FR, BL, BR]);
    assert_destinations(&lone_piece(PieceKind::FreeKing), expected, 43, &[(4, 5)]);
}

// D1-009-22: 盤端による切り詰め（第9条、第7条1項）。
#[test]
fn article_9_7_1_moves_are_truncated_at_the_board_edge() {
    // 先手側の隅 (1,12) の金将は盤内3升だけに到達する。
    let gold = position(
        Color::Black,
        &[(msq(1, 12), Color::Black, PieceKind::GoldGeneral)],
    );
    let expected: BTreeSet<Square> = [msq(1, 11), msq(2, 11), msq(2, 12)].into_iter().collect();
    assert_eq!(direct_destinations(&gold, msq(1, 12)), expected);

    // 先手香車 (6,2) の走りは最奥段 (6,1) で止まる。
    let lance = position(Color::Black, &[(msq(6, 2), Color::Black, PieceKind::Lance)]);
    let expected: BTreeSet<Square> = [msq(6, 1)].into_iter().collect();
    assert_eq!(direct_destinations(&lance, msq(6, 2)), expected);
}

// ---------------------------------------------------------------------------
// 第10条　成駒としてのみ現れる駒（D1-010-01〜09）
// ---------------------------------------------------------------------------

// D1-010-01: 白駒は前・前斜め・後へ任意の升数（第10条1項）。
#[test]
fn article_10_1_white_horse_slides() {
    let expected = ray_squares(ORIGIN, &[F, FL, FR, B]);
    assert_destinations(
        &lone_promoted(PieceKind::WhiteHorse),
        expected,
        21,
        &[(5, 7), (5, 6)],
    );
}

// D1-010-02: 鯨鯢は前・後・後斜めへ任意の升数（第10条2項）。
#[test]
fn article_10_2_whale_slides() {
    let expected = ray_squares(ORIGIN, &[F, B, BL, BR]);
    assert_destinations(
        &lone_promoted(PieceKind::Whale),
        expected,
        22,
        &[(5, 5), (5, 6)],
    );
}

// D1-010-03: 飛鹿は前後へ任意の升数、他の6方向へ1升（第10条3項）。
#[test]
fn article_10_3_flying_stag_moves() {
    let expected = union(
        ray_squares(ORIGIN, &[F, B]),
        step_squares(ORIGIN, &[L, R, FL, FR, BL, BR]),
    );
    assert_destinations(
        &lone_promoted(PieceKind::FlyingStag),
        expected,
        17,
        &[(4, 6)],
    );
}

// D1-010-04: 奔猪は左右・斜め4方向へ任意の升数（第10条4項）。
#[test]
fn article_10_4_free_boar_slides() {
    let expected = ray_squares(ORIGIN, &[L, R, FL, FR, BL, BR]);
    assert_destinations(&lone_promoted(PieceKind::FreeBoar), expected, 32, &[(6, 5)]);
}

// D1-010-05: 飛牛は前後・斜め4方向へ任意の升数（第10条5項）。
#[test]
fn article_10_5_flying_ox_slides() {
    let expected = ray_squares(ORIGIN, &[F, B, FL, FR, BL, BR]);
    assert_destinations(&lone_promoted(PieceKind::FlyingOx), expected, 32, &[(5, 6)]);
}

// D1-010-06: 太子は周囲8方向へ1升（第10条6項。王駒としての効果は領域D3）。
#[test]
fn article_10_6_crown_prince_steps() {
    let expected = step_squares(ORIGIN, &[F, B, L, R, FL, FR, BL, BR]);
    assert_destinations(
        &lone_promoted(PieceKind::CrownPrince),
        expected,
        8,
        &[(6, 4)],
    );
}

// D1-010-07: 角鷹は前以外の7方向へ任意の升数、前方は第11条の2段階（第10条7項）。
#[test]
fn article_10_7_horned_falcon_moves() {
    let board = lone_promoted(PieceKind::HornedFalcon);
    let expected = union(
        ray_squares(ORIGIN, &[B, L, R, FL, FR, BL, BR]),
        step_squares(ORIGIN, &[F, (0, -2)]),
    );
    assert_destinations(&board, expected, 40, &[(6, 3)]);
    // 手数は40升＋じっと1手の41手（空盤では mid ありの着手はない）。
    let moves = moves_from(&generated(&board), msq(6, 6));
    assert_eq!(moves.len(), 41);
    assert_eq!(jitto_moves(&moves, msq(6, 6)).len(), 1);
    assert!(moves.iter().all(|m| m.mid.is_none()));
}

// D1-010-08: 飛鷲は前後・左右・後斜めへ任意の升数、左右前斜めは2段階（第10条8項）。
#[test]
fn article_10_8_soaring_eagle_moves() {
    let board = lone_promoted(PieceKind::SoaringEagle);
    let expected = union(
        ray_squares(ORIGIN, &[F, B, L, R, BL, BR]),
        step_squares(ORIGIN, &[FL, FR, (-2, -2), (2, -2)]),
    );
    assert_destinations(&board, expected, 37, &[(3, 3)]);
    // 手数は37升＋じっと1手の38手。
    let moves = moves_from(&generated(&board), msq(6, 6));
    assert_eq!(moves.len(), 38);
    assert_eq!(jitto_moves(&moves, msq(6, 6)).len(), 1);
    assert!(moves.iter().all(|m| m.mid.is_none()));
}

// D1-010-09: 成駒は初期配置に現れず、成駒からの着手に成り変種はない
// （第10条見出し、第17条4項）。
#[test]
fn article_10_promoted_only_pieces_appear_only_by_promotion() {
    // 初期配置の92枚はすべて未成である（第4条・第5条）。
    let initial = Position::initial();
    for square in Square::all() {
        if let Some(piece) = initial.piece_at(square) {
            assert!(!piece.is_promoted(), "square={square:?}");
        }
    }

    let promoted_only = [
        PieceKind::WhiteHorse,
        PieceKind::Whale,
        PieceKind::FlyingStag,
        PieceKind::FreeBoar,
        PieceKind::FlyingOx,
        PieceKind::CrownPrince,
        PieceKind::HornedFalcon,
        PieceKind::SoaringEagle,
    ];
    for kind in promoted_only {
        // 敵陣の直前 (6,5) に置き、敵陣入りの着手があっても成り変種がない。
        let board = position_from_codes(
            Color::Black,
            &[(
                msq(6, 5),
                PieceCode::new_promoted(Color::Black, kind).unwrap(),
            )],
        );
        let moves = moves_from(&generated(&board), msq(6, 5));
        assert!(
            moves.iter().any(|m| m.to.rank() >= 8),
            "kind={kind:?} should reach the promotion zone"
        );
        assert!(moves.iter().all(|m| !m.promote), "kind={kind:?}");
    }
}
