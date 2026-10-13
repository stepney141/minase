//! 局面の表現と、着手の適用・巻き戻し。

use self::lion_trigger::LionTrigger;
use crate::board::{Bitboard, RAW_SQUARE_COUNT, Square};
use crate::mv::Move;
use crate::piece::{COLOR_COUNT, Color, PIECE_KIND_COUNT, PieceCode, PieceKind};

mod builder;
mod lion_trigger;
mod make_move;
mod placement;
mod promotion_rights;
mod setup;
mod validate;
mod zobrist;

pub use builder::{PositionBuildError, PositionBuilder};
pub use make_move::Undo;
pub use validate::PositionError;

/// 盤面、手番、および次の合法手に影響する一時状態を保持する中将棋の局面。
///
/// 一時状態は先獅子の状態と成り権保留集合であり、着手履歴は持たない。
/// 履歴を分離する理由は`docs/adr/keep-game-history-out-of-position-and-movegen.md`にある。
/// 導出した`PartialEq`と`Eq`はZobrist値などの派生値を含む全フィールドを比較する。
/// RULES.md第24条の同一局面の判定そのものではなく、反復は対局管理層の局面キーで判定する。
///
/// 着手の適用は、駒の配置と除去によって触れた升とその集計だけを増分更新する。
/// [`Self::unmake_move`]は[`Undo`]から全フィールドと両Zobrist値を復元し、局面を複製しない。
/// 復元値を再計算で作ることはないが、検証用のビルドでは再計算値との一致を検査する。
/// 局面の複製は、対局管理や検証など、着手の適用と復元ほど頻繁でない処理で用いる。
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
    /// 盤面の駒コードを直接読み、経由升と到達升のうち相手駒がある升だけをこの順に返す。
    /// 移動元には自駒があるので、居喰いとじっとの到達升もこの比較だけで除外される。
    pub fn captured_squares(&self, mv: Move) -> [Option<Square>; 2] {
        let enemy = self.board[mv.from.raw_index()]
            .color()
            .expect("move origin must contain a piece")
            .opposite();
        let mid = match mv.mid {
            Some(square) if self.board[square.raw_index()].color() == Some(enemy) => Some(square),
            _ => None,
        };
        let to = (self.board[mv.to.raw_index()].color() == Some(enemy)).then_some(mv.to);
        [mid, to]
    }
}

#[cfg(test)]
mod tests;
