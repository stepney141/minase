//! NNUEの局面供給元と、局面数に比例しない前処理情報。

use super::{Args, Error, Result};
use crate::{
    data::{Dataset, SpillStore, TeacherClass},
    nn::{Sha256Hasher, hex},
};
use std::path::Path;

/// B=1のメモリ保持とB≥2の一時ファイルを明示的に選ぶ。
pub(super) enum Store {
    /// 全局面と通し番号の列をメモリに保持する。
    Memory(Dataset),
    /// 破棄時に作業ディレクトリも削除するストア。
    Spill(SpillStore),
}

impl Store {
    /// 引数で指定された経路だけを構築し、教師の混合比を設定する。
    pub(super) fn open(args: &Args, workers: usize) -> Result<Self> {
        let parallelism = std::num::NonZeroUsize::new(workers)
            .ok_or(Error::Invalid("worker count must be positive"))?;
        Ok(if args.buckets == 1 {
            Self::Memory(
                Dataset::open(&args.data, args.lookahead()?, parallelism)?
                    .with_lambda(args.training.lambda)?,
            )
        } else {
            Self::Spill(
                SpillStore::create(
                    &args.data,
                    args.lookahead()?,
                    parallelism,
                    args.scratch_dir
                        .as_deref()
                        .ok_or(Error::Invalid("missing --scratch-dir"))?,
                    args.buckets,
                    args.bucket_positions
                        .ok_or(Error::Invalid("missing --bucket-positions"))?,
                )?
                .with_lambda(args.training.lambda)?,
            )
        })
    }

    /// 前処理に使う、間引く前の全訓練局面数を返す。
    pub(super) fn training_count(&self) -> u64 {
        match self {
            Self::Memory(d) => d.training_indices().len() as u64,
            Self::Spill(d) => d.training_count(),
        }
    }

    /// 検証局面数を返す。
    pub(super) fn validation_count(&self) -> u64 {
        match self {
            Self::Memory(d) => d.validation_indices().len() as u64,
            Self::Spill(d) => d.validation_count(),
        }
    }

    /// 全入力の規則セットを基準と照合する。
    pub(super) fn check_rules(&self, rules: minase_core::rules::Rules) -> Result<()> {
        let headers: Vec<_> = match self {
            Self::Memory(d) => d.headers().collect(),
            Self::Spill(d) => d.headers().collect(),
        };
        for header in headers {
            let codes = minase_core::rules::parse_rule_set(header.rule_set())?;
            if minase_core::rules::Rules::from_codes(&codes)? != rules {
                return Err(Error::Invalid("MNSD rule set differs from --rules").into());
            }
        }
        Ok(())
    }

    pub(super) fn validation_sample(
        &self,
        requested: usize,
        seed: u64,
    ) -> Result<Vec<(minase_core::Position, minase_core::Color)>> {
        let samples = match self {
            Self::Memory(d) => crate::data::floyd_sample(
                d.validation_indices().len() as u64,
                requested.min(d.validation_indices().len()),
                seed,
            )?
            .into_iter()
            .map(|i| d.record(d.validation_indices()[i as usize]))
            .collect::<crate::data::Result<Vec<_>>>()?,
            Self::Spill(d) => d
                .validation_sample(requested, seed)?
                .into_iter()
                .map(|r| r.sample)
                .collect(),
        };
        samples
            .into_iter()
            .map(|sample| Ok((sample.to_position()?, sample.side_to_move())))
            .collect()
    }
}

/// 設定記録に必要な入力単位の情報と集計値。局面の配列を保持しない。
pub(super) struct DatasetInfo<'a> {
    /// 入力順の正規化したパス。
    pub paths: Vec<&'a Path>,
    /// 分類番号順の教師情報。
    pub classes: &'a [TeacherClass],
    /// 分類番号順の混合比。
    pub lambdas: &'a [f64],
    /// 訓練局面の件数。
    pub training: u64,
    /// 検証局面の件数。
    pub validation: u64,
    /// 訓練局面のファイル順の通し番号の検査和。
    pub training_hash: String,
    /// 検証局面のファイル順の通し番号の検査和。
    pub validation_hash: String,
    /// 指定時だけ保存する、更新用の抽出集合の記録。
    pub subsample: Option<&'a super::subsample::Summary>,
}
impl<'a> From<&'a Dataset> for DatasetInfo<'a> {
    fn from(d: &'a Dataset) -> Self {
        let hash = |indices: &[u64]| {
            let mut hash = Sha256Hasher::default();
            for index in indices {
                hash.update(&index.to_le_bytes());
            }
            hex(&hash.finalize())
        };
        Self {
            paths: d.paths().collect(),
            classes: d.teacher_classes(),
            lambdas: d.teacher_lambdas(),
            training: d.training_indices().len() as u64,
            validation: d.validation_indices().len() as u64,
            training_hash: hash(d.training_indices()),
            validation_hash: hash(d.validation_indices()),
            subsample: None,
        }
    }
}
impl<'a> From<&'a Store> for DatasetInfo<'a> {
    fn from(store: &'a Store) -> Self {
        match store {
            Store::Memory(d) => d.into(),
            Store::Spill(d) => Self {
                paths: d.paths().collect(),
                classes: d.teacher_classes(),
                lambdas: d.teacher_lambdas(),
                training: d.training_count(),
                validation: d.validation_count(),
                training_hash: d.training_indices_sha256().to_owned(),
                validation_hash: d.validation_indices_sha256().to_owned(),
                subsample: None,
            },
        }
    }
}
