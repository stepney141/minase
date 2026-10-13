use minase::training::records::{Outcome, Record};
use minase_core::{Color, Position};

use super::features::{base_features, canonical_row, input_features};
use super::{CANONICAL_ROWS, Error, FEATURE_COLUMNS, InputFeatureSet, Result, SENTINEL};
use crate::data::{Dataset, build_targets_from_arrays, outcome_code};

/// バッチの教師値を[`build_targets_from_arrays`]の混合式で構築する。
pub fn build_targets(
    batch: &BatchSlot,
    teacher_ks: &[f64],
    teacher_lambdas: &[f64],
) -> Result<Vec<f32>> {
    build_targets_from_arrays(
        batch.teacher_scores(),
        batch.teacher_classes(),
        batch.outcomes(),
        teacher_ks,
        teacher_lambdas,
    )
    .map_err(Into::into)
}

struct Sample {
    position: Position,
    perspective: Color,
    score: f32,
    class: u32,
    outcome: Outcome,
}

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
/// GPUの後退で1区分が処理する逆索引要素数の上限。
pub const REVERSE_INDEX_SEGMENT_LEN: usize = 128;

/// r行e要素の逆索引の最大区分数`min(r,e) + floor((e−min(r,e))/128)`を返す。
/// 各非空行の最初の区分には1要素、後続の区分には追加の128要素を要する。
/// 最大バッチ数B_maxに対して、第1層のeは290×B_max、線形のeは145×B_maxである。
pub(crate) fn max_segment_count(rows: usize, entries: usize) -> usize {
    let nonempty = rows.min(entries);
    nonempty + (entries - nonempty) / REVERSE_INDEX_SEGMENT_LEN
}

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
#[derive(Debug, PartialEq, Eq)]
pub struct ReverseIndex {
    starts: Vec<u32>,
    entries: Vec<u32>,
    segment_starts: Vec<u32>,
    segment_offsets: Vec<u32>,
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

    /// 各行の区分番号の開始位置[行数＋1]。空行では次の行と同じ値を持つ。
    pub fn segment_starts(&self) -> &[u32] {
        &self.segment_starts
    }

    /// 区分ごとの逆索引の開始位置[区分数＋1]。最終要素は全逆索引要素数。
    /// 各区分は1行の範囲内にあり、長さは`REVERSE_INDEX_SEGMENT_LEN`以下。
    pub fn segment_offsets(&self) -> &[u32] {
        &self.segment_offsets
    }

    /// 空行を含めずに作った区分の数。
    pub fn segment_count(&self) -> usize {
        self.segment_offsets.len() - 1
    }

    /// 安定な計数ソートで行番号順に並べ、行内は局面、視点、列の昇順を保つ。
    /// 1行の中を[`REVERSE_INDEX_SEGMENT_LEN`]の128要素以下に区切り、空行には区分を作らない。
    /// この固定順を後退の縮約順として使う。
    /// 理由は`docs/adr/backpropagate-the-first-layer-through-a-reverse-index.md`にある。
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
        let mut segment_starts = Vec::with_capacity(rows + 1);
        let mut segment_offsets = Vec::with_capacity(max_segment_count(rows, entries.len()) + 1);
        segment_starts.push(0);
        segment_offsets.push(0);
        for range in starts.windows(2) {
            let mut offset = range[0];
            while offset < range[1] {
                offset = (offset + REVERSE_INDEX_SEGMENT_LEN as u32).min(range[1]);
                segment_offsets.push(offset);
            }
            segment_starts.push((segment_offsets.len() - 1) as u32);
        }
        Ok(Self {
            starts,
            entries,
            segment_starts,
            segment_offsets,
        })
    }
}

/// CPUで構築し、そのまま転送できる1バッチの面。配列はいずれも連続したVec。
///
/// 局面数をBとすると、`features`は`[B, 2, 145]`、`linear_features`は`[B, 145]`。
/// 局面、視点（手番側0、相手側1）、元の盤面のdense升の順で行優先に並ぶ。
/// 列144は先獅子である。残る局面ごとの配列はすべて`[B]`。
/// 第1層と線形の逆索引は各々[行数＋1]の開始位置と、発火数個のu32を持つ。
/// 後退用の区分表もワーカーが構築し、行・逆索引要素とも昇順を保つ。
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
    /// バッチ順の通し番号`[B]`を返す。
    pub fn record_indices(&self) -> &[u64] {
        &self.record_indices
    }
    /// 両視点の特徴`[B, 2, 145]`。P型は正準行×2＋回転、HalfKPは入力特徴番号。
    pub fn features(&self) -> &[u32] {
        &self.features
    }
    /// 手番側の線形経路の正準行番号`[B, 145]`を返す。
    pub fn linear_features(&self) -> &[u32] {
        &self.linear_features
    }
    /// 先獅子の枠を含まない盤上総駒数`[B]`を返す。
    pub fn piece_counts(&self) -> &[u8] {
        &self.piece_counts
    }
    /// 先読み後の教師探索値`[B]`を返す。単位はセンチポーン。
    pub fn teacher_scores(&self) -> &[f32] {
        &self.teacher_scores
    }
    /// 教師用Kを引く分類番号`[B]`を返す。
    pub fn teacher_classes(&self) -> &[u32] {
        &self.teacher_classes
    }
    /// 手番側から見た結果`[B]`を返す。負け0、引き分け1、勝ち2。
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
            return Err(crate::data::Error::TeacherSamples.into());
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
        Self::assemble_positions(
            feature_set,
            indices,
            samples.map(|sample| {
                let (record, score, class) = sample?;
                Ok(Sample {
                    position: record.to_position()?,
                    perspective: record.side_to_move(),
                    score,
                    class,
                    outcome: record.outcome(),
                })
            }),
        )
    }

    // 数値診断では配置の内部手番とは独立に評価視点を指定する。特徴抽出と逆索引は学習と共有する。
    pub(crate) fn from_evaluation_positions(
        feature_set: InputFeatureSet,
        positions: &[Position],
        perspective: Color,
    ) -> Result<Self> {
        let indices: Vec<_> = (0..positions.len() as u64).collect();
        Self::assemble_positions(
            feature_set,
            &indices,
            positions.iter().map(|position| {
                Ok(Sample {
                    position: position.clone(),
                    perspective,
                    score: 0.0,
                    class: 0,
                    outcome: Outcome::Draw,
                })
            }),
        )
    }

    fn assemble_positions(
        feature_set: InputFeatureSet,
        indices: &[u64],
        samples: impl Iterator<Item = Result<Sample>>,
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
            let sample = sample?;
            let bases = base_features(&sample.position, sample.perspective);
            let inputs = input_features(&sample.position, &bases, feature_set, sample.perspective);
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
            teacher_scores.push(sample.score);
            teacher_classes.push(sample.class);
            outcomes.push(outcome_code(sample.outcome));
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
