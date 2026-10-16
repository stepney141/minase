//! 逐次読み込みの受け手が全局面を圧縮列へ集める、B=1のデータ集合。
//! 分類番号と通し番号は、各ファイルの読み込み後に入力順で割り当てる。

use crate::data::{Outcome, Sample};
use std::{
    collections::HashSet,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    thread,
};

use super::sample::Metadata;
use super::stream::{FileSummary, read_file};
use super::teacher::estimate_cp_per_logit_by_passes;
use minase::training::records::{Header, Origin};
use serde::Serialize;

use super::{CpPerLogitEstimate, Error, Lookahead, Result, compute_lookahead, validate_lambda};

/// 教師の勝率尺度（1ロジットあたりのcp）を共有する分類。
///
/// 生成コミット（小文字）、重みの検査和、探索規模、規則セット、由来、先読み条件、適用したλで区別する。
/// 入力順に番号を付け、適格な局面がないファイルは分類を作らない。
/// 教師を使う前に[`Dataset::with_lambda`]でλを設定し、教師を使わない診断では`lambda`は`None`である。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TeacherClass {
    pub(crate) generation_commit: String,
    pub(crate) network_checksum: [u8; 32],
    pub(crate) search_budget: u64,
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

    /// 教師の探索規模を返す。αβ探索ならノード上限、モンテカルロ木探索なら探索回数である。
    pub fn search_budget(&self) -> u64 {
        self.search_budget
    }

    /// 先読みの条件を返す。省略時は元の探索値を使う。
    pub fn lookahead(&self) -> Option<Lookahead> {
        self.lookahead
    }
}

/// 1ファイルから受け取った全局面と教師情報をメモリに保持する。
struct LoadedFile {
    summary: FileSummary,
    positions: Vec<u8>,
    position_offsets: Vec<usize>,
    metadata: Vec<Metadata>,
    class: u32,
    scores: Vec<f32>,
    validation: Vec<bool>,
}

fn invalid(path: &Path, reason: &'static str) -> Error {
    Error::Dataset {
        path: path.to_owned(),
        reason,
    }
}

impl LoadedFile {
    /// 逐次読み込みの受け手で圧縮列、索引、教師情報を集める。
    fn open(path: &Path, lookahead: Option<Lookahead>) -> Result<Self> {
        let mut positions = Vec::new();
        let mut position_offsets = vec![0];
        let mut metadata = Vec::new();
        let mut scores = Vec::new();
        let mut validation = Vec::new();
        let summary = read_file(path, lookahead, &mut |emitted| {
            let sample = emitted.sample;
            let occupied = sample
                .board_codes()
                .iter()
                .filter(|&&code| code != 0)
                .count();
            let length = positions
                .len()
                .checked_add(21 + occupied)
                .filter(|&n| n <= isize::MAX as usize)
                .ok_or_else(|| invalid(path, "compact positions exceed address space"))?;
            sample.append_compact(&mut positions);
            position_offsets.push(length);
            metadata.push(sample.metadata);
            scores.push(emitted.teacher_score);
            validation.push(emitted.validation);
            Ok(())
        })?;
        Ok(Self {
            summary,
            positions,
            position_offsets,
            metadata,
            class: 0,
            scores,
            validation,
        })
    }
    fn record(&self, local: usize) -> Result<Sample> {
        let origin = self
            .summary
            .origin
            .ok_or_else(|| invalid(&self.summary.path, "sample has no game origin"))?;
        Ok(Sample::from_compact(
            &self.positions[self.position_offsets[local]..self.position_offsets[local + 1]],
            self.metadata[local],
            origin,
        ))
    }
}

/// B=1用に、逐次読み込みの受け手が適格な全局面をメモリへ集めたデータ集合。
/// 局面は圧縮したバイト列に保持し、選別後の通し番号で参照する。
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
    /// 1局ずつ再生して受け手へ渡し、シードと対局番号で訓練・検証集合へ分ける。
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
                            let file = LoadedFile::open(&path, lookahead);
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
                    .checked_add(file.summary.count)
                    .filter(|&count| count <= i64::MAX as u64)
                    .ok_or_else(|| invalid(&path, "combined record count exceeds i64"))?;
                let count = file.metadata.len();
                if file.summary.origin == Some(Origin::RandomSelfplay)
                    && !seeds.insert(file.summary.header.seed())
                {
                    return Err(invalid(&path, "random-selfplay file seeds must be unique"));
                }
                if let Some(origin) = file.summary.origin.filter(|_| count > 0) {
                    let class = TeacherClass {
                        generation_commit: file
                            .summary
                            .header
                            .generation_commit()
                            .to_ascii_lowercase(),
                        network_checksum: *file.summary.header.network_checksum(),
                        search_budget: file.summary.header.teacher_search_budget(),
                        rule_set: file.summary.header.rule_set().to_owned(),
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
                for (local, validation) in
                    std::mem::take(&mut file.validation).into_iter().enumerate()
                {
                    let indices = if validation {
                        &mut dataset.validation_indices
                    } else {
                        &mut dataset.training_indices
                    };
                    indices.push(start + local as u64);
                }
                dataset.files.push(file);
                dataset.offsets.push(end);
            }
        }
        Ok(dataset)
    }

    /// 自己対局には指定したλを、実戦対局には0を設定する。
    /// 教師値や教師の勝率尺度を計算する呼び手は、有限な0以上1以下の値を明示する必要がある。
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
        self.files.iter().map(|file| file.summary.path.as_path())
    }

    pub(crate) fn file_sample_counts(&self) -> impl Iterator<Item = u64> {
        self.files.iter().map(|file| file.summary.count)
    }

    /// 入力順の復号済みヘッダを返す。
    pub fn headers(&self) -> impl Iterator<Item = &Header> {
        self.files.iter().map(|file| &file.summary.header)
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

    /// 分類ごとに訓練集合だけを使って教師の勝率尺度を推定する。
    /// 保持したf32の教師探索値をf64へ広げ、分類ごとに入力順で逐次加算する。
    /// 標本の列を検査で1回、損失評価で66回なぞり、推定用の標本配列は作らない。
    /// λが0の分類は探索値を使わないので`None`を返す。
    /// 先読みを指定した場合は先読み値を使い、較正、学習、検証で同じ探索値を参照する。
    /// 推定の目標は訓練集合の終局結果だけであり、混合後の教師確率は使わない。
    /// λが0でない分類に訓練局面が1つもなければ[`Error::EmptyTeacherClass`]を返す。
    pub fn estimate_teacher_cp_per_logit(&self) -> Result<Vec<Option<CpPerLogitEstimate>>> {
        self.estimate_teacher_scales(|file, local, _| f64::from(file.scores[local]))
    }

    /// 外部で計算したf64の探索値で、分類ごとの訓練集合を較正する。
    /// `scores`は全適格局面の通し番号順で、長さは[`Self::record_count`]と一致させる。
    /// 分類と標本の順序、λが0の分類、空の分類の扱いは[`Self::estimate_teacher_cp_per_logit`]と同じ。
    pub fn estimate_teacher_cp_per_logit_with_scores(
        &self,
        scores: &[f64],
    ) -> Result<Vec<Option<CpPerLogitEstimate>>> {
        if scores.len() as u64 != self.record_count() {
            return Err(Error::TeacherSamples);
        }
        self.estimate_teacher_scales(|_, _, index| scores[index as usize])
    }

    fn estimate_teacher_scales(
        &self,
        score: impl Fn(&LoadedFile, usize, u64) -> f64,
    ) -> Result<Vec<Option<CpPerLogitEstimate>>> {
        if self.lambdas.len() != self.classes.len() {
            return Err(Error::TeacherLambda);
        }
        let mut values = Vec::with_capacity(self.classes.len());
        for class in 0..self.classes.len() {
            if self.lambdas[class] == 0.0 {
                values.push(None);
                continue;
            }
            let estimate = estimate_cp_per_logit_by_passes(|visit| {
                let mut count = 0_u64;
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
                        visit(
                            score(file, local, index),
                            outcome_code(file.metadata[local].outcome),
                        );
                        count += 1;
                    }
                }
                if count == 0 {
                    return Err(Error::EmptyTeacherClass { class });
                }
                Ok(())
            })?;
            values.push(Some(estimate));
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
