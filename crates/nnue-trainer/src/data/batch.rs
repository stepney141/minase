use minase::training::records::Record;
use serde::Serialize;

use super::features::{base_features, canonical_row, input_features};
use super::mnsd::outcome_code;
use super::{CANONICAL_ROWS, Dataset, Error, FEATURE_COLUMNS, InputFeatureSet, Result, SENTINEL};

/// 回転欄のビット幅。
pub const ROTATION_BITS: u32 = 1;
/// 列欄のビット幅。
pub const COLUMN_BITS: u32 = 8;
/// 視点欄のビット幅。
pub const PERSPECTIVE_BITS: u32 = 1;
/// 局面番号欄のビット幅。
pub const POSITION_BITS: u32 = 32 - ROTATION_BITS - COLUMN_BITS - PERSPECTIVE_BITS;
/// 回転欄の最下位ビット位置。
pub const ROTATION_SHIFT: u32 = 0;
/// 列欄の最下位ビット位置。
pub const COLUMN_SHIFT: u32 = ROTATION_SHIFT + ROTATION_BITS;
/// 視点欄の最下位ビット位置。
pub const PERSPECTIVE_SHIFT: u32 = COLUMN_SHIFT + COLUMN_BITS;
/// 局面番号欄の最下位ビット位置。
pub const POSITION_SHIFT: u32 = PERSPECTIVE_SHIFT + PERSPECTIVE_BITS;
/// 逆索引の1語で表現できるバッチ局面数の上限。
pub const MAX_BATCH_SIZE: usize = 1 << POSITION_BITS;

/// 逆索引の1要素。上位から局面22、視点1、列8、回転1ビットで保持する。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackedOccurrence(u32);

impl PackedOccurrence {
    /// 各欄の範囲を検査して1語に詰める。視点0が手番側、1が相手側。
    pub fn new(position: usize, perspective: u8, column: u8, rotated: bool) -> Result<Self> {
        if position >= MAX_BATCH_SIZE || perspective >= 2 || column as usize >= FEATURE_COLUMNS {
            return Err(Error::PackedOccurrence);
        }
        Ok(Self(
            (position as u32) << POSITION_SHIFT
                | u32::from(perspective) << PERSPECTIVE_SHIFT
                | u32::from(column) << COLUMN_SHIFT
                | u32::from(rotated) << ROTATION_SHIFT,
        ))
    }

    /// 逆索引の生の1語を読み、列の定義域を検査する。
    pub fn from_word(word: u32) -> Result<Self> {
        let entry = Self(word);
        if entry.column() as usize >= FEATURE_COLUMNS {
            return Err(Error::PackedOccurrence);
        }
        Ok(entry)
    }

    /// GPUへ転送するu32の表現を返す。
    pub fn word(self) -> u32 {
        self.0
    }

    /// バッチ内の局面番号を返す。
    pub fn position(self) -> usize {
        (self.0 >> POSITION_SHIFT) as usize
    }

    /// 視点を返す。手番側0、相手側1。
    pub fn perspective(self) -> u8 {
        ((self.0 >> PERSPECTIVE_SHIFT) & ((1 << PERSPECTIVE_BITS) - 1)) as u8
    }

    /// 元の盤面のdense升、または先獅子の列144を返す。
    pub fn column(self) -> u8 {
        ((self.0 >> COLUMN_SHIFT) & ((1 << COLUMN_BITS) - 1)) as u8
    }

    /// P型の第1層で半幅回転するかどうかを返す。
    pub fn rotated(self) -> bool {
        (self.0 >> ROTATION_SHIFT) & 1 != 0
    }
}

/// 計数ソートで構築した行ごとの逆索引。埋め草・番兵の要素は含まない。
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ReverseIndex {
    starts: Vec<u32>,
    entries: Vec<u32>,
}

impl ReverseIndex {
    /// 行数＋1個の開始位置。最終要素は全要素数である。
    pub fn starts(&self) -> &[u32] {
        &self.starts
    }

    /// 行ごとに局面番号、視点、列の昇順に並べた`PackedOccurrence`の生の1語。
    pub fn entries(&self) -> &[u32] {
        &self.entries
    }

    fn build(features: &[u32], rows: usize, perspectives: usize, p_encoded: bool) -> Result<Self> {
        let row_of = |feature: u32| {
            if p_encoded {
                feature as usize / 2
            } else {
                feature as usize
            }
        };
        let mut starts = vec![0_u32; rows + 1];
        for &feature in features {
            if feature != SENTINEL {
                starts[row_of(feature) + 1] += 1;
            }
        }
        // MAX_BATCH_SIZE × 2 × 145 < u32::MAXなので開始位置はu32に収まる。
        for row in 0..rows {
            starts[row + 1] += starts[row];
        }
        let mut entries = vec![0; starts[rows] as usize];
        let mut next = starts[..rows].to_vec();
        // 元配列は局面→視点→列の順。安定な計数ソートで縮約順を固定する。
        for (index, &feature) in features.iter().enumerate() {
            if feature == SENTINEL {
                continue;
            }
            let position = index / (perspectives * FEATURE_COLUMNS);
            let perspective = (index / FEATURE_COLUMNS % perspectives) as u8;
            let column = (index % FEATURE_COLUMNS) as u8;
            let rotated = p_encoded && feature & 1 != 0;
            let occurrence = PackedOccurrence::new(position, perspective, column, rotated)?;
            let row = row_of(feature);
            entries[next[row] as usize] = occurrence.word();
            next[row] += 1;
        }
        Ok(Self { starts, entries })
    }
}

/// CPUで構築し、そのまま転送できる1バッチの面。配列はいずれも連続したVec。
///
/// 局面数をBとすると、`features`は[B, 2, 145]、`linear_features`は[B, 145]。
/// 局面、視点（手番側0、相手側1）、元の盤面のdense升の順で行優先に並ぶ。
/// 列144は先獅子である。残る局面ごとの配列はすべて[B]。
/// 第1層と線形の逆索引は各々[行数＋1]の開始位置と、発火数個のu32を持つ。
#[derive(Debug, PartialEq)]
pub struct BatchSlot {
    feature_set: InputFeatureSet,
    record_indices: Vec<u64>,
    features: Vec<u32>,
    linear_features: Vec<u32>,
    piece_counts: Vec<u8>,
    teacher_scores: Vec<f32>,
    teacher_classes: Vec<u32>,
    outcomes: Vec<u8>,
    first_layer_inverse: ReverseIndex,
    linear_inverse: ReverseIndex,
}

impl BatchSlot {
    /// この面の入力特徴集合を返す。
    pub fn feature_set(&self) -> InputFeatureSet {
        self.feature_set
    }
    /// 局面数Bを返す。
    pub fn len(&self) -> usize {
        self.record_indices.len()
    }
    /// 局面がないかどうかを返す。構築済みの面は常に非空。
    pub fn is_empty(&self) -> bool {
        self.record_indices.is_empty()
    }
    /// バッチ順の通し番号[B]を返す。
    pub fn record_indices(&self) -> &[u64] {
        &self.record_indices
    }
    /// 両視点の特徴[B, 2, 145]。P型は正準行×2＋回転、HalfKPは入力特徴番号。
    pub fn features(&self) -> &[u32] {
        &self.features
    }
    /// 手番側の線形経路の正準行番号[B, 145]を返す。
    pub fn linear_features(&self) -> &[u32] {
        &self.linear_features
    }
    /// 先獅子の枠を含まない盤上総駒数[B]を返す。
    pub fn piece_counts(&self) -> &[u8] {
        &self.piece_counts
    }
    /// 先読み後の教師探索値[B]を返す。単位はセンチポーン。
    pub fn teacher_scores(&self) -> &[f32] {
        &self.teacher_scores
    }
    /// 教師用Kを引く分類番号[B]を返す。
    pub fn teacher_classes(&self) -> &[u32] {
        &self.teacher_classes
    }
    /// 手番側から見た結果[B]を返す。負け0、引き分け1、勝ち2。
    pub fn outcomes(&self) -> &[u8] {
        &self.outcomes
    }
    /// 両視点の第1層用逆索引を返す。
    pub fn first_layer_inverse(&self) -> &ReverseIndex {
        &self.first_layer_inverse
    }
    /// 手番側の線形経路用逆索引を返す。
    pub fn linear_inverse(&self) -> &ReverseIndex {
        &self.linear_inverse
    }

    fn build(dataset: &Dataset, feature_set: InputFeatureSet, indices: &[u64]) -> Result<Self> {
        Self::assemble(
            feature_set,
            indices,
            indices.iter().map(|&index| {
                Ok((
                    dataset.record(index)?,
                    dataset.teacher_score(index)?,
                    dataset.teacher_class(index)?,
                ))
            }),
        )
    }

    /// 合成記録または読み込み済み記録から面を作る。通し番号は記録の添字とする。
    /// 教師値と分類は記録と同数で、教師値は有限でなければならない。
    pub fn from_records(
        feature_set: InputFeatureSet,
        records: &[Record],
        teacher_scores: &[f32],
        teacher_classes: &[u32],
    ) -> Result<Self> {
        if records.len() != teacher_scores.len()
            || records.len() != teacher_classes.len()
            || teacher_scores.iter().any(|s| !s.is_finite())
        {
            return Err(Error::TeacherSamples);
        }
        let indices: Vec<u64> = (0..records.len() as u64).collect();
        Self::assemble(
            feature_set,
            &indices,
            records
                .iter()
                .zip(teacher_scores)
                .zip(teacher_classes)
                .map(|((record, &score), &class)| Ok((record.clone(), score, class))),
        )
    }

    fn assemble(
        feature_set: InputFeatureSet,
        indices: &[u64],
        samples: impl Iterator<Item = Result<(Record, f32, u32)>>,
    ) -> Result<Self> {
        if indices.is_empty() || indices.len() > MAX_BATCH_SIZE {
            return Err(Error::BatchSize {
                size: indices.len(),
            });
        }
        let mut features = Vec::with_capacity(indices.len() * 2 * FEATURE_COLUMNS);
        let mut linear_features = Vec::with_capacity(indices.len() * FEATURE_COLUMNS);
        let mut piece_counts = Vec::with_capacity(indices.len());
        let mut teacher_scores = Vec::with_capacity(indices.len());
        let mut teacher_classes = Vec::with_capacity(indices.len());
        let mut outcomes = Vec::with_capacity(indices.len());
        for sample in samples {
            let (record, score, class) = sample?;
            let position = record.to_position()?;
            let bases = base_features(&position);
            let inputs = input_features(&position, &bases, feature_set);
            features.extend(inputs.into_iter().flatten());
            linear_features.extend(bases[0].iter().map(|&base| {
                if base == SENTINEL {
                    SENTINEL
                } else {
                    canonical_row(base)
                }
            }));
            piece_counts.push(
                bases[0][..144]
                    .iter()
                    .filter(|&&base| base != SENTINEL)
                    .count() as u8,
            );
            teacher_scores.push(score);
            teacher_classes.push(class);
            outcomes.push(outcome_code(record.outcome()));
        }
        let first_layer_inverse = ReverseIndex::build(
            &features,
            feature_set.first_layer_rows(),
            2,
            feature_set == InputFeatureSet::P,
        )?;
        let linear_inverse = ReverseIndex::build(&linear_features, CANONICAL_ROWS, 1, false)?;
        Ok(Self {
            feature_set,
            record_indices: indices.to_vec(),
            features,
            linear_features,
            piece_counts,
            teacher_scores,
            teacher_classes,
            outcomes,
            first_layer_inverse,
            linear_inverse,
        })
    }
}

/// バッチごとにCPUワーカーを起動し、完了順にかかわらず指定順の面を返す。
///
/// 各要素はエポックの順列などから切り出した通し番号の範囲。呼び手は同時に必要な
/// 面の数だけ渡す。ワーカーが復号、特徴抽出、計数ソートをすべて担当する。
/// 全ワーカーは成功・失敗のどちらでもこの関数を抜ける前に回収する。
pub fn build_batches(
    dataset: &Dataset,
    feature_set: InputFeatureSet,
    batches: &[&[u64]],
) -> Result<Vec<BatchSlot>> {
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(batches.len());
        let mut spawn_error = None;
        for &indices in batches {
            match std::thread::Builder::new().spawn_scoped(scope, move || {
                BatchSlot::build(dataset, feature_set, indices)
            }) {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    spawn_error = Some(Error::Io(error));
                    break;
                }
            }
        }
        // collect::<Result<_>>()で途中終了せず、パニックしたワーカーもすべてjoinする。
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| Error::WorkerPanicked)
                    .and_then(|result| result)
            })
            .collect();
        if let Some(error) = spawn_error {
            return Err(error);
        }
        results.into_iter().collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occurrence_word_round_trips_all_fields_and_limits() {
        for position in [0, 16383, 16384, MAX_BATCH_SIZE - 1] {
            for view in 0..2 {
                for column in 0..FEATURE_COLUMNS as u8 {
                    for rotation in [false, true] {
                        let entry =
                            PackedOccurrence::new(position, view, column, rotation).unwrap();
                        assert_eq!(PackedOccurrence::from_word(entry.word()).unwrap(), entry);
                        assert_eq!(
                            (
                                entry.position(),
                                entry.perspective(),
                                entry.column(),
                                entry.rotated()
                            ),
                            (position, view, column, rotation)
                        );
                    }
                }
            }
        }
        // フィールド間の境界も、独立な固定語で検証する。
        assert_eq!(
            PackedOccurrence::new(16384, 1, 144, true).unwrap().word(),
            0x0100_0321
        );
        assert!(PackedOccurrence::new(MAX_BATCH_SIZE, 0, 0, false).is_err());
        assert!(PackedOccurrence::new(0, 2, 0, false).is_err());
        assert!(PackedOccurrence::new(0, 0, 145, false).is_err());
        assert!(PackedOccurrence::from_word(255 << COLUMN_SHIFT).is_err());
    }
}
