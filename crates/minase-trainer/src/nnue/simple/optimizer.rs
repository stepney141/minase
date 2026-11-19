use super::{Error, Parameter, Parameters, Result};
use crate::nn::AdamWGroup;

/// 第1層と後段のAdamWの係数。バイアスは全て後段群に属する。
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct OptimizerConfig {
    /// 第1層の実の重みと仮想行。
    pub ft: AdamWGroup,
    /// 第1層バイアス、L1、L2、およびL3。
    pub head: AdamWGroup,
}
impl Default for OptimizerConfig {
    fn default() -> Self {
        let group = AdamWGroup {
            lr: 0.001,
            weight_decay: 0.0,
        };
        Self {
            ft: group,
            head: group,
        }
    }
}
impl OptimizerConfig {
    /// 両群の学習率と重み減衰を検査する。
    pub fn validate(self) -> Result<()> {
        self.ft.validate()?;
        self.head.validate()?;
        Ok(())
    }
}

// The GPU receives exactly these rounded step constants. Keep per-element
// arithmetic in f32; only bias-correction powers and scaled rates use f64.
pub(in crate::nnue::simple) struct Coefficients {
    pub correction1: f32,
    pub correction2: f32,
    pub lr: f32,
    pub decay: f32,
}
impl Coefficients {
    pub fn new(step: u64, group: AdamWGroup, multiplier: f64) -> Result<Self> {
        group.validate()?;
        if !multiplier.is_finite() || multiplier <= 0.0 {
            return Err(Error::Invalid("learning rate multiplier"));
        }
        let lr = group.lr * multiplier;
        let decay = 1.0 - lr * group.weight_decay;
        if !(lr as f32).is_finite() || lr as f32 <= 0.0 || !(decay as f32).is_finite() {
            return Err(Error::Invalid(
                "AdamW scaled learning rate or decay overflow",
            ));
        }
        Ok(Self {
            correction1: (1.0 - 0.9_f64.powf(step as f64)) as f32,
            correction2: (1.0 - 0.999_f64.powf(step as f64)) as f32,
            lr: lr as f32,
            decay: decay as f32,
        })
    }
    fn update(&self, value: &mut f32, g: f32, m: &mut f32, v: &mut f32) {
        *value *= self.decay;
        *m += (g - *m) * 0.1;
        *v = *v * 0.999 + (g * g) * 0.001;
        *value -= self.lr * (*m / self.correction1) / ((*v / self.correction2).sqrt() + 1e-8);
    }
}

/// 両モーメントと更新回数を保持するAdamW。β1=0.9、β2=0.999、ε=1e-8。
#[derive(Clone, Debug, PartialEq)]
pub struct AdamW {
    /// 各自由パラメータの1次モーメント。
    pub m: Parameters,
    /// 各自由パラメータの2次モーメント。
    pub v: Parameters,
    /// 完了した更新回数。
    pub step: u64,
    /// 予定による倍率を掛ける前の群の係数。
    pub config: OptimizerConfig,
}
impl AdamW {
    /// 群の係数を検査し、両モーメントを0で初期化する。
    pub fn new(config: OptimizerConfig, width: usize) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            m: Parameters::zeros(width)?,
            v: Parameters::zeros(width)?,
            step: 0,
            config,
        })
    }
    /// 有限性、幅の一致、および非負の分散を検査する。
    pub fn validate(&self) -> Result<()> {
        if self.m.width() != self.v.width() {
            return Err(Error::Invalid("moment width mismatch"));
        }
        self.config.validate()?;
        self.m.validate()?;
        self.v.validate()?;
        for p in Parameter::ALL {
            if self.v[p].iter().any(|v| *v < 0.0) {
                return Err(Error::Invalid("negative AdamW variance"));
            }
        }
        Ok(())
    }
    /// GPUと同じ順の単精度のAdamWで全要素を更新し、所定の配列だけを制限する。
    /// 検査に失敗した場合は重みと最適化器を変更しない。倍率は正の有限数を要求する。
    pub fn update(
        &mut self,
        weights: &mut Parameters,
        gradients: &Parameters,
        multiplier: f64,
    ) -> Result<()> {
        if weights.width() != gradients.width()
            || weights.width() != self.m.width()
            || weights.width() != self.v.width()
        {
            return Err(Error::Invalid("optimizer width mismatch"));
        }
        weights.validate()?;
        gradients.validate()?;
        self.validate()?;
        if !multiplier.is_finite() || multiplier <= 0.0 {
            return Err(Error::Invalid("learning rate multiplier"));
        }
        let step = self
            .step
            .checked_add(1)
            .ok_or(Error::Invalid("AdamW step overflow"))?;
        let ft = Coefficients::new(step, self.config.ft, multiplier)?;
        let head = Coefficients::new(step, self.config.head, multiplier)?;
        // 全要素を先に試算してから書き込む。外部入力やオーバーフローで部分更新を残さない。
        for commit in [false, true] {
            for p in Parameter::ALL {
                let coeff = if matches!(p, Parameter::FtWeights | Parameter::FtVirtual) {
                    &ft
                } else {
                    &head
                };
                for i in 0..weights[p].len() {
                    let (mut value, mut m, mut v) = (weights[p][i], self.m[p][i], self.v[p][i]);
                    coeff.update(&mut value, gradients[p][i], &mut m, &mut v);
                    if !value.is_finite() || !m.is_finite() || !v.is_finite() {
                        return Err(Error::NonFinite(p.name(), i));
                    }
                    if let Some(limit) = p.limit() {
                        value = value.clamp(-limit, limit);
                    }
                    if commit {
                        weights[p][i] = value;
                        self.m[p][i] = m;
                        self.v[p][i] = v;
                    }
                }
            }
        }
        self.step = step;
        Ok(())
    }
}
