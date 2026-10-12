//! 最良PSTの量子化、MNPT版2とNPZの保存、エンジンとの誤差検査。

use super::{
    Batch, Checkpoint, Error, Model, Result,
    base::Base,
    batch::{FEATURE_COLUMNS, SENTINEL},
    linear,
    npz::encode_pst_npz,
    record::hash_u64,
};
use crate::{
    data::{Dataset, epoch_permutation},
    nn::atomic_write,
};
use minase::eval::pst::{Pst, evaluate_pst};
use serde::Serialize;
use std::path::Path;

/// 書き出したPSTの量子化誤差と合否。
#[derive(Debug, Serialize)]
pub struct QuantizationReport {
    /// 実際の検証標本数。
    pub samples: usize,
    /// 標本抽出のシード。
    pub seed: u64,
    /// 標本の集合。常にvalidation。
    pub split: &'static str,
    /// 選択順の通し番号をリトルエンディアンで連結したSHA-256。
    pub indices_sha256: String,
    /// 平均絶対誤差、センチポーン。
    pub mean_absolute_error_cp: f64,
    /// 最大絶対誤差、センチポーン。
    pub max_absolute_error_cp: f64,
    /// 平均絶対誤差の上限、センチポーン。
    pub limit_cp: f64,
    /// 平均絶対誤差が上限以下なら真。
    pub passed: bool,
}

/// f32をf64で8倍して偶数へ丸める。範囲外と非有限値は拒否する。
pub fn quantize(model: &Model) -> Result<Vec<[i16; 2]>> {
    model
        .expanded()
        .into_iter()
        .map(|row| {
            let mut integers = [0; 2];
            for (e, value) in row.into_iter().enumerate() {
                let quantized = (f64::from(value) * 8.0).round_ties_even();
                if !quantized.is_finite()
                    || !(f64::from(i16::MIN)..=f64::from(i16::MAX)).contains(&quantized)
                {
                    return Err(Error::Invalid("PST quantization exceeds i16 range"));
                }
                integers[e] = quantized as i16;
            }
            Ok(integers)
        })
        .collect()
}

// NumPyの連続したf32配列のpairwise sum。float_evaluateは学習のf64和とは別に、
// 145列をf32で加算してからf32のφで補間する。
fn numpy_sum(values: &[f32]) -> f32 {
    if values.len() < 8 {
        return values.iter().fold(-0.0, |sum, value| sum + value);
    }
    if values.len() > 128 {
        let half = (values.len() / 2) & !7;
        return numpy_sum(&values[..half]) + numpy_sum(&values[half..]);
    }
    let mut sums: [f32; 8] = values[..8].try_into().expect("at least 8 elements");
    let stop = values.len() & !7;
    for block in values[8..stop].as_chunks::<8>().0 {
        for (sum, &value) in sums.iter_mut().zip(block) {
            *sum += value;
        }
    }
    let sum =
        ((sums[0] + sums[1]) + (sums[2] + sums[3])) + ((sums[4] + sums[5]) + (sums[6] + sums[7]));
    values[stop..].iter().fold(sum, |sum, value| sum + value)
}

pub(super) fn write_best(
    dataset: &Dataset,
    checkpoint: &Checkpoint,
    directory: &Path,
    base: &Base,
    sample: usize,
) -> Result<QuantizationReport> {
    if checkpoint
        .progress
        .best
        .is_none_or(|best| best.epoch == 0 || best.epoch != checkpoint.progress.epoch)
    {
        return Err(Error::Invalid("checkpoint is not a completed best epoch"));
    }
    let quantized = quantize(&checkpoint.model)?;
    let pst = Pst::from_weights(&quantized, base.piece_values, base.k, &base.rule_set)?;
    atomic_write(
        &directory.join("pst-float.npz"),
        &encode_pst_npz(&checkpoint.model.expanded())?,
    )?;
    atomic_write(&directory.join("pst.bin"), &pst.encode())?;
    let written = Pst::decode(&std::fs::read(directory.join("pst.bin"))?)?;
    if written.weights().as_slice() != quantized
        || written.k().to_bits() != base.k.to_bits()
        || written.rule_set() != base.rule_set
        || (0..base.piece_values.len())
            .any(|state| written.piece_value_of_state(state) != base.piece_values[state])
    {
        return Err(Error::Invalid(
            "written MNPT differs from quantized PST or initial metadata",
        ));
    }
    let seed = checkpoint.progress.seed;
    let mut indices = epoch_permutation(dataset.validation_indices(), seed, 0);
    indices.truncate(sample);
    if indices.is_empty() {
        return Err(Error::Invalid("empty quantization sample"));
    }
    let mut total = 0.0;
    let mut maximum = 0.0_f64;
    for chunk in indices.chunks(256) {
        let batch = Batch::from_indices(dataset, chunk)?;
        for (b, &index) in chunk.iter().enumerate() {
            let sums = std::array::from_fn(|e| {
                let values: [f32; FEATURE_COLUMNS] = std::array::from_fn(|col| {
                    let row = batch.columns()[b * FEATURE_COLUMNS + col];
                    if row == SENTINEL {
                        0.0
                    } else {
                        checkpoint.model.weights()[2 * row as usize + e]
                    }
                });
                numpy_sum(&values)
            });
            let float = linear::blend(sums, batch.phase_numerators()[b][0]);
            let integer = evaluate_pst(&written, &dataset.record(index)?.to_position()?);
            let error = (f64::from(float) - f64::from(integer)).abs();
            if !error.is_finite() {
                return Err(Error::Invalid("non-finite PST quantization error"));
            }
            total += error;
            maximum = maximum.max(error);
        }
    }
    let mean = total / indices.len() as f64;
    Ok(QuantizationReport {
        samples: indices.len(),
        seed,
        split: "validation",
        indices_sha256: hash_u64(&indices),
        mean_absolute_error_cp: mean,
        max_absolute_error_cp: maximum,
        limit_cp: 2.0,
        passed: mean <= 2.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantization_float_sum_matches_numpy_f32_pairwise_order() {
        // 独立したNumPyのsum(dtype=float32)から採取したビット列。
        // 145列は128要素のブロック境界と末尾の端数をともに通る。
        for (count, expected) in [
            (1, 0x457ffe00),
            (7, 0x45800502),
            (8, 0x3f901000),
            (72, 0x41116eca),
            (73, 0x458047b7),
            (128, 0x41832c00),
            (145, 0x458091d2),
        ] {
            let values: Vec<f32> = (0..count)
                .map(|i| match i % 3 {
                    0 => 4095.875,
                    1 => -4095.5,
                    _ => (0.0001 * (i + 1) as f64) as f32,
                })
                .collect();
            assert_eq!(numpy_sum(&values).to_bits(), expected, "count={count}");
        }
    }
}
