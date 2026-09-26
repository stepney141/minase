//! 成りを保留した駒の権利の管理。

use super::Position;
use super::zobrist::ZobristKeys;
use crate::core::board::{Bitboard, Square};
use crate::core::piece::PieceKind;
use crate::core::promotion::in_promotion_zone;

impl Position {
    /// 成り権を保留中(P1・P2・P5)の駒がある升の集合を返す。
    #[inline]
    pub const fn promotion_deferred(&self) -> Bitboard {
        self.promotion_deferred
    }

    /// 成り権保留(P1・P2・P5)のビットを立て、権利ハッシュを更新する。
    pub(super) fn set_promotion_deferred(&mut self, square: Square, keys: &ZobristKeys) {
        if !self.promotion_deferred.contains(square) {
            self.promotion_deferred.set(square);
            self.rights_zobrist ^= keys.promotion_deferred(square);
        }
    }

    /// 成り権保留(P1・P2・P5)のビットを消し、権利ハッシュを更新する。
    pub(super) fn clear_promotion_deferred(&mut self, square: Square, keys: &ZobristKeys) {
        if self.promotion_deferred.contains(square) {
            self.promotion_deferred.clear(square);
            self.rights_zobrist ^= keys.promotion_deferred(square);
        }
    }

    /// 指定升の駒が成り権保留状態(P1・P2・P5)を持てるかどうかを返す。
    pub(super) fn promotion_deferred_is_valid(&self, square: Square) -> bool {
        self.piece_at(square).is_some_and(|piece| {
            let Some(color) = piece.color() else {
                return false;
            };
            !piece.is_promoted()
                && piece.kind().is_some_and(PieceKind::can_promote)
                && in_promotion_zone(color, square)
        })
    }
}
