#![forbid(unsafe_code)]

mod adjudication;
pub mod core;
mod game;
pub mod notation;
pub mod protocol;
mod repetition;
#[doc(hidden)]
pub mod rng;

#[cfg(test)]
mod test_util;

pub use crate::core::bitboard::Bitboard;
pub use crate::core::direction::Direction;
pub use crate::core::movegen::{IllegalMove, MoveGenerator};
pub use crate::core::mv::{CapturedPiece, Move, Undo};
pub use crate::core::piece::{Color, PieceCode, PieceKind};
pub use crate::core::position::{Position, PositionBuildError, PositionBuilder, PositionError};
pub use crate::core::rules::{PromotionChoice, RuleCode, Rules, RulesError};
pub use crate::core::square::{
    BOARD_FILES, BOARD_RANKS, BOARD_SQUARE_COUNT, RAW_SQUARE_COUNT, Square,
};
pub use game::{
    DrawReason, Game, GameBuildError, GameError, GameResult, GameStatus, IllegalMoveCause,
    WinReason,
};
pub use notation::sfen::{SfenError, parse_sfen, to_sfen};
