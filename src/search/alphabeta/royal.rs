//! 王駒の捕獲と利きの判定。

use crate::core::board::Bitboard;
use crate::core::mv::Move;
use crate::core::position::Position;

/// 手番側のいずれかの王駒に相手駒の疑似利きが届くかを返す。
///
/// `docs/plans/strength-stage4.md`の「設計判断」の王駒への利きの判定に従う。
/// 王駒の捕獲を禁じる規則はなく、王手放置も合法（RULES.md第8条）なので、
/// 王駒の升では疑似利きと実際の捕獲可能性が一致する。
pub(super) fn royal_under_attack(position: &Position) -> bool {
    let side = position.side_to_move();
    position.royal_pieces(side).into_iter().any(|square| {
        !position
            .attackers_to_by(side.opposite(), square, position.occupied())
            .is_empty()
    })
}

/// 着手が相手の残存王駒をすべて取るかを返す(第21条第1項)。
pub(super) fn captures_last_royal(position: &Position, mv: Move) -> bool {
    captured_last_royal(position, position.captured_squares(mv))
}

/// 生成済みの捕獲升から最後の王駒の捕獲を判定する。
pub(super) fn captured_last_royal(
    position: &Position,
    captured: [Option<crate::Square>; 2],
) -> bool {
    let opponent = position.side_to_move().opposite();
    let royals = position.royal_pieces(opponent);
    captures_all_royals(royals, royals.popcount(), captured)
}

/// 設計書movegen-speedup-2.md「段階5」に従い、ノードで求めた王駒集合を使う。
pub(super) fn captures_all_royals(
    royals: Bitboard,
    royal_count: u32,
    captured: [Option<crate::Square>; 2],
) -> bool {
    royal_count > 0
        && captured
            .into_iter()
            .flatten()
            .filter(|&square| royals.contains(square))
            .count()
            == royal_count as usize
}
