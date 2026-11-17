use crate::data::Dataset;

use super::features::{base_features, canonical_row};
use super::{CANONICAL_ROWS, InputFeatureSet, Result, SENTINEL};

/// 訓練集合だけから求めた特徴の観測回数。
pub struct Observations {
    canonical_counts: Vec<u64>,
    input_counts: Vec<u64>,
}

impl Observations {
    /// 訓練集合だけを1回走査し、手番側の正準行と両視点の入力特徴の出現回数を求める。
    /// 正準行は鏡映込みの回数の左半分に相当し、入力特徴は鏡映を追加せず数える。
    pub fn compute(dataset: &Dataset, feature_set: InputFeatureSet) -> Result<Self> {
        Self::from_positions(
            dataset
                .training_indices()
                .iter()
                .map(|&index| Ok(dataset.record(index)?.to_position()?)),
            feature_set,
        )
    }

    /// ファイル順の訓練局面の列を1回走査して観測回数を求める。
    pub fn from_positions(
        positions: impl Iterator<Item = Result<minase_core::Position>>,
        feature_set: InputFeatureSet,
    ) -> Result<Self> {
        let mut canonical_counts = vec![0; CANONICAL_ROWS];
        let mut input_counts = vec![0; feature_set.dimensions()];
        for position in positions {
            let position = position?;
            let bases = base_features(&position, position.side_to_move());
            for &base in &bases[0] {
                if base != SENTINEL {
                    // 元の特徴とその鏡映の片方だけが左半分に入る。
                    // 鏡映込みの回数の左半分に相当するため、1特徴を2回数えない。
                    canonical_counts[canonical_row(base) as usize] += 1;
                }
            }
            for &base in bases.iter().flatten() {
                if base != SENTINEL {
                    input_counts[base as usize] += 1;
                }
            }
        }
        Ok(Self {
            canonical_counts,
            input_counts,
        })
    }

    /// 手番側の鏡映込みの回数から左半分を取った6,841行を返す。埋め草は0。
    pub fn canonical_counts(&self) -> &[u64] {
        &self.canonical_counts
    }

    /// 両視点を鏡映なしで数えた入力特徴D行を返す。P型では基底特徴の番号順。
    pub fn input_counts(&self) -> &[u64] {
        &self.input_counts
    }

    /// P型の第1層で使う正準行のマスクを返す。埋め草はfalse。
    pub fn canonical_mask(&self) -> Vec<bool> {
        self.canonical_counts
            .iter()
            .map(|&count| count != 0)
            .collect()
    }

    /// 入力特徴D行と埋め草1行の観測マスクを返す。
    pub fn input_mask(&self) -> Vec<bool> {
        self.input_counts
            .iter()
            .map(|&count| count != 0)
            .chain(std::iter::once(false))
            .collect()
    }
}
