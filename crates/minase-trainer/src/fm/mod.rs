//! 固定整数PSTへ加えるFMのCPU学習、再開、量子化と補正の変換。

mod args;
pub mod batch;
mod checkpoint;
pub(crate) mod compute;
mod convert;
mod export;
pub(crate) mod model;
pub(crate) mod npz;
mod quantize;
pub(crate) mod record;
mod replay;
pub(crate) mod report;
mod train;

pub use args::Args;
pub use batch::{Batch, feature_observations, mirror_bit};
pub use checkpoint::{Checkpoint, Progress};
pub use compute::{Distribution, GradientNorm, Gradients, Losses, gradients};
pub use export::{CorrectionDistribution, QuantizationReport};
pub use model::{Adam, Model};
pub use record::{EpochRecord, StopReason, Summary};
pub use train::train;

pub use convert::{
    Command, QuarterArgs, QuarterReport, ScaleArgs, ScaleReport, quarter, run, scale,
};
pub use quantize::{Quantized, ZeroCorrectionReport, quantize, zero_correction_report};
pub use report::{CorrectionReport, DifferenceReport, ValidationReport, tuning_correction};

/// FMの学習、配列検査、変換または保存の結果。下位のエラーは原因を保持する。
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
