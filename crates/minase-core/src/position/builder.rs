//! 駒の配置による局面の構築。

use super::zobrist::zobrist_keys;
use super::{Position, PositionError};
use crate::board::Square;
use crate::piece::{Color, PieceCode};
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
///
/// 表現の不変条件を検査し、合法な対局で到達できるかは検査しない。
/// 王駒が0枚の局面も構築できる。
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
    ///
    /// [`Self::put`]は通常Zobrist値、[`Self::mark_promotion_deferred`]は成り権保留Zobrist値を
    /// 増分更新する。
    /// [`Position::validate`]で両値と再計算値の一致を検査し、そのまま返す。
    /// 検査後に再計算値で上書きしないことで、増分更新の誤りを隠さず検出する。
    pub fn finish(self) -> Result<Position, PositionBuildError> {
        self.position
            .validate()
            .map_err(PositionBuildError::InvalidPosition)?;
        Ok(self.position)
    }
}
