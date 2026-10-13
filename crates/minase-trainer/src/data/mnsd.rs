use std::{
    collections::HashSet,
    fs::File,
    path::{Path, PathBuf},
};

use memmap2::{Mmap, MmapOptions};
use minase::training::records::{self, HEADER_LEN, Header, Origin, Outcome, RECORD_LEN, Record};
use serde::Serialize;

use super::{
    Error, Lookahead, Result, compute_lookahead, estimate_k, is_validation, validate_lambda,
};

/// 教師用Kを共有する分類。
///
/// 生成コミット（小文字）、重みの検査和、ノード上限、規則セット、由来、先読み条件、適用したλで区別する。
/// 入力順に番号を付け、記録数0のファイルは分類を作らない。
/// 教師を使う前に[`Dataset::with_lambda`]でλを設定し、教師を使わない診断では`lambda`は`None`である。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TeacherClass {
    pub(crate) generation_commit: String,
    pub(crate) network_checksum: [u8; 32],
    pub(crate) nodes: u32,
    pub(crate) rule_set: String,
    pub(crate) origin: Origin,
    pub(crate) lookahead: Option<Lookahead>,
    pub(crate) lambda: Option<f64>,
}

impl TeacherClass {
    /// 生成に使った評価関数の検査和を返す。
    pub fn network_checksum(&self) -> &[u8; 32] {
        &self.network_checksum
    }

    /// 教師探索のノード上限を返す。
    pub fn nodes(&self) -> u32 {
        self.nodes
    }

    /// 先読みの条件を返す。省略時は元の探索値を使う。
    pub fn lookahead(&self) -> Option<Lookahead> {
        self.lookahead
    }
}

/// 通し番号に対応するファイル番号とバイトオフセット。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordLocation {
    /// 入力したファイル列の0始まりの番号。
    pub file: usize,
    /// ヘッダを含むファイル先頭からのバイト数。
    pub byte_offset: usize,
}

struct MappedFile {
    path: PathBuf,
    map: Mmap,
    header: Header,
    class: u32,
    scores: Vec<f32>,
    outcomes: Vec<u8>,
}

fn invalid(path: &Path, reason: &'static str) -> Error {
    Error::Dataset {
        path: path.to_owned(),
        reason,
    }
}

impl MappedFile {
    unsafe fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let length = file.metadata()?.len();
        if length < HEADER_LEN as u64 {
            return Err(invalid(path, "MNSD header is shorter than 136 bytes"));
        }
        // SAFETY: 呼び手は、この呼び出し中と写像の生存中に元ファイルを変更・切り詰めしない。
        let map = unsafe { MmapOptions::new().map(&file)? };
        let header = Header::decode(&map[..HEADER_LEN])?;
        let expected = header
            .record_count()
            .checked_mul(RECORD_LEN as u64)
            .and_then(|bytes| bytes.checked_add(HEADER_LEN as u64))
            .ok_or(records::Error::FileLengthOverflow)?;
        if length != expected {
            return Err(records::Error::FileLengthMismatch {
                expected,
                actual: length,
            }
            .into());
        }

        Ok(Self {
            path: path.to_owned(),
            map,
            header,
            class: 0,
            scores: Vec::new(),
            outcomes: Vec::new(),
        })
    }

    fn record(&self, local: usize) -> Result<Record> {
        let start = HEADER_LEN + local * RECORD_LEN;
        Ok(Record::decode(&self.map[start..start + RECORD_LEN])?)
    }
}

/// 複数のMNSDをmmapで保持し、同じ通し番号で前処理とバッチを参照する。
///
/// 元ファイルはこの型の生存中に変更・切り詰めしないこと。盤面全体は保持せず、教師探索値
/// （f32）と結果（u8）、訓練・検証の通し番号を保持する。
pub struct Dataset {
    files: Vec<MappedFile>,
    offsets: Vec<u64>,
    training_indices: Vec<u64>,
    validation_indices: Vec<u64>,
    classes: Vec<TeacherClass>,
    lambdas: Vec<f64>,
}

impl Dataset {
    /// ファイル長と全記録を検査し、シードと対局番号で訓練・検証集合へ分ける。
    /// 先読み指定時は対局・手数の順序も検証する。
    /// 正規化したパスの重複、乱数開始の自己対局のシード重複、由来の混在、合成の入力を拒否する。
    /// 教師値を使う場合は、返されたデータへ[`Self::with_lambda`]で混合比を設定する。
    ///
    /// MNSDをmemmap2で写すため、呼び手に元ファイルの不変性を要求する。
    /// 理由は`docs/adr/allow-unsafe-only-in-the-trainer-crate.md`にある。
    ///
    /// # Safety
    ///
    /// 呼び手は、この呼び出し中と返された`Dataset`の生存中に、渡したファイルを外部プロセスも含めて変更も切り詰めもしないことを保証しなければならない。
    pub unsafe fn open(paths: &[PathBuf], lookahead: Option<Lookahead>) -> Result<Self> {
        if paths.is_empty() {
            return Err(invalid(Path::new(""), "at least one MNSD file is required"));
        }
        let mut dataset = Self {
            files: Vec::new(),
            offsets: vec![0],
            training_indices: Vec::new(),
            validation_indices: Vec::new(),
            classes: Vec::new(),
            lambdas: Vec::new(),
        };
        let mut seeds = HashSet::new();
        let mut unique_paths = HashSet::new();
        for path in paths {
            let path = path.canonicalize()?;
            if !unique_paths.insert(path.clone()) {
                return Err(invalid(
                    &path,
                    "the same MNSD file was specified more than once",
                ));
            }
            // SAFETY: 元ファイルの不変性はDataset::openの呼び手が保証する。
            let mut file = unsafe { MappedFile::open(&path)? };
            let start = dataset.record_count();
            let end = start
                .checked_add(file.header.record_count())
                .filter(|&count| count <= i64::MAX as u64)
                .ok_or_else(|| invalid(&path, "combined record count exceeds i64"))?;
            let count = usize::try_from(file.header.record_count())
                .map_err(|_| invalid(&path, "record count exceeds address space"))?;
            let origin = if count > 0 {
                Some(file.record(0)?.origin())
            } else {
                None
            };
            if let Some(origin) = origin {
                if origin == Origin::Synthetic {
                    return Err(Error::SyntheticOrigin { path });
                }
                if origin == Origin::RandomSelfplay && !seeds.insert(file.header.seed()) {
                    return Err(invalid(&path, "random-selfplay file seeds must be unique"));
                }
                let class = TeacherClass {
                    generation_commit: file.header.generation_commit().to_ascii_lowercase(),
                    network_checksum: *file.header.network_checksum(),
                    nodes: file.header.teacher_nodes(),
                    rule_set: file.header.rule_set().to_owned(),
                    origin,
                    lookahead,
                    lambda: None,
                };
                let class_index = match dataset.classes.iter().position(|item| item == &class) {
                    Some(index) => index,
                    None => {
                        dataset.classes.push(class);
                        dataset.classes.len() - 1
                    }
                };
                file.class = u32::try_from(class_index)
                    .map_err(|_| invalid(&path, "teacher class count exceeds u32"))?;
            }
            let mut games = Vec::with_capacity(count);
            let mut plies = Vec::with_capacity(count);
            let mut scores = Vec::with_capacity(count);
            file.outcomes.reserve(count);
            for local in 0..count {
                let record = file.record(local)?;
                if record.origin() == Origin::Synthetic {
                    return Err(Error::SyntheticOrigin { path });
                }
                if Some(record.origin()) != origin {
                    return Err(Error::MixedOrigins { path });
                }
                games.push(record.game_number());
                plies.push(record.ply());
                scores.push(f64::from(record.score()));
                file.outcomes.push(outcome_code(record.outcome()));
                let validation = is_validation(file.header.seed(), record.game_number());
                let indices = if validation {
                    &mut dataset.validation_indices
                } else {
                    &mut dataset.training_indices
                };
                indices.push(start + local as u64);
            }
            let scores = match lookahead {
                Some(options) => compute_lookahead(&games, &plies, &scores, options)?,
                None => scores,
            };
            file.scores = scores.into_iter().map(|value| value as f32).collect();
            dataset.files.push(file);
            dataset.offsets.push(end);
        }
        Ok(dataset)
    }

    /// 自己対局には指定したλを、実戦対局には0を設定する。
    /// 教師値やKを計算する呼び手は、有限な0以上1以下の値を明示する必要がある。
    pub fn with_lambda(mut self, lambda: f64) -> Result<Self> {
        validate_lambda(lambda)?;
        self.lambdas = self
            .classes
            .iter_mut()
            .map(|class| {
                let value = if class.origin == Origin::HumanGame {
                    0.0
                } else {
                    lambda
                };
                class.lambda = Some(value);
                value
            })
            .collect();
        Ok(self)
    }

    /// 全ファイルを連結した記録数を返す。
    pub fn record_count(&self) -> u64 {
        self.offsets[self.offsets.len() - 1]
    }

    /// 入力順の絶対パスを返す。
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(|file| file.path.as_path())
    }

    /// 入力順の復号済みヘッダを返す。
    pub fn headers(&self) -> impl Iterator<Item = &Header> {
        self.files.iter().map(|file| &file.header)
    }

    /// 訓練局面の通し番号を元のファイル順で返す。
    pub fn training_indices(&self) -> &[u64] {
        &self.training_indices
    }

    /// 検証局面の通し番号を元のファイル順で返す。
    pub fn validation_indices(&self) -> &[u64] {
        &self.validation_indices
    }

    /// 分類番号順の教師情報を返す。
    pub fn teacher_classes(&self) -> &[TeacherClass] {
        &self.classes
    }

    /// 分類番号順の、明示的に設定した教師の混合比λを返す。
    pub fn teacher_lambdas(&self) -> &[f64] {
        &self.lambdas
    }

    fn locate(&self, index: u64) -> Result<(usize, usize)> {
        if index >= self.record_count() {
            return Err(Error::RecordIndex {
                index,
                count: self.record_count(),
            });
        }
        let file = self.offsets[1..].partition_point(|&end| end <= index);
        Ok((file, (index - self.offsets[file]) as usize))
    }

    /// 通し番号をファイル番号と記録先頭のバイトオフセットへ写す。
    pub fn location(&self, index: u64) -> Result<RecordLocation> {
        let (file, local) = self.locate(index)?;
        Ok(RecordLocation {
            file,
            byte_offset: HEADER_LEN + local * RECORD_LEN,
        })
    }

    /// 指定された記録を`Record::decode`で復号する。
    pub fn record(&self, index: u64) -> Result<Record> {
        let (file, local) = self.locate(index)?;
        self.files[file].record(local)
    }

    /// 先読み変換後の探索値を返す。先読みなしの場合は元の探索値。
    pub fn teacher_score(&self, index: u64) -> Result<f32> {
        let (file, local) = self.locate(index)?;
        Ok(self.files[file].scores[local])
    }

    /// 通し番号に対応する教師の分類番号を返す。
    pub fn teacher_class(&self, index: u64) -> Result<u32> {
        let (file, _) = self.locate(index)?;
        Ok(self.files[file].class)
    }

    /// 診断用に元の探索値から先読みを再計算し、f32へ丸めずに返す。
    pub(crate) fn diagnostic_teacher_scores(&self) -> Result<Vec<f64>> {
        let mut scores = Vec::with_capacity(self.record_count() as usize);
        for file in &self.files {
            let count = file.header.record_count() as usize;
            if count == 0 {
                continue;
            }
            let mut raw = Vec::with_capacity(count);
            let mut games = Vec::with_capacity(count);
            let mut plies = Vec::with_capacity(count);
            for local in 0..count {
                let record = file.record(local)?;
                raw.push(f64::from(record.score()));
                games.push(record.game_number());
                plies.push(record.ply());
            }
            match self.classes[file.class as usize].lookahead {
                Some(options) => scores.extend(compute_lookahead(&games, &plies, &raw, options)?),
                None => scores.extend(raw),
            }
        }
        Ok(scores)
    }

    /// 入力ファイルの番号と、そのファイル内の対局番号で対局を区別する。
    pub(crate) fn game_key(&self, index: u64) -> Result<GameKey> {
        let (file, local) = self.locate(index)?;
        Ok(GameKey(file, self.files[file].record(local)?.game_number()))
    }

    /// 分類ごとに訓練集合だけを使って教師用Kを推定する。
    /// 保持したf32の教師探索値をf64へ広げて校正し、標本は1分類ずつ確保する。
    /// λが0の分類は探索値を使わないのでNaNを返し、JSONの記録ではnullになる。
    /// 先読みを指定した場合は先読み値を使い、校正、学習、検証で同じ探索値を参照する。
    /// 推定の目標は訓練集合の終局結果だけであり、混合後の教師確率は使わない。
    /// λが0でない分類に訓練局面が1つもなければ[`Error::EmptyTeacherClass`]を返す。
    pub fn estimate_teacher_ks(&self) -> Result<Vec<f64>> {
        if self.lambdas.len() != self.classes.len() {
            return Err(Error::TeacherLambda);
        }
        let mut values = Vec::with_capacity(self.classes.len());
        for class in 0..self.classes.len() {
            if self.lambdas[class] == 0.0 {
                values.push(f64::NAN);
                continue;
            }
            let mut scores = Vec::new();
            let mut outcomes = Vec::new();
            for (file_index, file) in self.files.iter().enumerate() {
                if file.class as usize != class {
                    continue;
                }
                let start = self.offsets[file_index];
                let end = self.offsets[file_index + 1];
                let first = self
                    .training_indices
                    .partition_point(|&index| index < start);
                let last = self.training_indices.partition_point(|&index| index < end);
                for &index in &self.training_indices[first..last] {
                    let local = (index - start) as usize;
                    scores.push(f64::from(file.scores[local]));
                    outcomes.push(file.outcomes[local]);
                }
            }
            if scores.is_empty() {
                return Err(Error::EmptyTeacherClass { class });
            }
            values.push(estimate_k(&scores, &outcomes)?);
        }
        Ok(values)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GameKey(pub(crate) usize, pub(crate) u32);

pub(crate) fn outcome_code(outcome: Outcome) -> u8 {
    match outcome {
        Outcome::Loss => 0,
        Outcome::Draw => 1,
        Outcome::Win => 2,
    }
}
