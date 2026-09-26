//! 対局の進行管理と終局裁定。

mod adjudication;
mod error;
mod referee;
mod repetition;
mod result;

pub use error::{GameError, IllegalMoveCause};
pub use referee::Game;
pub use result::{DrawReason, GameResult, GameStatus, WinReason};

#[cfg(test)]
mod tests;
