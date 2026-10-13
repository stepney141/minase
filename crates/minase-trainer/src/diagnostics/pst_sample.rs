//! 教師分類と位相帯ごとの検証標本の抽出とNPYへの書き出し。
//! 位相帯は補間係数φを5等分した区間であり、境界は[`band`]に定める。
//! 分類と帯ごとの件数と抽出は[`select`]、保存形式は[`write_indices`]に従う。

use super::{Result, pst_reference::Features};
use crate::{
    data::Dataset,
    nn::npz::{self, Array},
};
use std::path::Path;

pub(super) const BANDS: usize = 5;
/// [`Features`]の位相分子qから、φ = q/90を5等分した帯番号を返す。
/// `band(q) = min(⌊q×5/90⌋, 4)`であり、帯0〜3は右端を含まず、帯4だけ右端φ=1を含む。
pub(super) fn band(q: u8) -> usize {
    (usize::from(q) * BANDS / 90).min(BANDS - 1)
}
/// 分類と帯の標本ファイル名`diagnostic-indices-generation{分類番号}-band{帯番号}.npy`を返す。
pub(super) fn filename(generation: usize, band: usize) -> String {
    format!("diagnostic-indices-generation{generation}-band{band}.npy")
}
pub(super) struct Sampling {
    pub training: Vec<[usize; BANDS]>,
    pub validation: Vec<[usize; BANDS]>,
    pub indices: Vec<[Vec<u64>; BANDS]>,
}

struct SplitMix64(u64);
impl SplitMix64 {
    fn below(&mut self, bound: u64) -> u64 {
        let threshold = bound.wrapping_neg() % bound;
        loop {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut value = self.0;
            value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            value ^= value >> 31;
            if value >= threshold {
                return value % bound;
            }
        }
    }
}

/// 訓練集合と検証集合の全局面を分類と帯で数え、検証集合から非復元抽出する。
/// `size`は`--sample-size`の値で、0を拒否する。
/// `seed`で初期化した1本のSplitMix64の乱数列を分類順、帯順に使い、各母集団から
/// `min(size, 母集団の大きさ)`件を部分Fisher–Yatesで抽出する。
/// `below`は棄却法で剰余の偏りを除き、抽出した通算番号は昇順に並べる。
pub(super) fn select(dataset: &Dataset, size: usize, seed: u64) -> Result<Sampling> {
    if size == 0 {
        return Err("diagnostic sample size must be positive".into());
    }
    let classes = dataset.teacher_classes().len();
    let mut samples = Sampling {
        training: vec![[0; BANDS]; classes],
        validation: vec![[0; BANDS]; classes],
        indices: (0..classes)
            .map(|_| std::array::from_fn(|_| Vec::new()))
            .collect(),
    };
    for &index in dataset.training_indices() {
        let class = dataset.teacher_class(index)? as usize;
        let q = Features::new(&dataset.record(index)?).q;
        samples.training[class][band(q)] += 1;
    }
    for &index in dataset.validation_indices() {
        let class = dataset.teacher_class(index)? as usize;
        let q = Features::new(&dataset.record(index)?).q;
        samples.indices[class][band(q)].push(index);
    }
    let mut rng = SplitMix64(seed);
    for (generation, pools) in samples.indices.iter_mut().enumerate() {
        for (band, pool) in pools.iter_mut().enumerate() {
            samples.validation[generation][band] = pool.len();
            let count = size.min(pool.len());
            // 分類、帯の順に1本の乱数列で非復元抽出する。
            for start in 0..count {
                let selected = start + rng.below((pool.len() - start) as u64) as usize;
                pool.swap(start, selected);
            }
            pool.truncate(count);
            pool.sort_unstable();
        }
    }
    Ok(samples)
}

/// 標本の通算番号をdtype `<i8`、形`[標本数]`のNPY 1.0として新規作成する。
/// 番号は入力を連結した0始まりの番号をリトルエンディアンi64で保存し、空の帯にも長さ0の配列を書く。
/// ファイル名は[`filename`]に従う。
pub(super) fn write_indices(path: &Path, indices: &[u64]) -> Result<()> {
    let mut data = Vec::with_capacity(indices.len() * 8);
    for &index in indices {
        data.extend(i64::try_from(index)?.to_le_bytes());
    }
    super::pst_positions::write_new(
        path,
        &npz::encode_npy(&Array {
            dtype: "<i8".into(),
            shape: vec![indices.len()],
            data,
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phase_intervals_include_only_the_final_right_endpoint() {
        for (q, expected) in [
            (0, 0),
            (17, 0),
            (18, 1),
            (35, 1),
            (36, 2),
            (53, 2),
            (54, 3),
            (71, 3),
            (72, 4),
            (90, 4),
        ] {
            assert_eq!(band(q), expected);
        }
    }
}
