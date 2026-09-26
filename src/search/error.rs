//! 探索入力と初期化のエラー。

use core::fmt;

use crate::search::MAX_PLY;

/// 探索入力または探索器の初期化に失敗した理由。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SearchError {
    /// 有限探索に停止条件が指定されていない。
    MissingLimit,
    /// 探索深さが1以上[`MAX_PLY`]以下でない。
    InvalidDepth {
        /// 指定された探索深さ。
        depth: u32,
    },
    /// ノード数上限が0である。
    ZeroNodeLimit,
    /// 固定探索時間が0 msである。
    ZeroMoveTime,
    /// 持ち時間、加算時間、秒読み時間がすべて0 msである。
    EmptyClock,
    /// 探索できるルート合法手がない。
    NoLegalMoves,
    /// 終局済みの対局が指定された。
    FinishedGame,
    /// 同期探索に無期限探索が指定された。
    InfiniteSynchronousSearch,
}

impl fmt::Display for SearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingLimit => formatter.write_str("search requires at least one limit"),
            Self::InvalidDepth { depth } => write!(
                formatter,
                "search depth must be between 1 and {MAX_PLY}, got {depth}"
            ),
            Self::ZeroNodeLimit => formatter.write_str("search node limit must be non-zero"),
            Self::ZeroMoveTime => formatter.write_str("search movetime must be non-zero"),
            Self::EmptyClock => formatter.write_str("search clock must contain non-zero time"),
            Self::NoLegalMoves => formatter.write_str("search requires at least one legal move"),
            Self::FinishedGame => formatter.write_str("search requires an ongoing game"),
            Self::InfiniteSynchronousSearch => {
                formatter.write_str("synchronous search cannot use an infinite limit")
            }
        }
    }
}

impl std::error::Error for SearchError {}
