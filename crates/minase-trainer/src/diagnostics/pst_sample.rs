//! 教師分類と位相帯ごとの検証標本の抽出とNPYへの書き出し。

use super::{Result, pst_reference::Features};
use crate::{
    data::Dataset,
    nn::npz::{self, Array},
};
use std::path::Path;

pub(super) const BANDS: usize = 5;
pub(super) fn band(q: u8) -> usize {
    (usize::from(q) * BANDS / 90).min(BANDS - 1)
}
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
