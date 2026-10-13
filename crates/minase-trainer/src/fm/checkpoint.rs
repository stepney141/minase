//! FM学習器の無圧縮NPZチェックポイント。
//! `latest.npz`と`best.npz`は次の配列を持ち、余分・欠落・型や形状の違いを拒否する。
//!
//! | 名前 | dtype | 形状 | 内容 |
//! | --- | --- | --- | --- |
//! | `k` | `<f4` | `[]` | 出力用K。 |
//! | `step` | `<u8` | `[]` | 最適化器の更新数。 |
//! | `fm_lr` | `<f8` | `[]` | FMの学習率。 |
//! | `fm_weight_decay` | `<f8` | `[]` | FMの重み減衰。 |
//! | `fm_weights_embedding` | `<f4` | `[FEATURE_COUNT + 1, FM_RANK]` | モデル重み。 |
//! | `fm_weights_a` | `<f4` | `[FM_RANK]` | モデル重み。 |
//! | `fm_m_embedding` | `<f4` | `[FEATURE_COUNT + 1, FM_RANK]` | AdamWのモーメント。 |
//! | `fm_m_a` | `<f4` | `[FM_RANK]` | AdamWのモーメント。 |
//! | `fm_v_embedding` | `<f4` | `[FEATURE_COUNT + 1, FM_RANK]` | AdamWのモーメント。 |
//! | `fm_v_a` | `<f4` | `[FM_RANK]` | AdamWのモーメント。 |
//! | `gradient_a` | `<f8` | `[min(step, 5)]` | 勾配ノルムの履歴。 |
//! | `gradient_embedding` | `<f8` | `[min(step, 5)]` | 勾配ノルムの履歴。 |
//! | `mask` | `\|b1` | `[FEATURE_COUNT]` | FMの観測マスク。 |
//! | `meta` | `\|u1` | `[JSONのバイト長]` | UTF-8のJSON。 |
//!
//! `FEATURE_COUNT = 13680`、`FM_RANK = 32`である。
//! 配列名の`m`は1次モーメント、`v`は2次モーメントを表す。
//! `embedding`は埋め草行を含む埋め込み、`a`は出力係数であり、勾配履歴は更新1から順に並ぶ。
//! `meta`は`kind = "fm"`、`epoch`、`best`、`elapsed_seconds`、`seed`、`config`を持つ。
//! `best`は`{epoch, validation_loss}`で、更新前はnull。時間は秒である。
//! `config`は`config.json`の`settings`、`inputs`、`preprocessing`をそのまま保持する。

use super::{Adam, GradientNorm, Model, Result};
use crate::nn::AdamWGroup;
pub use crate::nn::checkpoint::Progress;
use crate::nn::checkpoint::{Reader, Writer};
use minase::eval::{features::FEATURE_COUNT, pst::FM_RANK};
use std::path::Path;

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
    /// 全状態を型と浮動小数点のビット列を保ってNPZへ保存する。
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut w = Writer::new("fm", &self.progress)?;
        w.scalar("k", self.model.k);
        w.scalar("step", self.optimizer.step);
        w.scalar("fm_lr", self.optimizer.group.lr);
        w.scalar("fm_weight_decay", self.optimizer.group.weight_decay);
        w.values(
            "fm_weights_embedding",
            &[FEATURE_COUNT + 1, FM_RANK],
            &self.model.weights[..(FEATURE_COUNT + 1) * FM_RANK],
        );
        w.values(
            "fm_weights_a",
            &[FM_RANK],
            &self.model.weights[(FEATURE_COUNT + 1) * FM_RANK..],
        );
        w.values(
            "fm_m_embedding",
            &[FEATURE_COUNT + 1, FM_RANK],
            &self.optimizer.m[..(FEATURE_COUNT + 1) * FM_RANK],
        );
        w.values(
            "fm_m_a",
            &[FM_RANK],
            &self.optimizer.m[(FEATURE_COUNT + 1) * FM_RANK..],
        );
        w.values(
            "fm_v_embedding",
            &[FEATURE_COUNT + 1, FM_RANK],
            &self.optimizer.v[..(FEATURE_COUNT + 1) * FM_RANK],
        );
        w.values(
            "fm_v_a",
            &[FM_RANK],
            &self.optimizer.v[(FEATURE_COUNT + 1) * FM_RANK..],
        );
        w.values(
            "gradient_a",
            &[self.gradients.len()],
            &self.gradients.iter().map(|g| g.a).collect::<Vec<_>>(),
        );
        w.values(
            "gradient_embedding",
            &[self.gradients.len()],
            &self
                .gradients
                .iter()
                .map(|g| g.embedding)
                .collect::<Vec<_>>(),
        );
        w.mask("mask", &self.model.mask);
        Ok(w.encode()?)
    }

    /// 種別、配列の集合・型・形状とモデルの不変条件を検査する。
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let (mut r, progress) = Reader::new(bytes, "fm")?;
        let k = r.scalar::<f32>("k")?;
        let step = r.scalar::<u64>("step")?;
        let fm_lr = r.scalar::<f64>("fm_lr")?;
        let fm_weight_decay = r.scalar::<f64>("fm_weight_decay")?;
        let fm_weights_embedding =
            r.values::<f32>("fm_weights_embedding", &[FEATURE_COUNT + 1, FM_RANK])?;
        let fm_weights_a = r.values::<f32>("fm_weights_a", &[FM_RANK])?;
        let fm_m_embedding = r.values::<f32>("fm_m_embedding", &[FEATURE_COUNT + 1, FM_RANK])?;
        let fm_m_a = r.values::<f32>("fm_m_a", &[FM_RANK])?;
        let fm_v_embedding = r.values::<f32>("fm_v_embedding", &[FEATURE_COUNT + 1, FM_RANK])?;
        let fm_v_a = r.values::<f32>("fm_v_a", &[FM_RANK])?;
        let gradient_a = r.values::<f64>("gradient_a", &[step.min(5) as usize])?;
        let gradient_embedding = r.values::<f64>("gradient_embedding", &[step.min(5) as usize])?;
        let mask = r.mask("mask", FEATURE_COUNT)?;
        let fm_weights = [fm_weights_embedding, fm_weights_a].concat();
        let fm_m = [fm_m_embedding, fm_m_a].concat();
        let fm_v = [fm_v_embedding, fm_v_a].concat();
        let group = AdamWGroup {
            lr: fm_lr,
            weight_decay: fm_weight_decay,
        };
        let gradients = (0..step.min(5) as usize)
            .map(|i| GradientNorm {
                step: i as u64 + 1,
                a: gradient_a[i],
                embedding: gradient_embedding[i],
            })
            .collect();
        let model = Model {
            weights: fm_weights,
            mask,
            k,
        };
        let optimizer = Adam {
            m: fm_m,
            v: fm_v,
            step,
            group,
        };
        r.finish()?;
        let result = Self {
            model,
            optimizer,
            progress,
            gradients,
        };
        result.validate()?;
        Ok(result)
    }
    /// NPZチェックポイントを読み込む。
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        Self::decode(&std::fs::read(path)?)
    }
}
