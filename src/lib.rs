#![forbid(unsafe_code)]

pub mod attacks;
pub mod bitboard;
pub mod direction;
pub mod movegen;
pub mod mv;
pub mod piece;
pub mod position;
pub mod rules;
pub mod square;

pub use attacks::{AttackTables, attack_tables};
pub use bitboard::Bitboard;
pub use direction::Direction;
pub use movegen::{IllegalMove, MoveGenerator};
pub use mv::{CapturedPiece, Move, Undo};
pub use piece::{Color, PieceCode, PieceKind};
pub use position::{Position, PositionBuildError, PositionBuilder, PositionError};
pub use rules::{PromotionChoice, RuleCode, Rules, RulesError};
pub use square::{BOARD_FILES, BOARD_RANKS, BOARD_SQUARE_COUNT, RAW_SQUARE_COUNT, Square};
