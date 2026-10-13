use super::{Error, Masks, Model, Parameter, Parameters, Result, Shape};
use crate::nn::{AdamWGroup, BestEpoch, sha256};
use crate::nnue::{
    data::InputFeatureSet,
    reference::{AdamW, AdamWConfig},
};
use std::path::Path;

const MAGIC: &[u8; 8] = b"MNTRCKPT";
const VERSION: u32 = 2;

/// 再開に必要な進捗と入力の同一性。
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    /// 最後に完了したエポック。
    pub epoch: u64,
    /// 最良エポック。更新前はNone。
    pub best: Option<BestEpoch>,
    /// 累積の学習時間、秒。
    pub elapsed_seconds: f64,
    /// 初期化とエポック順列のシード。
    pub seed: u64,
    /// 実行設定のSHA-256。
    pub config_hash: [u8; 32],
    /// 入力のSHA-256。
    pub input_hash: [u8; 32],
}
/// パラメータ、固定行の初期値とマスク、AdamWの状態、進捗を保存する。
#[derive(Clone, Debug, PartialEq)]
pub struct Checkpoint {
    /// f32モデル。
    pub model: Model,
    /// AdamWの全状態。
    pub optimizer: AdamW,
    /// 駒除去差分の補助損失係数。
    pub removal_penalty: f64,
    /// 自己対局の教師混合比の上書き値。
    pub lambda_override: f64,
    /// 学習の進捗とハッシュ。
    pub progress: Progress,
}
impl Checkpoint {
    fn validate(&self) -> Result<()> {
        if !self.removal_penalty.is_finite()
            || self.removal_penalty < 0.0
            || !self.lambda_override.is_finite()
            || !(0.0..=1.0).contains(&self.lambda_override)
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
        if !self.progress.elapsed_seconds.is_finite() || self.progress.elapsed_seconds < 0.0 {
            return Err(Error::Invalid("elapsed time"));
        }
        if let Some(best) = self.progress.best
            && (best.epoch == 0
                || best.epoch > self.progress.epoch
                || !best.validation_loss.is_finite()
                || best.validation_loss < 0.0)
        {
            return Err(Error::Invalid("best epoch"));
        }
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
    /// リトルエンディアン形式へ変換し、末尾に全内容のSHA-256を付ける。
    ///
    /// 配置は`docs/reference/trainer/nnue-training.md`の「チェックポイントの形式」にある。
    /// 配列の長さは構造から導き、長さ欄を持たない。
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let s = self.model.shape();
        let mut bytes = MAGIC.to_vec();
        bytes.extend(VERSION.to_le_bytes());
        let arch = s.architecture();
        bytes.extend(arch.as_bytes());
        bytes.resize(12 + 96, 0);
        bytes.extend((s.width() as u32).to_le_bytes());
        bytes.extend(s.k().to_le_bytes());
        for value in [
            self.optimizer.step,
            self.progress.epoch,
            self.progress.best.map_or(0, |v| v.epoch),
        ] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(
            self.progress
                .best
                .map_or(f64::INFINITY, |v| v.validation_loss)
                .to_le_bytes(),
        );
        bytes.extend(self.progress.elapsed_seconds.to_le_bytes());
        bytes.extend(self.progress.seed.to_le_bytes());
        bytes.extend(self.progress.config_hash);
        bytes.extend(self.progress.input_hash);
        for group in [
            self.optimizer.config.linear,
            self.optimizer.config.ft,
            self.optimizer.config.head,
        ] {
            bytes.extend(group.lr.to_le_bytes());
            bytes.extend(group.weight_decay.to_le_bytes());
        }
        bytes.extend(self.removal_penalty.to_le_bytes());
        bytes.extend(self.lambda_override.to_le_bytes());
        for params in [&self.model.parameters, &self.optimizer.m, &self.optimizer.v] {
            for p in Parameter::ALL {
                floats(&mut bytes, &params[p]);
            }
        }
        for mask in [
            &self.model.masks.canonical,
            &self.model.masks.first_layer,
            &self.model.masks.virtual_rows,
        ] {
            bytes.extend(mask.iter().map(|&v| u8::from(v)));
        }
        floats(&mut bytes, &self.model.initial_linear);
        floats(&mut bytes, &self.model.initial_virtual);
        bytes.extend_from_slice(&sha256(&bytes));
        Ok(bytes)
    }
    /// 保存形式、配列長、有限性、マスクと検査和を検査して復号する。
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let end = bytes
            .len()
            .checked_sub(32)
            .ok_or(Error::Format("checkpoint length"))?;
        if sha256(&bytes[..end])[..] != bytes[end..] {
            return Err(Error::Format("checkpoint SHA-256"));
        }
        let mut r = Reader::new(&bytes[..end]);
        if r.take(8)? != MAGIC || r.u32()? != VERSION {
            return Err(Error::Format("checkpoint magic or version"));
        }
        let arch = r.take(96)?;
        let width = r.u32()? as usize;
        let k = r.f32()?;
        let mut shape = None;
        for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
            let s = Shape::new(set, width, k)?;
            let mut expected = s.architecture().into_bytes();
            expected.resize(96, 0);
            if arch == expected {
                shape = Some(s);
            }
        }
        let shape = shape.ok_or(Error::Format("checkpoint architecture"))?;
        let step = r.u64()?;
        let epoch = r.u64()?;
        let best_epoch = r.u64()?;
        let validation_loss = r.f64()?;
        let best = if best_epoch == 0 {
            if validation_loss != f64::INFINITY {
                return Err(Error::Format("missing best loss"));
            }
            None
        } else {
            Some(BestEpoch {
                epoch: best_epoch,
                validation_loss,
            })
        };
        let elapsed_seconds = r.f64()?;
        let seed = r.u64()?;
        let config_hash = r.take(32)?.try_into().expect("checked length");
        let input_hash = r.take(32)?.try_into().expect("checked length");
        let mut group = || -> Result<_> {
            Ok(AdamWGroup {
                lr: r.f64()?,
                weight_decay: r.f64()?,
            })
        };
        let config = AdamWConfig {
            linear: group()?,
            ft: group()?,
            head: group()?,
        };
        let removal_penalty = r.f64()?;
        let lambda_override = r.f64()?;
        // 最大幅でも形から長さを確定できる。切断された入力では大配列を確保しない。
        let parameter_count: usize = Parameter::ALL.iter().map(|&p| shape.len(p)).sum();
        let virtual_rows = shape.len(Parameter::Virtual) / width;
        let expected = 3 * parameter_count * 4
            + super::CANONICAL_ROWS
            + shape.rows()
            + virtual_rows
            + 4 * (shape.len(Parameter::Linear) + shape.len(Parameter::Virtual));
        if r.remaining().len() != expected {
            return Err(Error::Format("checkpoint payload length"));
        }
        let mut read_params = || -> Result<_> {
            let mut values = Parameters::zeros(shape);
            for p in Parameter::ALL {
                for value in &mut values[p] {
                    *value = r.f32()?;
                }
            }
            Ok(values)
        };
        let parameters = read_params()?;
        let m = read_params()?;
        let v = read_params()?;
        let mut mask = |count| -> Result<Vec<bool>> {
            r.take(count)?
                .iter()
                .map(|&b| match b {
                    0 => Ok(false),
                    1 => Ok(true),
                    _ => Err(Error::Format("mask byte")),
                })
                .collect()
        };
        let canonical = mask(super::CANONICAL_ROWS)?;
        let first_layer = mask(shape.rows())?;
        let virtual_mask = mask(virtual_rows)?;
        let masks = if shape.feature_set() == InputFeatureSet::P {
            Masks::new(
                shape.feature_set(),
                canonical,
                vec![false; shape.feature_set().dimensions() + 1],
            )?
        } else {
            Masks::new(shape.feature_set(), canonical, first_layer.clone())?
        };
        if masks.first_layer != first_layer || masks.virtual_rows != virtual_mask {
            return Err(Error::Format("inconsistent observation masks"));
        }
        let mut values = |count| -> Result<Vec<f32>> { (0..count).map(|_| r.f32()).collect() };
        let initial_linear = values(shape.len(Parameter::Linear))?;
        let initial_virtual = values(shape.len(Parameter::Virtual))?;
        let result = Self {
            model: Model {
                parameters,
                masks,
                initial_linear,
                initial_virtual,
            },
            optimizer: AdamW { m, v, step, config },
            removal_penalty,
            lambda_override,
            progress: Progress {
                epoch,
                best,
                elapsed_seconds,
                seed,
                config_hash,
                input_hash,
            },
        };
        result.validate()?;
        Ok(result)
    }
    /// ファイルへ書く。
    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        std::fs::write(path, self.encode()?)?;
        Ok(())
    }
    /// ファイルを読む。
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        Self::decode(&std::fs::read(path)?)
    }
    /// 再開前に実行設定と入力のハッシュを照合する。
    pub fn check_hashes(&self, config: &[u8; 32], input: &[u8; 32]) -> Result<()> {
        if config != &self.progress.config_hash || input != &self.progress.input_hash {
            return Err(Error::Invalid(
                "checkpoint configuration or input hash mismatch",
            ));
        }
        Ok(())
    }
}
fn floats(bytes: &mut Vec<u8>, values: &[f32]) {
    for &v in values {
        bytes.extend(v.to_le_bytes());
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    fn remaining(&self) -> &'a [u8] {
        self.bytes
    }
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        if len > self.bytes.len() {
            return Err(Error::Format("truncated input"));
        }
        let (value, rest) = self.bytes.split_at(len);
        self.bytes = rest;
        Ok(value)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("checked length"),
        ))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("checked length"),
        ))
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.u64()?))
    }
}
