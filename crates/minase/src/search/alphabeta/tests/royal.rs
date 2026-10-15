//! 王駒への利きを検査する。

use super::*;

// 王将が安全でも太子が攻撃されていれば真になる。
#[test]
fn royal_under_attack_includes_crown_prince_alone() {
    for attacked in [false, true] {
        let mut pieces = vec![
            (sq(0, 11), Color::White, PieceKind::King, false),
            (sq(11, 0), Color::Black, PieceKind::King, false),
            (sq(5, 6), Color::White, PieceKind::Rook, false),
        ];
        if attacked {
            pieces.push((sq(5, 0), Color::Black, PieceKind::CrownPrince, true));
        }
        let board = position_with_promoted_pieces(Color::Black, &pieces);
        assert_eq!(royal_under_attack(&board), attacked);
    }
}

// RULES.md第8条、第12条第5項。獅子は間の駒を跳び越えて距離2の王駒を取れる。
#[test]
fn article_8_royal_under_attack_includes_lion_jump_and_only_opponents() {
    for (lion, side, attacked) in [
        (Color::White, Color::Black, true),
        (Color::Black, Color::Black, false),
        (Color::White, Color::White, false),
    ] {
        let board = position(
            side,
            &[
                (sq(0, 11), Color::White, PieceKind::King),
                (sq(11, 0), Color::Black, PieceKind::King),
                (sq(9, 2), lion, PieceKind::Lion),
                (sq(10, 1), Color::Black, PieceKind::Pawn),
            ],
        );
        assert_eq!(royal_under_attack(&board), attacked);
    }
}
