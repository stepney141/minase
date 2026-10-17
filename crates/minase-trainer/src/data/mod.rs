//! 評価関数に依存しないMNSDの読み込み、教師値の構築とデータの分割。
//!
//! MNSDを1局ずつ再生して受け手へ渡す。B=1の[`Dataset`]は、受け手が全局面をメモリへ集めたものである。
//! 教師の勝率尺度は標本の列を検査で1回、損失評価で66回なぞって推定する。
//!
//! 教師探索値は手番側視点のセンチポーンで保持する。
//! 終局結果は手番側の負け0、引き分け1、勝ち2である。
//! [`build_targets_from_arrays`]は教師の勝率尺度（1ロジットあたりのcp）で探索値を確率へ変換し、分類ごとの混合比λで終局結果と混ぜる。
//! 教師値は`λ·σ(score/teacher_cp_per_logit) + (1−λ)·result/2`、`σ(x) = 1/(1+exp(−x))`である。
//! 教師の勝率尺度は分類ごとに訓練集合の探索値と終局結果だけから[`estimate_cp_per_logit`]で推定し、混合後の教師確率は推定に使わない。
//! モデルの勝率尺度は初期の重みファイルMNPTから引き継ぐ固定値であり、教師の勝率尺度とは別の量である。
//! 検証集合は対局単位で約5%（20分の1）を固定し、同じ対局の局面を訓練と検証に分けない。
//! 分割の規則は[`Dataset::open`]にある。

mod calibration;
mod error;
mod mnsd;
mod sample;
mod shuffle;
pub mod spill;
pub(crate) mod stream;
mod subsample;
mod teacher;

pub(crate) use calibration::TeacherCalibration;
pub use calibration::{CalibrateArgs, TeacherCpPerLogitSource, calibrate};
pub use error::Error;
pub use mnsd::{Dataset, TeacherClass};
pub(crate) use mnsd::{GameKey, outcome_code};
pub use sample::{Outcome, Sample};
pub use shuffle::{
    BucketAssigner, bucket_permutation, epoch_permutation, floyd_sample, hash64, is_validation,
    rao_sandelius_order,
};
pub use spill::{EpochReader, SpillRecord, SpillStore};
pub use subsample::TrainSubsample;
pub use teacher::{
    CpPerLogitEstimate, Lookahead, build_targets_from_arrays, compute_lookahead,
    estimate_cp_per_logit, parse_lambda, validate_lambda,
};

/// 共有データの読み込み、検証または教師値の構築結果。
pub type Result<T> = std::result::Result<T, Error>;
