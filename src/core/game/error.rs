//! 着手または対局操作が受理されなかった原因。

use core::fmt;

use crate::core::movegen::IllegalMove;
use crate::core::mv::Move;

/// 対局管理層が着手を拒否した原因。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IllegalMoveCause {
    /// 駒の動きまたは獅子の捕獲制限に反する着手。
    Movement,
    /// R2またはR3が禁止する反復着手(第31条)。
    Repetition,
}

impl fmt::Display for IllegalMoveCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Movement => formatter.write_str("illegal movement"),
            Self::Repetition => formatter.write_str("forbidden repetition"),
        }
    }
}

impl std::error::Error for IllegalMoveCause {}

/// 着手または対局操作が受理されなかった原因。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameError {
    /// 対局が既に終了している(第26条第12項)。
    GameAlreadyOver,
    /// 不合法な着手(第26条)。
    IllegalMove {
        /// 拒否された着手。
        mv: Move,
        /// 拒否の原因。
        cause: IllegalMoveCause,
    },
}

impl fmt::Display for GameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GameAlreadyOver => formatter.write_str("the game is already over"),
            Self::IllegalMove {
                mv,
                cause: IllegalMoveCause::Movement,
            } => IllegalMove(*mv).fmt(formatter),
            Self::IllegalMove {
                mv,
                cause: IllegalMoveCause::Repetition,
            } => write!(formatter, "the move is forbidden by repetition: {mv:?}"),
        }
    }
}

impl std::error::Error for GameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IllegalMove { cause, .. } => Some(cause),
            Self::GameAlreadyOver => None,
        }
    }
}

impl From<IllegalMove> for GameError {
    fn from(IllegalMove(mv): IllegalMove) -> Self {
        Self::IllegalMove {
            mv,
            cause: IllegalMoveCause::Movement,
        }
    }
}
