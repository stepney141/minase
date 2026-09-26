//! 中将棋のドメインロジックを担う中核モジュール。RULES.mdだけから正しさを検証できるコードのみを置く。

pub(crate) mod adjudication;
pub(crate) mod attacks;
pub mod board;
pub(crate) mod game;
pub mod movegen;
pub mod mv;
pub mod piece;
pub mod position;
pub mod predecessor;
pub(crate) mod repetition;
pub mod rules;
