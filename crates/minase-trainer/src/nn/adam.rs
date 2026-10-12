use super::{Error, Result};
use serde::Serialize;

/// AdamWの1群の固定係数。β₁=0.9、β₂=0.999、ε=1e-8。
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct AdamWGroup {
    /// 学習率。
    pub lr: f64,
    /// 分離した重み減衰。
    pub weight_decay: f64,
}
impl AdamWGroup {
    /// 学習率と重み減衰の有限性、符号を検査する。
    pub fn validate(self) -> Result<()> {
        if !self.lr.is_finite()
            || self.lr <= 0.0
            || !self.weight_decay.is_finite()
            || self.weight_decay < 0.0
        {
            return Err(Error::Invalid("AdamW learning rate or decay"));
        }
        Ok(())
    }
}
/// Adam/AdamWの1更新に共通する係数。
pub struct AdamCoefficients {
    lr: f64,
    decay: f32,
    correction1: f64,
    correction2: f64,
}
impl AdamCoefficients {
    /// 1から始まる更新回数からバイアス補正係数を作る。
    pub fn new(step: u64, group: AdamWGroup) -> Result<Self> {
        group.validate()?;
        if step == 0 {
            return Err(Error::Invalid("Adam step must be positive"));
        }
        Ok(Self {
            lr: group.lr,
            decay: (1.0 - group.lr * group.weight_decay) as f32,
            correction1: 1.0 - 0.9_f64.powf(step as f64),
            correction2: 1.0 - 0.999_f64.powf(step as f64),
        })
    }
    /// 有限な重み、勾配、両モーメントを1要素だけ更新する。分散は非負であること。
    /// 更新対象の選択と更新後の値の制約は呼び出し側が担う。
    pub fn update(&self, p: &mut f32, g: f32, m: &mut f32, v: &mut f32) {
        *p *= self.decay;
        // torchのlerp_とmul_/addcmul_の単精度演算に合わせる。
        *m += (g - *m) * 0.1;
        *v = *v * 0.999 + (g * g) * 0.001;
        let m_hat = f64::from(*m) / self.correction1;
        let v_hat = f64::from(*v) / self.correction2;
        *p -= (self.lr * m_hat / (v_hat.sqrt() + 1e-8)) as f32;
    }
}
