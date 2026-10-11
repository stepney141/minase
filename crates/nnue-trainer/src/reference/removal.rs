use super::{
    Forward, check_batch, head, linear_row, logit, phase, row_value, screlu, screlu_derivative,
};
use crate::{
    data::{BatchSlot, CANONICAL_ROWS, FEATURE_COLUMNS, SENTINEL},
    model::{Error, Model, Result, canonical},
};
use minase::eval::{
    features::{FEATURE_COUNT, PIECE_STATE_COUNT, mirror_feature, piece_state},
    handcrafted,
};
use minase_core::{Color, PieceCode, PieceKind};

const PHASE_DIVISOR: i64 = 90;
const EVALUATION_LIMIT: i64 = 28_999;
/// 補助損失の係数。Python train.pyのremoval_penaltyと同じ。
pub const REMOVAL_PENALTY: f64 = 1000.0;

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
    pub fn deltas(&self, batch: &BatchSlot, b: usize) -> (Vec<i32>, Vec<f64>) {
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
        (deltas, magnitude)
    }
}

/// カーネル2の中間値と部分和。除去列は元の盤面の144升に対応する。
#[derive(Debug)]
pub struct Removal {
    /// 除去後の評価E_r[B,144]、センチポーン。
    pub e_r: Vec<f64>,
    /// 除去差分Δ_r[B,144]、センチポーン。
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
    /// 局面ごとの補助損失。係数1000を含み、バッチ平均前。
    pub loss: Vec<f64>,
}

/// カーネル2。両視点の同じ升を除き、罰則と後退用の部分和を計算する。
pub fn removal_forward(
    model: &Model,
    batch: &BatchSlot,
    weights: &[f32],
    forward: &Forward,
    reference: &RemovalReference,
) -> Result<Removal> {
    check_batch(model, batch, weights)?;
    let n = batch.len();
    let h = model.shape().width();
    let k = f64::from(model.shape().k());
    let o = model.expanded_output_weights();
    let mut result = Removal {
        e_r: vec![0.0; n * 144],
        delta_r: vec![0.0; n * 144],
        ref_r: vec![0; n * 144],
        s_r: vec![0.0; n * 144],
        eligible: vec![false; n * 144],
        wrong_r: vec![false; n * 144],
        pen_r: vec![0.0; n * 144],
        d_r: vec![0.0; n * 144],
        s: vec![0.0; n],
        d_acc_removal: vec![0.0; n * 2 * h],
        d_output: vec![0.0; n * 2 * h],
        linear: vec![[0.0; 2]; n],
        loss: vec![0.0; n],
    };
    for b in 0..n {
        let (refs, magnitudes) = reference.deltas(batch, b);
        let mut count = 0;
        for (col, &reference) in refs.iter().enumerate() {
            let i = b * 144 + col;
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
        let scale = REMOVAL_PENALTY / (n * count.max(1)) as f64;
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
        let before_head: [f32; 2] =
            std::array::from_fn(|v| head(model, v, |j| forward.acc[(b * 2 + v) * h + j]));
        let mut d_acc = vec![0.0_f64; 2 * h];
        let mut d_output = vec![0.0_f64; 2 * h];
        let mut s = 0.0_f64;
        for (col, &magnitude) in magnitudes.iter().enumerate() {
            let i = b * 144 + col;
            let w = linear_row(model, batch.linear_features()[b * FEATURE_COLUMNS + col])
                .map(f64::from);
            let after_linear = after_q * (sums[0] - w[0]) + (90.0 - after_q) * (sums[1] - w[1]);
            let mut delta = (after_linear - before_linear) / 90.0;
            for (v, &before) in before_head.iter().enumerate() {
                let feature = batch.features()[(b * 2 + v) * FEATURE_COLUMNS + col];
                let after = head(model, v, |j| {
                    forward.acc[(b * 2 + v) * h + j] - row_value(model, weights, feature, j)
                });
                delta += k * f64::from(after - before);
            }
            result.delta_r[i] = delta;
            result.e_r[i] = f64::from(logit(model, batch, forward, b)) * k + delta;
            if !result.eligible[i] {
                continue;
            }
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
            result.loss[b] += REMOVAL_PENALTY * penalty / count.max(1) as f64 / k;
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
            for v in 0..2 {
                let feature = batch.features()[(b * 2 + v) * FEATURE_COLUMNS + col];
                for j in 0..h {
                    let after =
                        forward.acc[(b * 2 + v) * h + j] - row_value(model, weights, feature, j);
                    d_acc[v * h + j] += f64::from(d)
                        * f64::from(o[v * h + j])
                        * f64::from(screlu_derivative(after));
                    d_output[v * h + j] += f64::from(d) * f64::from(screlu(after));
                }
            }
        }
        result.s[b] = s as f32;
        for j in 0..2 * h {
            result.d_acc_removal[b * 2 * h + j] = d_acc[j] as f32;
            result.d_output[b * 2 * h + j] = d_output[j] as f32;
        }
        let u = phase(batch.piece_counts()[b], false);
        let after_u = phase(batch.piece_counts()[b], true);
        for e in 0..2 {
            result.linear[b][e] = (s * f64::from(after_u[e] - u[e]) / k) as f32;
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
}
