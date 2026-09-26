//! 中将棋の合法手生成ライブラリと対局エンジンMinase。
//!
//! 準拠する競技規則とローカルルールはRULES.mdが定める。`core`は
//! RULES.mdだけから正しさを検証できる盤・駒・合法手・裁定を提供し、
//! 表記(`notation`)、プロトコル(`protocol`)、探索(`search`)、
//! 評価(`eval`)、評価関数の学習データの形式(`training`)、
//! 学習局面の生成支援(`datagen`)はその外に置く。

pub mod core;
#[doc(hidden)]
pub mod datagen;
pub mod eval;
#[doc(hidden)]
pub mod harness;
pub mod notation;
pub mod protocol;
#[doc(hidden)]
pub mod rng;
pub mod search;
#[doc(hidden)]
pub mod stats;
pub mod training;

#[cfg(test)]
mod test_util;

pub use crate::core::board::Bitboard;
pub use crate::core::board::Direction;
pub use crate::core::board::{
    BOARD_FILES, BOARD_RANKS, BOARD_SQUARE_COUNT, RAW_SQUARE_COUNT, Square,
};
pub use crate::core::game::{
    DrawReason, Game, GameError, GameResult, GameStatus, IllegalMoveCause, WinReason,
};
pub use crate::core::movegen::{IllegalMove, MoveGenerator};
pub use crate::core::mv::Move;
pub use crate::core::piece::{Color, PieceCode, PieceKind};
pub use crate::core::position::{Position, PositionBuildError, PositionBuilder, PositionError};
pub use crate::core::predecessor::{PredecessorError, PredecessorGenerator};
pub use crate::core::promotion::PromotionChoice;
pub use crate::core::rules::{
    ExhaustionRule, LionRule, MoveRules, PromotionRule, RepetitionRule, RuleCode,
    RuleCodeParseError, RuleGroup, RuleSetParseError, Rules, RulesError,
};
pub use notation::sfen::{SfenError, parse_sfen, to_sfen};
