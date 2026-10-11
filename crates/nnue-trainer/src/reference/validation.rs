use super::{Forward, head, logit};
use crate::{
    data::BatchSlot,
    model::{Error, Model, Result},
};

/// カーネル9で集める統計量。非線形出力はセンチポーン単位。
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
    pub(crate) fn from_sums(sums: &[f64], positions: usize, width: usize) -> Self {
        Self {
            loss_sum: sums[0],
            correct: sums[1] as u64,
            decisive: sums[2] as u64,
            nonlinear_sum: sums[3],
            nonlinear_square_sum: sums[4],
            lower: sums[5] as u64,
            upper: sums[6] as u64,
            positions,
            activations: positions * 2 * width,
        }
    }
    pub(crate) fn sums(&self) -> [f64; 7] {
        [
            self.loss_sum,
            self.correct as f64,
            self.decisive as f64,
            self.nonlinear_sum,
            self.nonlinear_square_sum,
            self.lower as f64,
            self.upper as f64,
        ]
    }
}

/// カーネル9のCPU参照。符号0は勝ち予測に含めず、引き分けを分母から除く。
pub fn validation_forward(
    model: &Model,
    batch: &BatchSlot,
    f: &Forward,
    targets: &[f32],
) -> Result<Validation> {
    let n = batch.len();
    let h = model.shape().width();
    if n == 0
        || model.shape().feature_set() != batch.feature_set()
        || targets.len() != n
        || targets
            .iter()
            .any(|&v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        || f.acc.len() != n * 2 * h
        || f.lin.len() != n
    {
        return Err(Error::Invalid("validation inputs"));
    }
    let mut sums = [0.0; 7];
    for (b, &target) in targets.iter().enumerate() {
        let x = logit(model, batch, f, b);
        sums[0] += f64::from(x.max(0.0) - x * target + (-x.abs()).exp().ln_1p());
        if batch.outcomes()[b] != 1 {
            sums[1] += u8::from((x > 0.0) == (batch.outcomes()[b] == 2)) as f64;
            sums[2] += 1.0;
        }
        let nonlinear = head(model, 0, |j| f.acc[b * 2 * h + j])
            + head(model, 1, |j| f.acc[(b * 2 + 1) * h + j])
            + model.parameters()[crate::model::Parameter::OutputBias][0];
        let cp = f64::from(nonlinear) * f64::from(model.shape().k());
        sums[3] += cp;
        sums[4] += cp * cp;
    }
    sums[5] = f.acc.iter().filter(|&&x| x <= 0.0).count() as f64;
    sums[6] = f.acc.iter().filter(|&&x| x >= 1.0).count() as f64;
    Ok(Validation::from_sums(&sums, n, h))
}
