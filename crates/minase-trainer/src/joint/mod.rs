//! 鏡映共有PSTとFMを2群のAdamWで同時学習するCPU経路。
mod args;
mod batch;
mod checkpoint;
mod compute;
mod export;
mod model;
mod record;
mod train;

pub use args::Args;
pub use batch::Batch;
pub use checkpoint::{Checkpoint, Progress};
pub use compute::{Distribution, GradientNorm, Gradients, Losses, evaluate, gradients};
pub use export::QuantizationReport;
pub use model::{Adam, Model, initialize};
pub use record::{EpochRecord, StopReason, Summary};
pub use train::train;

/// 同時学習、配列検査、保存の結果。下位エラーの原因を保持する。
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
