//! 局面の不変条件の検査。

use super::Position;
use crate::core::board::{Bitboard, RAW_SQUARE_COUNT, Square};
use crate::core::piece::{Color, PieceKind};
use core::fmt;

/// [`Position`]の操作または[`Position::validate`]が検出する不正な状態。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PositionError {
    /// 先獅子の獅子捕獲升に手番側の駒がある。
    InvalidLionCapture {
        /// 問題の獅子捕獲升。
        square: Square,
    },
    /// 番兵位置に番兵コード以外が置かれている。
    PaddingIsNotWall {
        /// 検出した生の駒コード値。
        raw: u8,
    },
    /// 有効升に番兵コードが置かれている。
    ValidSquareIsWall {
        /// 問題の升。
        square: Square,
    },
    /// 駒コードから色または駒種を復元できない。
    InvalidPieceCode {
        /// 問題の升。
        square: Square,
    },
    /// 盤面配列と占有集合が食い違っている。
    OccupancyMismatch {
        /// 問題の升。
        square: Square,
    },
    /// 両対局者の駒集合が重なっている。
    ColorOverlap,
    /// 対局者別集合の集計が合わない。
    ColorAggregateMismatch {
        /// 問題の対局者。
        color: Color,
    },
    /// 駒種別集合が盤面配列と合わない。
    KindMismatch {
        /// 問題の升。
        square: Square,
        /// 問題の対局者。
        color: Color,
        /// 問題の駒種。
        kind: PieceKind,
    },
    /// いずれかのビットボードで番兵ビットが立っている。
    PaddingBitSet,
    /// 成り権保留ビットの升に適格な駒がない。
    InvalidPromotionDeferred {
        /// 問題の升。
        square: Square,
    },
    /// 通常Zobrist値が盤面・手番・先獅子状態からの再計算値と一致しない。
    ZobristMismatch,
    /// 成り権保留Zobrist値が成り権保留集合からの再計算値と一致しない。
    RightsZobristMismatch,
}

impl fmt::Display for PositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLionCapture { square } => write!(
                formatter,
                "invalid lion-capture square occupied by the side to move: {square:?}"
            ),
            _ => write!(formatter, "position invariant failed: {self:?}"),
        }
    }
}

impl std::error::Error for PositionError {}

impl Position {
    /// 盤面配列・ビットボード集計・両Zobrist値の整合など、内部不変条件を検査する。
    pub fn validate(&self) -> Result<(), PositionError> {
        let union = self.by_color[0] | self.by_color[1];
        if union != self.occupied {
            return Err(PositionError::ColorAggregateMismatch {
                color: Color::Black,
            });
        }
        if self.by_color[0].intersects(self.by_color[1]) {
            return Err(PositionError::ColorOverlap);
        }

        for color in Color::ALL {
            let mut kinds = Bitboard::EMPTY;
            for kind in PieceKind::ALL {
                kinds |= self.by_kind[color.index()][kind.index()];
            }
            if kinds != self.by_color[color.index()] {
                return Err(PositionError::ColorAggregateMismatch { color });
            }
        }

        for raw in 0..RAW_SQUARE_COUNT {
            match Square::from_raw(raw as u8) {
                None => {
                    if !self.board[raw].is_wall() {
                        return Err(PositionError::PaddingIsNotWall { raw: raw as u8 });
                    }
                }
                Some(square) => {
                    let piece = self.board[raw];
                    if piece.is_wall() {
                        return Err(PositionError::ValidSquareIsWall { square });
                    }
                    if piece.is_empty() {
                        if self.occupied.contains(square) {
                            return Err(PositionError::OccupancyMismatch { square });
                        }
                    } else {
                        let color = piece
                            .color()
                            .ok_or(PositionError::InvalidPieceCode { square })?;
                        let kind = piece
                            .kind()
                            .ok_or(PositionError::InvalidPieceCode { square })?;
                        if !self.occupied.contains(square) {
                            return Err(PositionError::OccupancyMismatch { square });
                        }
                        if !self.by_kind[color.index()][kind.index()].contains(square) {
                            return Err(PositionError::KindMismatch {
                                square,
                                color,
                                kind,
                            });
                        }
                    }
                }
            }
        }

        let padding_mask = [!Bitboard::VALID_WORD; 3];
        let has_padding = |board: Bitboard| {
            board
                .words()
                .iter()
                .zip(padding_mask)
                .any(|(word, mask)| word & mask != 0)
        };
        if has_padding(self.occupied)
            || self.by_color.into_iter().any(has_padding)
            || self.by_kind.into_iter().flatten().any(has_padding)
            || has_padding(self.promotion_deferred)
        {
            return Err(PositionError::PaddingBitSet);
        }
        for square in self.promotion_deferred {
            if !self.promotion_deferred_is_valid(square) {
                return Err(PositionError::InvalidPromotionDeferred { square });
            }
        }
        if self.zobrist != self.recompute_zobrist() {
            return Err(PositionError::ZobristMismatch);
        }
        if self.rights_zobrist != self.recompute_rights_zobrist() {
            return Err(PositionError::RightsZobristMismatch);
        }
        Ok(())
    }
}
