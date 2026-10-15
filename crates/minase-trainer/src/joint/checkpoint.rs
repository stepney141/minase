//! PSTとFMの同時学習器の無圧縮NPZチェックポイント。
//! `latest.npz`と`best.npz`は次の配列を持ち、余分・欠落・型や形状の違いを拒否する。
//!
//! | 名前 | dtype | 形状 | 内容 |
//! | --- | --- | --- | --- |
//! | `cp_per_logit` | `<f4` | `[]` | 勝率尺度（1ロジットあたりのcp）。 |
//! | `step` | `<u8` | `[]` | 最適化器の更新数。 |
//! | `pst_lr` | `<f8` | `[]` | PSTの学習率。 |
//! | `fm_lr` | `<f8` | `[]` | FMの学習率。 |
//! | `fm_weight_decay` | `<f8` | `[]` | FMの重み減衰。 |
//! | `pst_weights` | `<f4` | `[CANONICAL_ROWS, 2]` | モデル重み。 |
//! | `pst_m` | `<f4` | `[CANONICAL_ROWS, 2]` | AdamWのモーメント。 |
//! | `pst_v` | `<f4` | `[CANONICAL_ROWS, 2]` | AdamWのモーメント。 |
//! | `fm_weights_embedding` | `<f4` | `[FEATURE_COUNT + 1, FM_RANK]` | モデル重み。 |
//! | `fm_weights_a` | `<f4` | `[FM_RANK]` | モデル重み。 |
//! | `fm_m_embedding` | `<f4` | `[FEATURE_COUNT + 1, FM_RANK]` | AdamWのモーメント。 |
//! | `fm_m_a` | `<f4` | `[FM_RANK]` | AdamWのモーメント。 |
//! | `fm_v_embedding` | `<f4` | `[FEATURE_COUNT + 1, FM_RANK]` | AdamWのモーメント。 |
//! | `fm_v_a` | `<f4` | `[FM_RANK]` | AdamWのモーメント。 |
//! | `gradient_pst` | `<f8` | `[min(step, 5)]` | 勾配ノルムの履歴。 |
//! | `gradient_a` | `<f8` | `[min(step, 5)]` | 勾配ノルムの履歴。 |
//! | `gradient_embedding` | `<f8` | `[min(step, 5)]` | 勾配ノルムの履歴。 |
//! | `mask` | `\|b1` | `[FEATURE_COUNT]` | FMの観測マスク。 |
//! | `meta` | `\|u1` | `[JSONのバイト長]` | UTF-8のJSON。 |
//!
//! `CANONICAL_ROWS = 6841`、`FEATURE_COUNT = 13680`、`FM_RANK = 32`である。
//! 配列名の`m`は1次モーメント、`v`は2次モーメントを表す。
//! `embedding`は埋め草行を含む埋め込み、`a`は出力係数であり、勾配履歴は更新1から順に並ぶ。
//! `meta`は`kind = "joint"`、`epoch`、`best`、`elapsed_seconds`、`seed`、`config`を持つ。
//! `best`は`{epoch, validation_loss}`で、更新前はnull。時間は秒である。
//! `config`は`config.json`の`settings`、`inputs`、`preprocessing`をそのまま保持する。

use super::{Adam, GradientNorm, Model, Result};
use crate::nn::AdamWGroup;
pub use crate::nn::checkpoint::Progress;
use crate::nn::checkpoint::{Reader, Writer};
use crate::{fm, pst};
use minase::eval::{features::FEATURE_COUNT, pst::FM_RANK};
use pst::model::CANONICAL_ROWS;
use std::path::Path;

/// 同時学習専用のモデル、AdamW状態、進捗、最初の5更新の勾配ノルム。
#[derive(Clone, Debug, PartialEq)]
pub struct Checkpoint {
    /// 鏡映共有PSTと全特徴のFM。
    pub model: Model,
    /// 2群のAdamW。
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
        if self.gradients.len() != self.optimizer.step().min(5) as usize
            || self.gradients.iter().enumerate().any(|(i, g)| {
                g.step != i as u64 + 1
                    || !g.a.is_finite()
                    || !g.embedding.is_finite()
                    || !g.pst.is_finite()
                    || g.pst < 0.0
                    || g.a < 0.0
                    || g.embedding < 0.0
            })
        {
            return Err("joint checkpoint gradient history".into());
        }
        for row in 0..=FEATURE_COUNT {
            if row == FEATURE_COUNT || !self.model.fm.mask[row] {
                let range = row * FM_RANK..(row + 1) * FM_RANK;
                if self.optimizer.fm.m[range.clone()]
                    .iter()
                    .chain(&self.optimizer.fm.v[range])
                    .any(|&v| v != 0.0)
                {
                    return Err("joint checkpoint unobserved moments must be zero".into());
                }
            }
        }
        if self.optimizer.step() == 0
            && self
                .optimizer
                .pst
                .m
                .iter()
                .chain(&self.optimizer.pst.v)
                .chain(&self.optimizer.fm.m)
                .chain(&self.optimizer.fm.v)
                .any(|&v| v != 0.0)
        {
            return Err("joint checkpoint initial moments must be zero".into());
        }
        Ok(())
    }
    /// 全状態を型と浮動小数点のビット列を保ってNPZへ保存する。
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut w = Writer::new("joint", &self.progress)?;
        w.scalar("cp_per_logit", self.model.pst.cp_per_logit());
        w.scalar("step", self.optimizer.step());
        w.scalar("pst_lr", self.optimizer.pst.lr());
        w.scalar("fm_lr", self.optimizer.fm.group.lr);
        w.scalar("fm_weight_decay", self.optimizer.fm.group.weight_decay);
        w.values("pst_weights", &[CANONICAL_ROWS, 2], &self.model.pst.weights);
        w.values("pst_m", &[CANONICAL_ROWS, 2], &self.optimizer.pst.m);
        w.values("pst_v", &[CANONICAL_ROWS, 2], &self.optimizer.pst.v);
        w.values(
            "fm_weights_embedding",
            &[FEATURE_COUNT + 1, FM_RANK],
            &self.model.fm.weights[..(FEATURE_COUNT + 1) * FM_RANK],
        );
        w.values(
            "fm_weights_a",
            &[FM_RANK],
            &self.model.fm.weights[(FEATURE_COUNT + 1) * FM_RANK..],
        );
        w.values(
            "fm_m_embedding",
            &[FEATURE_COUNT + 1, FM_RANK],
            &self.optimizer.fm.m[..(FEATURE_COUNT + 1) * FM_RANK],
        );
        w.values(
            "fm_m_a",
            &[FM_RANK],
            &self.optimizer.fm.m[(FEATURE_COUNT + 1) * FM_RANK..],
        );
        w.values(
            "fm_v_embedding",
            &[FEATURE_COUNT + 1, FM_RANK],
            &self.optimizer.fm.v[..(FEATURE_COUNT + 1) * FM_RANK],
        );
        w.values(
            "fm_v_a",
            &[FM_RANK],
            &self.optimizer.fm.v[(FEATURE_COUNT + 1) * FM_RANK..],
        );
        w.values(
            "gradient_pst",
            &[self.gradients.len()],
            &self.gradients.iter().map(|g| g.pst).collect::<Vec<_>>(),
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
        w.mask("mask", &self.model.fm.mask);
        Ok(w.encode()?)
    }

    /// 種別、配列の集合・型・形状とモデルの不変条件を検査する。
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let (mut r, progress) = Reader::new(bytes, "joint")?;
        let cp_per_logit = r.scalar::<f32>("cp_per_logit")?;
        let step = r.scalar::<u64>("step")?;
        let pst_lr = r.scalar::<f64>("pst_lr")?;
        let fm_lr = r.scalar::<f64>("fm_lr")?;
        let fm_weight_decay = r.scalar::<f64>("fm_weight_decay")?;
        let pst_weights = r.values::<f32>("pst_weights", &[CANONICAL_ROWS, 2])?;
        let pst_m = r.values::<f32>("pst_m", &[CANONICAL_ROWS, 2])?;
        let pst_v = r.values::<f32>("pst_v", &[CANONICAL_ROWS, 2])?;
        let fm_weights_embedding =
            r.values::<f32>("fm_weights_embedding", &[FEATURE_COUNT + 1, FM_RANK])?;
        let fm_weights_a = r.values::<f32>("fm_weights_a", &[FM_RANK])?;
        let fm_m_embedding = r.values::<f32>("fm_m_embedding", &[FEATURE_COUNT + 1, FM_RANK])?;
        let fm_m_a = r.values::<f32>("fm_m_a", &[FM_RANK])?;
        let fm_v_embedding = r.values::<f32>("fm_v_embedding", &[FEATURE_COUNT + 1, FM_RANK])?;
        let fm_v_a = r.values::<f32>("fm_v_a", &[FM_RANK])?;
        let gradient_pst = r.values::<f64>("gradient_pst", &[step.min(5) as usize])?;
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
                pst: gradient_pst[i],
            })
            .collect();
        let model = Model {
            pst: pst::Model {
                weights: pst_weights,
                cp_per_logit,
            },
            fm: fm::Model {
                weights: fm_weights,
                mask,
                cp_per_logit,
            },
        };
        let optimizer = Adam {
            pst: pst::Adam {
                m: pst_m,
                v: pst_v,
                step,
                lr: pst_lr,
            },
            fm: fm::Adam {
                m: fm_m,
                v: fm_v,
                step,
                group,
            },
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
