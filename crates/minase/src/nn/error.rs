use core::fmt;
use std::collections::TryReserveError;
use std::io;
use std::num::ParseIntError;
use std::path::PathBuf;

use super::tensorrt_cache::GpuQueryError;
use super::{INPUT_PLANES, POLICY_SIZE};

/// ネットの読み込み、推論の準備、入力の符号化、または評価に失敗した理由。
#[derive(Debug)]
pub enum NnError {
    /// 159面の入力では表せない成り権の保留がある。
    PromotionDeferred,
    /// `ORT_DYLIB_PATH`が未設定または空である。
    RuntimeLibraryPathUnset,
    /// ONNX Runtimeのライブラリのパスを解決または検査できなかった。
    RuntimeLibraryPath {
        /// 指定されたパス。
        path: PathBuf,
        /// 元の入出力エラー。
        source: io::Error,
    },
    /// ONNX Runtimeのライブラリのパスが通常のファイルではない。
    RuntimeLibraryNotFile {
        /// 指定されたパス。
        path: PathBuf,
    },
    /// ONNX Runtimeの共有ライブラリを読み込めなかった。
    RuntimeInitialization(ort::LoadDynamicError),
    /// ONNX Runtimeの初期化中のパニックにより排他ロックが汚染された。
    RuntimeInitializationPoisoned,
    /// ネットのファイルを読み込めなかった。
    ReadNetwork {
        /// 読み込もうとしたパス。
        path: PathBuf,
        /// 元の入出力エラー。
        source: io::Error,
    },
    /// TensorRTのキャッシュ用ディレクトリを作成できなかった。
    CreateCacheDir {
        /// 作成しようとしたパス。
        path: PathBuf,
        /// 元の入出力エラー。
        source: io::Error,
    },
    /// TensorRTへ渡すキャッシュのパスをUTF-8で表せない。
    CachePathNotUtf8 {
        /// 指定されたパス。
        path: PathBuf,
    },
    /// GPUの環境変数の検査または情報の取得に失敗した。
    GpuQuery(GpuQueryError),
    /// CUDAの機器番号をONNX Runtimeが要求する整数型で表せない。
    InvalidDeviceId {
        /// 指定されたCUDAの機器番号。
        device_id: usize,
    },
    /// ONNX Runtimeが返したエラー。
    Runtime(ort::Error),
    /// ネットの入出力の名前、要素型、個数、または形が契約と異なる。
    ModelSignature {
        /// 期待する入出力の名前、要素型、および形。`-1`は動的なバッチ次元。
        expected: &'static str,
        /// 実際の入力の名前と型。テンソルの型は形も含む。
        inputs: Vec<(String, ort::value::ValueType)>,
        /// 実際の出力の名前と型。テンソルの型は形も含む。
        outputs: Vec<(String, ort::value::ValueType)>,
    },
    /// ネットのメタデータに`policy_labels`がない。
    MissingPolicyLabels,
    /// `policy_labels`にラベルが1つもない。
    EmptyPolicyLabels,
    /// `policy_labels`の要素を10進の非負整数として読めない。
    InvalidPolicyLabel {
        /// 表の中の0から始まる添字。
        index: usize,
        /// 読めなかった要素。
        value: String,
        /// 元の整数解析エラー。
        source: ParseIntError,
    },
    /// `policy_labels`が厳密な昇順ではない。
    PolicyLabelsNotAscending {
        /// 順序に反した要素の0から始まる添字。
        index: usize,
        /// 直前のラベル。
        previous: usize,
        /// 順序に反したラベル。
        label: usize,
    },
    /// `policy_labels`のラベルが方策全体の範囲を超えている。
    PolicyLabelOutOfRange {
        /// 表の中の0から始まる添字。
        index: usize,
        /// 範囲外のラベル。
        label: usize,
    },
    /// `policy_labels`の要素数が方策の出力幅と一致しない。
    PolicyLabelCount {
        /// 署名が指定する方策の出力幅。
        expected: usize,
        /// 表に含まれるラベルの数。
        actual: usize,
    },
    /// 最大バッチから入力バッファの要素数を表現できない。
    InvalidMaxBatch {
        /// 指定された最大バッチ。
        max: usize,
    },
    /// 入力バッファを確保できなかった。
    AllocateInput {
        /// 指定された最大バッチ。
        max: usize,
        /// 元の確保エラー。
        source: TryReserveError,
    },
    /// 依頼数が構築時に指定した最大バッチを超えている。
    BatchTooLarge {
        /// 依頼数。
        len: usize,
        /// 最大バッチ。
        max: usize,
    },
    /// 推論結果に必要な名前の出力がない。
    MissingOutput {
        /// 必要な出力の名前。
        name: &'static str,
    },
    /// 推論結果のテンソルの形が依頼したバッチと一致しない。
    OutputShape {
        /// 出力の名前。
        name: &'static str,
        /// 期待する形。
        expected: [usize; 2],
        /// 実際の形。
        actual: Vec<i64>,
    },
    /// 依頼された合法手のラベルがネットの`policy_labels`にない。
    MissingPolicyLabel {
        /// バッチ内の依頼の添字。
        query: usize,
        /// 詰める前の方策ラベル。
        label: usize,
    },
    /// 依頼した合法手のlogitまたは価値に非有限値がある。
    NonFiniteOutput {
        /// バッチ内の依頼の添字。
        query: usize,
    },
    /// 価値が勝率の範囲0〜1を外れている。
    ValueOutOfRange {
        /// バッチ内の依頼の添字。
        query: usize,
        /// 返された価値。
        value: f32,
    },
}

impl fmt::Display for NnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PromotionDeferred => {
                write!(
                    formatter,
                    "deferred promotion rights cannot be encoded in {INPUT_PLANES} planes"
                )
            }
            Self::RuntimeLibraryPathUnset => {
                formatter.write_str("ORT_DYLIB_PATH is unset or empty")
            }
            Self::RuntimeLibraryPath { path, source } => write!(
                formatter,
                "cannot resolve runtime library {}: {source}",
                path.display()
            ),
            Self::RuntimeLibraryNotFile { path } => {
                write!(
                    formatter,
                    "runtime library is not a file: {}",
                    path.display()
                )
            }
            Self::RuntimeInitialization(source) => {
                write!(formatter, "cannot initialize ONNX Runtime: {source}")
            }
            Self::RuntimeInitializationPoisoned => {
                formatter.write_str("ONNX Runtime initialization lock is poisoned")
            }
            Self::ReadNetwork { path, source } => {
                write!(
                    formatter,
                    "cannot read network {}: {source}",
                    path.display()
                )
            }
            Self::CreateCacheDir { path, source } => write!(
                formatter,
                "cannot create cache directory {}: {source}",
                path.display()
            ),
            Self::CachePathNotUtf8 { path } => {
                write!(formatter, "cache path is not UTF-8: {}", path.display())
            }
            Self::GpuQuery(source) => write!(formatter, "cannot query GPU: {source}"),
            Self::InvalidDeviceId { device_id } => {
                write!(
                    formatter,
                    "CUDA device ID exceeds the i32 range: {device_id}"
                )
            }
            Self::Runtime(source) => write!(formatter, "ONNX Runtime error: {source}"),
            Self::ModelSignature {
                expected,
                inputs,
                outputs,
            } => write!(
                formatter,
                "invalid model signature: expected {expected}; got inputs {inputs:?}, outputs {outputs:?}"
            ),
            Self::MissingPolicyLabels => formatter.write_str("missing policy_labels metadata"),
            Self::EmptyPolicyLabels => formatter.write_str("policy_labels must not be empty"),
            Self::InvalidPolicyLabel {
                index,
                value,
                source,
            } => write!(
                formatter,
                "invalid policy_labels entry {index} ({value:?}): {source}"
            ),
            Self::PolicyLabelsNotAscending {
                index,
                previous,
                label,
            } => write!(
                formatter,
                "policy_labels entry {index} ({label}) is not greater than {previous}"
            ),
            Self::PolicyLabelOutOfRange { index, label } => write!(
                formatter,
                "policy_labels entry {index} ({label}) is outside 0..{POLICY_SIZE}"
            ),
            Self::PolicyLabelCount { expected, actual } => write!(
                formatter,
                "policy_labels count {actual} differs from policy output width {expected}"
            ),
            Self::InvalidMaxBatch { max } => write!(formatter, "maximum batch is too large: {max}"),
            Self::AllocateInput { max, source } => {
                write!(formatter, "cannot allocate input for batch {max}: {source}")
            }
            Self::BatchTooLarge { len, max } => {
                write!(formatter, "batch size {len} exceeds maximum {max}")
            }
            Self::MissingOutput { name } => write!(formatter, "missing output: {name}"),
            Self::OutputShape {
                name,
                expected,
                actual,
            } => write!(
                formatter,
                "invalid {name} shape: expected {expected:?}, got {actual:?}"
            ),
            Self::MissingPolicyLabel { query, label } => write!(
                formatter,
                "policy label {label} for query {query} is missing from policy_labels"
            ),
            Self::NonFiniteOutput { query } => {
                write!(formatter, "non-finite output for query {query}")
            }
            Self::ValueOutOfRange { query, value } => {
                write!(
                    formatter,
                    "value {value} is outside [0, 1] for query {query}"
                )
            }
        }
    }
}

impl std::error::Error for NnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RuntimeLibraryPath { source, .. }
            | Self::ReadNetwork { source, .. }
            | Self::CreateCacheDir { source, .. } => Some(source),
            Self::RuntimeInitialization(source) => Some(source),
            Self::GpuQuery(source) => Some(source),
            Self::Runtime(source) => Some(source),
            Self::InvalidPolicyLabel { source, .. } => Some(source),
            Self::AllocateInput { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<ort::Error> for NnError {
    fn from(source: ort::Error) -> Self {
        Self::Runtime(source)
    }
}

impl From<GpuQueryError> for NnError {
    fn from(source: GpuQueryError) -> Self {
        Self::GpuQuery(source)
    }
}
