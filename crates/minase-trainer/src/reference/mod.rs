//! 設計書のカーネル1〜8に対応するCPU参照計算。GPUや学習ループを持たない。
//!
//! 配列は局面、視点、ユニットの順で行優先。Bは局面数、Hは展開後の幅。
//! 除去後の文脈と先獅子は変えない。線形除去差分はPython同様にf64で計算する。

mod optimizer;
mod parallel;
mod removal;
mod validation;
pub use optimizer::{AdamW, AdamWConfig, AdamWGroup, adamw};
use removal::removal_forward_parallel;
pub use removal::{Removal, RemovalReference, removal_forward, v0_piece_values};
pub use validation::{Validation, validation_forward};

use crate::{
    data::{
        BatchSlot, CANONICAL_ROWS, FEATURE_COLUMNS, InputFeatureSet, PackedOccurrence, SENTINEL,
    },
    model::{Error, Model, Parameter, Parameters, Result, Shape, rotated_unit},
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
impl Forward {
    fn check_shape(&self, n: usize, h: usize) -> Result<()> {
        if Some(self.acc.len()) != n.checked_mul(2 * h) || self.lin.len() != n {
            return Err(Error::Invalid("reference forward shape"));
        }
        Ok(())
    }
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
impl Output {
    fn check_shape(&self, n: usize, h: usize) -> Result<()> {
        if n == 0
            || [&self.f, &self.h, &self.d_acc, &self.d_output]
                .iter()
                .any(|values| Some(values.len()) != n.checked_mul(2 * h))
            || [&self.logits, &self.loss, &self.g]
                .iter()
                .any(|values| values.len() != n)
            || self.c.len() != n
        {
            return Err(Error::Invalid("reference output shape"));
        }
        Ok(())
    }
}
/// カーネル4で得たバッチ平均の損失。
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Losses {
    /// 二値交差エントロピー。
    pub bce: f64,
    /// 指定した係数を掛けた補助損失。
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
pub fn fold(model: &Model) -> Result<Vec<f32>> {
    check_parameters(model.parameters(), model.shape())?;
    Ok(if model.shape().feature_set() == InputFeatureSet::P {
        model.parameters()[Parameter::FirstLayer].to_vec()
    } else {
        model.expanded_first_layer()
    })
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
fn check_parameters(parameters: &Parameters, shape: Shape) -> Result<()> {
    if parameters.shape() != shape
        || Parameter::ALL
            .iter()
            .any(|&p| parameters[p].len() != shape.len(p))
    {
        return Err(Error::Invalid("reference parameter shape"));
    }
    Ok(())
}
fn check_batch_arrays(batch: &BatchSlot) -> Result<()> {
    let n = batch.len();
    if n == 0
        || Some(batch.features().len()) != n.checked_mul(2 * FEATURE_COLUMNS)
        || Some(batch.linear_features().len()) != n.checked_mul(FEATURE_COLUMNS)
        || batch.piece_counts().len() != n
        || batch.teacher_scores().len() != n
        || batch.teacher_classes().len() != n
        || batch.outcomes().len() != n
        || batch.first_layer_inverse().starts().len() != batch.feature_set().first_layer_rows() + 1
        || batch.linear_inverse().starts().len() != CANONICAL_ROWS + 1
    {
        return Err(Error::Invalid("reference batch shape"));
    }
    Ok(())
}
fn check_batch(model: &Model, batch: &BatchSlot) -> Result<()> {
    check_parameters(model.parameters(), model.shape())?;
    check_batch_arrays(batch)?;
    if batch.feature_set() != model.shape().feature_set() {
        return Err(Error::Invalid("reference batch feature set"));
    }
    Ok(())
}
fn check_weights(model: &Model, weights: &[f32]) -> Result<()> {
    if weights.len() != model.shape().len(Parameter::FirstLayer) {
        return Err(Error::Invalid("reference folded weights shape"));
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
    forward_parallel(model, batch, weights, 1)
}

fn forward_parallel(
    model: &Model,
    batch: &BatchSlot,
    weights: &[f32],
    threads: usize,
) -> Result<Forward> {
    check_batch(model, batch)?;
    check_weights(model, weights)?;
    let h = model.shape().width();
    let bias = model.expanded_biases();
    let chunks = parallel::map(parallel::ranges(batch.len(), threads)?, |range| {
        let mut acc = vec![0.0; range.len() * 2 * h];
        let mut lin = vec![[0.0; 2]; range.len()];
        for (local, b) in range.enumerate() {
            for (endpoint, value) in lin[local].iter_mut().enumerate() {
                *value = feature_sum(|col| {
                    linear_row(model, batch.linear_features()[b * FEATURE_COLUMNS + col])[endpoint]
                });
            }
            for v in 0..2 {
                for j in 0..h {
                    acc[(local * 2 + v) * h + j] = feature_sum(|col| {
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
    })?;
    let mut result = Forward {
        acc: Vec::with_capacity(batch.len() * 2 * h),
        lin: Vec::with_capacity(batch.len()),
    };
    for chunk in chunks {
        result.acc.extend(chunk.acc);
        result.lin.extend(chunk.lin);
    }
    Ok(result)
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
    check_batch(model, batch)?;
    let weights = fold(model)?;
    let f = forward(model, batch, &weights)?;
    Ok((0..batch.len())
        .map(|b| logit(model, batch, &f, b) * model.shape().k())
        .collect())
}

/// 量子化検査用に、線形和から非線形出力まで倍精度で評価する。
///
/// Pythonのfloat_evaluateと同じく、保存用に単精度で畳み込んだ第1層を使い、
/// 両半幅の活性化を個別に倍精度で重み付けする。結果の単位はセンチポーン。
pub fn evaluate_f64(model: &Model, batch: &BatchSlot) -> Result<Vec<f64>> {
    let weights = fold(model)?;
    evaluate_f64_with_weights(model, batch, &weights, f64::from(model.shape().k()))
}

// 診断では畳み込みを共有し、Pythonと同じく比較する整数ネットのKを使う。
pub(crate) fn evaluate_f64_with_weights(
    model: &Model,
    batch: &BatchSlot,
    weights: &[f32],
    k: f64,
) -> Result<Vec<f64>> {
    check_batch(model, batch)?;
    check_weights(model, weights)?;
    let h = model.shape().width();
    let biases = model.expanded_biases();
    let output = model.expanded_output_weights();
    let mut values = Vec::with_capacity(batch.len());
    for b in 0..batch.len() {
        let mut linear = [0.0_f64; 2];
        for col in 0..FEATURE_COLUMNS {
            let row = linear_row(model, batch.linear_features()[b * FEATURE_COLUMNS + col]);
            for (sum, value) in linear.iter_mut().zip(row) {
                *sum += f64::from(value);
            }
        }
        let ratio = f64::from(batch.piece_counts()[b].saturating_sub(2).min(90)) / 90.0;
        let cp = ratio * linear[0] + (1.0 - ratio) * linear[1];
        let mut nonlinear = f64::from(model.parameters()[Parameter::OutputBias][0]);
        for v in 0..2 {
            let mut head = 0.0_f64;
            for j in 0..h {
                let mut hidden = 0.0_f64;
                for col in 0..FEATURE_COLUMNS {
                    hidden += f64::from(row_value(
                        model,
                        weights,
                        batch.features()[(b * 2 + v) * FEATURE_COLUMNS + col],
                        j,
                    ));
                }
                hidden += f64::from(biases[j]);
                head += hidden.clamp(0.0, 1.0).powi(2) * f64::from(output[v * h + j]);
            }
            nonlinear += head;
        }
        values.push(cp + k * nonlinear);
    }
    Ok(values)
}

/// カーネル3。出力、BCE、g、A_v、および線形係数cを計算する。
pub fn output_backward(
    model: &Model,
    batch: &BatchSlot,
    forward: &Forward,
    removal: &Removal,
    targets: &[f32],
) -> Result<Output> {
    output_backward_parallel(model, batch, forward, removal, targets, 1)
}

fn output_backward_parallel(
    model: &Model,
    batch: &BatchSlot,
    forward: &Forward,
    removal: &Removal,
    targets: &[f32],
    threads: usize,
) -> Result<Output> {
    check_batch(model, batch)?;
    let h = model.shape().width();
    let n = batch.len();
    forward.check_shape(n, h)?;
    removal.check_shape(n, h)?;
    if targets.len() != n
        || targets
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(Error::Invalid("targets"));
    }
    let o = model.expanded_output_weights();
    let chunks = parallel::map(parallel::ranges(n, threads)?, |range| {
        let length = range.len();
        let begin = range.start * 2 * h;
        let end = range.end * 2 * h;
        let mut result = Output {
            f: forward.acc[begin..end].iter().map(|&x| screlu(x)).collect(),
            h: forward.acc[begin..end]
                .iter()
                .map(|&x| screlu_derivative(x))
                .collect(),
            logits: vec![0.0; length],
            loss: vec![0.0; length],
            g: vec![0.0; length],
            d_acc: vec![0.0; length * 2 * h],
            d_output: vec![0.0; length * 2 * h],
            c: vec![[0.0; 2]; length],
        };
        for (local, b) in range.enumerate() {
            let target = targets[b];
            let x = logit(model, batch, forward, b);
            let g = (1.0 / (1.0 + (-x).exp()) - target) / n as f32;
            result.logits[local] = x;
            result.g[local] = g;
            result.loss[local] = x.max(0.0) - x * target + (-x.abs()).exp().ln_1p();
            let u = phase(batch.piece_counts()[b], false);
            for (e, &u_e) in u.iter().enumerate() {
                result.c[local][e] = g * u_e / model.shape().k() + removal.linear[b][e];
            }
            for (vj, &o_vj) in o.iter().enumerate() {
                let i = b * 2 * h + vj;
                result.d_acc[local * 2 * h + vj] = ((f64::from(g) - f64::from(removal.s[b]))
                    * f64::from(o_vj)
                    * f64::from(result.h[local * 2 * h + vj])
                    + f64::from(removal.d_acc_removal[i]))
                    as f32;
                result.d_output[local * 2 * h + vj] = ((f64::from(g) - f64::from(removal.s[b]))
                    * f64::from(result.f[local * 2 * h + vj])
                    + f64::from(removal.d_output[i]))
                    as f32;
            }
        }
        Ok(result)
    })?;
    let mut result = Output {
        f: Vec::with_capacity(n * 2 * h),
        h: Vec::with_capacity(n * 2 * h),
        logits: Vec::with_capacity(n),
        loss: Vec::with_capacity(n),
        g: Vec::with_capacity(n),
        d_acc: Vec::with_capacity(n * 2 * h),
        d_output: Vec::with_capacity(n * 2 * h),
        c: Vec::with_capacity(n),
    };
    for chunk in chunks {
        result.f.extend(chunk.f);
        result.h.extend(chunk.h);
        result.logits.extend(chunk.logits);
        result.loss.extend(chunk.loss);
        result.g.extend(chunk.g);
        result.d_acc.extend(chunk.d_acc);
        result.d_output.extend(chunk.d_output);
        result.c.extend(chunk.c);
    }
    Ok(result)
}

/// カーネル4。局面順で部分和を縮約し、P型は左右半幅の勾配を合算する。
pub fn reduce_partials(
    model: &Model,
    output: &Output,
    removal: &Removal,
    gradient: &mut Parameters,
) -> Result<Losses> {
    let s = model.shape();
    let n = output.g.len();
    let h = s.width();
    let units = s.units();
    check_parameters(model.parameters(), s)?;
    check_parameters(gradient, s)?;
    output.check_shape(n, h)?;
    removal.check_shape(n, h)?;
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
    Ok(Losses {
        bce: output.loss.iter().map(|&v| f64::from(v)).sum::<f64>() / n as f64,
        auxiliary: removal.loss.iter().sum::<f64>() / n as f64,
    })
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
) -> Result<()> {
    first_layer_backward_parallel(model, batch, weights, forward, removal, output, gradient, 1)
}

#[allow(clippy::too_many_arguments)]
fn first_layer_backward_parallel(
    model: &Model,
    batch: &BatchSlot,
    weights: &[f32],
    forward: &Forward,
    removal: &Removal,
    output: &Output,
    gradient: &mut Parameters,
    threads: usize,
) -> Result<()> {
    check_batch(model, batch)?;
    check_weights(model, weights)?;
    check_parameters(gradient, model.shape())?;
    let h = model.shape().width();
    forward.check_shape(batch.len(), h)?;
    removal.check_shape(batch.len(), h)?;
    output.check_shape(batch.len(), h)?;
    let o = model.expanded_output_weights();
    let inverse = batch.first_layer_inverse();
    gradient[Parameter::FirstLayer].fill(0.0);
    let rows = inverse.starts().len() - 1;
    parallel::map(
        parallel::rows_mut(&mut gradient[Parameter::FirstLayer][..rows * h], h, threads)?,
        |(chunk, values)| {
            for (local, row_values) in values.chunks_mut(h).enumerate() {
                let row = chunk + local;
                for (j, value) in row_values.iter_mut().enumerate() {
                    let mut sum = 0.0_f64;
                    for &word in &inverse.entries()
                        [inverse.starts()[row] as usize..inverse.starts()[row + 1] as usize]
                    {
                        let entry =
                            PackedOccurrence::from_word(word).expect("validated reverse index");
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
                    *value = sum as f32;
                }
            }
            Ok(())
        },
    )?;
    Ok(())
}

/// カーネル6。線形逆索引からc-d_j u_j/Kを正準行へ足す。回転はしない。
pub fn linear_backward(
    model: &Model,
    batch: &BatchSlot,
    removal: &Removal,
    output: &Output,
    gradient: &mut Parameters,
) -> Result<()> {
    linear_backward_parallel(model, batch, removal, output, gradient, 1)
}

fn linear_backward_parallel(
    model: &Model,
    batch: &BatchSlot,
    removal: &Removal,
    output: &Output,
    gradient: &mut Parameters,
    threads: usize,
) -> Result<()> {
    check_batch(model, batch)?;
    check_parameters(gradient, model.shape())?;
    removal.check_shape(batch.len(), model.shape().width())?;
    output.check_shape(batch.len(), model.shape().width())?;
    let inverse = batch.linear_inverse();
    parallel::map(
        parallel::rows_mut(&mut gradient[Parameter::Linear], 2, threads)?,
        |(chunk, values)| {
            for (local, row_values) in values.chunks_mut(2).enumerate() {
                let row = chunk + local;
                for (endpoint, value) in row_values.iter_mut().enumerate() {
                    let mut sum = 0.0_f64;
                    for &word in &inverse.entries()
                        [inverse.starts()[row] as usize..inverse.starts()[row + 1] as usize]
                    {
                        let entry =
                            PackedOccurrence::from_word(word).expect("validated reverse index");
                        let b = entry.position();
                        let col = entry.column() as usize;
                        let d = if col < 144 {
                            removal.d_r[b * 144 + col]
                        } else {
                            0.0
                        };
                        sum += f64::from(output.c[b][endpoint])
                            - f64::from(d)
                                * f64::from(phase(batch.piece_counts()[b], true)[endpoint])
                                / f64::from(model.shape().k());
                    }
                    *value = sum as f32;
                }
            }
            Ok(())
        },
    )?;
    Ok(())
}

/// カーネル7。dV[p]=Σ_k dW[k*13680+p]を8区分の順で縮約する。
pub fn reduce_virtual(gradient: &mut Parameters) -> Result<()> {
    reduce_virtual_parallel(gradient, 1)
}

fn reduce_virtual_parallel(gradient: &mut Parameters, threads: usize) -> Result<()> {
    check_parameters(gradient, gradient.shape())?;
    if gradient.shape().feature_set() == InputFeatureSet::P {
        return Ok(());
    }
    let h = gradient.shape().width();
    let mut virtual_gradient = vec![0.0; gradient.shape().len(Parameter::Virtual)];
    parallel::map(
        parallel::rows_mut(&mut virtual_gradient[..FEATURE_COUNT * h], h, threads)?,
        |(chunk, values)| {
            for (local, value) in values.iter_mut().enumerate() {
                let i = chunk * h + local;
                let mut sum = 0.0_f64;
                for bucket in 0..8 {
                    sum +=
                        f64::from(gradient[Parameter::FirstLayer][bucket * FEATURE_COUNT * h + i]);
                }
                *value = sum as f32;
            }
            Ok(())
        },
    )?;
    gradient[Parameter::Virtual].copy_from_slice(&virtual_gradient);
    Ok(())
}

/// カーネル1〜7を1回実行する。更新はAdamW::updateで別に行う。
pub fn loss_and_gradients(
    model: &Model,
    batch: &BatchSlot,
    targets: &[f32],
    reference: &RemovalReference,
    removal_penalty: f64,
) -> Result<Gradients> {
    loss_and_gradients_parallel(model, batch, targets, reference, removal_penalty, 1)
}

/// 独立な局面と特徴行を分割し、各縮約は単スレッド参照と同じ順で計算する。
/// threadsは正でなければならない。
pub fn loss_and_gradients_parallel(
    model: &Model,
    batch: &BatchSlot,
    targets: &[f32],
    reference: &RemovalReference,
    removal_penalty: f64,
    threads: usize,
) -> Result<Gradients> {
    check_batch(model, batch)?;
    parallel::check_threads(threads)?;
    model.parameters().validate()?;
    let weights = fold(model)?;
    let forward = forward_parallel(model, batch, &weights, threads)?;
    let removal = removal_forward_parallel(
        model,
        batch,
        &weights,
        &forward,
        reference,
        removal_penalty,
        threads,
    )?;
    let output = output_backward_parallel(model, batch, &forward, &removal, targets, threads)?;
    let mut parameters = Parameters::zeros(model.shape());
    let losses = reduce_partials(model, &output, &removal, &mut parameters)?;
    first_layer_backward_parallel(
        model,
        batch,
        &weights,
        &forward,
        &removal,
        &output,
        &mut parameters,
        threads,
    )?;
    linear_backward_parallel(model, batch, &removal, &output, &mut parameters, threads)?;
    reduce_virtual_parallel(&mut parameters, threads)?;
    parameters.validate()?;
    if !losses.total().is_finite() {
        return Err(Error::Invalid("non-finite loss"));
    }
    Ok(Gradients { losses, parameters })
}
