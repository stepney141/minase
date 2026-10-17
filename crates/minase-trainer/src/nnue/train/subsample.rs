//! 前処理用の全訓練集合を保ち、更新用の件数、索引と抽出記録を作る。

use super::{Error, Result, store::Store};
use crate::{
    data::{Dataset, GameKey, TrainSubsample},
    nn::{Sha256Hasher, hex},
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct Summary {
    pub training_positions_before: u64,
    pub training_positions_after: u64,
    pub training_games_after: u64,
    /// 全適格局面の番号ではなく、間引く前の訓練集合内の0始まり番号を使う。
    pub training_indices_sha256: String,
}

pub(super) struct Selection {
    pub positions: u64,
    pub summary: Option<Summary>,
    /// メモリ経路で抽出した全適格局面の通し番号。一時ファイル経路では保持しない。
    indices: Option<Vec<u64>>,
}

impl Selection {
    pub fn new(store: &mut Store, spec: Option<TrainSubsample>) -> Result<Self> {
        let Some(spec) = spec else {
            return Ok(Self {
                positions: store.training_count(),
                summary: None,
                indices: None,
            });
        };
        let mut summary = Summary {
            training_positions_before: store.training_count(),
            training_positions_after: 0,
            training_games_after: 0,
            training_indices_sha256: String::new(),
        };
        let mut hash = Sha256Hasher::default();
        let mut previous_game = None;
        let mut ordinal = 0_u64;
        let mut keep = |key: GameKey, ply| {
            let selected = spec.keeps(key.0 as u64, key.1, ply);
            if selected {
                hash.update(&ordinal.to_le_bytes());
                summary.training_positions_after += 1;
                // MNSDはファイル内の対局番号の厳密な増加を読み込み時に検査する。
                if previous_game != Some(key) {
                    summary.training_games_after += 1;
                    previous_game = Some(key);
                }
            }
            ordinal += 1;
            selected
        };
        let indices = match store {
            Store::Memory(dataset) => {
                let mut indices = Vec::new();
                for &index in dataset.training_indices() {
                    let (key, ply) = dataset.subsample_key(index)?;
                    if keep(key, ply) {
                        indices.push(index);
                    }
                }
                Some(indices)
            }
            Store::Spill(store) => {
                for key in store.training_subsample_keys() {
                    let (key, ply) = key?;
                    keep(key, ply);
                }
                store.set_train_subsample(spec);
                None
            }
        };
        if summary.training_positions_after == 0 {
            return Err(Error::Invalid("--train-subsample selected 0 training positions").into());
        }
        summary.training_indices_sha256 = hex(&hash.finalize());
        Ok(Self {
            positions: summary.training_positions_after,
            summary: Some(summary),
            indices,
        })
    }

    pub fn indices<'a>(&'a self, dataset: &'a Dataset) -> &'a [u64] {
        match &self.indices {
            Some(indices) => indices,
            None => dataset.training_indices(),
        }
    }
}
