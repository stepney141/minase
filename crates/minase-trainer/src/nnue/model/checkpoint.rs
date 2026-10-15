//! NNUE学習器の無圧縮NPZチェックポイント。
//! `latest.npz`と`best.npz`は次の配列を持ち、余分・欠落・型や形状の違いを拒否する。
//! Hは幅、Rは第1層の行数（P型6841、HalfKP型109441）、Vは仮想行数（P型0、HalfKP型13681）、
//! Uは共有後の出力幅（P型H/2、HalfKP型H）である。
//!
//! | 名前 | dtype | 形状 | 内容 |
//! | --- | --- | --- | --- |
//! | `weights.linear.weight`, `m.linear.weight`, `v.linear.weight` | `<f4` | `[6841, 2]` | 線形重みと両モーメント。 |
//! | `weights.ft.embedding.weight`, `m.ft.embedding.weight`, `v.ft.embedding.weight` | `<f4` | `[R, H]` | 第1層と両モーメント。 |
//! | `weights.ft.virtual.weight`, `m.ft.virtual.weight`, `v.ft.virtual.weight` | `<f4` | `[V, H]` | 仮想行と両モーメント。 |
//! | `weights.head.biases`, `m.head.biases`, `v.head.biases` | `<f4` | `[U]` | バイアスと両モーメント。 |
//! | `weights.head.output_weights`, `m.head.output_weights`, `v.head.output_weights` | `<f4` | `[2, U]` | 出力重みと両モーメント。 |
//! | `weights.head.output_bias`, `m.head.output_bias`, `v.head.output_bias` | `<f4` | `[1]` | 出力バイアスと両モーメント。 |
//! | `canonical_mask`, `first_layer_mask`, `virtual_mask` | `\|b1` | `[6841]`, `[R]`, `[V]` | 観測マスク。 |
//! | `initial_linear`, `initial_virtual` | `<f4` | `[6841, 2]`, `[V, H]` | 未観測行を固定する初期値。 |
//! | `cp_per_logit` | `<f4` | `[]` | 勝率尺度（1ロジットあたりのcp）。 |
//! | `width`, `feature_set`, `step` | `<u8` | `[]` | 幅、特徴集合（P型0、HalfKP型1）、更新数。 |
//! | `optimizer_groups` | `<f8` | `[3, 2]` | 線形、第1層、出力部の順の学習率と重み減衰。 |
//! | `removal_penalty`, `lambda` | `<f8` | `[]` | 駒除去損失係数と教師混合比。 |
//! | `meta` | `\|u1` | `[JSONのバイト長]` | UTF-8のJSON。 |
//!
//! 配列名の`m`は1次モーメント、`v`は2次モーメントを表す。
//! `meta`は`kind = "nnue"`、`epoch`、`best`、`elapsed_seconds`、`seed`、`config`を持つ。
//! `best`は`{epoch, validation_loss}`で更新前はnull。時間は秒である。
//! `config`は`config.json`の`settings`、`inputs`、`preprocessing`をそのまま保持する。
//! GPUの状態は学習バックエンドのスナップショットでホストへ集めてから保存する。

use super::{Error, Masks, Model, Parameter, Parameters, Result, Shape};
pub use crate::nn::checkpoint::Progress;
use crate::nn::{
    AdamWGroup,
    checkpoint::{Configuration, Reader, Writer},
};
use crate::nnue::{
    data::InputFeatureSet,
    reference::{AdamW, AdamWConfig},
};
use std::path::Path;

/// パラメータ、固定行の初期値とマスク、AdamWの状態、進捗を保存する。
/// 配列の配置はこのモジュールの表に定める。
/// 復号時の受理条件は[`Self::decode`]に従う。
#[derive(Clone, Debug, PartialEq)]
pub struct Checkpoint {
    /// f32モデル。
    pub model: Model,
    /// AdamWの全状態。
    pub optimizer: AdamW,
    /// 駒除去差分の補助損失係数。
    pub removal_penalty: f64,
    /// 自己対局の教師混合比。
    pub lambda: f64,
    /// 学習の進捗と再開条件。
    pub progress: Progress,
}
impl Checkpoint {
    fn validate(&self) -> Result<()> {
        if !self.removal_penalty.is_finite()
            || self.removal_penalty < 0.0
            || !self.lambda.is_finite()
            || !(0.0..=1.0).contains(&self.lambda)
        {
            return Err(Error::Invalid("checkpoint loss coefficients"));
        }
        if self.model.shape() != self.optimizer.m.shape()
            || self.model.shape() != self.optimizer.v.shape()
        {
            return Err(Error::Invalid("checkpoint state shape"));
        }
        self.model.parameters.validate()?;
        self.optimizer.m.validate()?;
        self.optimizer.v.validate()?;
        self.optimizer.config.validate()?;
        for p in Parameter::ALL {
            if self.optimizer.v[p].iter().any(|&v| v < 0.0) {
                return Err(Error::Invalid("negative AdamW variance"));
            }
        }
        self.progress.validate()?;
        let model = &self.model;
        if model
            .initial_linear
            .iter()
            .any(|v| !v.is_finite() || !(-4096.0..=4095.875).contains(v))
            || model
                .initial_virtual
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 8.0)
        {
            return Err(Error::Invalid("fixed initial values"));
        }
        if model.initial_linear[model.initial_linear.len() - 2..]
            .iter()
            .any(|&v| v != 0.0)
            || (!model.initial_virtual.is_empty()
                && model.initial_virtual[model.initial_virtual.len() - model.shape().width()..]
                    .iter()
                    .any(|&v| v != 0.0))
        {
            return Err(Error::Invalid("initial padding"));
        }
        let mut fixed = model.clone();
        fixed.fix_unobserved();
        if fixed != *model {
            return Err(Error::Invalid("unobserved checkpoint parameters"));
        }
        Ok(())
    }
    /// 全状態を型と浮動小数点のビット列を保ってNPZへ保存する。
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let shape = self.model.shape();
        let mut w = Writer::new("nnue", &self.progress)?;
        w.scalar("width", shape.width() as u64);
        w.scalar(
            "feature_set",
            match shape.feature_set() {
                InputFeatureSet::P => 0_u64,
                InputFeatureSet::HalfKpHmB8 => 1,
            },
        );
        w.scalar("cp_per_logit", shape.cp_per_logit());
        w.scalar("step", self.optimizer.step);
        w.scalar("removal_penalty", self.removal_penalty);
        w.scalar("lambda", self.lambda);
        let groups = [
            self.optimizer.config.linear,
            self.optimizer.config.ft,
            self.optimizer.config.head,
        ];
        w.values(
            "optimizer_groups",
            &[3, 2],
            &groups
                .iter()
                .flat_map(|g| [g.lr, g.weight_decay])
                .collect::<Vec<_>>(),
        );
        for (prefix, params) in [
            ("weights", &self.model.parameters),
            ("m", &self.optimizer.m),
            ("v", &self.optimizer.v),
        ] {
            for p in Parameter::ALL {
                w.values(
                    &format!("{prefix}.{}", p.name()),
                    &dimensions(shape, p),
                    &params[p],
                );
            }
        }
        w.mask("canonical_mask", &self.model.masks.canonical);
        w.mask("first_layer_mask", &self.model.masks.first_layer);
        w.mask("virtual_mask", &self.model.masks.virtual_rows);
        w.values(
            "initial_linear",
            &dimensions(shape, Parameter::Linear),
            &self.model.initial_linear,
        );
        w.values(
            "initial_virtual",
            &dimensions(shape, Parameter::Virtual),
            &self.model.initial_virtual,
        );
        Ok(w.encode()?)
    }

    /// 種別、配列の集合・型・形状、有限性、マスク、初期値と進捗を検査する。
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let (mut r, progress) = Reader::new(bytes, "nnue")?;
        let width = usize::try_from(r.scalar::<u64>("width")?)
            .map_err(|_| Error::Format("checkpoint width"))?;
        let set = match r.scalar::<u64>("feature_set")? {
            0 => InputFeatureSet::P,
            1 => InputFeatureSet::HalfKpHmB8,
            _ => return Err(Error::Format("checkpoint feature set")),
        };
        let shape = Shape::new(set, width, r.scalar("cp_per_logit")?)?;
        let step = r.scalar("step")?;
        let removal_penalty = r.scalar("removal_penalty")?;
        let lambda = r.scalar("lambda")?;
        let groups = r.values::<f64>("optimizer_groups", &[3, 2])?;
        let group = |i: usize| AdamWGroup {
            lr: groups[2 * i],
            weight_decay: groups[2 * i + 1],
        };
        let config = AdamWConfig {
            linear: group(0),
            ft: group(1),
            head: group(2),
        };
        let mut params = |prefix| -> Result<Parameters> {
            // 全配列の長さを確認してからパラメータを確保する。
            let values = Parameter::ALL
                .map(|p| r.values::<f32>(&format!("{prefix}.{}", p.name()), &dimensions(shape, p)))
                .into_iter()
                .collect::<crate::nn::Result<Vec<_>>>()?;
            let mut params = Parameters::zeros(shape);
            for (p, values) in Parameter::ALL.into_iter().zip(values) {
                params[p].copy_from_slice(&values);
            }
            Ok(params)
        };
        let parameters = params("weights")?;
        let m = params("m")?;
        let v = params("v")?;
        let canonical = r.mask("canonical_mask", super::CANONICAL_ROWS)?;
        let first_layer = r.mask("first_layer_mask", shape.rows())?;
        let virtual_mask = r.mask("virtual_mask", shape.len(Parameter::Virtual) / width)?;
        let masks = if set == InputFeatureSet::P {
            Masks::new(set, canonical, vec![false; set.dimensions() + 1])?
        } else {
            Masks::new(set, canonical, first_layer.clone())?
        };
        if masks.first_layer != first_layer || masks.virtual_rows != virtual_mask {
            return Err(Error::Format("inconsistent observation masks"));
        }
        let initial_linear = r.values("initial_linear", &dimensions(shape, Parameter::Linear))?;
        let initial_virtual =
            r.values("initial_virtual", &dimensions(shape, Parameter::Virtual))?;
        r.finish()?;
        let result = Self {
            model: Model {
                parameters,
                masks,
                initial_linear,
                initial_virtual,
            },
            optimizer: AdamW { m, v, step, config },
            removal_penalty,
            lambda,
            progress,
        };
        result.validate()?;
        Ok(result)
    }
    /// 一時ファイルからの改名により原子的に保存する。
    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        Ok(crate::nn::atomic_write(path.as_ref(), &self.encode()?)?)
    }
    /// NPZチェックポイントを読む。
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        Self::decode(&std::fs::read(path)?)
    }
    /// 再開前に設定、入力、前処理結果を照合する。
    pub fn check_config(&self, config: &Configuration) -> Result<()> {
        Ok(self.progress.check_config(config)?)
    }
}
fn dimensions(shape: Shape, parameter: Parameter) -> Vec<usize> {
    match parameter {
        Parameter::Linear => vec![super::CANONICAL_ROWS, 2],
        Parameter::FirstLayer => vec![shape.rows(), shape.width()],
        Parameter::Virtual => vec![shape.len(parameter) / shape.width(), shape.width()],
        Parameter::Biases => vec![shape.units()],
        Parameter::OutputWeights => vec![2, shape.units()],
        Parameter::OutputBias => vec![1],
    }
}
