//! GPUの照会と、TensorRTのエンジンを設定ごとに分けるキャッシュの鍵。

use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::num::{NonZeroUsize, ParseIntError};
use std::path::PathBuf;
use std::process::{Command, ExitStatus};
use std::string::FromUtf8Error;

use sha2::{Digest, Sha256};

/// 空でない`XDG_CACHE_HOME`の下に置く、TensorRTのキャッシュの親を返す。
/// 未設定または空の場合は、別のディレクトリを補わず`None`を返す。
pub fn cache_root(xdg_cache_home: Option<&OsStr>) -> Option<PathBuf> {
    xdg_cache_home
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(path).join("minase/tensorrt"))
}

/// `nvidia-smi`で取得した1枚のGPUの情報。
#[derive(Debug, PartialEq, Eq)]
pub struct GpuInfo {
    /// GPUの機種名。
    pub name: String,
    /// GPUのメモリ容量。単位はMiB。
    pub memory_mib: u64,
}

/// 同じ機種名とメモリ容量を持つGPUの枚数と共通の情報。
#[derive(Debug, PartialEq, Eq)]
pub struct GpuInventory {
    /// GPUの枚数。
    pub count: NonZeroUsize,
    /// 全GPUに共通する機種名とメモリ容量。
    pub gpu: GpuInfo,
}

/// GPUの環境変数の検査、照会、または出力の解析に失敗した理由。
#[derive(Debug)]
pub enum GpuQueryError {
    /// 空値を含めて`CUDA_VISIBLE_DEVICES`が設定されていた。
    CudaVisibleDevicesSet,
    /// `nvidia-smi`を起動できなかった。
    Spawn(io::Error),
    /// `nvidia-smi`が正常終了しなかった。
    Exit {
        /// 終了状態。シグナルによる終了も含む。
        status: ExitStatus,
        /// 標準エラーの内容。
        stderr: String,
    },
    /// 標準出力がUTF-8ではなかった。
    InvalidUtf8(FromUtf8Error),
    /// GPUの情報を含む行がなかった。
    NoDevices,
    /// 機種名またはメモリ容量が異なるGPUが混在していた。
    MixedDevices {
        /// 最初の行のGPU。
        first: GpuInfo,
        /// 最初の行と異なるGPU。
        other: GpuInfo,
    },
    /// 機種名とメモリ容量を分けるコンマがなかった。
    MissingComma {
        /// 解析できなかった行。
        line: String,
    },
    /// GPUの機種名が空だった。
    EmptyName,
    /// メモリ容量が符号なし整数ではなかった。
    InvalidMemory {
        /// メモリ容量の欄。
        value: String,
        /// 元の整数解析エラー。
        source: ParseIntError,
    },
}

impl fmt::Display for GpuQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CudaVisibleDevicesSet => {
                formatter.write_str("CUDA_VISIBLE_DEVICES must be unset when using a GPU")
            }
            Self::Spawn(source) => write!(formatter, "cannot start nvidia-smi: {source}"),
            Self::Exit { status, stderr } => {
                write!(formatter, "nvidia-smi failed ({status}): {}", stderr.trim())
            }
            Self::InvalidUtf8(source) => {
                write!(formatter, "nvidia-smi output is not UTF-8: {source}")
            }
            Self::NoDevices => formatter.write_str("nvidia-smi reported no GPUs"),
            Self::MixedDevices { first, other } => write!(
                formatter,
                "GPUs must have the same model and memory: first GPU is {} with {} MiB, another is {} with {} MiB",
                first.name, first.memory_mib, other.name, other.memory_mib
            ),
            Self::MissingComma { line } => write!(formatter, "missing comma in GPU row: {line}"),
            Self::EmptyName => formatter.write_str("GPU name is empty"),
            Self::InvalidMemory { value, source } => {
                write!(formatter, "invalid GPU memory in MiB: {value}: {source}")
            }
        }
    }
}

impl std::error::Error for GpuQueryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn(source) => Some(source),
            Self::InvalidUtf8(source) => Some(source),
            Self::InvalidMemory { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// `CUDA_VISIBLE_DEVICES`が未設定であることを検査する。
///
/// `nvidia-smi`とCUDAが参照するGPUの集合が食い違わないよう、空値も拒む。
///
/// # Errors
/// 値が設定されていれば[`GpuQueryError::CudaVisibleDevicesSet`]を返す。
pub fn validate_cuda_visible_devices(value: Option<&OsStr>) -> Result<(), GpuQueryError> {
    if value.is_some() {
        return Err(GpuQueryError::CudaVisibleDevicesSet);
    }
    Ok(())
}

/// `nvidia-smi`の機種名とメモリ容量のCSVを解析する。
///
/// 空でない行をすべて解析し、機種名とメモリ容量が同じGPUの枚数と共通の情報を返す。
/// 各行を最後のコンマで分けて前後の空白を除く。
/// 機種名の中のコンマは機種名の一部として保持する。
///
/// # Errors
/// GPUがないか、機種名と容量がそろわないか、行の形式が不正なら[`GpuQueryError`]を返す。
pub fn parse_gpu_query(stdout: &str) -> Result<GpuInventory, GpuQueryError> {
    let lines: Vec<_> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let count = NonZeroUsize::new(lines.len()).ok_or(GpuQueryError::NoDevices)?;
    let mut lines = lines.into_iter();
    let gpu = parse_gpu_row(lines.next().ok_or(GpuQueryError::NoDevices)?)?;
    for line in lines {
        let other = parse_gpu_row(line)?;
        if other != gpu {
            return Err(GpuQueryError::MixedDevices { first: gpu, other });
        }
    }
    Ok(GpuInventory { count, gpu })
}

fn parse_gpu_row(line: &str) -> Result<GpuInfo, GpuQueryError> {
    let (name, memory) = line
        .rsplit_once(',')
        .ok_or_else(|| GpuQueryError::MissingComma {
            line: line.to_owned(),
        })?;
    let name = name.trim();
    if name.is_empty() {
        return Err(GpuQueryError::EmptyName);
    }
    let memory = memory.trim();
    let memory_mib = memory
        .parse()
        .map_err(|source| GpuQueryError::InvalidMemory {
            value: memory.to_owned(),
            source,
        })?;
    Ok(GpuInfo {
        name: name.to_owned(),
        memory_mib,
    })
}

/// `nvidia-smi`でGPUの枚数と、共通する機種名とメモリ容量を照会する。
///
/// # Errors
/// 起動失敗、異常終了、または出力の解析失敗を[`GpuQueryError`]で区別する。
/// 機種名またはメモリ容量が異なるGPUの混在はエラーにする。
pub fn query_gpus() -> Result<GpuInventory, GpuQueryError> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .map_err(GpuQueryError::Spawn)?;
    if !output.status.success() {
        return Err(GpuQueryError::Exit {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    let stdout = String::from_utf8(output.stdout).map_err(GpuQueryError::InvalidUtf8)?;
    parse_gpu_query(&stdout)
}

/// ネット、GPUの機種と容量、および最大バッチを区別するTensorRTのキャッシュの鍵。
///
/// ONNX Runtime 1.28と1.31の`TRTGenerateId`とエンジン名の`_sm<計算能力>`は、
/// TensorRT・ONNX Runtime・CUDAの版と計算能力を区別するため、ここには含めない。
/// メモリから読んだネットではそのハッシュに重みが入らないため、ネットのSHA-256が必要になる。
/// GPUの機種とメモリ容量も含め、大容量のGPU向けのエンジンを別のGPUで共有しない。
/// 最適化プロファイルが異なるとONNX Runtimeは同じ名前のエンジンを作り直すため、
/// 最大バッチも含め、異なる設定による同じディレクトリへの交互の上書きを避ける。
/// 根拠は`onnxruntime/core/providers/tensorrt/tensorrt_execution_provider_utils.h`の523〜596行と
/// `tensorrt_execution_provider.cc`の3512〜3525行および3555行（1.28.3と1.31.1で同じ）にある。
#[derive(Clone, Debug)]
pub struct TensorRtCacheKey {
    network_sha256: [u8; 32],
    gpu_name: String,
    gpu_memory_mib: u64,
    max_batch: NonZeroUsize,
}

impl TensorRtCacheKey {
    /// ネットの検査和、照会したGPU、および最大バッチから鍵を作る。
    pub fn new(network_sha256: [u8; 32], gpu: GpuInfo, max_batch: NonZeroUsize) -> Self {
        Self {
            network_sha256,
            gpu_name: gpu.name,
            gpu_memory_mib: gpu.memory_mib,
            max_batch,
        }
    }

    /// 鍵のSHA-256を小文字16進64文字のディレクトリ名として返す。
    ///
    /// 検査和32バイト、機種名のUTF-8のバイト数、機種名、メモリ容量、最大バッチの順に並べる。
    /// 整数はすべてリトルエンディアンの8バイトとし、文字列の境界を曖昧にしない。
    pub fn directory_name(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(self.network_sha256);
        digest.update((self.gpu_name.len() as u64).to_le_bytes());
        digest.update(self.gpu_name.as_bytes());
        digest.update(self.gpu_memory_mib.to_le_bytes());
        digest.update((self.max_batch.get() as u64).to_le_bytes());
        format!("{:x}", digest.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // [`TensorRtCacheKey`]の契約: 4項目のどれが異なっても別の64桁の小文字16進名になる。
    #[test]
    fn cache_key_distinguishes_each_component_and_is_stable() {
        let key = TensorRtCacheKey::new(
            [7; 32],
            GpuInfo {
                name: "NVIDIA GeForce RTX 3060 Ti".to_owned(),
                memory_mib: 8192,
            },
            NonZeroUsize::new(7).unwrap(),
        );
        let name = key.directory_name();
        assert_eq!(name, key.clone().directory_name());
        assert_eq!(name.len(), 64);
        assert!(
            name.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        );
        let mut variants = [key.clone(), key.clone(), key.clone(), key.clone()];
        variants[0].network_sha256[31] ^= 1;
        variants[1].gpu_name.push('X');
        variants[2].gpu_memory_mib += 1;
        variants[3].max_batch = NonZeroUsize::new(8).unwrap();
        for variant in variants {
            assert_ne!(name, variant.directory_name());
        }
    }

    // [`TensorRtCacheKey`]の契約: 機種名と数値の文字列表現の境界をずらしても同じ鍵にならない。
    #[test]
    fn cache_key_keeps_string_boundaries() {
        let name = |gpu_name: &str, memory_mib| {
            TensorRtCacheKey::new(
                [0; 32],
                GpuInfo {
                    name: gpu_name.to_owned(),
                    memory_mib,
                },
                NonZeroUsize::new(3).unwrap(),
            )
            .directory_name()
        };
        assert_ne!(name("A", 12), name("A1", 2));
        assert_ne!(name("A", 0xbc), name("AB", 0xc));
    }

    // [`parse_gpu_query`]の契約: 空白を除き、機種名中のコンマを保持する。
    #[test]
    fn parses_one_gpu_and_preserves_commas_in_model_names() {
        assert_eq!(
            parse_gpu_query("\n  NVIDIA GeForce RTX 3060 Ti, 8192 \n \n").unwrap(),
            GpuInventory {
                count: NonZeroUsize::new(1).unwrap(),
                gpu: GpuInfo {
                    name: "NVIDIA GeForce RTX 3060 Ti".to_owned(),
                    memory_mib: 8192,
                },
            }
        );
        assert_eq!(
            parse_gpu_query("GPU, Special, 1024").unwrap().gpu.name,
            "GPU, Special"
        );
    }

    // 複数GPU化の設計: 同じ機種名と容量の4枚を受理し、空白行は枚数に含めない。
    #[test]
    fn parses_four_matching_gpus() {
        let inventory = parse_gpu_query(
            "GPU, Special, 24576\n  GPU, Special , 24576 \n\nGPU, Special,24576\n\t\nGPU, Special, 24576\n",
        )
        .unwrap();
        assert_eq!(inventory.count.get(), 4);
        assert_eq!(
            inventory.gpu,
            GpuInfo {
                name: "GPU, Special".to_owned(),
                memory_mib: 24576,
            }
        );
    }

    // 複数GPU化の設計: 機種名または容量の混在では、最初のGPUと異なるGPUを保持する。
    #[test]
    fn rejects_mixed_models_or_memory_sizes() {
        for (name, memory_mib) in [("B", 1024), ("A", 2048)] {
            let stdout = format!("A, 1024\nA, 1024\nA, 1024\n{name}, {memory_mib}\n");
            let Err(GpuQueryError::MixedDevices { first, other }) = parse_gpu_query(&stdout) else {
                panic!("different GPU models or memory sizes must be rejected");
            };
            assert_eq!(
                first,
                GpuInfo {
                    name: "A".to_owned(),
                    memory_mib: 1024,
                }
            );
            assert_eq!(
                other,
                GpuInfo {
                    name: name.to_owned(),
                    memory_mib,
                }
            );
        }
    }

    // [`parse_gpu_query`]の契約: 0枚を拒み、どの行でも同じ形式の検査を行う。
    #[test]
    fn rejects_empty_queries_and_invalid_rows() {
        for stdout in ["", " \n\t\n"] {
            assert!(matches!(
                parse_gpu_query(stdout),
                Err(GpuQueryError::NoDevices)
            ));
        }
        for prefix in ["", "A, 1024\nA, 1024\nA, 1024\n"] {
            assert!(matches!(
                parse_gpu_query(&format!("{prefix}A 1024")),
                Err(GpuQueryError::MissingComma { line }) if line == "A 1024"
            ));
            assert!(matches!(
                parse_gpu_query(&format!("{prefix}  , 1024")),
                Err(GpuQueryError::EmptyName)
            ));
            for memory in ["", "unknown", "-1", "8192 MiB", "18446744073709551616"] {
                assert!(matches!(
                    parse_gpu_query(&format!("{prefix}A, {memory}")),
                    Err(GpuQueryError::InvalidMemory { value, .. }) if value == memory
                ));
            }
        }
    }

    // 複数GPU化の設計: GPUの枚数を変えても共通の機種名と容量から作るキャッシュの鍵は変わらない。
    #[test]
    fn cache_key_is_independent_of_gpu_count() {
        let single = parse_gpu_query("A, 1024").unwrap();
        let multiple = parse_gpu_query("A, 1024\nA, 1024\nA, 1024\nA, 1024").unwrap();
        let batch = NonZeroUsize::new(3).unwrap();
        assert_eq!(
            TensorRtCacheKey::new([7; 32], single.gpu, batch).directory_name(),
            TensorRtCacheKey::new([7; 32], multiple.gpu, batch).directory_name()
        );
    }

    // 複数GPU化の設計: CUDA_VISIBLE_DEVICESは未設定だけを受理し、空値を含む設定を拒む。
    #[test]
    fn cuda_visible_devices_must_be_unset() {
        validate_cuda_visible_devices(None).unwrap();
        for value in ["", "0", "0,1"] {
            assert!(matches!(
                validate_cuda_visible_devices(Some(OsStr::new(value))),
                Err(GpuQueryError::CudaVisibleDevicesSet)
            ));
        }
    }
}
