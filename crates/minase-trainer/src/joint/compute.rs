use super::{Batch, Model, Result};
use crate::{
    fm,
    nn::{bce_with_logits, bce_with_logits_gradient, parallel},
    pst::{self, linear},
};
use minase::eval::{features::FEATURE_COUNT, pst::FM_RANK};
use serde::{Deserialize, Serialize};

/// 1バッチの平均損失。removalは係数適用前、regは適用後。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Losses {
    /// 二値交差エントロピー。
    pub bce: f64,
    /// Kで割った駒除去差分の損失。
    pub removal: f64,
    /// FM補正の二乗平均。
    pub phi_squared_mean: f64,
    /// lambda_resを掛けた正則化項。
    pub reg: f64,
    /// すべての係数を適用した損失。
    pub total: f64,
}
/// 両モデルの全要素への勾配。
pub struct Gradients {
    /// バッチ平均の損失。
    pub losses: Losses,
    /// 鏡映共有のPST正準行。埋め草を含む。
    pub pst: Vec<f32>,
    /// V、埋め草、aの順。
    pub fm: Vec<f32>,
}

/// 正則化も鏡映も追加しない順伝播。戻り値は各局面のBCEとPhi。
pub fn evaluate(model: &Model, batch: &Batch) -> Result<Vec<(f64, f64)>> {
    model.validate()?;
    let cp = pst::evaluate(&model.pst, &batch.pst)?;
    cp.into_iter()
        .enumerate()
        .map(|(b, cp)| {
            let (_, _, phi) = fm::compute::statistics(&model.fm, &batch.columns[b]);
            let z = f64::from(cp / model.pst.k()) + phi;
            let loss = bce_with_logits(z, f64::from(batch.targets[b]));
            if !loss.is_finite() || !phi.is_finite() {
                return Err("non-finite joint validation".into());
            }
            Ok((loss, phi))
        })
        .collect()
}

struct Activation {
    losses: Losses,
    sums: [f64; FM_RANK],
    pairs: [f64; FM_RANK],
    derivative: f64,
    // The removal branch uses f32 embeddings and operations before its f64 conversion.
    removal_sums: [f32; FM_RANK],
    removal_upstream: [f32; 145],
    weighted_removals: [f64; FM_RANK],
    removal_a: [f64; FM_RANK],
    pst: Vec<(u32, [f64; 2])>,
}

fn position(
    model: &Model,
    batch: &Batch,
    b: usize,
    reference: &pst::RemovalReference,
    lambda_res: f64,
    removal_penalty: f64,
) -> Result<Activation> {
    let columns = &batch.columns[b];
    let pst_columns = batch.pst.columns_at(b);
    let numerators = batch.pst.phase_numerators()[b];
    let pst_sums = linear::sum_mirrored_f64(model.pst.weights(), pst_columns);
    let k = f64::from(model.pst.k());
    let n = batch.len() as f64;
    let (sums, pairs, phi) = fm::compute::statistics(&model.fm, columns);
    let z =
        f64::from(linear::blend(pst_sums.map(|v| v as f32), numerators[0]) / model.pst.k()) + phi;
    let bce_gradient = bce_with_logits_gradient(z, f64::from(batch.targets[b])) / n;
    let mut result = Activation {
        losses: Losses {
            bce: bce_with_logits(z, f64::from(batch.targets[b])),
            phi_squared_mean: phi * phi,
            ..Losses::default()
        },
        sums,
        pairs,
        derivative: bce_gradient + 2.0 * lambda_res * phi / n,
        removal_sums: sums.map(|v| v as f32),
        removal_upstream: [0.0; 145],
        weighted_removals: [0.0; FM_RANK],
        removal_a: [0.0; FM_RANK],
        pst: Vec::new(),
    };
    let (refs, magnitudes) = reference.deltas(&batch.pst, b)?;
    let mut derivatives = [0_i32; 145];
    let mut eligible = 0;
    for col in 0..144 {
        if !linear::eligible(pst_columns[col], refs[col]) {
            continue;
        }
        eligible += 1;
        let row = &model.fm.v()[columns[col] as usize];
        // a * V_j * (sum V - V_j) is evaluated in embedding precision,
        // including the rank reduction, before converting Delta Phi to f64.
        let delta_phi = -(0..FM_RANK)
            .map(|f| f64::from((model.fm.a()[f] * row[f]) * (result.removal_sums[f] - row[f])))
            .sum::<f64>() as f32;
        let delta = linear::removal_delta(
            pst_sums,
            linear::row(model.pst.weights(), pst_columns[col]).map(f64::from),
            numerators,
        ) + k * f64::from(delta_phi);
        let (loss, derivative) = linear::penalty(
            delta,
            refs[col].signum(),
            magnitudes[col],
            linear::wrong_direction(pst_columns[col], refs[col]),
        );
        result.losses.removal += loss;
        derivatives[col] = derivative;
    }
    result.losses.removal /= eligible.max(1) as f64 * k;
    let scale = removal_penalty / n / eligible.max(1) as f64;
    for col in 0..144 {
        let upstream = (f64::from(derivatives[col]) * scale) as f32;
        result.removal_upstream[col] = upstream;
        if upstream == 0.0 {
            continue;
        }
        let row = &model.fm.v()[columns[col] as usize];
        for (f, &v) in row.iter().enumerate() {
            result.weighted_removals[f] += f64::from(upstream * v);
            result.removal_a[f] += f64::from((-upstream * (result.removal_sums[f] - v)) * v);
        }
    }
    result.pst = linear::gradient_rows(
        pst_columns,
        numerators,
        &derivatives,
        bce_gradient / k,
        linear::phase(numerators[0]).map(f64::from),
        scale / k / 90.0,
    );
    Ok(result)
}

/// PST、V、aへBCE、正則化、駒除去の勾配を戻す。縮約順はthreadsによらない。
pub fn gradients(
    model: &Model,
    batch: &Batch,
    lambda_res: f64,
    removal_penalty: f64,
    threads: usize,
) -> Result<Gradients> {
    model.validate()?;
    if [lambda_res, removal_penalty]
        .iter()
        .any(|v| !v.is_finite() || *v < 0.0)
    {
        return Err("invalid joint loss coefficient".into());
    }
    let reference = pst::RemovalReference::v0();
    let chunks = parallel::map(parallel::ranges(batch.len(), threads)?, |range| {
        range
            .map(|b| position(model, batch, b, &reference, lambda_res, removal_penalty))
            .collect::<Result<Vec<_>>>()
    })?;
    let activations: Vec<_> = chunks.into_iter().flatten().collect();
    let mut occurrences = vec![Vec::new(); FEATURE_COUNT];
    for (b, columns) in batch.columns.iter().enumerate() {
        for (col, &feature) in columns.iter().enumerate() {
            if feature != fm::batch::PADDING && model.fm.mask()[feature as usize] {
                occurrences[feature as usize].push((b, col));
            }
        }
    }
    let chunks = parallel::map(
        parallel::ranges(FEATURE_COUNT, threads)?,
        |range| -> Result<Vec<f32>> {
            let mut values = Vec::with_capacity(range.len() * FM_RANK);
            for row in range {
                let mut gradient = [0.0; FM_RANK];
                for &(b, col) in &occurrences[row] {
                    let a = &activations[b];
                    for (f, g) in gradient.iter_mut().enumerate() {
                        let v = model.fm.v()[row][f];
                        let coefficient = model.fm.a()[f];
                        *g += a.derivative * f64::from(coefficient) * (a.sums[f] - f64::from(v));
                        // For removal j, every surviving feature gets -a*V_j.
                        // Its own removal additionally gets -a*(sum V - 2*V_j).
                        *g -= f64::from(coefficient)
                            * (a.weighted_removals[f]
                                + f64::from(a.removal_upstream[col])
                                    * (f64::from(a.removal_sums[f]) - 2.0 * f64::from(v)));
                    }
                }
                values.extend(gradient.map(|v| v as f32));
            }
            Ok(values)
        },
    )?;
    let mut fm: Vec<_> = chunks.into_iter().flatten().collect();
    fm.resize(fm::model::PARAMETERS, 0.0);
    let mut pst = vec![0.0_f64; model.pst.weights().len()];
    let mut a = [0.0; FM_RANK];
    let mut losses = Losses::default();
    for activation in activations {
        losses.bce += activation.losses.bce;
        losses.removal += activation.losses.removal;
        losses.phi_squared_mean += activation.losses.phi_squared_mean;
        for (row, gradient) in activation.pst {
            for e in 0..2 {
                pst[2 * row as usize + e] += gradient[e];
            }
        }
        for (f, g) in a.iter_mut().enumerate() {
            *g += activation.derivative * activation.pairs[f] + activation.removal_a[f];
        }
    }
    fm[fm::model::V_LEN..].copy_from_slice(&a.map(|v| v as f32));
    losses.bce /= batch.len() as f64;
    losses.removal /= batch.len() as f64;
    losses.phi_squared_mean /= batch.len() as f64;
    losses.reg = lambda_res * losses.phi_squared_mean;
    losses.total = losses.bce + losses.reg + removal_penalty * losses.removal;
    let pst: Vec<_> = pst.into_iter().map(|v| v as f32).collect();
    if !losses.total.is_finite() || pst.iter().chain(&fm).any(|v| !v.is_finite()) {
        return Err("non-finite joint loss or gradient".into());
    }
    Ok(Gradients { losses, pst, fm })
}

/// 全検証局面で測ったFM補正の分布。
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Distribution {
    /// ロジットの平均。
    pub mean: f64,
    /// ロジットの母標準偏差。
    pub std: f64,
    /// ロジットの最大絶対値。
    pub max_absolute: f64,
    /// 標本数。
    pub samples: usize,
    /// 常にlogit。
    pub unit: &'static str,
    /// Kを掛けた母標準偏差、センチポーン。
    pub std_cp: f64,
}
/// 最初の5更新の勾配ノルム。再開時にも引き継ぐ。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientNorm {
    /// 1から数える更新番号。
    pub step: u64,
    /// PSTのユークリッドノルム。
    pub pst: f64,
    /// Vのユークリッドノルム。
    pub embedding: f64,
    /// aのユークリッドノルム。
    pub a: f64,
}
impl GradientNorm {
    pub(super) fn new(step: u64, gradients: &Gradients) -> Self {
        let norm = |v: &[f32]| v.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>().sqrt();
        Self {
            step,
            pst: norm(&gradients.pst),
            embedding: norm(&gradients.fm[..fm::model::V_LEN]),
            a: norm(&gradients.fm[fm::model::V_LEN..]),
        }
    }
}
