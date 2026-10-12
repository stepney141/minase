//! PSTのCPU前進と後退。局面ごとの計算を並列化し、局面順にf64で縮約する。

use super::{
    Batch, Error, Model, RemovalReference, Result,
    batch::{FEATURE_COLUMNS, SENTINEL},
    linear,
    model::CANONICAL_ROWS,
};
use crate::nn::{bce_with_logits, bce_with_logits_gradient, parallel};
use std::collections::BTreeMap;

/// 係数を掛ける前の除去損失と二値交差エントロピーのバッチ平均。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Losses {
    /// 二値交差エントロピー。
    pub bce: f64,
    /// 元局面等重み、対象除去等重みでKで割った補助損失。
    pub removal: f64,
}

/// バッチ平均の損失と全正準行の勾配。
pub struct Gradients {
    /// 係数適用前の損失。
    pub losses: Losses,
    /// 埋め草を含む[6841, 2]の勾配。
    pub weights: Vec<f32>,
}

fn check(model: &Model, batch: &Batch, targets: &[f32]) -> Result<()> {
    model.validate()?;
    if targets.len() != batch.len()
        || targets
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(Error::Invalid("PST target shape or range"));
    }
    Ok(())
}

/// 固定順の倍精度和をf32へ戻し、Python model_logitsと同じ2端点を補間する。
pub fn evaluate(model: &Model, batch: &Batch) -> Result<Vec<f32>> {
    model.validate()?;
    let result: Vec<_> = (0..batch.len())
        .map(|b| {
            linear::blend(
                linear::sum_mirrored_f64(model.weights(), batch.columns_at(b)).map(|v| v as f32),
                batch.phase_numerators()[b][0],
            )
        })
        .collect();
    if result.iter().any(|v| !v.is_finite()) {
        return Err(Error::Invalid("non-finite PST evaluation"));
    }
    Ok(result)
}

/// 全検証局面の損失の和。呼び手が局面数で割り、除去損失は含めない。
pub fn validation_sum(model: &Model, batch: &Batch, targets: &[f32]) -> Result<f64> {
    check(model, batch, targets)?;
    Ok(evaluate(model, batch)?
        .into_iter()
        .zip(targets)
        .map(|(cp, &target)| bce_with_logits(f64::from(cp / model.k()), f64::from(target)))
        .sum())
}

/// 固定した局面順で勾配を合算する。threadsは結果のビット列を変えない。
pub fn gradients(
    model: &Model,
    batch: &Batch,
    targets: &[f32],
    reference: Option<&RemovalReference>,
    removal_penalty: f64,
    threads: usize,
) -> Result<Gradients> {
    check(model, batch, targets)?;
    if !removal_penalty.is_finite()
        || removal_penalty < 0.0
        || (removal_penalty > 0.0) != reference.is_some()
    {
        return Err(Error::Invalid("PST removal coefficient or reference"));
    }
    let chunks = parallel::map(parallel::ranges(batch.len(), threads)?, |range| {
        range
            .map(|b| position_gradient(model, batch, b, targets[b], reference, removal_penalty))
            .collect::<Result<Vec<_>>>()
    })?;
    let mut sums = vec![0.0_f64; CANONICAL_ROWS * 2];
    let mut losses = Losses::default();
    for (loss, gradient) in chunks.into_iter().flatten() {
        losses.bce += loss.bce;
        losses.removal += loss.removal;
        for (row, values) in gradient {
            for e in 0..2 {
                sums[2 * row as usize + e] += values[e];
            }
        }
    }
    losses.bce /= batch.len() as f64;
    losses.removal /= batch.len() as f64;
    let weights: Vec<_> = sums.into_iter().map(|value| value as f32).collect();
    if !losses.bce.is_finite()
        || !losses.removal.is_finite()
        || weights.iter().any(|v| !v.is_finite())
    {
        return Err(Error::Invalid("non-finite PST loss or gradient"));
    }
    Ok(Gradients { losses, weights })
}

type PositionGradient = (Losses, Vec<(u32, [f64; 2])>);

fn position_gradient(
    model: &Model,
    batch: &Batch,
    b: usize,
    target: f32,
    reference: Option<&RemovalReference>,
    coefficient: f64,
) -> Result<PositionGradient> {
    let columns = batch.columns_at(b);
    let numerators = batch.phase_numerators()[b];
    let sums = linear::sum_mirrored_f64(model.weights(), columns);
    let logit = f64::from(linear::blend(sums.map(|v| v as f32), numerators[0]) / model.k());
    let k = f64::from(model.k());
    let n = batch.len() as f64;
    let g = bce_with_logits_gradient(logit, f64::from(target)) / n / k;
    let u = linear::phase(numerators[0]).map(f64::from);
    let mut losses = Losses {
        bce: bce_with_logits(logit, f64::from(target)),
        removal: 0.0,
    };
    let mut derivatives = [0_i32; FEATURE_COLUMNS];
    let mut eligible_count = 0;
    if let Some(reference) = reference {
        let (refs, magnitudes) = reference.deltas(batch, b)?;
        for col in 0..144 {
            if !linear::eligible(columns[col], refs[col]) {
                continue;
            }
            eligible_count += 1;
            let delta = linear::removal_delta(
                sums,
                linear::row(model.weights(), columns[col]).map(f64::from),
                numerators,
            );
            let (penalty, derivative) = linear::penalty(
                delta,
                refs[col].signum(),
                magnitudes[col],
                linear::wrong_direction(columns[col], refs[col]),
            );
            losses.removal += penalty;
            derivatives[col] = derivative;
        }
        losses.removal /= eligible_count.max(1) as f64 * k;
    }
    let [q, after_q] = numerators.map(i32::from);
    let derivative_sum: i32 = derivatives.iter().sum();
    let common = (after_q - q) * derivative_sum;
    // 各除去で、自身は(-q, -(90-q))、残存行は(q'-q, q-q')。
    // 全行に残存係数を足して自身から(q',90-q')を引くと同じ整数係数になる。
    // 鏡映対が同じ行へ写る場合も整数で合算してから倍率を掛け、f32差分で相殺しない。
    let mut rows = BTreeMap::<u32, [i32; 3]>::new();
    for (col, &row) in columns.iter().enumerate() {
        if row == SENTINEL {
            continue;
        }
        let entry = rows.entry(row).or_default();
        entry[0] += 1;
        entry[1] += common - after_q * derivatives[col];
        entry[2] += -common - (90 - after_q) * derivatives[col];
    }
    let scale = coefficient / n / eligible_count.max(1) as f64 / k / 90.0;
    let gradient = rows
        .into_iter()
        .map(|(row, counts)| {
            (
                row,
                [
                    f64::from(counts[0]) * g * u[0] + f64::from(counts[1]) * scale,
                    f64::from(counts[0]) * g * u[1] + f64::from(counts[2]) * scale,
                ],
            )
        })
        .collect();
    Ok((losses, gradient))
}
