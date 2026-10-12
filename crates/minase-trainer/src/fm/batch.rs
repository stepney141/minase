//! 手番側の全特徴番号を持つFMバッチ。鏡映しても行を共有しない。

use super::Result;
use crate::data::{Dataset, build_targets_from_arrays};
use minase::{
    eval::{
        features::{FEATURE_COUNT, feature_index, lion_feature_index, mirror_feature},
        pst::{Pst, evaluate_pst_features},
    },
    training::records::{Outcome, Record},
};
use minase_core::Square;

/// 元の盤面の144列と先獅子の1列。
pub const FEATURE_COLUMNS: usize = 145;
/// 空升および先獅子がない列の埋め草。
pub const PADDING: u32 = FEATURE_COUNT as u32;

pub(crate) fn check_size(size: usize) -> Result<()> {
    if size == 0
        || size
            .checked_mul(FEATURE_COLUMNS * size_of::<u32>())
            .is_none_or(|n| n > isize::MAX as usize)
    {
        return Err("FM batch must be nonempty and fit in a feature array".into());
    }
    Ok(())
}

pub(crate) fn features(record: &Record, mirror: bool) -> Result<([u32; FEATURE_COLUMNS], u32)> {
    let position = record.to_position()?;
    let side = position.side_to_move();
    let mut columns = [PADDING; FEATURE_COLUMNS];
    let mut count = 0;
    for square in Square::all() {
        if let Some(piece) = position.piece_at(square) {
            let feature = feature_index(side, piece, square);
            let col = square.dense_index();
            let (col, feature) = if mirror {
                (col / 12 * 12 + 11 - col % 12, mirror_feature(feature))
            } else {
                (col, feature)
            };
            columns[col] = feature as u32;
            count += 1;
        }
    }
    if let Some(trigger) = position.lion_taken_by_non_lion() {
        let feature = lion_feature_index(side, trigger.square);
        columns[144] = if mirror {
            mirror_feature(feature)
        } else {
            feature
        } as u32;
    }
    Ok((columns, count))
}

/// 鏡映後の特徴、固定した整数PSTの評価、教師値を局面順に保持する。
#[derive(Debug, PartialEq)]
pub struct Batch {
    pub(super) columns: Vec<[u32; FEATURE_COLUMNS]>,
    pub(super) baseline: Vec<i32>,
    pub(super) targets: Vec<f32>,
}
impl Batch {
    /// 再生またはハッシュで決めた鏡映ビットを、指定順の局面へ適用する。
    pub fn from_indices(
        dataset: &Dataset,
        base: &Pst,
        indices: &[u64],
        mirror: &[u8],
        ks: &[f64],
    ) -> Result<Self> {
        let records = indices
            .iter()
            .map(|&i| dataset.record(i))
            .collect::<crate::data::Result<Vec<_>>>()?;
        let scores = indices
            .iter()
            .map(|&i| dataset.teacher_score(i))
            .collect::<crate::data::Result<Vec<_>>>()?;
        let classes = indices
            .iter()
            .map(|&i| dataset.teacher_class(i))
            .collect::<crate::data::Result<Vec<_>>>()?;
        let outcomes: Vec<_> = records
            .iter()
            .map(|r| match r.outcome() {
                Outcome::Loss => 0,
                Outcome::Draw => 1,
                Outcome::Win => 2,
            })
            .collect();
        let targets =
            build_targets_from_arrays(&scores, &classes, &outcomes, ks, dataset.teacher_lambdas())?;
        Self::from_records(base, &records, mirror, &targets)
    }

    /// 読み込み済み局面と教師値を使い、鏡映後の特徴から整数PSTも計算する。
    pub fn from_records(
        base: &Pst,
        records: &[Record],
        mirror: &[u8],
        targets: &[f32],
    ) -> Result<Self> {
        check_size(records.len())?;
        if records.len() != mirror.len()
            || records.len() != targets.len()
            || mirror.iter().any(|&b| b > 1)
            || targets
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err("FM batch mirror or target shape/range".into());
        }
        let mut batch = Self {
            columns: Vec::with_capacity(records.len()),
            baseline: Vec::with_capacity(records.len()),
            targets: targets.to_vec(),
        };
        for (record, &mirror) in records.iter().zip(mirror) {
            let (columns, count) = features(record, mirror != 0)?;
            batch.baseline.push(evaluate_pst_features(
                base,
                columns
                    .iter()
                    .filter(|&&f| f != PADDING)
                    .map(|&f| f as usize),
                count,
            ));
            batch.columns.push(columns);
        }
        Ok(batch)
    }
    /// 局面数。
    pub fn len(&self) -> usize {
        self.columns.len()
    }
    /// 構築したバッチは常に非空。
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }
    /// 盤面の升順で並ぶ[B,145]の全特徴番号。
    pub fn columns(&self) -> &[[u32; FEATURE_COLUMNS]] {
        &self.columns
    }
    /// 鏡映後の整数PST評価、センチポーン。
    pub fn baseline(&self) -> &[i32] {
        &self.baseline
    }
}

/// 訓練集合とその全左右鏡映だけから全特徴の出現回数を数える。
pub fn feature_observations(dataset: &Dataset) -> Result<Vec<u64>> {
    let mut counts = vec![0; FEATURE_COUNT];
    for &index in dataset.training_indices() {
        let (columns, _) = features(&dataset.record(index)?, false)?;
        for feature in columns.into_iter().filter(|&f| f != PADDING) {
            counts[feature as usize] += 1;
            counts[mirror_feature(feature as usize)] += 1;
        }
    }
    Ok(counts)
}

pub(super) fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

/// SplitMix64の混合を2段使い、シード、エポック、MNSD通し番号から鏡映の1ビットを得る。
pub fn mirror_bit(seed: u64, epoch: u64, index: u64) -> u8 {
    (mix64(mix64(seed ^ epoch.wrapping_mul(0x9E37_79B9_7F4A_7C15)) ^ index) & 1) as u8
}
