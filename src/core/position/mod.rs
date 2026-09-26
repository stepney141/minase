//! 局面の表現と、着手の適用・巻き戻し。

use self::lion_trigger::LionTrigger;
use crate::core::board::{Bitboard, RAW_SQUARE_COUNT, Square};
use crate::core::mv::Move;
use crate::core::piece::{COLOR_COUNT, Color, PIECE_KIND_COUNT, PieceCode, PieceKind};

mod builder;
mod lion_trigger;
mod make_move;
mod placement;
mod promotion_rights;
mod setup;
mod validate;
mod zobrist;

pub use builder::{PositionBuildError, PositionBuilder};
pub(crate) use make_move::Undo;
pub use validate::PositionError;

/// 中将棋の局面。盤面・手番・次の合法手に影響する一時状態を保持する。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Position {
    /// 各升の駒コード。番兵込みの生インデックスで引く。
    board: [PieceCode; RAW_SQUARE_COUNT],
    /// 駒がある升の集合。
    occupied: Bitboard,
    /// 対局者別の駒の集合。
    by_color: [Bitboard; COLOR_COUNT],
    /// 対局者別・駒種別の駒の集合。
    by_kind: [[Bitboard; PIECE_KIND_COUNT]; COLOR_COUNT],
    /// 手番側。
    side_to_move: Color,
    /// 直前の着手で獅子以外の駒に取られた獅子の情報。先獅子(第15条)の判定に使う。
    lion_taken_by_non_lion: Option<LionTrigger>,
    /// 現局面のzobristハッシュ。着手のたびに増分更新する。
    zobrist: u64,
    /// 成り権を保留中(P1・P2・P5)の駒がある升の集合。
    promotion_deferred: Bitboard,
    /// 成り権保留状態(P1・P2・P5)のzobristハッシュ。
    rights_zobrist: u64,
}

impl Position {
    /// 指定升の駒を返す。空升なら`None`を返す。
    #[inline]
    pub fn piece_at(&self, square: Square) -> Option<PieceCode> {
        let piece = self.board[square.raw_index()];
        (!piece.is_empty()).then_some(piece)
    }

    /// 駒がある升の集合を返す。
    #[inline]
    pub const fn occupied(&self) -> Bitboard {
        self.occupied
    }

    /// 指定した対局者の駒の集合を返す。
    #[inline]
    pub const fn pieces_of(&self, color: Color) -> Bitboard {
        self.by_color[color.index()]
    }

    /// 指定した対局者・駒種の駒の集合を返す。
    #[inline]
    pub const fn pieces_of_kind(&self, color: Color, kind: PieceKind) -> Bitboard {
        self.by_kind[color.index()][kind.index()]
    }

    /// 手番側を返す。
    #[inline]
    pub const fn side_to_move(&self) -> Color {
        self.side_to_move
    }

    /// 王駒(王将・玉将・太子、第3条)の集合を返す。
    pub fn royal_pieces(&self, color: Color) -> Bitboard {
        self.pieces_of_kind(color, PieceKind::King)
            | self.pieces_of_kind(color, PieceKind::CrownPrince)
    }

    /// 捕獲升を最大2升返す。
    ///
    /// 捕獲升とは、着手で実際に相手駒を取る升をいう。獅子、角鷹および
    /// 飛鷲の2段階移動では、1手で最大2升の相手駒を取る(第11条第3項・
    /// 第12条第4項)。
    /// 設計書movegen-speedup-2.md「段階5」に従い、盤面の駒コードを直接調べる。
    pub(crate) fn captured_squares(&self, mv: Move) -> [Option<Square>; 2] {
        let enemy = self.board[mv.from.raw_index()]
            .color()
            .expect("move origin must contain a piece")
            .opposite();
        let mid = match mv.mid {
            Some(square) if self.board[square.raw_index()].color() == Some(enemy) => Some(square),
            _ => None,
        };
        // 移動元は自駒なので、居喰いの到達升もこの比較だけで除外できる。
        let to = (self.board[mv.to.raw_index()].color() == Some(enemy)).then_some(mv.to);
        [mid, to]
    }
}

#[cfg(test)]
mod tests;
