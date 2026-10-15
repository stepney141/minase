//! 中将棋の規則と表記を提供するライブラリ。
//!
//! RULES.mdに基づき、盤、駒、着手、局面、駒の動き、合法手生成、成り、規則セット、
//! 審判、および直前局面生成を実装する。
//! [`notation`]はSFENとUSIおよびCECPの指し手表記を扱う。
//! 実行ファイルと試験が共有する決定的な乱数は、安定した公開面に含めず、`rng`を`#[doc(hidden)]`で公開する。
//! 試験標本の`test_util`はライブラリ自身の試験とフィーチャ`test-util`で利用できる。
//! 探索、評価、およびプロトコルはエンジン`minase`が担う。
//! 分担の理由は`docs/adr/split-rules-library-from-engine.md`にある。

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
    DrawReason, Game, GameError, GameResult, GameStatus, IllegalMoveCause, RepetitionRootMoves,
    WinReason, move_was_attacking,
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
