//! CUDAのデバイス管理とバッチ転送。

pub(in crate::nnue) mod device;
pub(in crate::nnue) mod ring;
pub use device::Device;
pub use ring::{BatchTicket, HostBuffer, HostRing, TransferTiming};

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
/// GPU操作の結果。
pub type Result<T> = std::result::Result<T, Error>;

fn require_len(actual: usize, expected: usize) -> Result<()> {
    if actual < expected {
        Err(Error::Invalid("array shorter than kernel extent"))
    } else {
        Ok(())
    }
}

impl From<crate::data::Error> for Error {
    fn from(error: crate::data::Error) -> Self {
        Self::Data(error.into())
    }
}
