//! 裸玉の即時裁定と有効な駒の判定。

use super::super::result::{DrawReason, GameResult, WinReason};
use super::exhaustion::dead_pieces;
use crate::board::{Bitboard, Square};
use crate::movegen::MoveGenerator;
use crate::piece::{Color, PieceKind};
use crate::position::Position;

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
        // 条件(b): 裸玉側の駒が相手のいずれの王駒も次の着手で取れない。
        // 条件(c): 価値ある駒が3枚以上か、いずれも裸玉に隣接していない。
        if !position.royal_pieces(opponent).is_empty()
            && effective_pieces.popcount() >= 2
            && !can_capture_any_royal(position, generator, bare_color)
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

    // 双方が裸玉で、いずれの王駒も相手の次の着手で取られ得なければ引き分けとする。
    if bare_kings.into_iter().all(|king| king.is_some())
        && Color::ALL
            .into_iter()
            .all(|color| !can_capture_any_royal(position, generator, color))
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

/// 指定側の駒が相手のいずれかの王駒を次の着手で取れるかを返す（第22条第8項、第32条E3）。
pub(super) fn can_capture_any_royal(
    position: &Position,
    generator: &MoveGenerator,
    attacker: Color,
) -> bool {
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
