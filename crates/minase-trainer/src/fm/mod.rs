//! FM専用の量子化、MNPT版3の保存、補正の縮小と倍率の焼き込み。

mod convert;
mod npz;
mod quantize;
mod report;

pub use convert::{
    Command, QuarterArgs, QuarterReport, ScaleArgs, ScaleReport, quarter, run, scale,
};
pub use quantize::{Quantized, ZeroCorrectionReport, quantize, zero_correction_report};
pub use report::{CorrectionReport, DifferenceReport, ValidationReport, tuning_correction};

/// FMの配列検査、変換または保存の結果。下位のエラーは原因を保持する。
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
