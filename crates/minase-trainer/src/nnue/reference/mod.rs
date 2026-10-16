//! NNUEの前進、後退、および更新のCPU参照計算。
//! 配列は局面、視点、ユニットの順で行優先とし、Bは局面数、Hは展開後の幅である。
//! Uは共有後の幅で、P型ではH/2、それ以外ではHに等しい。
//! BCEは二値交差エントロピーを表す。
//! 処理番号は1が[`forward`]、2が[`output_backward`]、3が[`reduce_partials`]、4が[`first_layer_backward`]である。
//! 続いて5が[`linear_backward`]、6が[`reduce_virtual`]、7が[`AdamW::update`]、8が[`validation_forward`]である。
//! 前進と出力部はGPUと同じ加算順のf32で計算し、後退の積と縮約はf64で計算する。
//! GPUとの照合は`|GPU−CPU| ≤ 1e-6 + 1e-5×|CPU|`を許容する。
//! 理由は`docs/adr/backpropagate-the-first-layer-through-a-reverse-index.md`にある。

mod optimizer;
mod rows;
use crate::nn::parallel;
use rows::rows_mut;
mod removal;
mod validation;
pub use optimizer::{AdamW, AdamWConfig};
pub(crate) use removal::remove_piece;
pub use removal::{RemovalReference, v0_piece_values};
pub use validation::{Validation, validation_forward};

use crate::nnue::{
    data::{
        BatchSlot, CANONICAL_ROWS, FEATURE_COLUMNS, InputFeatureSet, PackedOccurrence, SENTINEL,
    },
    model::{Error, Model, Parameter, Parameters, Result, Shape, rotated_unit},
};
use minase::eval::features::FEATURE_COUNT;

/// [`forward`]の出力。
#[derive(Clone, Debug)]
pub struct Forward {
    /// 累算器acc`[B, 2, H]`。
    pub acc: Vec<f32>,
    /// 線形端点和lin`[B, 2]`。手番側視点。
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
/// [`output_backward`]の出力。縮約前の寄与を局面順に保持する。
#[derive(Debug)]
pub struct Output {
    /// SCReLUの値f(acc)`[B, 2, H]`。
    pub f: Vec<f32>,
    /// SCReLUの微分h(acc)`[B, 2, H]`。
    pub h: Vec<f32>,
    /// ロジット`[B]`。
    pub logits: Vec<f32>,
    /// 局面ごとの二値交差エントロピー`[B]`。バッチ平均前。
    pub loss: Vec<f32>,
    /// バッチ平均BCEのロジット微分g`[B]`。
    pub g: Vec<f32>,
    /// A_v = g×o_v×h(acc_v)、`[B, 2, H]`。
    pub d_acc: Vec<f32>,
    /// 出力重みへの全幅の寄与`[B, 2, H]`。
    pub d_output: Vec<f32>,
    /// 線形係数c`[B, 2]`。
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
/// [`reduce_partials`]で得たバッチ平均の損失。
#[derive(Clone, Copy, Debug)]
pub struct Losses {
    /// 二値交差エントロピー。
    pub bce: f64,
}
/// 1バッチの参照計算。パラメータは変更しない。
pub struct Gradients {
    /// 二値交差エントロピー。
    pub losses: Losses,
    /// 全パラメータの勾配。
    pub parameters: Parameters,
}

/// SCReLU f(x)=clamp(x,0,1)²。
pub fn screlu(x: f32) -> f32 {
    x.clamp(0.0, 1.0).powi(2)
}
/// SCReLUの微分を、端点を含む0≤x≤1では2x、範囲外では0とする。
pub fn screlu_derivative(x: f32) -> f32 {
    if (0.0..=1.0).contains(&x) {
        2.0 * x
    } else {
        0.0
    }
}
/// 盤上総駒数Nから`φ = min(90, max(0, N−2))/90`、`u = (φ, 1−φ)`をf32で求める。
/// 先獅子の列はNに含めない。CUDAの`phase`も同じ式と精度で計算する。
pub(crate) fn phase(count: u8) -> [f32; 2] {
    let phi = f32::from(count.saturating_sub(2).min(90)) / 90.0;
    [phi, 1.0 - phi]
}

/// HalfKPの合成重み`C = W + V[base]`を作る。
/// P型では回転前の正準行を返し、どちらも埋め草を含む。
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

/// 145列を、段0〜11、筋0〜5の順に左右の`value(左)+value(右)`を先に組んでf32で足す。
/// 最後に先獅子の列144を加えるため、左右鏡映しても加算順が変わらない。
/// GPUの`forward_acc`と`forward_linear`もこの順序を使う。
fn feature_sum(mut value: impl FnMut(usize) -> f32) -> f32 {
    let mut result = 0.0;
    for rank in 0..12 {
        for file in 0..6 {
            result += value(rank * 12 + file) + value(rank * 12 + 11 - file);
        }
    }
    result + value(144)
}

/// 145列の特徴行と線形の端点を、`feature_sum`の順序で足す。
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
    let chunks =
        parallel::map::<_, _, Error, _>(parallel::ranges(batch.len(), threads)?, |range| {
            let mut acc = vec![0.0; range.len() * 2 * h];
            let mut lin = vec![[0.0; 2]; range.len()];
            for (local, b) in range.enumerate() {
                for (endpoint, value) in lin[local].iter_mut().enumerate() {
                    *value = feature_sum(|col| {
                        linear_row(model, batch.linear_features()[b * FEATURE_COLUMNS + col])
                            [endpoint]
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

/// 1視点の出力をj=0〜U−1の順にf32で足す。
/// 各項は`f(acc_j)×o_j`で、P型では重みを掛ける前に`f(acc_(j+U))`も加える。
/// GPUではレーン0が単位順に加算して同じ丸めを再現し、0付近のロジットの符号を保つ。
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
/// 勝率尺度（1ロジットあたりのcp）を`cp_per_logit`とする。
/// ロジットを`(u0×lin0 + u1×lin1)/cp_per_logit + ((head0 + head1) + bias)`で計算する。
/// 出力部の加算順は[`head`]、補間係数は[`phase`]に従う。
pub(crate) fn logit(model: &Model, batch: &BatchSlot, f: &Forward, b: usize) -> f32 {
    let h = model.shape().width();
    let u = phase(batch.piece_counts()[b]);
    let nonlinear = head(model, 0, |j| f.acc[b * 2 * h + j])
        + head(model, 1, |j| f.acc[(b * 2 + 1) * h + j])
        + model.parameters()[Parameter::OutputBias][0];
    (u[0] * f.lin[b][0] + u[1] * f.lin[b][1]) / model.shape().cp_per_logit() + nonlinear
}
/// 浮動小数点の評価をセンチポーンで返す。学習と同じf32の前進を使う。
pub fn evaluate(model: &Model, batch: &BatchSlot) -> Result<Vec<f32>> {
    check_batch(model, batch)?;
    let weights = fold(model)?;
    let f = forward(model, batch, &weights)?;
    Ok((0..batch.len())
        .map(|b| logit(model, batch, &f, b) * model.shape().cp_per_logit())
        .collect())
}

/// 量子化検査用の浮動小数点評価をセンチポーンで返す。
/// 第1層の畳み込みはf32で行い、その重みを用いた線形和、隠れ値、SCReLU、出力内積はf64で計算する。
/// 特徴は列順に足し、両半幅の活性化を個別に重み付けし、非線形出力に勝率尺度を掛ける。
/// 整数推論の128で割る近似は使わない。
pub fn evaluate_f64(model: &Model, batch: &BatchSlot) -> Result<Vec<f64>> {
    let weights = fold(model)?;
    evaluate_f64_with_weights(
        model,
        batch,
        &weights,
        f64::from(model.shape().cp_per_logit()),
    )
}

// 診断では畳み込みを共有し、比較する整数ネットのcp_per_logitを使う。
pub(crate) fn evaluate_f64_with_weights(
    model: &Model,
    batch: &BatchSlot,
    weights: &[f32],
    cp_per_logit: f64,
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
        values.push(cp + cp_per_logit * nonlinear);
    }
    Ok(values)
}

/// 出力と二値交差エントロピー、および後退の局面ごとの寄与を求める。
/// ロジットxと教師値tに対し、`g = (σ(x)−t)/B`と`loss = max(x,0)−x×t+log1p(exp(−|x|))`をf32で計算する。
/// 線形係数は`c = g×u/cp_per_logit`である。
/// 活性化fとその微分hを用い、`d_acc = g×o×h(acc)`と`d_output = g×f(acc)`はf64で計算してf32へ丸める。
/// 教師値が有限かつ`[0,1]`でなければエラーを返す。
pub fn output_backward(
    model: &Model,
    batch: &BatchSlot,
    forward: &Forward,
    targets: &[f32],
) -> Result<Output> {
    output_backward_parallel(model, batch, forward, targets, 1)
}

fn output_backward_parallel(
    model: &Model,
    batch: &BatchSlot,
    forward: &Forward,
    targets: &[f32],
    threads: usize,
) -> Result<Output> {
    check_batch(model, batch)?;
    let h = model.shape().width();
    let n = batch.len();
    forward.check_shape(n, h)?;
    if targets.len() != n
        || targets
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(Error::Invalid("targets"));
    }
    let o = model.expanded_output_weights();
    let chunks = parallel::map::<_, _, Error, _>(parallel::ranges(n, threads)?, |range| {
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
            let u = phase(batch.piece_counts()[b]);
            for (e, &u_e) in u.iter().enumerate() {
                result.c[local][e] = g * u_e / model.shape().cp_per_logit();
            }
            for (vj, &o_vj) in o.iter().enumerate() {
                result.d_acc[local * 2 * h + vj] =
                    (f64::from(g) * f64::from(o_vj) * f64::from(result.h[local * 2 * h + vj]))
                        as f32;
                result.d_output[local * 2 * h + vj] =
                    (f64::from(g) * f64::from(result.f[local * 2 * h + vj])) as f32;
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

/// バイアスと出力重みの勾配を局面方向にf64で足してf32へ丸める。
/// P型では単位jとj+Uの勾配を合算し、出力バイアスの勾配はΣgとする。
/// 損失は`bce = Σloss/B`である。
/// GPUは`gradient_partials`で連続する128局面ごとの部分和を作り、`reduce_partials`でブロック番号順に足す。
pub fn reduce_partials(
    model: &Model,
    output: &Output,
    gradient: &mut Parameters,
) -> Result<Losses> {
    let s = model.shape();
    let n = output.g.len();
    let h = s.width();
    let units = s.units();
    check_parameters(model.parameters(), s)?;
    check_parameters(gradient, s)?;
    output.check_shape(n, h)?;
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
    })
}

/// 第1層の勾配を逆索引の要素順で求める。
/// 要素の局面b、視点vに対する単位jの寄与は`A_v[u]`である。
/// Aは[`Output::d_acc`]、uはP型の回転ビットが立つ場合だけ`(j+H/2) mod H`とする。
/// CPUは行全体をf64で足し、GPUは128要素以下の区分内をf32、区分間を昇順にf64で足す。
/// アトミック加算は使わない。
/// 理由は`docs/adr/backpropagate-the-first-layer-through-a-reverse-index.md`にある。
pub fn first_layer_backward(
    model: &Model,
    batch: &BatchSlot,
    output: &Output,
    gradient: &mut Parameters,
) -> Result<()> {
    first_layer_backward_parallel(model, batch, output, gradient, 1)
}

fn first_layer_backward_parallel(
    model: &Model,
    batch: &BatchSlot,
    output: &Output,
    gradient: &mut Parameters,
    threads: usize,
) -> Result<()> {
    check_batch(model, batch)?;
    check_parameters(gradient, model.shape())?;
    let h = model.shape().width();
    output.check_shape(batch.len(), h)?;
    let inverse = batch.first_layer_inverse();
    gradient[Parameter::FirstLayer].fill(0.0);
    let rows = inverse.starts().len() - 1;
    parallel::map::<_, _, Error, _>(
        rows_mut(&mut gradient[Parameter::FirstLayer][..rows * h], h, threads)?,
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
                        let unit = rotated_unit(entry.rotated(), j, h);
                        let i = (b * 2 + v) * h + unit;
                        sum += f64::from(output.d_acc[i]);
                    }
                    *value = sum as f32;
                }
            }
            Ok(())
        },
    )?;
    Ok(())
}

/// 手番側の線形逆索引を走査し、`dL[row][e] = Σc[b][e]`を正準行へ足す。
/// 半幅回転はせず、CPUはf64で累算する。
/// GPUは1ワープが1区分を処理し、レーンtが要素t, t+32, …をf32で足して距離16,8,4,2,1の木で縮約する。
/// 区分間は昇順にf64で足す。
pub fn linear_backward(
    model: &Model,
    batch: &BatchSlot,
    output: &Output,
    gradient: &mut Parameters,
) -> Result<()> {
    linear_backward_parallel(model, batch, output, gradient, 1)
}

fn linear_backward_parallel(
    model: &Model,
    batch: &BatchSlot,
    output: &Output,
    gradient: &mut Parameters,
    threads: usize,
) -> Result<()> {
    check_batch(model, batch)?;
    check_parameters(gradient, model.shape())?;
    output.check_shape(batch.len(), model.shape().width())?;
    let inverse = batch.linear_inverse();
    parallel::map::<_, _, Error, _>(
        rows_mut(&mut gradient[Parameter::Linear], 2, threads)?,
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
                        sum += f64::from(output.c[b][endpoint]);
                    }
                    *value = sum as f32;
                }
            }
            Ok(())
        },
    )?;
    Ok(())
}

/// HalfKPの仮想行の勾配`dV[p] = Σ_(k=0..7) dW[k×13,680+p]`を区分番号順にf64で足す。
/// 埋め草行13,680には0を書き、P型では何もしない。
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
    parallel::map::<_, _, Error, _>(
        rows_mut(&mut virtual_gradient[..FEATURE_COUNT * h], h, threads)?,
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

/// 前進から仮想行の勾配縮約までを1回実行する。
/// 更新は[`AdamW::update`]で別に行う。
pub fn loss_and_gradients(model: &Model, batch: &BatchSlot, targets: &[f32]) -> Result<Gradients> {
    loss_and_gradients_parallel(model, batch, targets, 1)
}

/// 局面方向の前進、出力部と、行方向の第1層、線形、仮想行を[`parallel::ranges`]で分割する。
/// 各要素の縮約順は単一スレッドと同じなので、正の`threads`の値によらずビット単位で同じ結果になる。
/// パラメータ、勾配、および損失の有限性を検査する。
pub fn loss_and_gradients_parallel(
    model: &Model,
    batch: &BatchSlot,
    targets: &[f32],
    threads: usize,
) -> Result<Gradients> {
    check_batch(model, batch)?;
    parallel::check_threads(threads)?;
    model.parameters().validate()?;
    let weights = fold(model)?;
    let forward = forward_parallel(model, batch, &weights, threads)?;
    let output = output_backward_parallel(model, batch, &forward, targets, threads)?;
    let mut parameters = Parameters::zeros(model.shape());
    let losses = reduce_partials(model, &output, &mut parameters)?;
    first_layer_backward_parallel(model, batch, &output, &mut parameters, threads)?;
    linear_backward_parallel(model, batch, &output, &mut parameters, threads)?;
    reduce_virtual_parallel(&mut parameters, threads)?;
    parameters.validate()?;
    if !losses.bce.is_finite() {
        return Err(Error::Invalid("non-finite loss"));
    }
    Ok(Gradients { losses, parameters })
}
