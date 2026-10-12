//! 局面内および局面間の加算順を固定したFMの順伝播と後退。

use super::{
    Batch, Model, Result,
    batch::PADDING,
    model::{PARAMETERS, V_LEN},
};
use crate::nn::{bce_with_logits, bce_with_logits_gradient, parallel};
use minase::eval::{features::FEATURE_COUNT, pst::FM_RANK};
use serde::{Deserialize, Serialize};

/// バッチ平均のBCEと、係数を掛けた正則化項。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Losses {
    /// 二値交差エントロピー。
    pub bce: f64,
    /// lambda_res * mean(Phi²)。
    pub reg: f64,
}
impl Losses {
    /// 最適化に使う損失。
    pub fn total(self) -> f64 {
        self.bce + self.reg
    }
}
/// 全パラメータのf32勾配と平均損失。
pub struct Gradients {
    /// 平均損失。
    pub losses: Losses,
    /// V、埋め草、aの順の勾配。
    pub weights: Vec<f32>,
}

struct Activation {
    sums: [f64; FM_RANK],
    pairs: [f64; FM_RANK],
    phi: f64,
    bce: f64,
    derivative: f64,
}
fn forward(
    model: &Model,
    batch: &Batch,
    coefficient: f64,
    threads: usize,
) -> Result<Vec<Activation>> {
    model.validate()?;
    let chunks = parallel::map(
        parallel::ranges(batch.len(), threads)?,
        |range| -> Result<Vec<_>> {
            range
                .map(|b| {
                    let (sums, pairs, phi) = statistics(model, &batch.columns[b]);
                    let logit = f64::from(batch.baseline[b] as f32 / model.k()) + phi;
                    let target = f64::from(batch.targets[b]);
                    let bce = bce_with_logits(logit, target);
                    let derivative = (bce_with_logits_gradient(logit, target)
                        + 2.0 * coefficient * phi)
                        / batch.len() as f64;
                    if !phi.is_finite() || !bce.is_finite() || !derivative.is_finite() {
                        return Err("non-finite FM loss or residual".into());
                    }
                    Ok(Activation {
                        sums,
                        pairs,
                        phi,
                        bce,
                        derivative,
                    })
                })
                .collect()
        },
    )?;
    Ok(chunks.into_iter().flatten().collect())
}

/// Vとaの両方を微分する。各行は局面順でf64加算し、最後にf32へ丸める。
pub fn gradients(
    model: &Model,
    batch: &Batch,
    lambda_res: f64,
    threads: usize,
) -> Result<Gradients> {
    if !lambda_res.is_finite() || lambda_res < 0.0 {
        return Err("invalid FM regularization coefficient".into());
    }
    let activations = forward(model, batch, lambda_res, threads)?;
    let mut occurrences = vec![Vec::new(); FEATURE_COUNT];
    for (b, columns) in batch.columns.iter().enumerate() {
        for &feature in columns {
            if feature != PADDING && model.mask[feature as usize] {
                occurrences[feature as usize].push(b);
            }
        }
    }
    let chunks = parallel::map(
        parallel::ranges(FEATURE_COUNT, threads)?,
        |range| -> Result<Vec<f32>> {
            let mut values = Vec::with_capacity(range.len() * FM_RANK);
            for row in range {
                let mut gradient = [0.0; FM_RANK];
                for &b in &occurrences[row] {
                    let activation = &activations[b];
                    for (f, g) in gradient.iter_mut().enumerate() {
                        *g += activation.derivative
                            * f64::from(model.a()[f])
                            * (activation.sums[f] - f64::from(model.v()[row][f]));
                    }
                }
                values.extend(gradient.map(|g| g as f32));
            }
            Ok(values)
        },
    )?;
    let mut weights: Vec<_> = chunks.into_iter().flatten().collect();
    weights.resize(PARAMETERS, 0.0);
    let mut a = [0.0; FM_RANK];
    let mut losses = Losses::default();
    for activation in &activations {
        losses.bce += activation.bce;
        losses.reg += activation.phi * activation.phi;
        for (f, g) in a.iter_mut().enumerate() {
            *g += activation.derivative * activation.pairs[f];
        }
    }
    weights[V_LEN..].copy_from_slice(&a.map(|g| g as f32));
    losses.bce /= batch.len() as f64;
    losses.reg = lambda_res * (losses.reg / batch.len() as f64);
    if !losses.total().is_finite() || weights.iter().any(|g| !g.is_finite()) {
        return Err("non-finite FM loss or gradient".into());
    }
    Ok(Gradients { losses, weights })
}

/// 検証集合全体のFM補正の分布。単位はロジット。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Distribution {
    /// 平均。
    pub mean: f64,
    /// 母標準偏差。
    pub std: f64,
    /// 最大絶対値。
    pub max_absolute: f64,
    /// 全検証局面数。
    pub samples: usize,
}
pub(super) fn validation(model: &Model, batch: &Batch) -> Result<Vec<(f64, f64)>> {
    Ok(forward(model, batch, 0.0, 1)?
        .into_iter()
        .map(|a| (a.bce, a.phi))
        .collect())
}

/// 最初の5更新の勾配ノルム。チェックポイントにも保存して再開時に引き継ぐ。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientNorm {
    /// 1から始まる更新回数。
    pub step: u64,
    /// aのユークリッドノルム。
    pub a: f64,
    /// Vのユークリッドノルム。
    pub embedding: f64,
}
impl GradientNorm {
    pub(super) fn new(step: u64, gradient: &[f32]) -> Self {
        let norm = |v: &[f32]| v.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>().sqrt();
        Self {
            step,
            a: norm(&gradient[V_LEN..]),
            embedding: norm(&gradient[..V_LEN]),
        }
    }
}

pub(crate) fn statistics(model: &Model, columns: &[u32]) -> ([f64; FM_RANK], [f64; FM_RANK], f64) {
    let mut sums = [0.0; FM_RANK];
    let mut squares = [0.0; FM_RANK];
    for &feature in columns {
        let start = feature as usize * FM_RANK;
        for f in 0..FM_RANK {
            let v = f64::from(model.weights[start + f]);
            sums[f] += v;
            squares[f] += v * v;
        }
    }
    let pairs = std::array::from_fn(|f| 0.5 * (sums[f] * sums[f] - squares[f]));
    let phi: f64 = pairs
        .iter()
        .zip(model.a())
        .map(|(&pair, &a)| pair * f64::from(a))
        .sum();
    (sums, pairs, phi)
}
