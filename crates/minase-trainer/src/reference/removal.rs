use super::{
    Forward, check_batch, check_batch_arrays, check_weights, linear_row, parallel, phase,
    row_value, screlu, screlu_derivative,
};
use crate::{
    data::{BatchSlot, CANONICAL_ROWS, FEATURE_COLUMNS, InputFeatureSet, SENTINEL},
    model::{Error, Model, Parameter, Result, canonical},
};
use minase::eval::{
    features::{FEATURE_COUNT, PIECE_STATE_COUNT, mirror_feature, piece_state},
    handcrafted,
};
use minase_core::{Color, PieceCode, PieceKind};

const PHASE_DIVISOR: i64 = 90;
const EVALUATION_LIMIT: i64 = 28_999;

/// mnpt.initial_piece_valuesと同じv0駒価値。公開APIから取り、定数を複製しない。
pub fn v0_piece_values() -> [i32; PIECE_STATE_COUNT] {
    let mut values = [0; PIECE_STATE_COUNT];
    for kind in PieceKind::ALL {
        let value = handcrafted::piece_value(kind);
        values[kind.index()] = value;
        if let Some(piece) = PieceCode::new(Color::Black, kind) {
            values[piece_state(piece)] = value;
        }
    }
    values
}

/// 鏡映共有の整数PST。1/8センチポーンの2端点を正準行で保持する。
#[derive(Clone, Debug)]
pub struct RemovalReference {
    rows: Vec<[i64; 2]>,
}
impl RemovalReference {
    /// GPUへ一度だけ転送する、正準行順の整数2端点。
    pub(crate) fn rows(&self) -> &[[i64; 2]] {
        &self.rows
    }

    /// Python make_removal_reference(v0,v0)と同じ表。
    pub fn v0() -> Self {
        let values = v0_piece_values();
        let mut rows = vec![[0; 2]; CANONICAL_ROWS];
        for (row, value) in rows[..94 * 72].iter_mut().enumerate() {
            let sign = if row / (47 * 72) == 0 { 1 } else { -1 };
            *value = [sign * i64::from(values[row / 72 % 47]) * 8; 2];
        }
        Self { rows }
    }
    /// 独立な整数PSTを参照として使う。境界と上限制約の試験にも使える。
    pub fn new(endpoints: &[[i16; 2]]) -> Result<Self> {
        if endpoints.len() != FEATURE_COUNT {
            return Err(Error::Invalid("removal reference length"));
        }
        let mut rows = vec![[0; 2]; CANONICAL_ROWS];
        for base in 0..FEATURE_COUNT {
            if endpoints[base] != endpoints[mirror_feature(base)] {
                return Err(Error::Invalid("removal reference mirror sharing"));
            }
            rows[canonical(base)] = endpoints[base].map(i64::from);
        }
        Ok(Self { rows })
    }
    /// _numeratorsの整数演算による差分と、切り捨て前の差分の大きさ。
    ///
    /// Pythonは符号だけを切り捨て・評価上限適用後の差分から取り、罰則の大きさは
    /// 切り捨て前から取る。v0でも評価上限で符号が0になる場合を区別する。
    pub fn deltas(&self, batch: &BatchSlot, b: usize) -> Result<(Vec<i32>, Vec<f64>)> {
        check_batch_arrays(batch)?;
        if b >= batch.len() || self.rows.len() != CANONICAL_ROWS {
            return Err(Error::Invalid("removal reference shape or position"));
        }
        let columns = &batch.linear_features()[b * FEATURE_COLUMNS..(b + 1) * FEATURE_COLUMNS];
        let row = |col: usize| {
            if columns[col] == SENTINEL {
                [0; 2]
            } else {
                self.rows[columns[col] as usize]
            }
        };
        let mut sums = [0_i64; 2];
        for col in 0..FEATURE_COLUMNS {
            for (s, value) in sums.iter_mut().zip(row(col)) {
                *s += value;
            }
        }
        let q = i64::from(batch.piece_counts()[b].saturating_sub(2)).min(PHASE_DIVISOR);
        let after_q = i64::from(batch.piece_counts()[b].saturating_sub(3)).min(PHASE_DIVISOR);
        let before = q * sums[0] + (PHASE_DIVISOR - q) * sums[1];
        let integer = |x: i64| (x / (PHASE_DIVISOR * 8)).clamp(-EVALUATION_LIMIT, EVALUATION_LIMIT);
        let mut deltas = vec![0; 144];
        let mut magnitude = vec![0.0; 144];
        for col in 0..144 {
            let w = row(col);
            let after = after_q * (sums[0] - w[0]) + (PHASE_DIVISOR - after_q) * (sums[1] - w[1]);
            deltas[col] = (integer(after) - integer(before)) as i32;
            magnitude[col] = ((after - before) as f64 / (PHASE_DIVISOR * 8) as f64).abs();
        }
        Ok((deltas, magnitude))
    }
}

/// カーネル2の中間値と部分和。除去列は元の盤面の144升に対応する。
#[derive(Debug)]
pub struct Removal {
    /// 除去後の評価E_r[B,144]、センチポーン。不適格な列は0。
    pub e_r: Vec<f64>,
    /// 除去差分Δ_r[B,144]、センチポーン。不適格な列は0。
    pub delta_r: Vec<f64>,
    /// 整数参照差分ref_r[B,144]。
    pub ref_r: Vec<i32>,
    /// 参照差分の符号s_r[B,144]。
    pub s_r: Vec<f32>,
    /// 番兵・王駒・参照符号0を除く適格性[B,144]。
    pub eligible: Vec<bool>,
    /// 参照の向きが駒の所有者から期待する向きと逆か[B,144]。
    pub wrong_r: Vec<bool>,
    /// 適格な除去の罰則pen_r[B,144]、センチポーン。
    pub pen_r: Vec<f64>,
    /// K∂L_aux/∂Δ_r、[B,144]。
    pub d_r: Vec<f32>,
    /// S=Σ_r d_r、[B]。
    pub s: Vec<f32>,
    /// Σ_r d_r o_v h(acc_v-W_r)、[B,2,H]。
    pub d_acc_removal: Vec<f32>,
    /// Σ_r d_r f(acc_v-W_r)、[B,2,H]。
    pub d_output: Vec<f32>,
    /// 線形係数S(u_r-u)/K、[B,2]。
    pub linear: Vec<[f32; 2]>,
    /// 局面ごとの補助損失。指定した係数を含み、バッチ平均前。
    pub loss: Vec<f64>,
}
impl Removal {
    pub(super) fn check_shape(&self, n: usize, h: usize) -> Result<()> {
        let columns = n.checked_mul(144);
        if [&self.e_r, &self.delta_r, &self.pen_r]
            .iter()
            .any(|values| Some(values.len()) != columns)
            || [&self.s_r, &self.d_r]
                .iter()
                .any(|values| Some(values.len()) != columns)
            || [&self.eligible, &self.wrong_r]
                .iter()
                .any(|values| Some(values.len()) != columns)
            || Some(self.ref_r.len()) != columns
            || self.s.len() != n
            || self.loss.len() != n
            || self.linear.len() != n
            || [&self.d_acc_removal, &self.d_output]
                .iter()
                .any(|values| Some(values.len()) != n.checked_mul(2 * h))
        {
            return Err(Error::Invalid("reference removal shape"));
        }
        Ok(())
    }
}

// GPUのremoval_headと同じf32縮約。各レーンはlane, lane+32, ...の順に足し、
// 距離16, 8, 4, 2, 1の木でレーン0へ集める。未使用レーンの部分和は0。
fn removal_head(model: &Model, v: usize, mut hidden: impl FnMut(usize) -> f32) -> f32 {
    let shape = model.shape();
    let units = shape.units();
    let mut partials = [0.0_f32; 32];
    for (lane, sum) in partials.iter_mut().enumerate() {
        for j in (lane..units).step_by(32) {
            let mut activation = screlu(hidden(j));
            if shape.feature_set() == InputFeatureSet::P {
                activation += screlu(hidden(j + units));
            }
            *sum += activation * model.parameters()[Parameter::OutputWeights][v * units + j];
        }
    }
    for distance in [16, 8, 4, 2, 1] {
        for lane in 0..distance {
            partials[lane] += partials[lane + distance];
        }
    }
    partials[0]
}

/// カーネル2。両視点の同じ升を除き、罰則と後退用の部分和を計算する。
pub fn removal_forward(
    model: &Model,
    batch: &BatchSlot,
    weights: &[f32],
    forward: &Forward,
    reference: &RemovalReference,
    removal_penalty: f64,
) -> Result<Removal> {
    removal_forward_parallel(
        model,
        batch,
        weights,
        forward,
        reference,
        removal_penalty,
        1,
    )
}

pub(super) fn removal_forward_parallel(
    model: &Model,
    batch: &BatchSlot,
    weights: &[f32],
    forward: &Forward,
    reference: &RemovalReference,
    removal_penalty: f64,
    threads: usize,
) -> Result<Removal> {
    if !removal_penalty.is_finite() || removal_penalty < 0.0 {
        return Err(Error::Invalid(
            "removal penalty must be finite and nonnegative",
        ));
    }
    check_batch(model, batch)?;
    check_weights(model, weights)?;
    let n = batch.len();
    let h = model.shape().width();
    forward.check_shape(n, h)?;
    let k = f64::from(model.shape().k());
    let o = model.expanded_output_weights();
    let chunks = parallel::map(parallel::ranges(n, threads)?, |range| {
        let length = range.len();
        let mut result = Removal {
            e_r: vec![0.0; length * 144],
            delta_r: vec![0.0; length * 144],
            ref_r: vec![0; length * 144],
            s_r: vec![0.0; length * 144],
            eligible: vec![false; length * 144],
            wrong_r: vec![false; length * 144],
            pen_r: vec![0.0; length * 144],
            d_r: vec![0.0; length * 144],
            s: vec![0.0; length],
            d_acc_removal: vec![0.0; length * 2 * h],
            d_output: vec![0.0; length * 2 * h],
            linear: vec![[0.0; 2]; length],
            loss: vec![0.0; length],
        };
        for (local, b) in range.enumerate() {
            let (refs, magnitudes) = reference.deltas(batch, b)?;
            let mut count = 0;
            for (col, &reference) in refs.iter().enumerate() {
                let i = local * 144 + col;
                let row = batch.linear_features()[b * FEATURE_COLUMNS + col];
                let state = row as usize / 72 % 47;
                result.ref_r[i] = reference;
                result.s_r[i] = reference.signum() as f32;
                result.eligible[i] = row != SENTINEL
                    && (row as usize) < 94 * 72
                    && ![11, 21].contains(&state)
                    && reference != 0;
                result.wrong_r[i] = (row < 47 * 72 && reference > 0)
                    || ((47 * 72..94 * 72).contains(&row) && reference < 0);
                count += usize::from(result.eligible[i]);
            }
            let scale = removal_penalty / (n * count.max(1)) as f64;
            let mut sums = [0.0_f64; 2];
            for col in 0..FEATURE_COLUMNS {
                for (sum, w) in sums.iter_mut().zip(linear_row(
                    model,
                    batch.linear_features()[b * FEATURE_COLUMNS + col],
                )) {
                    *sum += f64::from(w);
                }
            }
            let q = f64::from(batch.piece_counts()[b].saturating_sub(2).min(90));
            let after_q = f64::from(batch.piece_counts()[b].saturating_sub(3).min(90));
            let before_linear = q * sums[0] + (90.0 - q) * sums[1];
            let before_head: [f32; 2] = std::array::from_fn(|v| {
                removal_head(model, v, |j| forward.acc[(b * 2 + v) * h + j])
            });
            // E_rの基準となるEも、補助損失の縮約順で求める。
            let u = phase(batch.piece_counts()[b], false);
            let nonlinear =
                before_head[0] + before_head[1] + model.parameters()[Parameter::OutputBias][0];
            let before_e = f64::from(
                (u[0] * forward.lin[b][0] + u[1] * forward.lin[b][1]) / model.shape().k()
                    + nonlinear,
            ) * k;
            let mut d_acc = vec![0.0_f64; 2 * h];
            let mut d_output = vec![0.0_f64; 2 * h];
            let mut s = 0.0_f64;
            for (col, &magnitude) in magnitudes.iter().enumerate() {
                let i = local * 144 + col;
                if !result.eligible[i] {
                    continue;
                }
                let w = linear_row(model, batch.linear_features()[b * FEATURE_COLUMNS + col])
                    .map(f64::from);
                let after_linear = after_q * (sums[0] - w[0]) + (90.0 - after_q) * (sums[1] - w[1]);
                let mut delta = (after_linear - before_linear) / 90.0;
                for (v, &before) in before_head.iter().enumerate() {
                    let feature = batch.features()[(b * 2 + v) * FEATURE_COLUMNS + col];
                    let after = removal_head(model, v, |j| {
                        forward.acc[(b * 2 + v) * h + j] - row_value(model, weights, feature, j)
                    });
                    delta += k * f64::from(after - before);
                }
                result.delta_r[i] = delta;
                result.e_r[i] = before_e + delta;
                let sign = f64::from(result.s_r[i]);
                let signed = sign * delta;
                let lower = magnitude.min(0.25);
                let penalty = (lower - signed).max(0.0)
                    + if result.wrong_r[i] {
                        (signed - magnitude).max(0.0)
                    } else {
                        0.0
                    };
                result.pen_r[i] = penalty;
                result.loss[local] += removal_penalty * penalty / count.max(1) as f64 / k;
                let derivative = if signed < lower {
                    -sign
                } else if result.wrong_r[i] && signed > magnitude {
                    sign
                } else {
                    0.0
                };
                let d = (scale * derivative) as f32;
                result.d_r[i] = d;
                s += f64::from(d);
                if d == 0.0 {
                    continue;
                }
                for v in 0..2 {
                    let feature = batch.features()[(b * 2 + v) * FEATURE_COLUMNS + col];
                    for j in 0..h {
                        let after = forward.acc[(b * 2 + v) * h + j]
                            - row_value(model, weights, feature, j);
                        d_acc[v * h + j] += f64::from(d)
                            * f64::from(o[v * h + j])
                            * f64::from(screlu_derivative(after));
                        d_output[v * h + j] += f64::from(d) * f64::from(screlu(after));
                    }
                }
            }
            result.s[local] = s as f32;
            for j in 0..2 * h {
                result.d_acc_removal[local * 2 * h + j] = d_acc[j] as f32;
                result.d_output[local * 2 * h + j] = d_output[j] as f32;
            }
            let after_u = phase(batch.piece_counts()[b], true);
            for e in 0..2 {
                result.linear[local][e] = (s * f64::from(after_u[e] - u[e]) / k) as f32;
            }
        }
        if result
            .delta_r
            .iter()
            .chain(&result.loss)
            .any(|v| !v.is_finite())
        {
            return Err(Error::Invalid("non-finite removal values"));
        }
        Ok(result)
    })?;
    let mut result = Removal {
        e_r: Vec::with_capacity(n * 144),
        delta_r: Vec::with_capacity(n * 144),
        ref_r: Vec::with_capacity(n * 144),
        s_r: Vec::with_capacity(n * 144),
        eligible: Vec::with_capacity(n * 144),
        wrong_r: Vec::with_capacity(n * 144),
        pen_r: Vec::with_capacity(n * 144),
        d_r: Vec::with_capacity(n * 144),
        s: Vec::with_capacity(n),
        d_acc_removal: Vec::with_capacity(n * 2 * h),
        d_output: Vec::with_capacity(n * 2 * h),
        linear: Vec::with_capacity(n),
        loss: Vec::with_capacity(n),
    };
    for chunk in chunks {
        result.e_r.extend(chunk.e_r);
        result.delta_r.extend(chunk.delta_r);
        result.ref_r.extend(chunk.ref_r);
        result.s_r.extend(chunk.s_r);
        result.eligible.extend(chunk.eligible);
        result.wrong_r.extend(chunk.wrong_r);
        result.pen_r.extend(chunk.pen_r);
        result.d_r.extend(chunk.d_r);
        result.s.extend(chunk.s);
        result.d_acc_removal.extend(chunk.d_acc_removal);
        result.d_output.extend(chunk.d_output);
        result.linear.extend(chunk.linear);
        result.loss.extend(chunk.loss);
    }
    Ok(result)
}
