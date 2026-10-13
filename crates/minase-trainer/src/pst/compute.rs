//! PSTのCPU前進と後退。局面ごとの計算を並列化し、局面順にf64で縮約する。

use super::{
    Batch, Error, Model, RemovalReference, Result, batch::FEATURE_COLUMNS, linear,
    model::CANONICAL_ROWS,
};
use crate::nn::{bce_with_logits, bce_with_logits_gradient, parallel};

/// 係数を掛ける前の除去損失と二値交差エントロピーのバッチ平均。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Losses {
    /// 二値交差エントロピー。
    pub bce: f64,
    /// 各局面の適格な除去の罰則を平均し、Kで割ってロジット単位にしてからバッチで平均した値。
    /// 対象0件の局面は0であり、除去係数はまだ掛けていない。
    pub removal: f64,
}

/// バッチ平均の損失と全正準行の勾配。
pub struct Gradients {
    /// 係数適用前の損失。
    pub losses: Losses,
    /// 埋め草を含む`[6841, 2]`の勾配。
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

/// 固定順の倍精度和をf32へ戻し、序中盤と終盤の2端点を補間する。
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

/// 損失`BCE(E/K) + removal_penalty×removal`の勾配を、局面順を固定して合算する。
/// 除去係数が正であることと参照の指定は同値でなければエラーにし、係数0なら除去計算を省く。
/// `threads`は結果のビット列を変えない。
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

/// 1局面の二値交差エントロピーと[`Losses::removal`]を求め、バッチ平均に対応する勾配を返す。
/// 除去の対象数が0なら除去損失を0とする。
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
    let scale = coefficient / n / eligible_count.max(1) as f64 / k / 90.0;
    let gradient = linear::gradient_rows(columns, numerators, &derivatives, g, u, scale);
    Ok((losses, gradient))
}
