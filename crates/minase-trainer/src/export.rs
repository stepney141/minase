//! 最良チェックポイントのMNUE版5への書き出しと、整数推論の量子化誤差検査。

use std::{fmt, fs, path::Path};

use minase::eval::{
    features::PIECE_STATE_COUNT,
    nnue::{
        self, Network,
        feature_set::{FeatureSet, HalfKpHmB8, P},
        head::DirectScrelu,
    },
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    data::{self, Dataset, InputFeatureSet, build_batches, epoch_permutation},
    model::{self, Checkpoint, quantize},
    reference,
};

// CPU参照計算の中間配列を検証標本の総数によらず一定サイズに保つ。
const REFERENCE_BATCH: usize = 256;

/// 書き出し、復号または量子化誤差検査の失敗。
#[derive(Debug)]
pub enum Error {
    /// 入出力が失敗した。
    Io(std::io::Error),
    /// 検証集合の読み込みまたはバッチ構築が失敗した。
    Data(data::Error),
    /// チェックポイント、量子化または参照計算が失敗した。
    Model(model::Error),
    /// 書き出したMNUEを推論側で復号できなかった。
    Mnue(nnue::Error),
    /// 最良エポックまたは検証標本の条件を満たさない。
    Invalid(&'static str),
    /// 浮動小数点の評価または誤差が非有限になった。
    NonFinite {
        /// 問題の局面の通し番号。
        record: u64,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Data(error) => write!(f, "{error}"),
            Self::Model(error) => write!(f, "{error}"),
            Self::Mnue(error) => write!(f, "exported MNUE validation failed: {error}"),
            Self::Invalid(reason) => write!(f, "invalid export input: {reason}"),
            Self::NonFinite { record } => {
                write!(f, "non-finite quantization error at record {record}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Data(error) => Some(error),
            Self::Model(error) => Some(error),
            Self::Mnue(error) => Some(error),
            Self::Invalid(_) | Self::NonFinite { .. } => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<data::Error> for Error {
    fn from(error: data::Error) -> Self {
        Self::Data(error)
    }
}

impl From<model::Error> for Error {
    fn from(error: model::Error) -> Self {
        Self::Model(error)
    }
}

/// 書き出しと量子化誤差検査の結果。
pub type Result<T> = std::result::Result<T, Error>;

/// 固定シードで選んだ検証標本の、浮動小数点評価と整数評価の差。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QuantizationReport {
    /// 実際に検査した局面数。要求数と検証集合の大きさの小さい方。
    pub samples: usize,
    /// 標本を選んだ乱数の種。
    pub seed: u64,
    /// 標本を取った集合。常に`validation`。
    pub split: &'static str,
    /// 選択順の通し番号を64ビットのリトルエンディアンで連結したSHA-256。
    pub indices_sha256: String,
    /// 平均絶対誤差、センチポーン。
    pub mean_absolute_error_cp: f64,
    /// 最大絶対誤差、センチポーン。
    pub max_absolute_error_cp: f64,
    /// 平均絶対誤差の上限、センチポーン。NNUEはK/40、PSTは2。
    pub limit_cp: f64,
    /// 平均絶対誤差が上限以下なら真。
    pub passed: bool,
}

fn validation_sample(dataset: &Dataset, requested: usize, seed: u64) -> Result<Vec<u64>> {
    if requested == 0 {
        return Err(Error::Invalid("validation sample must be positive"));
    }
    if dataset.validation_indices().is_empty() {
        return Err(Error::Invalid("validation set is empty"));
    }
    // 学習のエポックや終了理由によらず、同じ入力とseedなら同じ標本を使う。
    let mut indices = epoch_permutation(dataset.validation_indices(), seed, 0);
    indices.truncate(requested);
    Ok(indices)
}

/// 最良モデルを`nnue.bin`と`nnue-float.bin`へ書き、検証標本の量子化誤差を求める。
///
/// `checkpoint`は1以上の最良エポックそのものを渡す。浮動小数点の出力には
/// パラメータに加え、同じ時点の最適化器の状態と進捗も保存する。
/// 書き出したMNUEをファイルから読み直し、`minase`の復号と整数推論で検査する。
/// 平均絶対誤差がK/40を超えても両ファイルと測定値は保持し、`passed = false`を返す。
/// 呼び手は測定値を記録して`quantization_error`、終了コード1として扱う。
pub fn write_best(
    dataset: &Dataset,
    checkpoint: &Checkpoint,
    output_dir: &Path,
    piece_values: &[i32; PIECE_STATE_COUNT],
    rule_set: &str,
    validation_sample_size: usize,
) -> Result<QuantizationReport> {
    match checkpoint.progress.best {
        Some(best) if best.epoch > 0 && best.epoch == checkpoint.progress.epoch => {}
        _ => return Err(Error::Invalid("checkpoint is not a completed best epoch")),
    }
    let indices = validation_sample(dataset, validation_sample_size, checkpoint.progress.seed)?;
    let floating = checkpoint.encode()?;
    let quantized = quantize(&checkpoint.model)?.encode(piece_values, rule_set)?;
    fs::create_dir_all(output_dir)?;
    fs::write(output_dir.join("nnue-float.bin"), floating)?;
    let path = output_dir.join("nnue.bin");
    fs::write(&path, quantized)?;
    let written = fs::read(path)?;
    match checkpoint.model.shape().feature_set() {
        InputFeatureSet::P => measure::<P>(dataset, checkpoint, &written, &indices),
        InputFeatureSet::HalfKpHmB8 => {
            measure::<HalfKpHmB8>(dataset, checkpoint, &written, &indices)
        }
    }
}

fn measure<F: FeatureSet>(
    dataset: &Dataset,
    checkpoint: &Checkpoint,
    written: &[u8],
    indices: &[u64],
) -> Result<QuantizationReport> {
    let network = Network::<F, DirectScrelu>::decode(written).map_err(Error::Mnue)?;
    let mut sum = 0.0;
    let mut maximum = 0.0_f64;
    let mut checksum = Sha256::new();
    for batch_indices in indices.chunks(REFERENCE_BATCH) {
        let batch = build_batches(
            dataset,
            checkpoint.model.shape().feature_set(),
            &[batch_indices],
        )?
        .pop()
        .expect("one batch was requested");
        let floating = reference::evaluate_f64(&checkpoint.model, &batch)?;
        for (&index, &floating_cp) in batch_indices.iter().zip(&floating) {
            let position = dataset
                .record(index)?
                .to_position()
                .map_err(data::Error::from)?;
            let integer_cp = nnue::evaluate(&network, &position);
            let error = (floating_cp - f64::from(integer_cp)).abs();
            if !error.is_finite() {
                return Err(Error::NonFinite { record: index });
            }
            sum += error;
            maximum = maximum.max(error);
            checksum.update(index.to_le_bytes());
        }
    }
    let mean = sum / indices.len() as f64;
    let limit = f64::from(network.k()) / 40.0;
    Ok(QuantizationReport {
        samples: indices.len(),
        seed: checkpoint.progress.seed,
        split: "validation",
        indices_sha256: format!("{:x}", checksum.finalize()),
        mean_absolute_error_cp: mean,
        max_absolute_error_cp: maximum,
        limit_cp: limit,
        passed: mean <= limit,
    })
}
