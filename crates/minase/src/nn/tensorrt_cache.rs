//! GPUの照会と、TensorRTのエンジンを設定ごとに分けるキャッシュの鍵。

use std::fmt;
use std::io;
use std::num::{NonZeroUsize, ParseIntError};
use std::process::{Command, ExitStatus};
use std::string::FromUtf8Error;

use sha2::{Digest, Sha256};

/// `nvidia-smi`で取得した1枚のGPUの情報。
#[derive(Debug, PartialEq, Eq)]
pub struct GpuInfo {
    /// GPUの機種名。
    pub name: String,
    /// GPUのメモリ容量。単位はMiB。
    pub memory_mib: u64,
}

/// GPUの照会または出力の解析に失敗した理由。
#[derive(Debug)]
pub enum GpuQueryError {
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
    /// GPUがちょうど1枚ではなかった。
    DeviceCount {
        /// 空でない行の数。
        count: usize,
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
            Self::Spawn(source) => write!(formatter, "cannot start nvidia-smi: {source}"),
            Self::Exit { status, stderr } => {
                write!(formatter, "nvidia-smi failed ({status}): {}", stderr.trim())
            }
            Self::InvalidUtf8(source) => {
                write!(formatter, "nvidia-smi output is not UTF-8: {source}")
            }
            Self::DeviceCount { count } => {
                write!(formatter, "expected exactly one GPU, got {count}")
            }
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

/// `nvidia-smi`の機種名とメモリ容量のCSVを解析する。
///
/// 空でない行を1行だけ受理し、最後のコンマで分けて前後の空白を除く。
/// 機種名の中のコンマは機種名の一部として保持する。
///
/// # Errors
/// GPUの枚数、行の形式、機種名、または容量が不正なら[`GpuQueryError`]を返す。
pub fn parse_gpu_query(stdout: &str) -> Result<GpuInfo, GpuQueryError> {
    let lines: Vec<_> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.len() != 1 {
        return Err(GpuQueryError::DeviceCount { count: lines.len() });
    }
    let (name, memory) = lines[0]
        .rsplit_once(',')
        .ok_or_else(|| GpuQueryError::MissingComma {
            line: lines[0].to_owned(),
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

/// `nvidia-smi`でGPUの機種名とメモリ容量を照会する。
///
/// # Errors
/// 起動失敗、異常終了、または出力の解析失敗を[`GpuQueryError`]で区別する。
/// 複数のGPUがある場合に使用するGPUを推測せず、エラーにする。
pub fn query_gpu() -> Result<GpuInfo, GpuQueryError> {
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
/// ONNX Runtime 1.28の`TRTGenerateId`とエンジン名の`_sm<計算能力>`は、
/// TensorRT・ONNX Runtime・CUDAの版と計算能力を区別するため、ここには含めない。
/// メモリから読んだネットではそのハッシュに重みが入らないため、ネットのSHA-256が必要になる。
/// GPUの機種とメモリ容量も含め、大容量のGPU向けのエンジンを別のGPUで共有しない。
/// 最適化プロファイルが異なるとONNX Runtimeは同じ名前のエンジンを作り直すため、
/// 最大バッチも含め、異なる設定による同じディレクトリへの交互の上書きを避ける。
/// 根拠は`onnxruntime/core/providers/tensorrt/tensorrt_execution_provider_utils.h`の523〜596行と
/// `tensorrt_execution_provider.cc`の3512〜3525行および3555行（いずれも1.28）にある。
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

    // [`parse_gpu_query`]の契約: 1枚だけを受理し、空白を除き、機種名中のコンマを保持する。
    #[test]
    fn parses_one_gpu_and_rejects_invalid_rows() {
        assert_eq!(
            parse_gpu_query("\n  NVIDIA GeForce RTX 3060 Ti, 8192 \n \n").unwrap(),
            GpuInfo {
                name: "NVIDIA GeForce RTX 3060 Ti".to_owned(),
                memory_mib: 8192,
            }
        );
        assert_eq!(
            parse_gpu_query("GPU, Special, 1024").unwrap().name,
            "GPU, Special"
        );
        assert!(matches!(
            parse_gpu_query(" \n"),
            Err(GpuQueryError::DeviceCount { count: 0 })
        ));
        assert!(matches!(
            parse_gpu_query("A, 1024\nB, 2048\n"),
            Err(GpuQueryError::DeviceCount { count: 2 })
        ));
        assert!(matches!(
            parse_gpu_query("A 1024"),
            Err(GpuQueryError::MissingComma { .. })
        ));
        assert!(matches!(
            parse_gpu_query(", 1024"),
            Err(GpuQueryError::EmptyName)
        ));
        for memory in ["unknown", "-1", "8192 MiB", "18446744073709551616"] {
            assert!(matches!(
                parse_gpu_query(&format!("A, {memory}")),
                Err(GpuQueryError::InvalidMemory { .. })
            ));
        }
    }
}
