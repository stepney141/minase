//! Simpleの照合・計測の引数。CUDA処理はフェーズ4bで実装する。
use crate::nnue::{
    options::{DeviceKind, TrainingOptions},
    train,
};
use serde_json::Value;
use std::path::PathBuf;
#[derive(Clone, clap::Args)]
/// CPU/CUDA一致検査の条件。
pub struct CheckArgs {
    #[arg(long)]
    /// 1バッチの局面数。
    pub batch: usize,
    #[arg(long)]
    /// 初期化と合成局面の乱数シード。
    pub seed: u64,
    #[arg(long)]
    /// 教師の勝率尺度、cp/ロジット。
    pub teacher_cp_per_logit: f64,
    #[command(flatten)]
    /// 学習率、重み減衰と教師の混合比。
    pub training: TrainingOptions,
}
#[derive(clap::Args)]
/// 更新時間の計測条件。
pub struct BenchArgs {
    #[command(flatten)]
    /// 合成入力と最適化器の条件。
    pub check: CheckArgs,
    #[arg(long, value_enum, default_value_t = DeviceKind::Cuda)]
    /// 計測する実行デバイス。
    pub device: DeviceKind,
    #[arg(long, default_value_t = 10)]
    /// 計測する更新数。
    pub steps: usize,
    #[arg(long)]
    /// CPUの入力供給の並列度。
    pub threads: Option<usize>,
    /// バッチ供給の計測に使うMNSDファイル。
    #[arg(long, required = true, num_args = 1..)]
    pub data: Vec<PathBuf>,
    /// 前進、後退、最適化と転送の時間内訳を出力する。
    #[arg(long)]
    pub profile: bool,
}
/// SimpleのCUDA実装が未実装であることを明示する。
pub fn check(_args: CheckArgs) -> train::Result<Value> {
    Err(super::Error::CudaUnavailable.into())
}
/// CUDAを含む更新計測はフェーズ4bで実装する。
pub fn bench(_args: BenchArgs) -> train::Result<Value> {
    Err(super::Error::CudaUnavailable.into())
}
