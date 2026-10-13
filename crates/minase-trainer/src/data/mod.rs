//! 評価関数に依存しないMNSDの読み込み、教師値の構築とデータの分割。
//!
//! 教師探索値は手番側視点のセンチポーンで保持する。
//! 終局結果は手番側の負け0、引き分け1、勝ち2である。
//! [`build_targets_from_arrays`]は教師用Kで探索値を確率へ変換し、分類ごとの混合比λで終局結果と混ぜる。
//! 教師値は`λ·σ(score/K_teacher) + (1−λ)·result/2`、`σ(x) = 1/(1+exp(−x))`である。
//! 教師用Kは分類ごとに訓練集合の探索値と終局結果だけから[`estimate_k`]で推定し、混合後の教師確率は推定に使わない。
//! モデルの出力用Kは初期の重みファイルMNPTから引き継ぐ固定値であり、教師用Kとは別の量である。
//! 検証集合は対局単位で約5%（20分の1）を固定し、同じ対局の局面を訓練と検証に分けない。
//! 分割の規則は[`Dataset::open`]にある。

mod error;
mod mnsd;
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
