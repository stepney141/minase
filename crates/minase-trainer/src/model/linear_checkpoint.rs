use super::{
    Error, LinearAdam, LinearModel, Progress, Result,
    checkpoint::{envelope, floats, open, seal},
};
use crate::data::CANONICAL_ROWS;
use std::path::Path;

const ARCHITECTURE: &str = "features=P;linear=mirrored-tapered2;model=PST";

/// PSTの重みとAdam状態。外枠、進捗、検査和はNNUEと共有する。
#[derive(Clone, Debug, PartialEq)]
pub struct LinearCheckpoint {
    /// 鏡映共有PST。
    pub model: LinearModel,
    /// Adamの全状態。
    pub optimizer: LinearAdam,
    /// 再開と最良モデルの選択に必要な進捗。
    pub progress: Progress,
}

impl LinearCheckpoint {
    fn validate(&self) -> Result<()> {
        self.model.validate()?;
        self.optimizer.validate()?;
        self.progress.validate()?;
        if self
            .model
            .weights()
            .iter()
            .any(|v| !(super::linear::LINEAR_MIN..=super::linear::LINEAR_MAX).contains(v))
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
        let model = LinearModel {
            weights: read()?,
            k,
        };
        let optimizer = LinearAdam {
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
