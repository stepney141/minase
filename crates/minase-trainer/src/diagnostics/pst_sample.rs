//! 教師分類・位相帯ごとの検証標本と、照合用NPYの入出力。

use super::{Result, pst_reference::Features};
use crate::{
    data::Dataset,
    nn::npz::{self, Array},
};
use std::{fs, path::Path};

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

pub(super) fn select(
    dataset: &Dataset,
    size: usize,
    seed: u64,
    replay: Option<&Path>,
) -> Result<Sampling> {
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
            if let Some(directory) = replay {
                let path = directory.join(filename(generation, band));
                let chosen = read_indices(&path)?;
                validate_replay(&chosen, pool, count)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                *pool = chosen;
            } else {
                // 分類、帯の順に1本の乱数列で非復元抽出する。
                for start in 0..count {
                    let selected = start + rng.below((pool.len() - start) as u64) as usize;
                    pool.swap(start, selected);
                }
                pool.truncate(count);
                pool.sort_unstable();
            }
        }
    }
    Ok(samples)
}

fn read_indices(path: &Path) -> Result<Vec<u64>> {
    let array = npz::decode_npy(&fs::read(path)?)?;
    if array.dtype != "<i8" || array.shape.len() != 1 {
        return Err("replay sample must be a one-dimensional little-endian i64 NPY array".into());
    }
    array
        .data
        .as_chunks::<8>()
        .0
        .iter()
        .map(|&bytes| {
            u64::try_from(i64::from_le_bytes(bytes))
                .map_err(|_| "negative replay sample index".into())
        })
        .collect()
}

fn validate_replay(chosen: &[u64], pool: &[u64], count: usize) -> Result<()> {
    if chosen.len() != count {
        return Err("replay sample count differs from min(sample-size, band population)".into());
    }
    if chosen.windows(2).any(|w| w[0] >= w[1]) {
        return Err("replay sample indices must be sorted and unique".into());
    }
    if chosen
        .iter()
        .any(|index| pool.binary_search(index).is_err())
    {
        return Err(
            "replay sample index is outside its validation teacher class and phase band".into(),
        );
    }
    Ok(())
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
    fn replay_requires_the_prescribed_count_and_unique_membership() {
        assert!(validate_replay(&[2, 4], &[2, 4, 8], 2).is_ok());
        assert!(validate_replay(&[], &[], 0).is_ok());
        for chosen in [&[2, 2][..], &[4, 2], &[2, 9], &[2], &[]] {
            assert!(validate_replay(chosen, &[2, 4, 8], 2).is_err());
        }
    }
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
