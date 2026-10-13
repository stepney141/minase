//! PST学習器の無圧縮NPZチェックポイント。
//! `latest.npz`と`best.npz`は次の配列を持ち、余分・欠落・型や形状の違いを拒否する。
//!
//! | 名前 | dtype | 形状 | 内容 |
//! | --- | --- | --- | --- |
//! | `k` | `<f4` | `[]` | 出力用K。 |
//! | `step` | `<u8` | `[]` | 最適化器の更新数。 |
//! | `pst_lr` | `<f8` | `[]` | PSTの学習率。 |
//! | `pst_weights` | `<f4` | `[CANONICAL_ROWS, 2]` | モデル重み。 |
//! | `pst_m` | `<f4` | `[CANONICAL_ROWS, 2]` | AdamWのモーメント。 |
//! | `pst_v` | `<f4` | `[CANONICAL_ROWS, 2]` | AdamWのモーメント。 |
//! | `meta` | `\|u1` | `[JSONのバイト長]` | UTF-8のJSON。 |
//!
//! `CANONICAL_ROWS = 6841`である。
//! 配列名の`m`は1次モーメント、`v`は2次モーメントを表す。
//! `meta`は`kind = "pst"`、`epoch`、`best`、`elapsed_seconds`、`seed`、`config`を持つ。
//! `best`は`{epoch, validation_loss}`で、更新前はnull。時間は秒である。
//! `config`は`config.json`の`settings`、`inputs`、`preprocessing`をそのまま保持する。

use super::{
    Adam, Error, Model, Result,
    model::{CANONICAL_ROWS, LINEAR_MAX, LINEAR_MIN},
};
pub use crate::nn::checkpoint::Progress;
use crate::nn::checkpoint::{Reader, Writer};
use std::path::Path;

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

    /// 全状態を型と浮動小数点のビット列を保ってNPZへ保存する。
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut w = Writer::new("pst", &self.progress)?;
        w.scalar("k", self.model.k());
        w.scalar("step", self.optimizer.step());
        w.scalar("pst_lr", self.optimizer.lr());
        w.values("pst_weights", &[CANONICAL_ROWS, 2], self.model.weights());
        w.values("pst_m", &[CANONICAL_ROWS, 2], self.optimizer.m());
        w.values("pst_v", &[CANONICAL_ROWS, 2], self.optimizer.v());
        Ok(w.encode()?)
    }

    /// 種別、配列の集合・型・形状とモデルの不変条件を検査する。
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let (mut r, progress) = Reader::new(bytes, "pst")?;
        let k = r.scalar::<f32>("k")?;
        let step = r.scalar::<u64>("step")?;
        let pst_lr = r.scalar::<f64>("pst_lr")?;
        let pst_weights = r.values::<f32>("pst_weights", &[CANONICAL_ROWS, 2])?;
        let pst_m = r.values::<f32>("pst_m", &[CANONICAL_ROWS, 2])?;
        let pst_v = r.values::<f32>("pst_v", &[CANONICAL_ROWS, 2])?;
        let model = Model {
            weights: pst_weights,
            k,
        };
        let optimizer = Adam {
            m: pst_m,
            v: pst_v,
            step,
            lr: pst_lr,
        };
        r.finish()?;
        let result = Self {
            model,
            optimizer,
            progress,
        };
        result.validate()?;
        Ok(result)
    }
    /// NPZチェックポイントを読み込む。
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        Self::decode(&std::fs::read(path)?)
    }
}
