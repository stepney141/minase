//! 最良チェックポイントのMNUE版5への書き出しと、整数推論の量子化誤差と駒除去差分の符号反転の検査。

use std::{fmt, fs, path::Path};

use crate::diagnostics::statistics::{Reversals, reversal_statistics};
use minase::eval::{
    features::{PIECE_STATE_COUNT, piece_state},
    nnue::{
        self, Network,
        feature_set::{FeatureSet, HalfKpHmB8, P},
        head::DirectScrelu,
    },
};
use minase_core::Square;
use serde::Serialize;

use crate::{
    data::{Dataset, epoch_permutation},
    nn::{Sha256Hasher, hex},
    nnue::{
        data::{self, BatchSlot, InputFeatureSet, build_batches},
        model::{self, Checkpoint, quantize},
        reference,
    },
};

// CPU参照計算の中間配列を検証標本の総数によらず一定サイズに保つ。
const REFERENCE_BATCH: usize = 256;

/// 書き出し、復号または検証標本の検査の失敗。
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
                write!(f, "non-finite export measurement at record {record}")
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

/// 書き出しと検証標本の検査の結果。
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
    /// 勝率尺度（1ロジットあたりのcp）を`cp_per_logit`とする。
    /// 平均絶対誤差に許す上限`cp_per_logit / 40`、センチポーン。
    pub limit_cp: f64,
    /// 平均絶対誤差が上限以下なら真。
    pub passed: bool,
}

/// 量子化誤差と同じ検証標本について、整数ネットの駒除去差分を集計する。
/// 王将・玉将の状態11と太子の状態21を除き、記録された手番側から全再計算する。
/// 閾値は設けず、学習結果のstatusにも影響しない。
#[derive(Debug, Serialize)]
pub struct RemovalSignReport {
    /// 検査した局面数。除ける駒のない局面も数える。
    pub positions: usize,
    /// 参照差分が非零の除去の件数。反転割合の母数。
    pub removals: usize,
    /// 反転の件数・割合と、反転した評価差の絶対値の平均・最大。
    pub reversals: Reversals,
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

/// 最良モデルを`nnue.bin`へ書き、同じ検証標本の量子化誤差と駒除去差分の符号反転を求める。
///
/// `checkpoint`は`best.npz`に保存した1以上の最良エポックそのものを渡す。
/// 書き出したMNUEをファイルから読み直し、`minase`の復号と整数推論で検査する。
/// 標本は検証集合を`epoch_permutation(seed, 0)`で並べた先頭`--validation-sample`件で、既定は10,000件である。
/// 平均絶対誤差が`cp_per_logit / 40`を超えても重みと測定値は保持し、`passed = false`を返す。
/// 呼び手は測定値を記録して`quantization_error`、終了コード1として扱う。
/// 符号反転は参照差分が非零の除去を母数とし、閾値を設けない。母数0や非有限値はエラーにする。
pub fn write_best(
    dataset: &Dataset,
    checkpoint: &Checkpoint,
    output_dir: &Path,
    piece_values: &[i32; PIECE_STATE_COUNT],
    rule_set: &str,
    validation_sample_size: usize,
) -> Result<(QuantizationReport, RemovalSignReport)> {
    match checkpoint.progress.best {
        Some(best) if best.epoch > 0 && best.epoch == checkpoint.progress.epoch => {}
        _ => return Err(Error::Invalid("checkpoint is not a completed best epoch")),
    }
    let indices = validation_sample(dataset, validation_sample_size, checkpoint.progress.seed)?;
    let quantized = quantize(&checkpoint.model)?.encode(piece_values, rule_set)?;
    fs::create_dir_all(output_dir)?;
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

/// 浮動小数点の`reference::evaluate_f64`と、復号したファイルの`nnue::evaluate`の平均絶対誤差と最大誤差を記録する。
/// 閾値は書き出したMNUEヘッダのf32の勝率尺度を使った`cp_per_logit / 40`であり、等しい場合は合格とする。
/// `cp_per_logit / 40`は旧NNUEから引き継いだ基準であり、棋力に対する安全性は確認されていない。
fn measure<F: FeatureSet>(
    dataset: &Dataset,
    checkpoint: &Checkpoint,
    written: &[u8],
    indices: &[u64],
) -> Result<(QuantizationReport, RemovalSignReport)> {
    let network = Network::<F, DirectScrelu>::decode(written).map_err(Error::Mnue)?;
    let mut sum = 0.0;
    let mut maximum = 0.0_f64;
    let mut checksum = Sha256Hasher::default();
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
            checksum.update(&index.to_le_bytes());
        }
    }
    let mean = sum / indices.len() as f64;
    let limit = f64::from(network.cp_per_logit()) / 40.0;
    let removal_sign = measure_removal_sign(dataset, checkpoint, &network, indices)?;
    Ok((
        QuantizationReport {
            samples: indices.len(),
            seed: checkpoint.progress.seed,
            split: "validation",
            indices_sha256: hex(&checksum.finalize()),
            mean_absolute_error_cp: mean,
            max_absolute_error_cp: maximum,
            limit_cp: limit,
            passed: mean <= limit,
        },
        removal_sign,
    ))
}

impl From<crate::data::Error> for Error {
    fn from(error: crate::data::Error) -> Self {
        Self::Data(error.into())
    }
}

/// 整数ネットを除去前後で全再計算し、診断と共通の規則で反転を数える。
fn measure_removal_sign<F: FeatureSet>(
    dataset: &Dataset,
    checkpoint: &Checkpoint,
    network: &Network<F, DirectScrelu>,
    indices: &[u64],
) -> Result<RemovalSignReport> {
    let reference = reference::RemovalReference::v0();
    let mut pairs = Vec::new();
    for &index in indices {
        let sample = dataset.record(index)?;
        let position = sample.to_position()?;
        let perspective = sample.side_to_move();
        let batch = BatchSlot::from_evaluation_positions(
            checkpoint.model.shape().feature_set(),
            std::slice::from_ref(&position),
            perspective,
        )?;
        let (deltas, magnitudes) = reference.deltas(&batch, 0)?;
        if magnitudes.iter().any(|value| !value.is_finite()) {
            return Err(Error::NonFinite { record: index });
        }
        let before = nnue::evaluate_for_diagnostics(network, &position, perspective);
        for square in Square::all() {
            let Some(piece) = position.piece_at(square) else {
                continue;
            };
            if matches!(piece_state(piece), 11 | 21) {
                continue;
            }
            let after = reference::remove_piece(&position, square)?;
            let delta = nnue::evaluate_for_diagnostics(network, &after, perspective) - before;
            pairs.push((delta, deltas[square.dense_index()]));
        }
    }
    let reversals = reversal_statistics(pairs.into_iter());
    if reversals.total == 0 {
        return Err(Error::Invalid(
            "sample contains no removals with nonzero reference differences",
        ));
    }
    Ok(RemovalSignReport {
        positions: indices.len(),
        removals: reversals.total,
        reversals,
    })
}
