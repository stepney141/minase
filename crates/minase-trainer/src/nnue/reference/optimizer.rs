use super::rows::rows_mut;
use crate::nn::parallel;
use crate::nn::{AdamCoefficients, AdamWGroup};
use crate::nnue::{
    data::InputFeatureSet,
    model::{Error, Model, Parameter, Parameters, Result, Shape},
};
use minase::eval::features::FEATURE_COUNT;
use serde::Serialize;

/// 線形、第1層、出力部の3群の係数。
/// 既定値は線形が学習率0.03と重み減衰0、第1層と出力部が学習率10⁻³と重み減衰10⁻⁴である。
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
        Ok(self.head.validate()?)
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
    /// Linear、Virtual、FirstLayer、Biases、OutputWeights、OutputBiasの順に観測行を更新する。
    /// 更新式は[`AdamCoefficients::update`]に従い、勾配0の観測行も更新する。
    /// 線形は[−4096, 4095.875]、出力重みは±1.98、その他は±8に切り詰める。
    /// HalfKPの第1層だけは今回更新した仮想行Vを使い、[−8−V, 8−V]に制限する。
    /// 未観測行は更新も切り詰めもせず、パラメータと両モーメントを保持する。
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
            let coefficients = AdamCoefficients::new(step, self.config.group(p))?;
            // 第1層の切り詰めは、先に更新した仮想行を読みながら行う。
            let virtual_rows = if p == Parameter::FirstLayer {
                model.parameters()[Parameter::Virtual].to_vec()
            } else {
                Vec::new()
            };
            parallel::map::<_, _, Error, _>(
                rows_mut(model.parameter_mut(p), 1, threads)?
                    .into_iter()
                    .zip(rows_mut(&mut self.m[p], 1, threads)?)
                    .zip(rows_mut(&mut self.v[p], 1, threads)?),
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
