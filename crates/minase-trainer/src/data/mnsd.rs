use crate::data::{Outcome, Sample};
use std::{
    collections::HashSet,
    fs::File,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    thread,
};

use super::sample::Metadata;
use minase::training::records::{
    Exclusion, Header, Origin, Outcome as GameOutcome, Reader, TeacherKind, Value,
};
use serde::Serialize;
use std::io::BufReader;

use super::{
    Error, Lookahead, Result, compute_lookahead, estimate_k, is_validation, validate_lambda,
};

/// 教師用Kを共有する分類。
///
/// 生成コミット（小文字）、重みの検査和、ノード上限、規則セット、由来、先読み条件、適用したλで区別する。
/// 入力順に番号を付け、適格な局面がないファイルは分類を作らない。
/// 教師を使う前に[`Dataset::with_lambda`]でλを設定し、教師を使わない診断では`lambda`は`None`である。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TeacherClass {
    pub(crate) generation_commit: String,
    pub(crate) network_checksum: [u8; 32],
    pub(crate) nodes: u64,
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
    pub fn nodes(&self) -> u64 {
        self.nodes
    }

    /// 先読みの条件を返す。省略時は元の探索値を使う。
    pub fn lookahead(&self) -> Option<Lookahead> {
        self.lookahead
    }
}

struct LoadedFile {
    path: PathBuf,
    header: Header,
    origin: Option<Origin>,
    positions: Vec<u8>,
    position_offsets: Vec<usize>,
    metadata: Vec<Metadata>,
    class: u32,
    scores: Vec<f32>,
}

fn invalid(path: &Path, reason: &'static str) -> Error {
    Error::Dataset {
        path: path.to_owned(),
        reason,
    }
}

impl LoadedFile {
    fn open(path: &Path) -> Result<Self> {
        let mut reader = Reader::new(BufReader::new(File::open(path)?))?;
        if reader.header().teacher_kind() == TeacherKind::Mcts {
            return Err(Error::MctsTeacher {
                path: path.to_owned(),
            });
        }
        let mut file = Self {
            path: path.to_owned(),
            header: reader.header().clone(),
            origin: None,
            positions: Vec::new(),
            position_offsets: vec![0],
            metadata: Vec::new(),
            class: 0,
            scores: Vec::new(),
        };
        while let Some(game) = reader.read_game()? {
            let record = game.record();
            if record.origin() == Origin::Synthetic {
                return Err(Error::SyntheticOrigin {
                    path: path.to_owned(),
                });
            }
            if file.origin.is_some_and(|origin| origin != record.origin()) {
                return Err(Error::MixedOrigins {
                    path: path.to_owned(),
                });
            }
            file.origin = Some(record.origin());
            let GameOutcome::Game(result) = record.outcome() else {
                return Err(Error::SyntheticOrigin {
                    path: path.to_owned(),
                });
            };
            for (index, (entry, position)) in
                record.entries().iter().zip(game.positions()).enumerate()
            {
                if entry.exclusion() != Exclusion::None {
                    continue;
                }
                let Value::AlphaBeta(score) = entry.value() else {
                    continue;
                };
                let ply = u16::try_from(index)
                    .ok()
                    .and_then(|i| record.start_ply().checked_add(i))
                    .ok_or_else(|| invalid(path, "sample ply exceeds u16"))?;
                let sample = Sample::from_position(
                    position,
                    score,
                    Outcome::from_game_result(result, position.side_to_move()),
                    record.game_number(),
                    ply,
                    record.origin(),
                )?;
                let length = file
                    .positions
                    .len()
                    .checked_add(21 + position.occupied().popcount() as usize)
                    .filter(|&n| n <= isize::MAX as usize)
                    .ok_or_else(|| invalid(path, "compact positions exceed address space"))?;
                sample.append_compact(&mut file.positions);
                file.position_offsets.push(length);
                file.metadata.push(sample.metadata);
            }
        }
        Ok(file)
    }
    fn record(&self, local: usize) -> Result<Sample> {
        let origin = self
            .origin
            .ok_or_else(|| invalid(&self.path, "sample has no game origin"))?;
        Ok(Sample::from_compact(
            &self.positions[self.position_offsets[local]..self.position_offsets[local + 1]],
            self.metadata[local],
            origin,
        ))
    }
}

/// 適格な局面だけを圧縮したバイト列に保持し、選別後の通し番号で参照する。
/// 局面は18バイトの占有集合、手番・先獅子の3バイト、駒ごとの1バイトからなる。
/// ファイルごとのusize索引と教師情報を別の配列に保持する。
pub struct Dataset {
    files: Vec<LoadedFile>,
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
    /// ファイルの読み込みと手順の再生を最大`parallelism`スレッドで行う。
    /// 結果の集約とエラーの選択は入力順に行い、並列度によって変わらない。
    pub fn open(
        paths: &[PathBuf],
        lookahead: Option<Lookahead>,
        parallelism: NonZeroUsize,
    ) -> Result<Self> {
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
        for paths in paths.chunks(parallelism.get()) {
            let loaded = thread::scope(|scope| {
                let handles: Vec<_> = paths
                    .iter()
                    .map(|path| {
                        thread::Builder::new().spawn_scoped(scope, move || {
                            let path = path.canonicalize()?;
                            let file = LoadedFile::open(&path);
                            Ok::<_, Error>((path, file))
                        })
                    })
                    .collect();
                // 失敗時も全スレッドを回収してから入力順に結果を処理する。
                handles
                    .into_iter()
                    .map(|handle| {
                        handle.map_err(Error::Io).and_then(|handle| {
                            handle.join().map_err(|_| {
                                Error::Io(std::io::Error::other("MNSD reader worker panicked"))
                            })?
                        })
                    })
                    .collect::<Vec<_>>()
            });
            for result in loaded {
                let (path, file) = result?;
                if !unique_paths.insert(path.clone()) {
                    return Err(invalid(
                        &path,
                        "the same MNSD file was specified more than once",
                    ));
                }

                let mut file = file?;
                let start = dataset.record_count();
                let end = start
                    .checked_add(file.metadata.len() as u64)
                    .filter(|&count| count <= i64::MAX as u64)
                    .ok_or_else(|| invalid(&path, "combined record count exceeds i64"))?;
                let count = file.metadata.len();
                if file.origin == Some(Origin::RandomSelfplay) && !seeds.insert(file.header.seed())
                {
                    return Err(invalid(&path, "random-selfplay file seeds must be unique"));
                }
                if let Some(origin) = file.origin.filter(|_| count > 0) {
                    let class = TeacherClass {
                        generation_commit: file.header.generation_commit().to_ascii_lowercase(),
                        network_checksum: *file.header.network_checksum(),
                        nodes: file.header.teacher_scale(),
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
                for local in 0..count {
                    let metadata = file.metadata[local];
                    games.push(metadata.game);
                    plies.push(metadata.ply);
                    scores.push(f64::from(metadata.score));
                    let validation = is_validation(file.header.seed(), metadata.game);
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

    /// 全ファイルの適格な局面の総数を返す。
    pub fn record_count(&self) -> u64 {
        self.offsets[self.offsets.len() - 1]
    }

    /// 入力順の絶対パスを返す。
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(|file| file.path.as_path())
    }

    pub(crate) fn file_sample_counts(&self) -> impl Iterator<Item = u64> {
        self.files.iter().map(|file| file.metadata.len() as u64)
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

    /// 通し番号に対応する標本を圧縮列から取り出す。
    pub fn record(&self, index: u64) -> Result<Sample> {
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
            let count = file.metadata.len();
            if count == 0 {
                continue;
            }
            let mut raw = Vec::with_capacity(count);
            let mut games = Vec::with_capacity(count);
            let mut plies = Vec::with_capacity(count);
            for local in 0..count {
                let metadata = file.metadata[local];
                raw.push(f64::from(metadata.score));
                games.push(metadata.game);
                plies.push(metadata.ply);
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
        Ok(GameKey(file, self.files[file].metadata[local].game))
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
                    outcomes.push(outcome_code(file.metadata[local].outcome));
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
