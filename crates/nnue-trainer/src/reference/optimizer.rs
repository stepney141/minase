use super::parallel;
use crate::{
    data::InputFeatureSet,
    model::{Error, Model, Parameter, Parameters, Result, Shape},
};
use minase::eval::features::FEATURE_COUNT;
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
    /// カーネル8。観測行の全要素を更新して切り詰める。
    ///
    /// バッチに現れなかった観測行もモーメントと減衰で動く。未観測行は
    /// パラメータと両モーメントを保ち、切り詰めも行わない。
    pub fn update(&mut self, model: &mut Model, gradient: &Parameters) -> Result<()> {
        self.update_parallel(model, gradient, 1)
    }

    /// 各群の独立した要素を標準ライブラリのスレッドへ分けて更新する。
    /// threadsは正でなければならず、更新順は単スレッド参照と同じ。
    pub fn update_parallel(
        &mut self,
        model: &mut Model,
        gradient: &Parameters,
        threads: usize,
    ) -> Result<()> {
        parallel::check_threads(threads)?;
        if model.shape() != self.m.shape()
            || model.shape() != self.v.shape()
            || model.shape() != gradient.shape()
        {
            return Err(Error::Invalid("AdamW parameter shape"));
        }
        self.config.validate()?;
        gradient.validate()?;
        model.parameters().validate()?;
        self.m.validate()?;
        self.v.validate()?;
        if Parameter::ALL
            .iter()
            .any(|&p| self.v[p].iter().any(|&v| v < 0.0))
        {
            return Err(Error::Invalid("negative AdamW variance"));
        }
        let step = self
            .step
            .checked_add(1)
            .ok_or(Error::Invalid("AdamW step overflow"))?;
        let shape = model.shape();
        let masks = model.masks().clone();
        // 特徴行の範囲は、今回更新して切り詰めた仮想行から求める。
        for p in [
            Parameter::Linear,
            Parameter::Virtual,
            Parameter::FirstLayer,
            Parameter::Biases,
            Parameter::OutputWeights,
            Parameter::OutputBias,
        ] {
            let coefficients = Coefficients::new(step, self.config.group(p));
            // 第1層の切り詰めは、先に更新した仮想行を読みながら行う。
            let virtual_rows = if p == Parameter::FirstLayer {
                model.parameters()[Parameter::Virtual].to_vec()
            } else {
                Vec::new()
            };
            parallel::map(
                parallel::rows_mut(model.parameter_mut(p), 1, threads)?
                    .into_iter()
                    .zip(parallel::rows_mut(&mut self.m[p], 1, threads)?)
                    .zip(parallel::rows_mut(&mut self.v[p], 1, threads)?),
                |(((start, values), (_, first_moments)), (_, second_moments))| {
                    for (local, ((value, m), v)) in values
                        .iter_mut()
                        .zip(first_moments)
                        .zip(second_moments)
                        .enumerate()
                    {
                        let i = start + local;
                        let observed = match p {
                            Parameter::Linear => masks.canonical()[i / 2],
                            Parameter::FirstLayer => masks.first_layer()[i / shape.width()],
                            Parameter::Virtual => masks.virtual_rows()[i / shape.width()],
                            _ => true,
                        };
                        if !observed {
                            continue;
                        }
                        let (lower, upper) = match p {
                            Parameter::Linear => (-4096.0, 4095.875),
                            Parameter::OutputWeights => (-1.98, 1.98),
                            Parameter::FirstLayer
                                if shape.feature_set() == InputFeatureSet::HalfKpHmB8 =>
                            {
                                let base = i / shape.width() % FEATURE_COUNT;
                                let v = virtual_rows[base * shape.width() + i % shape.width()];
                                (-8.0 - v, 8.0 - v)
                            }
                            _ => (-8.0, 8.0),
                        };
                        coefficients.update(value, gradient[p][i], m, v);
                        *value = value.clamp(lower, upper);
                    }
                    Ok(())
                },
            )?;
        }
        self.step = step;
        model.parameters().validate()
    }
}

struct Coefficients {
    lr: f64,
    decay: f32,
    correction1: f64,
    correction2: f64,
}
impl Coefficients {
    fn new(step: u64, group: AdamWGroup) -> Self {
        Self {
            lr: group.lr,
            decay: (1.0 - group.lr * group.weight_decay) as f32,
            correction1: 1.0 - 0.9_f64.powf(step as f64),
            correction2: 1.0 - 0.999_f64.powf(step as f64),
        }
    }
    fn update(&self, p: &mut f32, g: f32, m: &mut f32, v: &mut f32) {
        *p *= self.decay;
        // torchのlerp_とmul_/addcmul_の単精度演算に合わせる。
        *m += (g - *m) * 0.1;
        *v = *v * 0.999 + (g * g) * 0.001;
        let m_hat = f64::from(*m) / self.correction1;
        let v_hat = f64::from(*v) / self.correction2;
        *p -= (self.lr * m_hat / (v_hat.sqrt() + 1e-8)) as f32;
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
    let coefficients = Coefficients::new(step, group);
    for i in 0..p.len() {
        coefficients.update(&mut p[i], g[i], &mut m[i], &mut v[i]);
    }
    Ok(())
}
