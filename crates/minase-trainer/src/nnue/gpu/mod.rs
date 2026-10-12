//! CUDAの文脈、固定メモリの環、NVRTC、および前進・補助損失・後退の計算。
//!
//! GPUが使えない場合は明示的なエラーを返す。CPU参照への代替経路は持たない。

pub mod bench;
mod device;
mod engine;
pub mod equivalence;
mod optimizer;
mod ring;
mod step;
mod step_equivalence;

pub use device::Device;
pub use engine::{Evaluation, Evaluator};
pub use ring::{BatchTicket, HostBuffer, HostRing, TransferTiming};
pub use step::{Step, StepMetrics, StepResult};

use cudarc::{driver::DriverError, nvrtc::CompileError};
use std::fmt;

/// GPU基盤と等価性検査の失敗。
#[derive(Debug)]
pub enum Error {
    /// CUDAドライバの共有ライブラリが見つからない。
    DriverUnavailable,
    /// CUDAの初期化またはデバイスの取得に失敗した。
    DeviceUnavailable(DriverError),
    /// NVRTCの共有ライブラリが見つからない。
    CompilerUnavailable,
    /// CUDAの転送、確保、イベントまたは起動の失敗。
    Driver(DriverError),
    /// NVRTCのコンパイルの失敗。診断ログを保持する。
    Compile(CompileError),
    /// 配列形状、容量、または面の状態が契約に違反した。
    Invalid(&'static str),
    /// 損失、勾配または更新後の状態が有限値でなかった。
    NonFinite(&'static str),
    /// データの構築に失敗した。
    Data(crate::nnue::data::Error),
    /// モデルまたは参照計算の入力が不正だった。
    Model(crate::nnue::model::Error),
    /// GPUとCPU参照または反復実行が一致しなかった。
    Equivalence(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DriverUnavailable => f.write_str("CUDA driver library is unavailable"),
            Self::DeviceUnavailable(e) => write!(f, "CUDA device is unavailable: {e}"),
            Self::CompilerUnavailable => f.write_str("NVRTC library is unavailable"),
            Self::Driver(e) => write!(f, "CUDA: {e}"),
            Self::Compile(e) => write!(f, "NVRTC compilation failed: {e}"),
            Self::Invalid(s) => write!(f, "invalid GPU input or ring state: {s}"),
            Self::NonFinite(s) => write!(f, "non-finite {s}"),
            Self::Data(e) => e.fmt(f),
            Self::Model(e) => e.fmt(f),
            Self::Equivalence(s) => write!(f, "equivalence check failed: {s}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::DeviceUnavailable(e) | Self::Driver(e) => Some(e),
            Self::Compile(e) => Some(e),
            Self::Data(e) => Some(e),
            Self::Model(e) => Some(e),
            _ => None,
        }
    }
}
impl From<DriverError> for Error {
    fn from(e: DriverError) -> Self {
        Self::Driver(e)
    }
}
impl From<CompileError> for Error {
    fn from(e: CompileError) -> Self {
        Self::Compile(e)
    }
}
impl From<crate::nnue::data::Error> for Error {
    fn from(e: crate::nnue::data::Error) -> Self {
        Self::Data(e)
    }
}
impl From<crate::nnue::model::Error> for Error {
    fn from(e: crate::nnue::model::Error) -> Self {
        Self::Model(e)
    }
}
/// GPU操作の結果。
pub type Result<T> = std::result::Result<T, Error>;

const THREADS: usize = 128;
const WARPS_PER_BLOCK: usize = THREADS / 32;

fn require_len(actual: usize, expected: usize) -> Result<()> {
    if actual < expected {
        Err(Error::Invalid("array shorter than kernel extent"))
    } else {
        Ok(())
    }
}

fn launch_config(threads: usize) -> Result<cudarc::driver::LaunchConfig> {
    let blocks = threads.div_ceil(THREADS);
    if blocks == 0 || blocks > i32::MAX as usize {
        return Err(Error::Invalid("launch grid size"));
    }
    Ok(cudarc::driver::LaunchConfig {
        grid_dim: (blocks as u32, 1, 1),
        block_dim: (THREADS as u32, 1, 1),
        shared_mem_bytes: 0,
    })
}

impl From<crate::data::Error> for Error {
    fn from(error: crate::data::Error) -> Self {
        Self::Data(error.into())
    }
}
