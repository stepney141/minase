//! 探索方式に依存しない入力、出力、および実行の境界。
//!
//! [`Searcher`]が方式ごとの状態とオプションを束ね、[`SearchHandle`]との間で所有権を受け渡す。
//! [`SearchLimits`]、[`SearchSnapshot`]、[`SearchEvent`]は各方式が共通に使う。
//! αβ探索に固有の処理は`alphabeta`へ閉じ、同期版の[`search()`]は
//! 呼び出し側が用意した置換表と[`HistoryTables`]でαβ探索を実行する。

pub mod alphabeta;
mod budget;
#[cfg(test)]
mod budget_tests;
mod clock;
mod error;
mod events;
mod handle;
mod limits;
pub mod mcts;
mod searcher;
mod snapshot;
#[cfg(feature = "search-stats")]
mod stats;

#[cfg(feature = "search-stats")]
pub use stats::SearchStats;

pub use alphabeta::history::HistoryTables;
#[cfg(test)]
pub(crate) use alphabeta::history_decay_for_test;
#[cfg(test)]
pub(crate) use alphabeta::tt::TtValue;
pub(crate) use alphabeta::tt::{Bound, Hit};
pub use alphabeta::tt::{
    DEFAULT_SIZE_MB as DEFAULT_TT_SIZE_MB, TranspositionTable, TranspositionTableError,
};
pub use error::SearchError;
pub use events::{SearchEvent, SearchResult, StopReason};
pub use handle::{SearchHandle, SearchJoinError, search};
pub use limits::{ClockLimits, SearchLimits};
pub use searcher::{
    PrepareError, SearchOption, SearchOptionKind, SearchOptionValue, Searcher, SetOptionError,
    StartError, StartFailure,
};
pub use snapshot::{SearchRepetition, SearchSnapshot};

use core::num::NonZeroUsize;

/// 王駒の捕獲または合法手なしによる勝敗を表す評価値の基準。
/// 勝ちの値は `MATE - d`、負けの値はその負値とし、dは双方の各1着手を1手と数えた根から終局までの手数である。
/// dには最後の王駒を取る着手も含める。
pub const MATE: i32 = 30_000;

/// 探索が扱う最大ply。
pub const MAX_PLY: u32 = 256;

/// 絶対値がこの値以上なら詰み帯として扱う閾値。
/// 静的評価の絶対値はこの値未満に保ち、置換表に保存する評価値は詰み距離の変換後も`i16`に収める。
pub const MATE_THRESHOLD: i32 = MATE - MAX_PLY as i32;

/// 反復による勝敗を表す専用の帯の勝ちの基準値。勝ちは `REPETITION_WIN - ply`、負けはその負値。
///
/// R1の攻撃側の負けと、R2・R3で合法手がすべて禁止手の局面の負けに使う。
/// どちらも対局履歴と探索経路に依存する値なので、置換表では手数補正をせず、打ち切りにも使わない。
/// 通常の評価値はNNUEの`EVALUATION_LIMIT`で絶対値28,999以下に制限され、この帯に届かない。
pub const REPETITION_WIN: i32 = MATE_THRESHOLD - 1;

/// 絶対値がこの値以上`MATE_THRESHOLD`未満なら、反復による勝敗の専用の帯とする。
/// 用途と置換表での扱いは[`REPETITION_WIN`]に従う。
pub const REPETITION_BAND_START: i32 = REPETITION_WIN - MAX_PLY as i32;

const _: () = assert!(crate::eval::nnue::EVALUATION_LIMIT < REPETITION_BAND_START);
const _: () = assert!(REPETITION_BAND_START < MATE_THRESHOLD);

/// 評価値が、経路に依存する反復の勝敗の専用の帯にあるかを返す。
pub(crate) fn in_repetition_band(score: i32) -> bool {
    (REPETITION_BAND_START..MATE_THRESHOLD).contains(&score.abs())
}

/// 引き分けを表す評価値。
pub const DRAW_SCORE: i32 = 0;

/// RULES.md第31条R1の引き分けの値の絶対値の上限。
/// 評価関数の出力の範囲と同じであり、[`REPETITION_BAND_START`]以上の
/// 反復の勝敗の専用の帯にも、詰みの帯にも入らない。
pub const DRAW_VALUE_LIMIT: i32 = crate::eval::nnue::EVALUATION_LIMIT;

const _: () = assert!(DRAW_VALUE_LIMIT < REPETITION_BAND_START);

/// 探索に使う既定のワーカー数。
/// 利用可能なCPU数で暗黙に変えず、並列探索は呼び出し側が明示したときに使う。
/// 判断の理由は`docs/adr/keep-default-threads-at-one.md`にある。
pub const DEFAULT_THREADS: NonZeroUsize = NonZeroUsize::new(1).unwrap();
