use super::{Adam, GradientNorm, Model, Result, model::PARAMETERS};
use crate::nn::{AdamWGroup, BestEpoch};
use minase::eval::{features::FEATURE_COUNT, pst::FM_RANK};
use sha2::{Digest, Sha256};
use std::path::Path;

const MAGIC: &[u8; 8] = b"MNFMCKPT";
const VERSION: u32 = 1;
const ARCHITECTURE: &str = "features=P;rank=32;fixed=integer-PST;model=FM";

/// FM専用のモデル、AdamW状態、進捗、最初の5更新の勾配ノルム。
#[derive(Clone, Debug, PartialEq)]
pub struct Checkpoint {
    /// 全特徴のVとa。
    pub model: Model,
    /// 1群のAdamW。
    pub optimizer: Adam,
    /// 再開および最良エポックの選択に必要な進捗。
    pub progress: Progress,
    /// 最初の5更新の勾配ノルム。
    pub gradients: Vec<GradientNorm>,
}
impl Checkpoint {
    fn validate(&self) -> Result<()> {
        self.model.validate()?;
        self.optimizer.validate()?;
        self.progress.validate()?;
        if self.gradients.len() != self.optimizer.step.min(5) as usize
            || self.gradients.iter().enumerate().any(|(i, g)| {
                g.step != i as u64 + 1
                    || !g.a.is_finite()
                    || !g.embedding.is_finite()
                    || g.a < 0.0
                    || g.embedding < 0.0
            })
        {
            return Err("FM checkpoint gradient history".into());
        }
        for row in 0..=FEATURE_COUNT {
            if row == FEATURE_COUNT || !self.model.mask[row] {
                let range = row * FM_RANK..(row + 1) * FM_RANK;
                if self.optimizer.m[range.clone()]
                    .iter()
                    .chain(&self.optimizer.v[range])
                    .any(|&v| v != 0.0)
                {
                    return Err("FM checkpoint unobserved moments must be zero".into());
                }
            }
        }
        if self.optimizer.step == 0
            && self
                .optimizer
                .m
                .iter()
                .chain(&self.optimizer.v)
                .any(|&v| v != 0.0)
        {
            return Err("FM checkpoint initial moments must be zero".into());
        }
        Ok(())
    }
    /// 識別子、版とSHA-256を付け、f32のビット列を保持する。
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut bytes = envelope(ARCHITECTURE);
        bytes.extend(self.model.k.to_le_bytes());
        bytes.extend(self.optimizer.step.to_le_bytes());
        self.progress.encode_into(&mut bytes);
        bytes.extend(self.optimizer.group.lr.to_le_bytes());
        bytes.extend(self.optimizer.group.weight_decay.to_le_bytes());
        for g in &self.gradients {
            bytes.extend(g.a.to_le_bytes());
            bytes.extend(g.embedding.to_le_bytes());
        }
        bytes.extend(self.model.mask.iter().map(|&v| u8::from(v)));
        for values in [&self.model.weights, &self.optimizer.m, &self.optimizer.v] {
            floats(&mut bytes, values);
        }
        Ok(seal(bytes))
    }
    /// 構造、検査和、有限性、マスク、埋め草と進捗を検査する。
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = open(bytes)?;
        let mut architecture = ARCHITECTURE.as_bytes().to_vec();
        architecture.resize(96, 0);
        if r.take(96)? != architecture {
            return Err("checkpoint architecture is not FM".into());
        }
        let k = r.f32()?;
        let step = r.u64()?;
        let progress = Progress::decode_from(&mut r)?;
        let group = AdamWGroup {
            lr: r.f64()?,
            weight_decay: r.f64()?,
        };
        let gradients = (1..=step.min(5))
            .map(|step| {
                Ok(GradientNorm {
                    step,
                    a: r.f64()?,
                    embedding: r.f64()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mask = r.take(FEATURE_COUNT)?;
        if mask.iter().any(|&v| v > 1) {
            return Err("FM checkpoint mask must be bool".into());
        }
        let mask = mask.iter().map(|&v| v != 0).collect();
        if r.remaining().len() != 3 * PARAMETERS * 4 {
            return Err("FM checkpoint payload length".into());
        }
        let mut read = || (0..PARAMETERS).map(|_| r.f32()).collect::<Result<Vec<_>>>();
        let model = Model {
            weights: read()?,
            mask,
            k,
        };
        let optimizer = Adam {
            m: read()?,
            v: read()?,
            step,
            group,
        };
        let result = Self {
            model,
            optimizer,
            progress,
            gradients,
        };
        result.validate()?;
        Ok(result)
    }
    /// FMチェックポイントを読み込む。
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        Self::decode(&std::fs::read(path)?)
    }
}

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
impl Progress {
    pub(super) fn validate(&self) -> Result<()> {
        if !self.elapsed_seconds.is_finite() || self.elapsed_seconds < 0.0 {
            return Err("elapsed time".into());
        }
        if let Some(best) = self.best
            && (best.epoch == 0
                || best.epoch > self.epoch
                || !best.validation_loss.is_finite()
                || best.validation_loss < 0.0)
        {
            return Err("best epoch".into());
        }
        Ok(())
    }

    pub(super) fn encode_into(&self, bytes: &mut Vec<u8>) {
        bytes.extend(self.epoch.to_le_bytes());
        bytes.extend(self.best.map_or(0, |v| v.epoch).to_le_bytes());
        bytes.extend(
            self.best
                .map_or(f64::INFINITY, |v| v.validation_loss)
                .to_le_bytes(),
        );
        bytes.extend(self.elapsed_seconds.to_le_bytes());
        bytes.extend(self.seed.to_le_bytes());
        bytes.extend(self.config_hash);
        bytes.extend(self.input_hash);
    }

    pub(super) fn decode_from(r: &mut Reader<'_>) -> Result<Self> {
        let epoch = r.u64()?;
        let best_epoch = r.u64()?;
        let validation_loss = r.f64()?;
        let best = if best_epoch == 0 {
            if validation_loss != f64::INFINITY {
                return Err("missing best loss".into());
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
        let progress = Self {
            epoch,
            best,
            elapsed_seconds,
            seed,
            config_hash,
            input_hash,
        };
        progress.validate()?;
        Ok(progress)
    }

    /// 再開前に、軌跡を変える設定と入力のハッシュを照合する。
    pub fn check_hashes(&self, config: &[u8; 32], input: &[u8; 32]) -> Result<()> {
        if config != &self.config_hash || input != &self.input_hash {
            return Err("checkpoint configuration or input hash mismatch".into());
        }
        Ok(())
    }
}

pub(super) fn floats(bytes: &mut Vec<u8>, values: &[f32]) {
    for &v in values {
        bytes.extend(v.to_le_bytes());
    }
}

// FMチェックポイントの識別子、版、構造名、検査和。
pub(super) fn envelope(architecture: &str) -> Vec<u8> {
    let mut bytes = MAGIC.to_vec();
    bytes.extend(VERSION.to_le_bytes());
    bytes.extend(architecture.as_bytes());
    bytes.resize(12 + 96, 0);
    bytes
}

pub(super) fn seal(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.extend_from_slice(&Sha256::digest(&bytes));
    bytes
}

pub(super) fn open(bytes: &[u8]) -> Result<Reader<'_>> {
    let end = bytes.len().checked_sub(32).ok_or("checkpoint length")?;
    if Sha256::digest(&bytes[..end])[..] != bytes[end..] {
        return Err("checkpoint SHA-256".into());
    }
    let mut reader = Reader::new(&bytes[..end]);
    if reader.take(8)? != MAGIC || reader.u32()? != VERSION {
        return Err("checkpoint magic or version".into());
    }
    Ok(reader)
}

pub(super) struct Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Reader<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    pub(super) fn remaining(&self) -> &'a [u8] {
        self.bytes
    }
    pub(super) fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        if len > self.bytes.len() {
            return Err("truncated input".into());
        }
        let (value, rest) = self.bytes.split_at(len);
        self.bytes = rest;
        Ok(value)
    }
    pub(super) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("checked length"),
        ))
    }
    pub(super) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("checked length"),
        ))
    }
    pub(super) fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    pub(super) fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.u64()?))
    }
}
