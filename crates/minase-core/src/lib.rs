//! 中将棋のドメインロジックを担う中核モジュール。RULES.mdだけから正しさを検証できるコードのみを置く。

pub(crate) mod attacks;
pub mod board;
pub mod game;
pub mod movegen;
pub mod mv;
pub mod piece;
pub mod position;
pub mod predecessor;
pub mod promotion;
pub mod rules;

pub mod notation;
#[doc(hidden)]
pub mod rng;

#[cfg(any(test, feature = "test-util"))]
pub mod test_util;

pub use crate::board::Bitboard;
pub use crate::board::Direction;
pub use crate::board::{BOARD_FILES, BOARD_RANKS, BOARD_SQUARE_COUNT, RAW_SQUARE_COUNT, Square};
pub use crate::game::{
    DrawReason, Game, GameError, GameResult, GameStatus, IllegalMoveCause, WinReason,
};
pub use crate::movegen::{IllegalMove, MoveGenerator};
pub use crate::mv::Move;
pub use crate::piece::{Color, PieceCode, PieceKind};
pub use crate::position::{Position, PositionBuildError, PositionBuilder, PositionError};
pub use crate::predecessor::{PredecessorError, PredecessorGenerator};
pub use crate::promotion::PromotionChoice;
pub use crate::rules::{
    ExhaustionRule, LionRule, MoveRules, PromotionRule, RepetitionRule, RuleCode,
    RuleCodeParseError, RuleGroup, RuleSetParseError, Rules, RulesError,
};
pub use notation::sfen::{SfenError, parse_sfen, to_sfen};
