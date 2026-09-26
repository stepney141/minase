//! 評価関数と静止探索を使って着手を選ぶ探索。

pub(crate) mod alphabeta;
mod error;
mod events;
mod handle;
mod limits;
mod snapshot;

pub use alphabeta::tt::{
    DEFAULT_SIZE_MB as DEFAULT_TT_SIZE_MB, TranspositionTable, TranspositionTableError,
};
pub use error::SearchError;
pub use events::{SearchEvent, SearchResult, StopReason};
pub use handle::{SearchHandle, search, start_search};
pub use limits::{ClockLimits, SearchLimits};
pub use snapshot::SearchSnapshot;

use core::num::NonZeroUsize;

/// 詰みを表す評価値。
pub const MATE: i32 = 30_000;

/// 探索が扱う最大ply。
pub const MAX_PLY: u32 = 256;

/// 詰み手数を含む評価値の下限。
pub const MATE_THRESHOLD: i32 = MATE - MAX_PLY as i32;

/// 引き分けを表す評価値。
pub const DRAW_SCORE: i32 = 0;

/// 探索に使う既定のワーカー数。
pub const DEFAULT_THREADS: NonZeroUsize = NonZeroUsize::new(1).unwrap();
