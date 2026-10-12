//! ランク32のFMと、全要素を更新する1群のAdamW。

use super::{Result, batch::mix64};
use crate::nn::{AdamCoefficients, AdamWGroup, parallel};
use minase::eval::{features::FEATURE_COUNT, pst::FM_RANK};

pub(super) const V_LEN: usize = (FEATURE_COUNT + 1) * FM_RANK;
pub(super) const PARAMETERS: usize = V_LEN + FM_RANK;

/// 全特徴の埋め込みV、埋め草、出力係数aと観測マスク。
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub(super) weights: Vec<f32>,
    pub(super) mask: Vec<bool>,
    pub(super) k: f32,
}
impl Model {
    /// 観測行だけをN(0,1/55)で初期化し、aと未観測行を0にする。
    pub fn initialize(mask: Vec<bool>, k: f32, seed: u64) -> Result<Self> {
        if mask.len() != FEATURE_COUNT {
            return Err("FM observation mask shape".into());
        }
        let mut model = Self {
            weights: vec![0.0; PARAMETERS],
            mask,
            k,
        };
        let mut state = seed;
        let mut uniform = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            ((mix64(state) >> 11) as f64 + 0.5) / ((1_u64 << 53) as f64)
        };
        for (row, &observed) in model.weights[..FEATURE_COUNT * FM_RANK]
            .as_chunks_mut::<FM_RANK>()
            .0
            .iter_mut()
            .zip(&model.mask)
        {
            if !observed {
                continue;
            }
            for pair in row.as_chunks_mut::<2>().0 {
                let radius = (-2.0 * uniform().ln() / 55.0).sqrt();
                let angle = std::f64::consts::TAU * uniform();
                pair[0] = (radius * angle.cos()) as f32;
                pair[1] = (radius * angle.sin()) as f32;
            }
        }
        model.validate()?;
        Ok(model)
    }

    /// 埋め草を除くVとaを読み、有限性と未観測行の0を検査する。
    pub fn from_weights(
        v: &[[f32; FM_RANK]],
        a: [f32; FM_RANK],
        mask: Vec<bool>,
        k: f32,
    ) -> Result<Self> {
        if v.len() != FEATURE_COUNT {
            return Err("FM V shape".into());
        }
        let mut weights: Vec<f32> = v.iter().flatten().copied().collect();
        weights.extend([0.0; FM_RANK]);
        weights.extend(a);
        let model = Self { weights, mask, k };
        model.validate()?;
        Ok(model)
    }
    /// 埋め草を除く[13680,32]のV。
    pub fn v(&self) -> &[[f32; FM_RANK]] {
        self.weights[..FEATURE_COUNT * FM_RANK]
            .as_chunks::<FM_RANK>()
            .0
    }
    /// ランク32の出力係数。
    pub fn a(&self) -> &[f32; FM_RANK] {
        self.weights[V_LEN..].try_into().expect("FM rank")
    }
    /// 全特徴の観測マスク。埋め草を含まない。
    pub fn mask(&self) -> &[bool] {
        &self.mask
    }
    /// 固定整数PSTの勝率尺度。
    pub fn k(&self) -> f32 {
        self.k
    }
    pub(super) fn fix_unobserved(&mut self) {
        for (i, row) in self.weights[..V_LEN]
            .as_chunks_mut::<FM_RANK>()
            .0
            .iter_mut()
            .enumerate()
        {
            if i == FEATURE_COUNT || !self.mask[i] {
                row.fill(0.0);
            }
        }
    }
    pub(super) fn validate(&self) -> Result<()> {
        if self.weights.len() != PARAMETERS
            || self.mask.len() != FEATURE_COUNT
            || !self.k.is_finite()
            || self.k <= 0.0
            || self.weights.iter().any(|v| !v.is_finite())
        {
            return Err("invalid FM model shape or non-finite weights/K".into());
        }
        for (i, row) in self.weights[..V_LEN]
            .as_chunks::<FM_RANK>()
            .0
            .iter()
            .enumerate()
        {
            if (i == FEATURE_COUNT || !self.mask[i]) && row.iter().any(|&v| v != 0.0) {
                return Err("FM unobserved and padding rows must be zero".into());
            }
        }
        Ok(())
    }
}

/// Vとaを1群で扱うAdamW。埋め草の状態も保持する。
#[derive(Clone, Debug, PartialEq)]
pub struct Adam {
    pub(super) m: Vec<f32>,
    pub(super) v: Vec<f32>,
    pub(super) step: u64,
    pub(super) group: AdamWGroup,
}
impl Adam {
    /// PyTorchと同じ固定β、εと、指定された学習率・重み減衰を使う。
    pub fn new(lr: f64, weight_decay: f64) -> Result<Self> {
        let group = AdamWGroup { lr, weight_decay };
        group.validate()?;
        Ok(Self {
            m: vec![0.0; PARAMETERS],
            v: vec![0.0; PARAMETERS],
            step: 0,
            group,
        })
    }
    /// バッチに現れない行も毎回更新する。各要素の演算はスレッド数によらない。
    pub fn update(&mut self, model: &mut Model, gradients: &[f32], threads: usize) -> Result<()> {
        model.validate()?;
        self.validate()?;
        parallel::check_threads(threads)?;
        if gradients.len() != PARAMETERS || gradients.iter().any(|g| !g.is_finite()) {
            return Err("invalid FM gradient shape or non-finite gradient".into());
        }
        let step = self.step.checked_add(1).ok_or("FM Adam step overflow")?;
        let coefficients = AdamCoefficients::new(step, self.group)?;
        let chunk = PARAMETERS.div_ceil(threads.min(PARAMETERS));
        let pieces: Vec<_> = model
            .weights
            .chunks_mut(chunk)
            .zip(self.m.chunks_mut(chunk))
            .zip(self.v.chunks_mut(chunk))
            .zip(gradients.chunks(chunk))
            .collect();
        parallel::map(pieces, |(((weights, m), v), gradients)| -> Result<()> {
            for (((p, m), v), &g) in weights.iter_mut().zip(m).zip(v).zip(gradients) {
                coefficients.update(p, g, m, v);
            }
            Ok(())
        })?;
        self.step = step;
        model.fix_unobserved();
        model.validate()?;
        self.validate()
    }
    /// 更新回数。
    pub fn step(&self) -> u64 {
        self.step
    }
    /// V、埋め草、aの順の1次モーメント。
    pub fn m(&self) -> &[f32] {
        &self.m
    }
    /// V、埋め草、aの順の2次モーメント。
    pub fn v(&self) -> &[f32] {
        &self.v
    }
    pub(super) fn validate(&self) -> Result<()> {
        self.group.validate()?;
        if self.m.len() != PARAMETERS
            || self.v.len() != PARAMETERS
            || self.m.iter().any(|v| !v.is_finite())
            || self.v.iter().any(|v| !v.is_finite() || *v < 0.0)
        {
            return Err("invalid FM Adam state".into());
        }
        Ok(())
    }
}
