//! 駒の配置と除去の基本操作。

use super::zobrist::ZobristKeys;
use super::{Position, PositionBuildError};
use crate::core::board::Square;
use crate::core::piece::PieceCode;

impl Position {
    /// 駒を置き、占有集合とzobristハッシュを増分更新する。
    pub(super) fn put_piece(
        &mut self,
        square: Square,
        piece: PieceCode,
        keys: &ZobristKeys,
    ) -> Result<(), PositionBuildError> {
        self.put_piece_without_hash(square, piece)?;
        self.zobrist ^= keys.piece(square, piece);
        Ok(())
    }

    /// 「段階6」（movegen-speedup-2.md）の復元用に、盤面と占有集合だけを更新する。
    pub(super) fn put_piece_without_hash(
        &mut self,
        square: Square,
        piece: PieceCode,
    ) -> Result<(), PositionBuildError> {
        if piece.is_empty() || piece.is_wall() {
            return Err(PositionBuildError::EmptyOrWallPiece);
        }
        if self.piece_at(square).is_some() {
            return Err(PositionBuildError::SquareOccupied { square });
        }

        let color = piece.color().expect("validated piece must have a color");
        let kind = piece.kind().expect("validated piece must have a kind");
        self.board[square.raw_index()] = piece;
        self.occupied.set(square);
        self.by_color[color.index()].set(square);
        self.by_kind[color.index()][kind.index()].set(square);
        Ok(())
    }

    /// 駒を取り除き、占有集合とzobristハッシュを増分更新して駒を返す。
    pub(super) fn remove_piece(&mut self, square: Square, keys: &ZobristKeys) -> PieceCode {
        let piece = self.remove_piece_without_hash(square);
        self.zobrist ^= keys.piece(square, piece);
        piece
    }

    /// 「段階6」（movegen-speedup-2.md）の復元用に、ハッシュを変更せず駒を除く。
    pub(super) fn remove_piece_without_hash(&mut self, square: Square) -> PieceCode {
        let piece = self.board[square.raw_index()];
        debug_assert!(!piece.is_empty() && !piece.is_wall());
        let color = piece.color().expect("occupied square must have a color");
        let kind = piece.kind().expect("occupied square must have a kind");

        self.board[square.raw_index()] = PieceCode::EMPTY;
        self.occupied.clear(square);
        self.by_color[color.index()].clear(square);
        self.by_kind[color.index()][kind.index()].clear(square);
        piece
    }
}
