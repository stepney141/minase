//! 第9条（初期駒の動き）・第10条（成駒としてのみ現れる駒）のテスト。
//!
//! 期待到達升集合は、条文の方向・距離定義（先手基準ベクトル）から
//! step_squares / ray_squares で組み立てる。実装の移動テーブルは参照しない。

use std::collections::BTreeSet;

use super::dir::{B, BL, BR, F, FL, FR, L, R};
use super::{direct_destinations, generated, moves_from, msq, ray_squares, step_squares, union};
use crate::board::Square;
use crate::piece::{Color, PieceCode, PieceKind};
use crate::position::Position;
use crate::test_util::{position, position_from_codes};

/// 先手駒を1枚だけ置く升を(6,6)とする。
const ORIGIN: (i16, i16) = (6, 6);

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

/// 到達升集合が条文から導いた期待集合と一致することを検査する。
fn assert_destinations(board: &Position, expected: BTreeSet<Square>) {
    assert_eq!(direct_destinations(board, msq(6, 6)), expected);
}

// ---------------------------------------------------------------------------
// 第9条　初期駒21種の到達升集合（獅子はlion_moves.rs）
// ---------------------------------------------------------------------------

#[test]
fn article_9_initial_piece_destinations_match_the_rule() {
    let cases = [
        // 王将・玉将は周囲8方向へ1升。
        (
            "article_9_king_steps_one_square_in_eight_directions",
            PieceCode::new(Color::Black, PieceKind::King).unwrap(),
            step_squares(ORIGIN, &[F, B, L, R, FL, FR, BL, BR]),
        ),
        // 金将は前・前斜め・左右・後へ1升。
        (
            "article_9_gold_general_steps",
            PieceCode::new(Color::Black, PieceKind::GoldGeneral).unwrap(),
            step_squares(ORIGIN, &[F, FL, FR, L, R, B]),
        ),
        // 銀将は前および斜め4方向へ1升。
        (
            "article_9_silver_general_steps",
            PieceCode::new(Color::Black, PieceKind::SilverGeneral).unwrap(),
            step_squares(ORIGIN, &[F, FL, FR, BL, BR]),
        ),
        // 銅将は前・前斜め・後へ1升。
        (
            "article_9_copper_general_steps",
            PieceCode::new(Color::Black, PieceKind::CopperGeneral).unwrap(),
            step_squares(ORIGIN, &[F, FL, FR, B]),
        ),
        // 猛豹は前後および斜め4方向へ1升。
        (
            "article_9_ferocious_leopard_steps",
            PieceCode::new(Color::Black, PieceKind::FerociousLeopard).unwrap(),
            step_squares(ORIGIN, &[F, B, FL, FR, BL, BR]),
        ),
        // 盲虎は前以外の7方向へ1升。
        (
            "article_9_blind_tiger_steps",
            PieceCode::new(Color::Black, PieceKind::BlindTiger).unwrap(),
            step_squares(ORIGIN, &[B, L, R, FL, FR, BL, BR]),
        ),
        // 醉象は後以外の7方向へ1升。
        (
            "article_9_drunk_elephant_steps",
            PieceCode::new(Color::Black, PieceKind::DrunkElephant).unwrap(),
            step_squares(ORIGIN, &[F, L, R, FL, FR, BL, BR]),
        ),
        // 歩兵は前へ1升。
        (
            "article_9_pawn_steps_forward_only",
            PieceCode::new(Color::Black, PieceKind::Pawn).unwrap(),
            step_squares(ORIGIN, &[F]),
        ),
        // 仲人は前後へ1升。
        (
            "article_9_go_between_steps",
            PieceCode::new(Color::Black, PieceKind::GoBetween).unwrap(),
            step_squares(ORIGIN, &[F, B]),
        ),
        // 香車は前へ任意の升数。
        (
            "article_9_lance_slides_forward",
            PieceCode::new(Color::Black, PieceKind::Lance).unwrap(),
            ray_squares(ORIGIN, &[F]),
        ),
        // 反車は前後へ任意の升数。
        (
            "article_9_reverse_chariot_slides",
            PieceCode::new(Color::Black, PieceKind::ReverseChariot).unwrap(),
            ray_squares(ORIGIN, &[F, B]),
        ),
        // 横行は左右へ任意の升数、前後へ1升。
        (
            "article_9_side_mover_moves",
            PieceCode::new(Color::Black, PieceKind::SideMover).unwrap(),
            union(ray_squares(ORIGIN, &[L, R]), step_squares(ORIGIN, &[F, B])),
        ),
        // 竪行は前後へ任意の升数、左右へ1升。
        (
            "article_9_vertical_mover_moves",
            PieceCode::new(Color::Black, PieceKind::VerticalMover).unwrap(),
            union(ray_squares(ORIGIN, &[F, B]), step_squares(ORIGIN, &[L, R])),
        ),
        // 角行は斜めへ任意の升数。
        (
            "article_9_bishop_slides_diagonally",
            PieceCode::new(Color::Black, PieceKind::Bishop).unwrap(),
            ray_squares(ORIGIN, &[FL, FR, BL, BR]),
        ),
        // 飛車は縦横へ任意の升数。
        (
            "article_9_rook_slides_orthogonally",
            PieceCode::new(Color::Black, PieceKind::Rook).unwrap(),
            ray_squares(ORIGIN, &[F, B, L, R]),
        ),
        // 龍馬は斜めへ任意の升数、縦横へ1升。
        (
            "article_9_dragon_horse_moves",
            PieceCode::new(Color::Black, PieceKind::DragonHorse).unwrap(),
            union(
                ray_squares(ORIGIN, &[FL, FR, BL, BR]),
                step_squares(ORIGIN, &[F, B, L, R]),
            ),
        ),
        // 龍王は縦横へ任意の升数、斜めへ1升。
        (
            "article_9_dragon_king_moves",
            PieceCode::new(Color::Black, PieceKind::DragonKing).unwrap(),
            union(
                ray_squares(ORIGIN, &[F, B, L, R]),
                step_squares(ORIGIN, &[FL, FR, BL, BR]),
            ),
        ),
        // 麒麟は斜めへ1升、縦横へ2升跳ぶ。
        (
            "article_9_kirin_steps_and_jumps",
            PieceCode::new(Color::Black, PieceKind::Kirin).unwrap(),
            union(
                step_squares(ORIGIN, &[FL, FR, BL, BR]),
                step_squares(ORIGIN, &[(0, -2), (0, 2), (-2, 0), (2, 0)]),
            ),
        ),
        // 鳳凰は縦横へ1升、斜めへ2升跳ぶ。
        (
            "article_9_phoenix_steps_and_jumps",
            PieceCode::new(Color::Black, PieceKind::Phoenix).unwrap(),
            union(
                step_squares(ORIGIN, &[F, B, L, R]),
                step_squares(ORIGIN, &[(-2, -2), (2, -2), (-2, 2), (2, 2)]),
            ),
        ),
        // 奔王は8方向へ任意の升数。
        (
            "article_9_free_king_slides_in_eight_directions",
            PieceCode::new(Color::Black, PieceKind::FreeKing).unwrap(),
            ray_squares(ORIGIN, &[F, B, L, R, FL, FR, BL, BR]),
        ),
    ];
    for (article, code, expected) in cases {
        let board = position_from_codes(Color::Black, &[(msq(6, 6), code)]);
        assert_eq!(
            direct_destinations(&board, msq(6, 6)),
            expected,
            "{article}: {code:?}"
        );
    }
}

// 盤端による切り詰め（第9条、第7条1項）。
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
// 第10条　成駒としてのみ現れる駒
// ---------------------------------------------------------------------------

#[test]
fn article_10_promoted_only_piece_destinations_match_the_rule() {
    let cases = [
        // 白駒は前・前斜め・後へ任意の升数（第10条1項）。
        (
            "article_10_1_white_horse_slides",
            PieceCode::new_promoted(Color::Black, PieceKind::WhiteHorse).unwrap(),
            ray_squares(ORIGIN, &[F, FL, FR, B]),
        ),
        // 鯨鯢は前・後・後斜めへ任意の升数（第10条2項）。
        (
            "article_10_2_whale_slides",
            PieceCode::new_promoted(Color::Black, PieceKind::Whale).unwrap(),
            ray_squares(ORIGIN, &[F, B, BL, BR]),
        ),
        // 飛鹿は前後へ任意の升数、他の6方向へ1升（第10条3項）。
        (
            "article_10_3_flying_stag_moves",
            PieceCode::new_promoted(Color::Black, PieceKind::FlyingStag).unwrap(),
            union(
                ray_squares(ORIGIN, &[F, B]),
                step_squares(ORIGIN, &[L, R, FL, FR, BL, BR]),
            ),
        ),
        // 奔猪は左右・斜め4方向へ任意の升数（第10条4項）。
        (
            "article_10_4_free_boar_slides",
            PieceCode::new_promoted(Color::Black, PieceKind::FreeBoar).unwrap(),
            ray_squares(ORIGIN, &[L, R, FL, FR, BL, BR]),
        ),
        // 飛牛は前後・斜め4方向へ任意の升数（第10条5項）。
        (
            "article_10_5_flying_ox_slides",
            PieceCode::new_promoted(Color::Black, PieceKind::FlyingOx).unwrap(),
            ray_squares(ORIGIN, &[F, B, FL, FR, BL, BR]),
        ),
        // 太子は周囲8方向へ1升（第10条6項）。
        (
            "article_10_6_crown_prince_steps",
            PieceCode::new_promoted(Color::Black, PieceKind::CrownPrince).unwrap(),
            step_squares(ORIGIN, &[F, B, L, R, FL, FR, BL, BR]),
        ),
    ];
    for (article, code, expected) in cases {
        let board = position_from_codes(Color::Black, &[(msq(6, 6), code)]);
        assert_eq!(
            direct_destinations(&board, msq(6, 6)),
            expected,
            "{article}: {code:?}"
        );
    }
}

// 角鷹は前以外の7方向へ任意の升数、前方は第11条の2段階（第10条7項）。
#[test]
fn article_10_7_horned_falcon_moves() {
    let board = lone_promoted(PieceKind::HornedFalcon);
    let expected = union(
        ray_squares(ORIGIN, &[B, L, R, FL, FR, BL, BR]),
        step_squares(ORIGIN, &[F, (0, -2)]),
    );
    assert_destinations(&board, expected);
    // 手数は40升＋じっと1手の41手（空盤では mid ありの着手はない）。
    let moves = moves_from(&generated(&board), msq(6, 6));
    assert_eq!(moves.len(), 41);
    assert!(moves.iter().all(|m| m.mid.is_none()));
}

// 飛鷲は前後・左右・後斜めへ任意の升数、左右前斜めは2段階（第10条8項）。
#[test]
fn article_10_8_soaring_eagle_moves() {
    let board = lone_promoted(PieceKind::SoaringEagle);
    let expected = union(
        ray_squares(ORIGIN, &[F, B, L, R, BL, BR]),
        step_squares(ORIGIN, &[FL, FR, (-2, -2), (2, -2)]),
    );
    assert_destinations(&board, expected);
    // 手数は37升＋じっと1手の38手。
    let moves = moves_from(&generated(&board), msq(6, 6));
    assert_eq!(moves.len(), 38);
    assert!(moves.iter().all(|m| m.mid.is_none()));
}
