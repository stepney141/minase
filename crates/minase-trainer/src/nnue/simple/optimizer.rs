use super::{Error, Parameter, Parameters, Result};
use crate::nn::{AdamCoefficients, AdamWGroup};

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
    /// 既存のAdamCoefficientsの算術で更新し、所定の配列だけを制限する。
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
        let coefficients = |g: AdamWGroup| -> Result<AdamCoefficients> {
            let g = AdamWGroup {
                lr: g.lr * multiplier,
                weight_decay: g.weight_decay,
            };
            if !(1.0 - g.lr * g.weight_decay).is_finite()
                || (1.0 - g.lr * g.weight_decay).abs() > f64::from(f32::MAX)
            {
                return Err(Error::Invalid("AdamW decay overflow"));
            }
            Ok(AdamCoefficients::new(step, g)?)
        };
        let ft = coefficients(self.config.ft)?;
        let head = coefficients(self.config.head)?;
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
