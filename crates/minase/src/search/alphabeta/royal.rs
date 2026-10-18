//! 王駒の捕獲と利きの判定。

use minase_core::board::Bitboard;
use minase_core::mv::Move;
use minase_core::position::Position;

/// 手番側のいずれかの王駒に相手駒の疑似利きが届くかを返す。
///
/// 第8条では王手放置も合法であり、王駒の捕獲を禁じる規則もないため、王駒の升では疑似利きと捕獲可能性が一致する。
/// 王将または玉将と太子の両方がある場合も、どちらか1枚に利きが届けば真とする。
/// 枝刈りから除く範囲の理由は`docs/adr/extend-shallow-pruning-to-stockfish-ranges-and-skip-it-when-a-royal-is-attacked.md`にある。
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
    captured: [Option<minase_core::Square>; 2],
) -> bool {
    let opponent = position.side_to_move().opposite();
    let royals = position.royal_pieces(opponent);
    captures_all_royals(royals, royals.popcount(), captured)
}

/// ノードで求めた相手の王駒集合と枚数を使い、捕獲升が残存王駒をすべて含むかを返す。
/// 王駒が0枚なら偽とし、2枚残る場合は両方の捕獲を要求する。
pub(super) fn captures_all_royals(
    royals: Bitboard,
    royal_count: u32,
    captured: [Option<minase_core::Square>; 2],
) -> bool {
    royal_count > 0
        && captured
            .into_iter()
            .flatten()
            .filter(|&square| royals.contains(square))
            .count()
            == royal_count as usize
}
