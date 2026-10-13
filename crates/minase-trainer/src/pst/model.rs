//! 第1層を持たない、鏡映共有された2端点のPST。

use super::{Error, Result};
use crate::nn::{AdamCoefficients, AdamWGroup};
use minase::eval::features::{FEATURE_COUNT, mirror_feature};

/// 1/8センチポーンへ量子化したi16の下限に対応する−4096センチポーン。
pub(super) const LINEAR_MIN: f32 = -4096.0;
/// 1/8センチポーンへ量子化したi16の上限に対応する4095.875センチポーン。
pub(super) const LINEAR_MAX: f32 = 4095.875;

fn constrain_linear(weights: &mut [f32]) -> Result<()> {
    if weights.iter().any(|v| !v.is_finite()) {
        return Err(Error::Invalid("non-finite PST weight"));
    }
    for value in weights {
        *value = value.clamp(LINEAR_MIN, LINEAR_MAX);
    }
    Ok(())
}

/// 13,680特徴を鏡映共有する6,840行と、常に0の埋め草行6,840の計6,841行。
pub(crate) const CANONICAL_ROWS: usize = FEATURE_COUNT / 2 + 1;

/// `square = base mod 144`、`rank = square/12`、`file = square mod 12`として、`floor(base/144)×72 + rank×6 + min(file,11−file)`へ写す。
pub(crate) fn canonical(base: usize) -> usize {
    let square = base % 144;
    let file = square % 12;
    base / 144 * 72 + square / 12 * 6 + file.min(11 - file)
}

/// センチポーン単位の正準行`[6841, 2]`と出力用K。末尾はゼロの埋め草。
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub(crate) weights: Vec<f32>,
    pub(crate) k: f32,
}

impl Model {
    /// 特徴番号順の実数2端点を受け取り、左右の鏡映対の平均を共有行に置く。
    /// Kは正の有限値、入力は有限かつ[−4096,4095.875]センチポーンを要求する。
    pub fn initialize(initial: &[[f32; 2]], k: f32) -> Result<Self> {
        if !k.is_finite() || k <= 0.0 {
            return Err(Error::Invalid("PST K must be finite and positive"));
        }
        if initial.len() != FEATURE_COUNT
            || initial
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || !(LINEAR_MIN..=LINEAR_MAX).contains(v))
        {
            return Err(Error::Invalid(
                "initial PST must be finite [13680, 2] in -4096..4095.875",
            ));
        }
        Self::from_endpoints(initial, k)
    }

    /// 同時学習用に、有限な2端点を鏡映対で平均し、保存範囲への事前制限は課さない。
    /// 範囲を少し超えるNPZの値でもi16へ丸めると基準と一致し得るため、呼び手が平均の前に量子化して照合する。
    pub(crate) fn from_endpoints(initial: &[[f32; 2]], k: f32) -> Result<Self> {
        if initial.len() != FEATURE_COUNT
            || initial.iter().flatten().any(|v| !v.is_finite())
            || !k.is_finite()
            || k <= 0.0
        {
            return Err(Error::Invalid("PST endpoint shape, finite values or K"));
        }
        let mut weights = vec![0.0; CANONICAL_ROWS * 2];
        for base in (0..FEATURE_COUNT).filter(|base| base % 12 < 6) {
            for endpoint in 0..2 {
                weights[2 * canonical(base) + endpoint] =
                    (initial[base][endpoint] + initial[mirror_feature(base)][endpoint]) / 2.0;
            }
        }
        let model = Self { weights, k };
        model.validate()?;
        Ok(model)
    }

    /// 正準行順の重みを返す。各行は序中盤、終盤の順。
    pub fn weights(&self) -> &[f32] {
        &self.weights
    }

    /// 埋め草以外の重みを編集する。長さは変更できない。
    pub fn weights_mut(&mut self) -> &mut [f32] {
        &mut self.weights[..FEATURE_COUNT]
    }

    /// ロジットへの換算に使う尺度。
    pub fn k(&self) -> f32 {
        self.k
    }

    /// 鏡映対を展開した13,680特徴の2端点を返す。
    pub fn expanded(&self) -> Vec<[f32; 2]> {
        (0..FEATURE_COUNT)
            .map(|base| {
                let i = 2 * canonical(base);
                [self.weights[i], self.weights[i + 1]]
            })
            .collect()
    }

    /// 重みの有限性を検査し、[−4096,4095.875]センチポーンへ切り詰める。
    /// Adamの各更新直後に呼び、非有限値はエラーにする。
    pub fn constrain(&mut self) -> Result<()> {
        constrain_linear(&mut self.weights)?;
        self.validate()
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.weights.len() != CANONICAL_ROWS * 2 || !self.k.is_finite() || self.k <= 0.0 {
            return Err(Error::Invalid("linear model shape or K"));
        }
        if self.weights.iter().any(|v| !v.is_finite()) {
            return Err(Error::Invalid("non-finite PST weight"));
        }
        if self.weights[FEATURE_COUNT..].iter().any(|&v| v != 0.0) {
            return Err(Error::Invalid("linear model padding"));
        }
        Ok(())
    }
}

/// PSTの重みに対応する、重み減衰0のAdamWの両モーメントと更新回数。
/// β₁=0.9、β₂=0.999、ε=10⁻⁸で、式は[`AdamCoefficients::update`]に従う。
/// バッチに現れない行もモーメントで毎回更新する。
#[derive(Clone, Debug, PartialEq)]
pub struct Adam {
    pub(crate) m: Vec<f32>,
    pub(crate) v: Vec<f32>,
    pub(crate) step: u64,
    pub(crate) lr: f64,
}

impl Adam {
    /// 重み減衰0、β₁=0.9、β₂=0.999、ε=10⁻⁸のAdamを作る。
    pub fn new(lr: f64) -> Result<Self> {
        AdamWGroup {
            lr,
            weight_decay: 0.0,
        }
        .validate()?;
        Ok(Self {
            m: vec![0.0; CANONICAL_ROWS * 2],
            v: vec![0.0; CANONICAL_ROWS * 2],
            step: 0,
            lr,
        })
    }
    /// 1次モーメント。
    pub fn m(&self) -> &[f32] {
        &self.m
    }
    /// 2次モーメント。
    pub fn v(&self) -> &[f32] {
        &self.v
    }
    /// 更新回数。
    pub fn step(&self) -> u64 {
        self.step
    }
    /// 固定の学習率。
    pub fn lr(&self) -> f64 {
        self.lr
    }

    pub(crate) fn validate(&self) -> Result<()> {
        AdamWGroup {
            lr: self.lr,
            weight_decay: 0.0,
        }
        .validate()?;
        if self.m.len() != CANONICAL_ROWS * 2
            || self.v.len() != CANONICAL_ROWS * 2
            || self.m.iter().chain(&self.v).any(|v| !v.is_finite())
            || self.v.iter().any(|&v| v < 0.0)
            || self.m[FEATURE_COUNT..]
                .iter()
                .chain(&self.v[FEATURE_COUNT..])
                .any(|&v| v != 0.0)
        {
            return Err(Error::Invalid("linear Adam state"));
        }
        Ok(())
    }

    /// バッチにない行もモーメントで更新し、直後に切り詰める。
    pub fn update(&mut self, model: &mut Model, gradient: &[f32]) -> Result<()> {
        self.update_parallel(model, gradient, 1)
    }

    pub(crate) fn update_parallel(
        &mut self,
        model: &mut Model,
        gradient: &[f32],
        threads: usize,
    ) -> Result<()> {
        crate::nn::parallel::check_threads(threads)?;
        model.validate()?;
        self.validate()?;
        if gradient.len() != self.m.len() || gradient[FEATURE_COUNT..].iter().any(|&v| v != 0.0) {
            return Err(Error::Invalid("linear gradient shape or padding"));
        }
        let step = self
            .step
            .checked_add(1)
            .ok_or(Error::Invalid("Adam step overflow"))?;
        if gradient.iter().any(|value| !value.is_finite()) {
            return Err(Error::Invalid("non-finite PST gradient"));
        }
        let coefficients = AdamCoefficients::new(
            step,
            AdamWGroup {
                lr: self.lr,
                weight_decay: 0.0,
            },
        )?;
        let chunk = gradient.len().div_ceil(threads.min(gradient.len()));
        let pieces: Vec<_> = model
            .weights
            .chunks_mut(chunk)
            .zip(self.m.chunks_mut(chunk))
            .zip(self.v.chunks_mut(chunk))
            .zip(gradient.chunks(chunk))
            .collect();
        crate::nn::parallel::map(pieces, |(((weights, m), v), gradients)| -> Result<()> {
            for (((p, m), v), &g) in weights.iter_mut().zip(m).zip(v).zip(gradients) {
                coefficients.update(p, g, m, v);
            }
            Ok(())
        })?;
        model.constrain()?;
        self.validate()?;
        self.step = step;
        Ok(())
    }
}
