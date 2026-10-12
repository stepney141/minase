//! 手番側の鏡映共有行と教師情報だけを持つPST専用バッチ。

use super::{Error, Result, model::canonical};
use crate::data::Dataset;
use minase::{
    eval::features::{feature_index, lion_feature_index},
    training::records::{Outcome, Record},
};
use minase_core::Square;

/// 元の盤面の144升と先獅子の1列。
pub const FEATURE_COLUMNS: usize = 145;
/// 空升または先獅子がない列。
pub const SENTINEL: u32 = u32::MAX;

pub(super) fn check_size(size: usize) -> Result<()> {
    if size == 0
        || size
            .checked_mul(FEATURE_COLUMNS * size_of::<u32>())
            .is_none_or(|bytes| bytes > isize::MAX as usize)
    {
        return Err(Error::Invalid(
            "PST batch must be nonempty and fit in a feature array",
        ));
    }
    Ok(())
}

/// 手番側の正準行[B,145]、除去前後の位相分子[B,2]と教師値。
#[derive(Debug, PartialEq)]
pub struct Batch {
    columns: Vec<u32>,
    phase_numerators: Vec<[u8; 2]>,
    teacher_scores: Vec<f32>,
    teacher_classes: Vec<u32>,
    outcomes: Vec<u8>,
}
impl Batch {
    /// 記録を指定順に復元する。特徴は元の盤面の升順で、列144が先獅子。
    pub fn from_indices(dataset: &Dataset, indices: &[u64]) -> Result<Self> {
        Self::assemble(
            indices.len(),
            indices.iter().map(|&index| {
                Ok((
                    dataset.record(index)?,
                    dataset.teacher_score(index)?,
                    dataset.teacher_class(index)?,
                ))
            }),
        )
    }

    /// 読み込み済みの記録、有限な探索値、分類番号からバッチを作る。
    pub fn from_records(records: &[Record], scores: &[f32], classes: &[u32]) -> Result<Self> {
        if records.len() != scores.len() || records.len() != classes.len() {
            return Err(Error::Invalid("PST teacher array lengths differ"));
        }
        Self::assemble(
            records.len(),
            records
                .iter()
                .zip(scores)
                .zip(classes)
                .map(|((record, &score), &class)| Ok((record.clone(), score, class))),
        )
    }

    fn assemble(
        len: usize,
        records: impl Iterator<Item = Result<(Record, f32, u32)>>,
    ) -> Result<Self> {
        check_size(len)?;
        let mut batch = Self {
            columns: Vec::with_capacity(len * FEATURE_COLUMNS),
            phase_numerators: Vec::with_capacity(len),
            teacher_scores: Vec::with_capacity(len),
            teacher_classes: Vec::with_capacity(len),
            outcomes: Vec::with_capacity(len),
        };
        for sample in records {
            let (record, score, class) = sample?;
            if !score.is_finite() {
                return Err(Error::Invalid("non-finite PST teacher score"));
            }
            let position = record.to_position()?;
            let side = record.side_to_move();
            let mut columns = [SENTINEL; FEATURE_COLUMNS];
            let mut count = 0_u8;
            for square in Square::all() {
                if let Some(piece) = position.piece_at(square) {
                    columns[square.dense_index()] =
                        canonical(feature_index(side, piece, square)) as u32;
                    count += 1;
                }
            }
            if let Some(trigger) = position.lion_taken_by_non_lion() {
                columns[144] = canonical(lion_feature_index(side, trigger.square)) as u32;
            }
            batch.columns.extend(columns);
            batch.phase_numerators.push([
                count.saturating_sub(2).min(90),
                count.saturating_sub(3).min(90),
            ]);
            batch.teacher_scores.push(score);
            batch.teacher_classes.push(class);
            batch.outcomes.push(match record.outcome() {
                Outcome::Loss => 0,
                Outcome::Draw => 1,
                Outcome::Win => 2,
            });
        }
        Ok(batch)
    }
    /// バッチの局面数。
    pub fn len(&self) -> usize {
        self.outcomes.len()
    }
    /// バッチが空かを返す。構築済みのバッチは常に非空。
    pub fn is_empty(&self) -> bool {
        self.outcomes.is_empty()
    }
    /// 元の升順に並ぶ手番側の正準行[B,145]。
    pub fn columns(&self) -> &[u32] {
        &self.columns
    }
    /// 除去前後の補間分子[B,2]。分母はいずれも90。
    pub fn phase_numerators(&self) -> &[[u8; 2]] {
        &self.phase_numerators
    }
    /// 先読み後の教師探索値、センチポーン。
    pub fn teacher_scores(&self) -> &[f32] {
        &self.teacher_scores
    }
    /// 教師用Kとλの分類番号。
    pub fn teacher_classes(&self) -> &[u32] {
        &self.teacher_classes
    }
    /// 手番側の負け0、引き分け1、勝ち2。
    pub fn outcomes(&self) -> &[u8] {
        &self.outcomes
    }
    pub(super) fn columns_at(&self, index: usize) -> &[u32] {
        &self.columns[index * FEATURE_COLUMNS..(index + 1) * FEATURE_COLUMNS]
    }
}
