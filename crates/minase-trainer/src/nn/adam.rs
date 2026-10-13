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
    /// 有限な重みp、勾配g、両モーメントmとvを1要素更新する。
    /// `p ← p×f32(1−lr×wd)`、`m ← m+(g−m)×0.1`、`v ← 0.999v+0.001g²`をf32で計算する。
    /// `m̂ = m/(1−0.9^t)`、`v̂ = v/(1−0.999^t)`をf64で求め、`p ← p−f32(lr×m̂/(sqrt(v̂)+1e−8))`とする。
    /// tは1から数える更新回数で、各学習器のチェックポイントに保存し、再開後も継続する。
    /// NNUE、PST、FM、同時学習で共用し、対象の選択と更新後の制約は呼び手が担う。
    /// 分散vは非負でなければならない。
    /// 理由は`docs/adr/use-pytorch-adamw-with-bias-correction.md`にある。
    pub fn update(&self, p: &mut f32, g: f32, m: &mut f32, v: &mut f32) {
        *p *= self.decay;
        // モーメントの更新は積と和ごとに単精度で丸める。
        *m += (g - *m) * 0.1;
        *v = *v * 0.999 + (g * g) * 0.001;
        let m_hat = f64::from(*m) / self.correction1;
        let v_hat = f64::from(*v) / self.correction2;
        *p -= (self.lr * m_hat / (v_hat.sqrt() + 1e-8)) as f32;
    }
}
