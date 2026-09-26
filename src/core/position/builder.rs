//! 駒の配置による局面の構築。

use super::zobrist::zobrist_keys;
use super::{Position, PositionError};
use crate::core::board::Square;
use crate::core::piece::{Color, PieceCode};
use core::fmt;

/// [`PositionBuilder`]による局面構築のエラー。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PositionBuildError {
    /// 既に駒がある升へ配置しようとした。
    SquareOccupied {
        /// 問題の升。
        square: Square,
    },
    /// 空升または番兵のコードを駒として配置しようとした。
    EmptyOrWallPiece,
    /// 完成した局面が不変条件を満たさない。
    InvalidPosition(PositionError),
}

impl fmt::Display for PositionBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "position construction failed: {self:?}")
    }
}

impl std::error::Error for PositionBuildError {}

/// 駒を1枚ずつ置いて局面を組み立てるビルダー。
pub struct PositionBuilder {
    /// 構築中の局面。
    pub(super) position: Position,
}

impl PositionBuilder {
    /// 指定手番の空局面から構築を始める。
    pub fn new(side_to_move: Color) -> Self {
        Self {
            position: Position::empty(side_to_move),
        }
    }

    /// 指定升へ駒を置く。
    pub fn put(&mut self, square: Square, piece: PieceCode) -> Result<(), PositionBuildError> {
        self.position.put_piece(square, piece, zobrist_keys())
    }

    /// 指定升の駒を成り権保留中(P1・P2・P5)として記録する。
    pub fn mark_promotion_deferred(&mut self, square: Square) -> Result<(), PositionBuildError> {
        if !self.position.promotion_deferred_is_valid(square) {
            return Err(PositionBuildError::InvalidPosition(
                PositionError::InvalidPromotionDeferred { square },
            ));
        }
        self.position.set_promotion_deferred(square, zobrist_keys());
        Ok(())
    }

    /// 両Zobrist値を含む不変条件を検査して局面を返す。
    pub fn finish(self) -> Result<Position, PositionBuildError> {
        self.position
            .validate()
            .map_err(PositionBuildError::InvalidPosition)?;
        Ok(self.position)
    }
}
