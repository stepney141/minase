//! 中将棋のドメインロジックを担う中核モジュール。RULES.mdだけから正しさを検証できるコードのみを置く。

pub(crate) mod adjudication;
mod attacks;
pub mod bitboard;
pub mod direction;
pub(crate) mod game;
pub mod movegen;
pub mod mv;
pub mod piece;
pub mod position;
pub(crate) mod repetition;
pub mod rules;
pub mod square;
