use super::{Forward, check_batch, head, logit, phase, screlu};
use crate::nnue::{
    data::BatchSlot,
    model::{Error, Model, Result},
};

/// [`validation_forward`]で集める統計量。
/// 非線形出力はセンチポーン単位である。
#[derive(Clone, Debug, PartialEq)]
pub struct Validation {
    /// 局面ごとのBCEの和。
    pub loss_sum: f64,
    /// 引き分けを除き、勝敗とロジットの正負が一致した局面数。
    pub correct: u64,
    /// 引き分けでない局面数。
    pub decisive: u64,
    /// 非線形出力の和。
    pub nonlinear_sum: f64,
    /// 非線形出力の二乗の和。
    pub nonlinear_square_sum: f64,
    /// acc ≤ 0の要素数。
    pub lower: u64,
    /// acc ≥ 1の要素数。
    pub upper: u64,
    /// 局面数。
    pub positions: usize,
    /// 累算器の要素数B×2×H。
    pub activations: usize,
    /// 隠れ層h ≤ 0の要素数。
    pub hidden_lower: u64,
    /// 隠れ層h ≥ 1の要素数。
    pub hidden_upper: u64,
    /// 隠れ層の要素数B×16。隠れ層がなければ0。
    pub hidden_activations: usize,
}
impl Validation {
    /// バッチ平均のBCE。
    pub fn loss(&self) -> f64 {
        self.loss_sum / self.positions as f64
    }
    /// 符号一致率。全局面が引き分けならNone。
    pub fn sign_accuracy(&self) -> Option<f64> {
        (self.decisive != 0).then(|| self.correct as f64 / self.decisive as f64)
    }
    /// 非線形出力の母標準偏差、センチポーン。
    pub fn nonlinear_std_cp(&self) -> f64 {
        let mean = self.nonlinear_sum / self.positions as f64;
        (self.nonlinear_square_sum / self.positions as f64 - mean * mean)
            .max(0.0)
            .sqrt()
    }
    /// acc ≤ 0とacc ≥ 1の割合。
    pub fn saturation(&self) -> [f64; 2] {
        [
            self.lower as f64 / self.activations as f64,
            self.upper as f64 / self.activations as f64,
        ]
    }
    /// 隠れ層の飽和率。隠れ層がなければNone。
    pub fn hidden_saturation(&self) -> Option<[f64; 2]> {
        (self.hidden_activations != 0).then(|| {
            [
                self.hidden_lower as f64 / self.hidden_activations as f64,
                self.hidden_upper as f64 / self.hidden_activations as f64,
            ]
        })
    }
    /// sumsの列は順にBCE和、一致数、勝敗局面数、非線形cp和、二乗和、
    /// acc≤0数、acc≥1数、h≤0数、h≥1数、h要素数。CPUとGPUで共通。
    pub(crate) fn from_sums(sums: &[f64; 10], positions: usize, width: usize) -> Self {
        Self {
            loss_sum: sums[0],
            correct: sums[1] as u64,
            decisive: sums[2] as u64,
            nonlinear_sum: sums[3],
            nonlinear_square_sum: sums[4],
            lower: sums[5] as u64,
            upper: sums[6] as u64,
            hidden_lower: sums[7] as u64,
            hidden_upper: sums[8] as u64,
            hidden_activations: sums[9] as usize,
            positions,
            activations: positions * 2 * width,
        }
    }
    pub(crate) fn sums(&self) -> [f64; 10] {
        [
            self.loss_sum,
            self.correct as f64,
            self.decisive as f64,
            self.nonlinear_sum,
            self.nonlinear_square_sum,
            self.lower as f64,
            self.upper as f64,
            self.hidden_lower as f64,
            self.hidden_upper as f64,
            self.hidden_activations as f64,
        ]
    }
}

/// 検証用の二値交差エントロピー、勝敗一致数、非線形出力と飽和数を集める。
/// 符号0は勝ち予測に含めず、引き分けを一致率の分母から除く。
pub fn validation_forward(
    model: &Model,
    batch: &BatchSlot,
    f: &Forward,
    targets: &[f32],
) -> Result<Validation> {
    check_batch(model, batch)?;
    let n = batch.len();
    let h = model.shape().width();
    f.check_shape(n, h)?;
    if targets.len() != n
        || targets
            .iter()
            .any(|&v| !v.is_finite() || !(0.0..=1.0).contains(&v))
    {
        return Err(Error::Invalid("validation inputs"));
    }
    let mut sums = [0.0; 10];
    for (b, &target) in targets.iter().enumerate() {
        let x = logit(model, batch, f, b);
        sums[0] += f64::from(x.max(0.0) - x * target + (-x.abs()).exp().ln_1p());
        if batch.outcomes()[b] != 1 {
            sums[1] += u8::from((x > 0.0) == (batch.outcomes()[b] == 2)) as f64;
            sums[2] += 1.0;
        }
        let activations: Vec<_> = f.acc[b * 2 * h..(b + 1) * 2 * h]
            .iter()
            .map(|&x| screlu(x))
            .collect();
        let output = head(model, &activations, phase(batch.piece_counts()[b])[0]);
        sums[7] += output.h.iter().filter(|&&x| x <= 0.0).count() as f64;
        sums[8] += output.h.iter().filter(|&&x| x >= 1.0).count() as f64;
        sums[9] += output.h.len() as f64;
        let cp = f64::from(output.value) * f64::from(model.shape().cp_per_logit());
        sums[3] += cp;
        sums[4] += cp * cp;
    }
    sums[5] = f.acc.iter().filter(|&&x| x <= 0.0).count() as f64;
    sums[6] = f.acc.iter().filter(|&&x| x >= 1.0).count() as f64;
    Ok(Validation::from_sums(&sums, n, h))
}
