//! 評価関数に依存しないMNSDの読み込み、教師値の構築とデータの分割。
//!
//! 教師探索値は手番側視点のセンチポーンで保持する。教師用Kで勝率へ変換し、
//! 来歴の分類ごとの混合比λで手番側の終局結果と混ぜて教師値を作る。

mod error;
mod mnsd;
pub mod replay;
mod shuffle;
mod teacher;

pub use error::Error;
pub use mnsd::{Dataset, RecordLocation, TeacherClass};
pub(crate) use mnsd::{GameKey, outcome_code};
pub use shuffle::{epoch_permutation, hash64, is_validation};
pub use teacher::{
    Lookahead, build_targets_from_arrays, compute_lookahead, estimate_k, validate_lambda_override,
};

/// 共有データの読み込み、検証または教師値の構築結果。
pub type Result<T> = std::result::Result<T, Error>;
