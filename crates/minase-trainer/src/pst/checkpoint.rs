use super::{
    Adam, Error, Model, Result,
    model::{CANONICAL_ROWS, LINEAR_MAX, LINEAR_MIN},
};
use crate::nn::BestEpoch;
use sha2::{Digest, Sha256};
use std::path::Path;

const MAGIC: &[u8; 8] = b"MNTRCKPT";
const VERSION: u32 = 2;

const ARCHITECTURE: &str = "features=P;linear=mirrored-tapered2;model=PST";

/// PSTの重み、Adam状態と再開に必要な進捗を保存する。
#[derive(Clone, Debug, PartialEq)]
pub struct Checkpoint {
    /// 鏡映共有PST。
    pub model: Model,
    /// Adamの全状態。
    pub optimizer: Adam,
    /// 再開と最良モデルの選択に必要な進捗。
    pub progress: Progress,
}

impl Checkpoint {
    fn validate(&self) -> Result<()> {
        self.model.validate()?;
        self.optimizer.validate()?;
        self.progress.validate()?;
        if self
            .model
            .weights()
            .iter()
            .any(|v| !(LINEAR_MIN..=LINEAR_MAX).contains(v))
        {
            return Err(Error::Invalid("checkpoint PST range"));
        }
        Ok(())
    }

    /// モデル識別子とSHA-256を付けてリトルエンディアンへ符号化する。
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut bytes = envelope(ARCHITECTURE);
        bytes.extend(self.model.k().to_le_bytes());
        bytes.extend(self.optimizer.step().to_le_bytes());
        self.progress.encode_into(&mut bytes);
        bytes.extend(self.optimizer.lr().to_le_bytes());
        for values in [self.model.weights(), self.optimizer.m(), self.optimizer.v()] {
            floats(&mut bytes, values);
        }
        Ok(seal(bytes))
    }

    /// 識別子、検査和、配列長、有限性、埋め草、進捗を検査して復号する。
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = open(bytes)?;
        let mut architecture = ARCHITECTURE.as_bytes().to_vec();
        architecture.resize(96, 0);
        if r.take(96)? != architecture {
            return Err(Error::Format("checkpoint architecture is not PST"));
        }
        let k = r.f32()?;
        let step = r.u64()?;
        let progress = Progress::decode_from(&mut r)?;
        let lr = r.f64()?;
        let len = CANONICAL_ROWS * 2;
        if r.remaining().len() != 3 * len * 4 {
            return Err(Error::Format("PST checkpoint payload length"));
        }
        let mut read = || -> Result<Vec<f32>> { (0..len).map(|_| r.f32()).collect() };
        let model = Model {
            weights: read()?,
            k,
        };
        let optimizer = Adam {
            m: read()?,
            v: read()?,
            step,
            lr,
        };
        let result = Self {
            model,
            optimizer,
            progress,
        };
        result.validate()?;
        Ok(result)
    }

    /// ファイルを読み、PST用チェックポイントとして検査する。
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
            return Err(Error::Invalid("elapsed time"));
        }
        if let Some(best) = self.best
            && (best.epoch == 0
                || best.epoch > self.epoch
                || !best.validation_loss.is_finite()
                || best.validation_loss < 0.0)
        {
            return Err(Error::Invalid("best epoch"));
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
            return Err(Error::Invalid(
                "checkpoint configuration or input hash mismatch",
            ));
        }
        Ok(())
    }
}

pub(super) fn floats(bytes: &mut Vec<u8>, values: &[f32]) {
    for &v in values {
        bytes.extend(v.to_le_bytes());
    }
}

// PSTチェックポイントの識別子、版、構造名、検査和。
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
    let end = bytes
        .len()
        .checked_sub(32)
        .ok_or(Error::Format("checkpoint length"))?;
    if Sha256::digest(&bytes[..end])[..] != bytes[end..] {
        return Err(Error::Format("checkpoint SHA-256"));
    }
    let mut reader = Reader::new(&bytes[..end]);
    if reader.take(8)? != MAGIC || reader.u32()? != VERSION {
        return Err(Error::Format("checkpoint magic or version"));
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
            return Err(Error::Format("truncated input"));
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
