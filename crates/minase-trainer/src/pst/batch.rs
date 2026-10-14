//! 手番側の鏡映共有行と教師情報だけを持つPST専用バッチ。

use super::{Error, Result, model::canonical};
use crate::data::Dataset;
use crate::data::{Outcome, Sample};
use minase::eval::features::{feature_index, lion_feature_index};
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

/// 手番側の正準行`[B,145]`、除去前後の位相分子`[B,2]`と教師値。
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
    pub fn from_records(records: &[Sample], scores: &[f32], classes: &[u32]) -> Result<Self> {
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
        records: impl Iterator<Item = Result<(Sample, f32, u32)>>,
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
    /// 元の升順に並ぶ手番側の正準行`[B,145]`。
    pub fn columns(&self) -> &[u32] {
        &self.columns
    }
    /// 王駒を含む盤上総駒数nから、除去前の`q = min(max(n−2,0),90)`と除去後の`q′ = min(max(n−3,0),90)`を返す。
    /// 先獅子は数えず、両分子の分母は90である。
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
    /// 重みは鏡映対で共有するため、各段の列順だけを反転し、先獅子の列144は動かさない。
    pub(crate) fn reflect(&mut self, mirrors: &[u8]) {
        for (columns, &mirror) in self.columns.chunks_mut(FEATURE_COLUMNS).zip(mirrors) {
            if mirror != 0 {
                for rank in columns[..144].chunks_mut(12) {
                    rank.reverse();
                }
            }
        }
    }
    pub(crate) fn columns_at(&self, index: usize) -> &[u32] {
        &self.columns[index * FEATURE_COLUMNS..(index + 1) * FEATURE_COLUMNS]
    }
}
