use super::{Result, ScaleArgs, npz::FloatWeights};
use crate::data::{Dataset, epoch_permutation, replay::read_sample};
use minase::eval::{
    features::active_features,
    pst::{self, FM_RANK, FmWeights, Pst},
};
use serde::Serialize;

/// 検証集合の同じ標本に対する、実数・整数評価と調整用補正の比較。
#[derive(Debug, Serialize)]
pub struct ValidationReport {
    /// 抽出した標本数。
    pub samples: usize,
    /// 標本抽出のシード。
    pub seed: u64,
    /// 常にvalidation。
    pub split: &'static str,
    /// 実数評価と出力MNPTの整数評価の差。
    pub float_vs_integer: DifferenceReport,
    /// 調整用補正と焼き込んだ補正の差。
    pub tuning_vs_baked: DifferenceReport,
    /// 補正の母標準偏差。
    pub correction_cp: CorrectionReport,
}

/// センチポーン単位の絶対差。
#[derive(Debug, Serialize)]
pub struct DifferenceReport {
    /// 平均絶対差。
    pub mean_absolute_cp: f64,
    /// 最大絶対差。
    pub max_absolute_cp: f64,
}

/// 補正の母標準偏差、センチポーン。
#[derive(Debug, Serialize)]
pub struct CorrectionReport {
    /// 書き出す重みの補正の標準偏差。
    pub output_std: f64,
    /// 入力重みへ調整用倍率を掛けた補正の標準偏差。
    pub tuning_std: f64,
}

/// trunc(N·FmScale/(1024·2^(2e+1)))をi128で計算する。
/// 倍率は0以上1024以下、指数は0以上30以下に制限する。
pub fn tuning_correction(numerator: i64, fm_scale: i32, exponent: u32) -> Result<i64> {
    if !(0..=1024).contains(&fm_scale) || exponent > 30 {
        return Err("invalid FM scale or exponent".into());
    }
    Ok((i128::from(numerator) * i128::from(fm_scale) / (1024_i128 << (2 * exponent + 1))) as i64)
}

// Pythonのfm_accumulatorsと同じAとC。特徴は検査済み局面からの最大145個。
fn accumulators(fm: FmWeights<'_>, features: &[usize]) -> ([i64; FM_RANK], i64) {
    let mut sums = [0_i64; FM_RANK];
    let mut self_sum = 0_i64;
    for &feature in features {
        for (f, sum) in sums.iter_mut().enumerate() {
            let u = i64::from(fm.embeddings[feature][f]);
            *sum += u;
            self_sum += u * u * i64::from(fm.signs[f]);
        }
    }
    (sums, self_sum)
}

fn numerator(fm: FmWeights<'_>, features: &[usize]) -> i64 {
    let (sums, self_sum) = accumulators(fm, features);
    sums.iter()
        .zip(fm.signs)
        .map(|(&a, &d)| a * a * i64::from(d))
        .sum::<i64>()
        - self_sum
}

pub(super) fn correction(fm: FmWeights<'_>, features: &[usize]) -> i64 {
    numerator(fm, features) / (1_i64 << (2 * fm.exponent + 1))
}

pub(super) fn phi(v: &[[f64; FM_RANK]], a: &[f64; FM_RANK], features: &[usize]) -> f64 {
    let mut sums = [0.0; FM_RANK];
    let mut squares = [0.0; FM_RANK];
    for &feature in features {
        for f in 0..FM_RANK {
            let value = v[feature][f];
            sums[f] += value;
            squares[f] += value * value;
        }
    }
    let pairs: [f64; FM_RANK] = std::array::from_fn(|f| (sums[f] * sums[f] - squares[f]) * a[f]);
    0.5 * numpy_sum(&pairs)
}

// NumPyの連続f64配列のpairwise sum。標準偏差も平均との差の二乗を同じ順序で足す。
fn numpy_sum(values: &[f64]) -> f64 {
    if values.len() < 8 {
        return values.iter().fold(-0.0, |sum, value| sum + value);
    }
    if values.len() > 128 {
        let half = (values.len() / 2) & !7;
        return numpy_sum(&values[..half]) + numpy_sum(&values[half..]);
    }
    let mut sums: [f64; 8] = values[..8].try_into().expect("at least 8 elements");
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

fn difference(values: &[f64]) -> DifferenceReport {
    DifferenceReport {
        mean_absolute_cp: numpy_sum(values) / values.len() as f64,
        max_absolute_cp: values.iter().copied().fold(0.0, f64::max),
    }
}

pub(super) fn std(values: &[f64]) -> f64 {
    let mean = numpy_sum(values) / values.len() as f64;
    let squares: Vec<_> = values
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .collect();
    (numpy_sum(&squares) / values.len() as f64).sqrt()
}

pub(super) fn scale_report(
    source: &Pst,
    output: &Pst,
    weights: &FloatWeights,
    scaled_a: &[f64; FM_RANK],
    args: &ScaleArgs,
) -> Result<ValidationReport> {
    // SAFETY: ほかの学習器コマンドと同じく、実行中の入力MNSDを変更しない契約で開く。
    let dataset = unsafe { Dataset::open(&args.data, None, None)? };
    let count = args
        .validation_sample
        .min(dataset.validation_indices().len());
    if count == 0 {
        return Err("validation set must be nonempty".into());
    }
    let indices = if let Some(path) = &args.replay_sample {
        read_sample(path, &dataset, count, true)?
    } else {
        let mut indices = epoch_permutation(dataset.validation_indices(), args.seed, 0);
        indices.truncate(count);
        indices.sort_unstable();
        indices
    };
    let source_fm = source.fm_weights().expect("validated MNPT version 3");
    let mut errors = Vec::with_capacity(count);
    let mut differences = Vec::with_capacity(count);
    let mut baked_corrections = Vec::with_capacity(count);
    let mut tuning_corrections = Vec::with_capacity(count);
    for chunk in indices.chunks(args.batch) {
        for &index in chunk {
            let position = dataset.record(index)?.to_position()?;
            let mut features = Vec::with_capacity(145);
            active_features(&position, |feature| features.push(feature));
            let baseline = f64::from(pst::evaluate_pst(source, &position));
            let floating = baseline + f64::from(source.k()) * phi(&weights.v, scaled_a, &features);
            let integer = f64::from(pst::evaluate(output, &position));
            let tuning = tuning_correction(
                numerator(source_fm, &features),
                args.fm_scale,
                source_fm.exponent,
            )?;
            let baked = output
                .fm_weights()
                .map_or(0, |fm| correction(fm, &features));
            let error = (floating - integer).abs();
            if !error.is_finite() {
                return Err("non-finite quantization error".into());
            }
            errors.push(error);
            differences.push((tuning - baked).abs() as f64);
            baked_corrections.push(baked as f64);
            tuning_corrections.push(tuning as f64);
        }
    }
    Ok(ValidationReport {
        samples: count,
        seed: args.seed,
        split: "validation",
        float_vs_integer: difference(&errors),
        tuning_vs_baked: difference(&differences),
        correction_cp: CorrectionReport {
            output_std: std(&baked_corrections),
            tuning_std: std(&tuning_corrections),
        },
    })
}

#[cfg(test)]
mod tests;
