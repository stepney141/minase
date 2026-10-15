//! 対局の進行管理と終局裁定。
//!
//! `adjudication`はRULES.md第25条や第32条E3の「裁定」に対応する名前である。
//! 終局裁定の`adjudication`と反復の`repetition`は、審判[`Game`]の内部で使う子モジュールである。

mod adjudication;
mod error;
mod referee;
mod repetition;
mod result;
mod root_repetition;

pub use error::{GameError, IllegalMoveCause};
pub use referee::Game;
pub use repetition::{move_was_attacking, repetition_key};
pub use result::{DrawReason, GameResult, GameStatus, WinReason};
pub use root_repetition::RepetitionRootMoves;

#[cfg(test)]
mod tests;
