//! CUDAによるNNUEの前進、後退、およびAdamW更新。
//! `kernels/forward.cu`が特徴の和、`head.cu`が出力と検証、`backward.cu`が逆索引による後退を担う。
//! `reduce.cu`は部分和の縮約、`optimizer.cu`はパラメータとモーメントの更新を担う。
//! 計算と転送に別々のストリームを使い、全起動は1ブロック128スレッドとする。
//! `atomicAdd`は使わず、`atomicOr`は異常フラグにだけ使う。
//! 幅と特徴集合は起動引数で渡し、テンプレートやソース置換で変えない。
//! GPUが使えなければエラーを返し、CPUへ切り替えない。
//! GPUを要するテストは`gpu_`で始め、`#[ignore]`を付けて明示実行する。
//! 理由は`docs/adr/write-trainer-kernels-in-cuda-cpp-via-nvrtc.md`と`docs/adr/archive/allow-unsafe-only-in-the-trainer-crate.md`にある。

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

/// 学習率の倍率は有限かつ正でなければならない。
fn validate_multiplier(multiplier: f64) -> Result<()> {
    if !multiplier.is_finite() || multiplier <= 0.0 {
        return Err(Error::Invalid(
            "learning-rate multiplier must be finite and positive",
        ));
    }
    Ok(())
}

/// フェーズ3bで実装する出力部を、デバイス確保・学習開始前に拒否する。
pub(crate) fn check_head(head: crate::nnue::model::HeadKind) -> Result<()> {
    if head != crate::nnue::model::HeadKind::DirectScrelu {
        return Err(Error::Invalid(
            "CUDA backend does not implement this head yet",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_heads_are_rejected_before_cuda_is_loaded() {
        use crate::nnue::{model::HeadKind, options::TrainingOptions};
        for head in [
            HeadKind::DirectScreluPhase2,
            HeadKind::ScreluHidden16,
            HeadKind::ScreluHidden16Phase2,
        ] {
            assert!(matches!(
                equivalence::run(
                    crate::nnue::data::InputFeatureSet::P,
                    4,
                    1,
                    1,
                    TrainingOptions::new(1.0),
                    head
                ),
                Err(Error::Invalid(
                    "CUDA backend does not implement this head yet"
                ))
            ));
        }
    }

    #[test]
    fn multiplier_requires_a_positive_finite_value() {
        for multiplier in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(
                matches!(
                    validate_multiplier(multiplier),
                    Err(Error::Invalid(
                        "learning-rate multiplier must be finite and positive"
                    ))
                ),
                "{multiplier}"
            );
        }
        for multiplier in [f64::MIN_POSITIVE, 0.5, 1.0, f64::MAX] {
            validate_multiplier(multiplier).unwrap();
        }
    }
}
