use crate::model::{Error, Model, Parameter, Parameters, Result, Shape};
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
    pub(crate) fn validate(self) -> Result<()> {
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
/// Python make_optimizerと同じ線形、第1層、出力部の3群。
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct AdamWConfig {
    /// 線形正準行。
    pub linear: AdamWGroup,
    /// 第1層と仮想行。
    pub ft: AdamWGroup,
    /// バイアス、出力重み、出力バイアス。
    pub head: AdamWGroup,
}
impl Default for AdamWConfig {
    fn default() -> Self {
        Self {
            linear: AdamWGroup {
                lr: 0.03,
                weight_decay: 0.0,
            },
            ft: AdamWGroup {
                lr: 1e-3,
                weight_decay: 1e-4,
            },
            head: AdamWGroup {
                lr: 1e-3,
                weight_decay: 1e-4,
            },
        }
    }
}
impl AdamWConfig {
    /// 3群の係数を検査する。
    pub fn validate(self) -> Result<()> {
        self.linear.validate()?;
        self.ft.validate()?;
        self.head.validate()
    }
    fn group(self, p: Parameter) -> AdamWGroup {
        match p {
            Parameter::Linear => self.linear,
            Parameter::FirstLayer | Parameter::Virtual => self.ft,
            _ => self.head,
        }
    }
}

/// 全要素の1次・2次モーメントと更新回数。
#[derive(Clone, Debug, PartialEq)]
pub struct AdamW {
    pub(crate) m: Parameters,
    pub(crate) v: Parameters,
    pub(crate) step: u64,
    pub(crate) config: AdamWConfig,
}
impl AdamW {
    /// モーメント0、更新回数0で作る。
    pub fn new(shape: Shape, config: AdamWConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            m: Parameters::zeros(shape),
            v: Parameters::zeros(shape),
            step: 0,
            config,
        })
    }
    /// 1次モーメント。
    pub fn m(&self) -> &Parameters {
        &self.m
    }
    /// 2次モーメント。
    pub fn v(&self) -> &Parameters {
        &self.v
    }
    /// 完了した更新回数。
    pub fn step(&self) -> u64 {
        self.step
    }
    /// 3群の係数。
    pub fn config(&self) -> AdamWConfig {
        self.config
    }
    /// カーネル8。全要素を更新してから切り詰め、未観測行を復元する。
    ///
    /// バッチに現れなかった行もモーメントと減衰で動く。未観測行についても
    /// PyTorchと同じ状態更新を行い、パラメータだけをfix_unobservedで戻す。
    pub fn update(&mut self, model: &mut Model, gradient: &Parameters) -> Result<()> {
        if model.shape() != self.m.shape() || model.shape() != gradient.shape() {
            return Err(Error::Invalid("AdamW parameter shape"));
        }
        gradient.validate()?;
        model.parameters().validate()?;
        let step = self
            .step
            .checked_add(1)
            .ok_or(Error::Invalid("AdamW step overflow"))?;
        for p in Parameter::ALL {
            adamw(
                model.parameter_mut(p),
                &gradient[p],
                self.m.index_mut_slice(p),
                self.v.index_mut_slice(p),
                step,
                self.config.group(p),
            )?;
        }
        self.step = step;
        model.constrain()
    }
}

// 明示的なスライス借用で、異なる状態の対応を更新式の引数に保つ。
impl Parameters {
    fn index_mut_slice(&mut self, p: Parameter) -> &mut [f32] {
        &mut self[p]
    }
}

/// AdamWの1群を更新する。stepは1から始まり、ゼロ勾配の要素も必ず更新する。
pub fn adamw(
    p: &mut [f32],
    g: &[f32],
    m: &mut [f32],
    v: &mut [f32],
    step: u64,
    group: AdamWGroup,
) -> Result<()> {
    group.validate()?;
    if step == 0 || p.len() != g.len() || p.len() != m.len() || p.len() != v.len() {
        return Err(Error::Invalid("AdamW shape or step"));
    }
    if p.iter()
        .chain(g)
        .chain(m.iter())
        .chain(v.iter())
        .any(|x| !x.is_finite())
        || v.iter().any(|&x| x < 0.0)
    {
        return Err(Error::Invalid(
            "AdamW non-finite values or negative variance",
        ));
    }
    let correction1 = 1.0 - 0.9_f64.powf(step as f64);
    let correction2 = 1.0 - 0.999_f64.powf(step as f64);
    for i in 0..p.len() {
        p[i] *= (1.0 - group.lr * group.weight_decay) as f32;
        // torchのlerp_とmul_/addcmul_の単精度演算に合わせる。
        m[i] += (g[i] - m[i]) * 0.1;
        v[i] = v[i] * 0.999 + (g[i] * g[i]) * 0.001;
        let m_hat = f64::from(m[i]) / correction1;
        let v_hat = f64::from(v[i]) / correction2;
        p[i] -= (group.lr * m_hat / (v_hat.sqrt() + 1e-8)) as f32;
    }
    Ok(())
}
