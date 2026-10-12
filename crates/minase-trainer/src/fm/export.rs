//! 最良FMのMNPT版3と実数の保存、および補正縮小前後の量子化検査。

use super::{
    Model, Quantized, Result, ZeroCorrectionReport,
    npz::encode_model,
    quantize,
    record::hash_u64,
    report::{correction, phi, std},
    zero_correction_report,
};
use crate::{
    data::{Dataset, epoch_permutation},
    nn::atomic_write,
};
use minase::eval::{
    features::{FEATURE_COUNT, active_features},
    pst::{self, FM_RANK, Pst},
};
use serde::Serialize;
use std::path::Path;

/// 同じ検証標本における、元の補正と1/4補正の整数化誤差。
#[derive(Debug, Serialize)]
pub struct QuantizationReport {
    /// FMの整数指数。
    pub exponent: u32,
    /// 標本数。
    pub samples: usize,
    /// SplitMix64のシード。
    pub seed: u64,
    /// 常にvalidation。
    pub split: &'static str,
    /// 抽出順の通し番号のSHA-256。
    pub indices_sha256: String,
    /// 元の補正の平均絶対誤差、センチポーン。
    pub mean_absolute_error_cp: f64,
    /// 元の補正の最大絶対誤差、センチポーン。
    pub max_absolute_error_cp: f64,
    /// 1/4補正の平均絶対誤差、センチポーン。
    pub quarter_mean_absolute_error_cp: f64,
    /// 1/4補正の最大絶対誤差、センチポーン。
    pub quarter_max_absolute_error_cp: f64,
    /// 両方の平均絶対誤差の上限、センチポーン。
    pub limit_cp: f64,
    /// 両方の平均絶対誤差が2cp以下。
    pub passed: bool,
}
/// 整数FM補正の母標準偏差、センチポーン。
#[derive(Debug, Serialize)]
pub struct CorrectionDistribution {
    /// 検証標本数。
    pub samples: usize,
    /// 標本抽出のシード。
    pub seed: u64,
    /// 常にvalidation。
    pub split: &'static str,
    /// 補正をそのまま使った標準偏差。
    pub std_before_quarter: f64,
    /// 補正を1/4にした標準偏差。
    pub std_after_quarter: f64,
}
pub(super) struct Exported {
    pub status: &'static str,
    pub zero: ZeroCorrectionReport,
    pub quantization: QuantizationReport,
    pub correction: CorrectionDistribution,
}

pub(super) fn write_best(
    dataset: &Dataset,
    model: &Model,
    base: &Pst,
    directory: &Path,
    sample: usize,
    seed: u64,
) -> Result<Exported> {
    model.validate()?;
    atomic_write(&directory.join("fm-float.npz"), &encode_model(model)?)?;
    let v: Vec<_> = model.v().iter().map(|row| row.map(f64::from)).collect();
    let a = model.a().map(f64::from);
    let fm = if v
        .iter()
        .all(|row| row.iter().zip(a).all(|(&v, a)| v == 0.0 || a == 0.0))
    {
        Quantized {
            embeddings: vec![[0; FM_RANK]; FEATURE_COUNT],
            signs: [1; FM_RANK],
            exponent: 0,
        }
    } else {
        quantize(&v, &a, f64::from(base.k()))?
    };
    let zero = zero_correction_report(&fm.embeddings, &fm.signs, fm.exponent + 1)?;
    if fm.exponent == 30 && !zero.identically_zero {
        return Err("quarter correction exponent would exceed 30".into());
    }
    let output = Pst::from_weights_with_fm(
        base.weights(),
        std::array::from_fn(|s| base.piece_value_of_state(s)),
        base.k(),
        base.rule_set(),
        fm.as_weights(),
    )?;
    atomic_write(&directory.join("fm.bin"), &output.encode())?;
    let written = Pst::decode(&std::fs::read(directory.join("fm.bin"))?)?;
    if written.weights() != base.weights()
        || written.k().to_bits() != base.k().to_bits()
        || written.rule_set() != base.rule_set()
        || (0..pst::PIECE_STATE_COUNT)
            .any(|s| written.piece_value_of_state(s) != base.piece_value_of_state(s))
        || written.fm_weights() != Some(fm.as_weights())
    {
        return Err("written MNPT differs from fixed base or exported FM".into());
    }
    let mut indices = epoch_permutation(dataset.validation_indices(), seed, 0);
    indices.truncate(sample);
    if indices.is_empty() {
        return Err("empty FM quantization sample".into());
    }
    let mut errors = [0.0; 2];
    let mut maxima = [0.0_f64; 2];
    let mut corrections = [
        Vec::with_capacity(indices.len()),
        Vec::with_capacity(indices.len()),
    ];
    for &index in &indices {
        let position = dataset.record(index)?.to_position()?;
        let mut features = Vec::with_capacity(145);
        active_features(&position, |f| features.push(f));
        let baseline = i64::from(pst::evaluate_pst(base, &position));
        let floating = f64::from(base.k()) * phi(&v, &a, &features);
        let integer = correction(fm.as_weights(), &features);
        for (i, divisor) in [1_i64, 4].into_iter().enumerate() {
            let value = integer / divisor;
            let error = (baseline as f64 + floating / divisor as f64
                - (baseline + value).clamp(-28999, 28999) as f64)
                .abs();
            if !error.is_finite() {
                return Err("non-finite FM quantization error".into());
            }
            errors[i] += error;
            maxima[i] = maxima[i].max(error);
            corrections[i].push(value as f64);
        }
    }
    let mean = errors.map(|v| v / indices.len() as f64);
    Ok(Exported {
        status: if zero.identically_zero {
            "excluded_zero_correction"
        } else {
            "candidate"
        },
        zero,
        quantization: QuantizationReport {
            exponent: fm.exponent,
            samples: indices.len(),
            seed,
            split: "validation",
            indices_sha256: hash_u64(&indices),
            mean_absolute_error_cp: mean[0],
            max_absolute_error_cp: maxima[0],
            quarter_mean_absolute_error_cp: mean[1],
            quarter_max_absolute_error_cp: maxima[1],
            limit_cp: 2.0,
            passed: mean.iter().all(|&v| v <= 2.0),
        },
        correction: CorrectionDistribution {
            samples: indices.len(),
            seed,
            split: "validation",
            std_before_quarter: std(&corrections[0]),
            std_after_quarter: std(&corrections[1]),
        },
    })
}
