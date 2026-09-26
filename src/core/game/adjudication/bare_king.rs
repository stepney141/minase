//! 裸玉の即時裁定と有効な駒の判定。

use super::super::result::{DrawReason, GameResult, WinReason};
use super::exhaustion::is_last_rank;
use crate::core::board::{Bitboard, Square};
use crate::core::movegen::MoveGenerator;
use crate::core::piece::{Color, PieceKind};
use crate::core::position::Position;

/// 裸玉即時裁定(第32条E3)の結果を現在局面から返す。
pub(super) fn bare_king_result(
    position: &Position,
    generator: &MoveGenerator,
) -> Option<GameResult> {
    let bare_kings = Color::ALL.map(|color| bare_king_square(position, color));

    for bare_color in Color::ALL {
        let Some(bare_king) = bare_kings[bare_color.index()] else {
            continue;
        };
        let opponent = bare_color.opposite();
        let effective_pieces = effective_pieces(position, opponent);
        // 条件(a): 相手が王駒を持ち、価値ある駒を2枚以上持つ。
        // 条件(b): 裸玉側が相手王駒へ王手をかけていない。
        // 条件(c): 価値ある駒が3枚以上か、いずれも裸玉に隣接していない。
        if !position.royal_pieces(opponent).is_empty()
            && effective_pieces.popcount() >= 2
            && !pieces_give_check(position, generator, bare_color)
            && (effective_pieces.popcount() >= 3
                || effective_pieces
                    .into_iter()
                    .all(|square| !squares_are_adjacent(bare_king, square)))
        {
            return Some(GameResult::Win {
                winner: opponent,
                reason: WinReason::BareKing,
            });
        }
    }

    // 双方が裸玉で、いずれの王駒にも王手がかかっていなければ引き分けとする。
    if bare_kings.into_iter().all(|king| king.is_some())
        && Color::ALL
            .into_iter()
            .all(|color| !pieces_give_check(position, generator, color))
    {
        Some(GameResult::Draw {
            reason: DrawReason::BareKing,
        })
    } else {
        None
    }
}

/// 死に駒を除く自駒が王駒1枚だけなら、その升を返す。
fn bare_king_square(position: &Position, color: Color) -> Option<Square> {
    let remaining = position.pieces_of(color) & !dead_pieces(position, color);
    (remaining.popcount() == 1)
        .then(|| remaining.lsb())
        .flatten()
        .filter(|&square| position.royal_pieces(color).contains(square))
}

/// 歩兵・仲人と死んだ香車を除く価値ある駒の集合を返す。
fn effective_pieces(position: &Position, color: Color) -> Bitboard {
    position.pieces_of(color)
        & !position.pieces_of_kind(color, PieceKind::Pawn)
        & !position.pieces_of_kind(color, PieceKind::GoBetween)
        & !dead_pieces(position, color)
}

/// 最奥段で移動不能となった歩兵・香車の集合を返す。
fn dead_pieces(position: &Position, color: Color) -> Bitboard {
    Bitboard::from_squares(
        (position.pieces_of_kind(color, PieceKind::Pawn)
            | position.pieces_of_kind(color, PieceKind::Lance))
        .into_iter()
        .filter(|&square| is_last_rank(color, square)),
    )
}

/// 指定側の駒が相手のいずれかの王駒へ王手をかけているかを返す。
fn pieces_give_check(position: &Position, generator: &MoveGenerator, attacker: Color) -> bool {
    let probe = position.clone_with_side_to_move(attacker);
    let opponent_royals = probe.royal_pieces(attacker.opposite());
    let mut moves = Vec::new();
    generator.generate_moves(&probe, &mut moves);

    moves.into_iter().any(|candidate| {
        probe
            .captured_squares(candidate)
            .into_iter()
            .flatten()
            .any(|square| opponent_royals.contains(square))
    })
}

/// 2升がチェビシェフ距離1で隣接するかを返す。
fn squares_are_adjacent(first: Square, second: Square) -> bool {
    first.file().abs_diff(second.file()) <= 1
        && first.rank().abs_diff(second.rank()) <= 1
        && first != second
}
