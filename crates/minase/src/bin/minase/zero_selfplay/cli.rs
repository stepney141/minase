//! 既定値を持たない自己対局の生成条件と実行条件。

use std::{num::NonZeroUsize, path::PathBuf};

use clap::ValueEnum;
use minase::datagen::git::validate_commit_hash;
use minase::nn::{ExecutionProvider, tensorrt_cache::cache_root};
use serde::{Deserialize, Serialize};

use super::Error;

/// AlphaZero型の自己対局生成器の起動引数。
#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// 生成条件、塊、および集計を保存するディレクトリ。
    #[arg(long, required = true)]
    pub(super) output_dir: PathBuf,
    /// 方策と価値を評価するONNX形式のネット。
    #[arg(long, required = true)]
    pub(super) nn_model: PathBuf,
    /// 推論に使う実行プロバイダ。
    #[arg(long, required = true, value_enum)]
    pub(super) nn_backend: Backend,
    /// 生成器の40桁のコミットハッシュ。
    #[arg(long, required = true, value_parser = parse_commit)]
    pub(super) generation_commit: String,
    /// サイクルの基本シード。
    #[arg(long, required = true)]
    pub(super) seed: u64,
    /// 自己対局を始める局面の種類。
    #[arg(long, required = true, value_enum)]
    pub(super) start_position: StartPosition,
    /// サイクルで生成する対局数の上限。
    #[arg(long, required = true, value_parser = clap::value_parser!(u32).range(1..))]
    pub(super) games: u32,
    /// 探索した全局面から数える訓練局面数の目標。1以上とし、塊0からの累計で判定する。
    #[arg(long, required = true, value_parser = clap::value_parser!(u64).range(1..))]
    pub(super) target_positions: u64,
    /// 1つの塊の対局数。最後の塊は端数を許す。
    #[arg(long, required = true, value_parser = clap::value_parser!(u32).range(1..))]
    pub(super) chunk_games: u32,
    /// 全手番のシミュレーション数。1以上。
    #[arg(long, required = true, value_parser = clap::value_parser!(u32).range(1..))]
    pub(super) simulations: u32,
    /// 投了の閾値。有限な0以上1以下の値とし、0なら投了しない。
    #[arg(long, required = true, value_parser = parse_resign_threshold)]
    pub(super) resign_threshold: f64,
    /// 推論器を1つずつ所有するスレッド数。
    #[arg(long, required = true)]
    pub(super) threads: NonZeroUsize,
    /// 使用するGPUの枚数。スレッド数をこの枚数の倍数とし、CPUでは1を指定する。
    #[arg(long, required = true, value_name = "N")]
    pub(super) devices: NonZeroUsize,
    /// 各スレッドの同時対局数と推論器の最大バッチ。
    #[arg(long, required = true)]
    pub(super) games_per_thread: NonZeroUsize,
}

impl Arguments {
    /// GPUやONNX Runtimeに触れる前に、スレッド数と機器数の組合せを検査する。
    pub(super) fn validate_execution(&self) -> Result<(), Error> {
        if !self.threads.get().is_multiple_of(self.devices.get()) {
            return Err(Error::ThreadsNotMultipleOfDevices {
                threads: self.threads,
                devices: self.devices,
            });
        }
        if matches!(self.nn_backend, Backend::Cpu) && self.devices.get() != 1 {
            return Err(Error::CpuDeviceCount {
                devices: self.devices,
            });
        }
        Ok(())
    }
}

/// 自己対局の開始局面と序盤の作り方。
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum StartPosition {
    ShuffledSetup,
    Initial,
}

impl std::fmt::Display for StartPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ShuffledSetup => "shuffled-setup",
            Self::Initial => "initial",
        })
    }
}

/// 明示的に選ぶ推論の実行プロバイダ。
#[derive(Clone, Copy, ValueEnum)]
pub(super) enum Backend {
    #[value(name = "tensorrt")]
    TensorRt,
    Cuda,
    Cpu,
}

impl Backend {
    pub(super) fn provider(self, device_id: usize) -> Result<ExecutionProvider, Error> {
        Ok(match self {
            Self::TensorRt => ExecutionProvider::TensorRt {
                cache_root: cache_root(std::env::var_os("XDG_CACHE_HOME").as_deref())
                    .ok_or(Error::CacheRootUnset)?,
                device_id,
            },
            Self::Cuda => ExecutionProvider::Cuda { device_id },
            Self::Cpu => ExecutionProvider::Cpu,
        })
    }
}

fn parse_commit(text: &str) -> Result<String, std::io::Error> {
    validate_commit_hash(text)?;
    Ok(text.to_owned())
}

fn parse_resign_threshold(text: &str) -> Result<f64, ThresholdError> {
    let threshold = text.parse::<f64>().map_err(ThresholdError::Parse)?;
    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        return Err(ThresholdError::Range(threshold));
    }
    Ok(threshold)
}

/// 投了閾値の構文または範囲の誤りを保持する。
#[derive(Debug)]
enum ThresholdError {
    Parse(std::num::ParseFloatError),
    Range(f64),
}

impl std::fmt::Display for ThresholdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(error) => write!(f, "invalid resignation threshold: {error}"),
            Self::Range(value) => write!(
                f,
                "resignation threshold must be finite and within [0, 1]: {value}"
            ),
        }
    }
}
impl std::error::Error for ThresholdError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Parse(error) => Some(error),
            Self::Range(_) => None,
        }
    }
}
