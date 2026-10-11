//! 設計書のカーネル1〜8に対応するCPU参照計算。GPUや学習ループを持たない。
//!
//! 配列は局面、視点、ユニットの順で行優先。Bは局面数、Hは展開後の幅。
//! 除去後の文脈と先獅子は変えない。線形除去差分はPython同様にf64で計算する。

mod optimizer;
mod removal;
pub use optimizer::{AdamW, AdamWConfig, AdamWGroup, adamw};
pub use removal::{Removal, RemovalReference, removal_forward, v0_piece_values};

use crate::{
    data::{
        BatchSlot, CANONICAL_ROWS, FEATURE_COLUMNS, InputFeatureSet, PackedOccurrence, SENTINEL,
    },
    model::{Error, Model, Parameter, Parameters, Result, rotated_unit},
};
use minase::eval::features::FEATURE_COUNT;

/// カーネル1の出力。
#[derive(Clone, Debug)]
pub struct Forward {
    /// 累算器acc[B, 2, H]。
    pub acc: Vec<f32>,
    /// 線形端点和lin[B, 2]。手番側視点。
    pub lin: Vec<[f32; 2]>,
}
/// カーネル3の出力。縮約前の寄与を局面順に保持する。
#[derive(Debug)]
pub struct Output {
    /// SCReLUの値f(acc)[B, 2, H]。
    pub f: Vec<f32>,
    /// SCReLUの微分h(acc)[B, 2, H]。
    pub h: Vec<f32>,
    /// ロジット[B]。
    pub logits: Vec<f32>,
    /// 局面ごとの二値交差エントロピー[B]。バッチ平均前。
    pub loss: Vec<f32>,
    /// バッチ平均BCEのロジット微分g[B]。
    pub g: Vec<f32>,
    /// A_v = (g-S)o_v h(acc_v) + d_acc_removal、[B, 2, H]。
    pub d_acc: Vec<f32>,
    /// 出力重みへの全幅の寄与[B, 2, H]。
    pub d_output: Vec<f32>,
    /// 線形係数c[B, 2]。
    pub c: Vec<[f32; 2]>,
}
/// カーネル4で得たバッチ平均の損失。
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Losses {
    /// 二値交差エントロピー。
    pub bce: f64,
    /// 係数1000を掛けた補助損失。
    pub auxiliary: f64,
}
impl Losses {
    /// 学習する損失の和。
    pub fn total(self) -> f64 {
        self.bce + self.auxiliary
    }
}
/// 1バッチの参照計算。パラメータは変更しない。
pub struct Gradients {
    /// 二値交差エントロピーと補助損失。
    pub losses: Losses,
    /// 全パラメータの勾配。
    pub parameters: Parameters,
}

/// SCReLU f(x)=clamp(x,0,1)²。
pub fn screlu(x: f32) -> f32 {
    x.clamp(0.0, 1.0).powi(2)
}
/// PyTorch clampと同じ端点を含む微分。x>1の飽和域は0。
pub fn screlu_derivative(x: f32) -> f32 {
    if (0.0..=1.0).contains(&x) {
        2.0 * x
    } else {
        0.0
    }
}
pub(crate) fn phase(count: u8, removed: bool) -> [f32; 2] {
    let phi = f32::from(count.saturating_sub(if removed { 3 } else { 2 }).min(90)) / 90.0;
    [phi, 1.0 - phi]
}

/// HalfKPのC=W+V[base]を作る。P型では回転前の正準行を返す。埋め草を含む。
pub fn fold(model: &Model) -> Vec<f32> {
    if model.shape().feature_set() == InputFeatureSet::P {
        model.parameters()[Parameter::FirstLayer].to_vec()
    } else {
        model.expanded_first_layer()
    }
}
pub(crate) fn row_value(model: &Model, weights: &[f32], feature: u32, j: usize) -> f32 {
    if feature == SENTINEL {
        return 0.0;
    }
    let s = model.shape();
    let (row, unit) = if s.feature_set() == InputFeatureSet::P {
        (
            feature as usize / 2,
            rotated_unit(feature & 1 != 0, j, s.width()),
        )
    } else {
        (feature as usize, j)
    };
    weights[row * s.width() + unit]
}
pub(crate) fn linear_row(model: &Model, feature: u32) -> [f32; 2] {
    if feature == SENTINEL {
        [0.0; 2]
    } else {
        let i = 2 * feature as usize;
        [
            model.parameters()[Parameter::Linear][i],
            model.parameters()[Parameter::Linear][i + 1],
        ]
    }
}
fn check_batch(model: &Model, batch: &BatchSlot, weights: &[f32]) -> Result<()> {
    if batch.is_empty()
        || batch.feature_set() != model.shape().feature_set()
        || weights.len() != model.shape().len(Parameter::FirstLayer)
    {
        return Err(Error::Invalid("reference batch or folded weights shape"));
    }
    Ok(())
}

// Python _feature_sumと同じく左右の升を先に対にする。鏡映時も同じ加算順になる。
fn feature_sum(mut value: impl FnMut(usize) -> f32) -> f32 {
    let mut result = 0.0;
    for rank in 0..12 {
        for file in 0..6 {
            result += value(rank * 12 + file) + value(rank * 12 + 11 - file);
        }
    }
    result + value(144)
}

/// カーネル1。145列のgather-sumと線形の端点和を求める。
pub fn forward(model: &Model, batch: &BatchSlot, weights: &[f32]) -> Result<Forward> {
    check_batch(model, batch, weights)?;
    let h = model.shape().width();
    let bias = model.expanded_biases();
    let mut acc = vec![0.0; batch.len() * 2 * h];
    let mut lin = vec![[0.0; 2]; batch.len()];
    for (b, linear) in lin.iter_mut().enumerate() {
        for (endpoint, value) in linear.iter_mut().enumerate() {
            *value = feature_sum(|col| {
                linear_row(model, batch.linear_features()[b * FEATURE_COLUMNS + col])[endpoint]
            });
        }
        for v in 0..2 {
            for j in 0..h {
                acc[(b * 2 + v) * h + j] = feature_sum(|col| {
                    row_value(
                        model,
                        weights,
                        batch.features()[(b * 2 + v) * FEATURE_COLUMNS + col],
                        j,
                    )
                }) + bias[j];
            }
        }
    }
    Ok(Forward { acc, lin })
}

pub(crate) fn head(model: &Model, v: usize, mut hidden: impl FnMut(usize) -> f32) -> f32 {
    let s = model.shape();
    let mut result = 0.0;
    for j in 0..s.units() {
        let mut activation = screlu(hidden(j));
        if s.feature_set() == InputFeatureSet::P {
            activation += screlu(hidden(j + s.units()));
        }
        result += activation * model.parameters()[Parameter::OutputWeights][v * s.units() + j];
    }
    result
}
pub(crate) fn logit(model: &Model, batch: &BatchSlot, f: &Forward, b: usize) -> f32 {
    let h = model.shape().width();
    let u = phase(batch.piece_counts()[b], false);
    let nonlinear = head(model, 0, |j| f.acc[b * 2 * h + j])
        + head(model, 1, |j| f.acc[(b * 2 + 1) * h + j])
        + model.parameters()[Parameter::OutputBias][0];
    (u[0] * f.lin[b][0] + u[1] * f.lin[b][1]) / model.shape().k() + nonlinear
}
/// 浮動小数点の評価をセンチポーンで返す。学習と同じf32の前進を使う。
pub fn evaluate(model: &Model, batch: &BatchSlot) -> Result<Vec<f32>> {
    let weights = fold(model);
    let f = forward(model, batch, &weights)?;
    Ok((0..batch.len())
        .map(|b| logit(model, batch, &f, b) * model.shape().k())
        .collect())
}

/// 分類ごとの教師用Kを使う。λ=1で、勝敗は混ぜない。
pub fn targets(batch: &BatchSlot, teacher_ks: &[f64]) -> Result<Vec<f32>> {
    batch
        .teacher_scores()
        .iter()
        .zip(batch.teacher_classes())
        .map(|(&score, &class)| {
            let k = *teacher_ks
                .get(class as usize)
                .ok_or(Error::Invalid("teacher class"))?;
            if !k.is_finite() || k <= 0.0 || !score.is_finite() {
                return Err(Error::Invalid("teacher K or score"));
            }
            Ok((1.0 / (1.0 + (-f64::from(score) / k).exp())) as f32)
        })
        .collect()
}

/// カーネル3。出力、BCE、g、A_v、および線形係数cを計算する。
pub fn output_backward(
    model: &Model,
    batch: &BatchSlot,
    forward: &Forward,
    removal: &Removal,
    targets: &[f32],
) -> Result<Output> {
    let h = model.shape().width();
    let n = batch.len();
    if targets.len() != n
        || targets
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(Error::Invalid("targets"));
    }
    let o = model.expanded_output_weights();
    let mut result = Output {
        f: forward.acc.iter().map(|&x| screlu(x)).collect(),
        h: forward.acc.iter().map(|&x| screlu_derivative(x)).collect(),
        logits: vec![0.0; n],
        loss: vec![0.0; n],
        g: vec![0.0; n],
        d_acc: vec![0.0; n * 2 * h],
        d_output: vec![0.0; n * 2 * h],
        c: vec![[0.0; 2]; n],
    };
    for (b, &target) in targets.iter().enumerate() {
        let x = logit(model, batch, forward, b);
        let g = (1.0 / (1.0 + (-x).exp()) - target) / n as f32;
        result.logits[b] = x;
        result.g[b] = g;
        result.loss[b] = x.max(0.0) - x * target + (-x.abs()).exp().ln_1p();
        let u = phase(batch.piece_counts()[b], false);
        for (e, &u_e) in u.iter().enumerate() {
            result.c[b][e] = g * u_e / model.shape().k() + removal.linear[b][e];
        }
        for (vj, &o_vj) in o.iter().enumerate() {
            let i = b * 2 * h + vj;
            result.d_acc[i] = ((f64::from(g) - f64::from(removal.s[b]))
                * f64::from(o_vj)
                * f64::from(result.h[i])
                + f64::from(removal.d_acc_removal[i])) as f32;
            result.d_output[i] = ((f64::from(g) - f64::from(removal.s[b])) * f64::from(result.f[i])
                + f64::from(removal.d_output[i])) as f32;
        }
    }
    Ok(result)
}

/// カーネル4。局面順で部分和を縮約し、P型は左右半幅の勾配を合算する。
pub fn reduce_partials(
    model: &Model,
    output: &Output,
    removal: &Removal,
    gradient: &mut Parameters,
) -> Losses {
    let s = model.shape();
    let n = output.g.len();
    let h = s.width();
    let units = s.units();
    for j in 0..units {
        let mut bias = 0.0_f64;
        for v in 0..2 {
            let mut out = 0.0_f64;
            for b in 0..n {
                for full in (j..h).step_by(units) {
                    let i = (b * 2 + v) * h + full;
                    bias += f64::from(output.d_acc[i]);
                    out += f64::from(output.d_output[i]);
                }
            }
            gradient[Parameter::OutputWeights][v * units + j] = out as f32;
        }
        gradient[Parameter::Biases][j] = bias as f32;
    }
    gradient[Parameter::OutputBias][0] = output.g.iter().map(|&v| f64::from(v)).sum::<f64>() as f32;
    Losses {
        bce: output.loss.iter().map(|&v| f64::from(v)).sum::<f64>() / n as f64,
        auxiliary: removal.loss.iter().sum::<f64>() / n as f64,
    }
}

/// カーネル5。逆索引を固定順で走査し、除去した行自身の寄与を引く。
pub fn first_layer_backward(
    model: &Model,
    batch: &BatchSlot,
    weights: &[f32],
    forward: &Forward,
    removal: &Removal,
    output: &Output,
    gradient: &mut Parameters,
) {
    let h = model.shape().width();
    let o = model.expanded_output_weights();
    let inverse = batch.first_layer_inverse();
    gradient[Parameter::FirstLayer].fill(0.0);
    for row in 0..inverse.starts().len() - 1 {
        for j in 0..h {
            let mut sum = 0.0_f64;
            for &word in &inverse.entries()
                [inverse.starts()[row] as usize..inverse.starts()[row + 1] as usize]
            {
                let entry = PackedOccurrence::from_word(word).expect("validated reverse index");
                let b = entry.position();
                let v = entry.perspective() as usize;
                let col = entry.column() as usize;
                let unit = rotated_unit(entry.rotated(), j, h);
                let i = (b * 2 + v) * h + unit;
                let d = if col < 144 {
                    removal.d_r[b * 144 + col]
                } else {
                    0.0
                };
                let feature = batch.features()[(b * 2 + v) * FEATURE_COLUMNS + col];
                sum += f64::from(output.d_acc[i])
                    - f64::from(d)
                        * f64::from(o[v * h + unit])
                        * f64::from(screlu_derivative(
                            forward.acc[i] - row_value(model, weights, feature, unit),
                        ));
            }
            gradient[Parameter::FirstLayer][row * h + j] = sum as f32;
        }
    }
}

/// カーネル6。線形逆索引からc-d_j u_j/Kを正準行へ足す。回転はしない。
pub fn linear_backward(
    model: &Model,
    batch: &BatchSlot,
    removal: &Removal,
    output: &Output,
    gradient: &mut Parameters,
) {
    let inverse = batch.linear_inverse();
    for row in 0..CANONICAL_ROWS {
        for endpoint in 0..2 {
            let mut sum = 0.0_f64;
            for &word in &inverse.entries()
                [inverse.starts()[row] as usize..inverse.starts()[row + 1] as usize]
            {
                let entry = PackedOccurrence::from_word(word).expect("validated reverse index");
                let b = entry.position();
                let col = entry.column() as usize;
                let d = if col < 144 {
                    removal.d_r[b * 144 + col]
                } else {
                    0.0
                };
                sum += f64::from(output.c[b][endpoint])
                    - f64::from(d) * f64::from(phase(batch.piece_counts()[b], true)[endpoint])
                        / f64::from(model.shape().k());
            }
            gradient[Parameter::Linear][row * 2 + endpoint] = sum as f32;
        }
    }
}

/// カーネル7。dV[p]=Σ_k dW[k*13680+p]を8区分の順で縮約する。
pub fn reduce_virtual(gradient: &mut Parameters) {
    if gradient.shape().feature_set() == InputFeatureSet::P {
        return;
    }
    let h = gradient.shape().width();
    for base in 0..FEATURE_COUNT {
        for j in 0..h {
            let mut sum = 0.0_f64;
            for bucket in 0..8 {
                sum += f64::from(
                    gradient[Parameter::FirstLayer][(bucket * FEATURE_COUNT + base) * h + j],
                );
            }
            gradient[Parameter::Virtual][base * h + j] = sum as f32;
        }
    }
    gradient[Parameter::Virtual][FEATURE_COUNT * h..].fill(0.0);
}

/// カーネル1〜7を1回実行する。更新はAdamW::updateで別に行う。
pub fn loss_and_gradients(
    model: &Model,
    batch: &BatchSlot,
    teacher_ks: &[f64],
    reference: &RemovalReference,
) -> Result<Gradients> {
    model.parameters().validate()?;
    let weights = fold(model);
    let forward = forward(model, batch, &weights)?;
    let removal = removal_forward(model, batch, &weights, &forward, reference)?;
    let output = output_backward(
        model,
        batch,
        &forward,
        &removal,
        &targets(batch, teacher_ks)?,
    )?;
    let mut parameters = Parameters::zeros(model.shape());
    let losses = reduce_partials(model, &output, &removal, &mut parameters);
    first_layer_backward(
        model,
        batch,
        &weights,
        &forward,
        &removal,
        &output,
        &mut parameters,
    );
    linear_backward(model, batch, &removal, &output, &mut parameters);
    reduce_virtual(&mut parameters);
    parameters.validate()?;
    if !losses.total().is_finite() {
        return Err(Error::Invalid("non-finite loss"));
    }
    Ok(Gradients { losses, parameters })
}
