//! 中将棋の合法手生成ライブラリと対局エンジンMinase。
//!
//! 準拠する競技規則とローカルルールはRULES.mdが定める。`core`は
//! RULES.mdだけから正しさを検証できる盤・駒・合法手・裁定を提供し、
//! 表記(`notation`)、プロトコル(`protocol`)、探索(`search`)、
//! 評価(`eval`)はその外に置く。

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod core;
pub mod eval;
pub mod notation;
pub mod protocol;
#[doc(hidden)]
pub mod rng;
pub mod search;
#[doc(hidden)]
pub mod stats;

#[cfg(test)]
mod test_util;

pub use crate::core::bitboard::Bitboard;
pub use crate::core::direction::Direction;
pub use crate::core::game::{
    DrawReason, Game, GameError, GameResult, GameStatus, IllegalMoveCause, WinReason,
};
pub use crate::core::movegen::{IllegalMove, MoveGenerator};
pub use crate::core::mv::{CapturedPiece, Move, Undo};
pub use crate::core::piece::{Color, PieceCode, PieceKind};
pub use crate::core::position::{Position, PositionBuildError, PositionBuilder, PositionError};
pub use crate::core::rules::{
    ExhaustionRule, LionRule, MoveRules, PromotionChoice, PromotionRule, RepetitionRule, RuleCode,
    RuleGroup, Rules, RulesError,
};
pub use crate::core::square::{
    BOARD_FILES, BOARD_RANKS, BOARD_SQUARE_COUNT, RAW_SQUARE_COUNT, Square,
};
pub use notation::sfen::{SfenError, parse_sfen, to_sfen};
