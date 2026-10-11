use std::{
    collections::HashSet,
    fs::File,
    path::{Path, PathBuf},
};

use memmap2::{Mmap, MmapOptions};
use minase::training::{
    provenance::{Provenance, ResultOrigin, StartOrigin},
    records::{self, HEADER_LEN, Header, Outcome, RECORD_LEN, Record},
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{Error, Lookahead, Result, compute_lookahead, estimate_k, is_validation};

/// 教師用Kを共有する分類。ファイル順で最初に現れた分類から番号を付ける。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TeacherClass {
    network_checksum: [u8; 32],
    nodes: u32,
    lookahead: Option<Lookahead>,
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

        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(".provenance.json");
        let provenance = Provenance::read(File::open(&sidecar)?)?;
        if provenance.result_origin != ResultOrigin::Selfplay
            || provenance.start_origin != StartOrigin::Random
            || provenance.games.is_some()
        {
            return Err(invalid(
                path,
                "only random selfplay is supported; game-ID SHA-256 splits are unsupported",
            ));
        }
        let checksum = format!("{:x}", Sha256::digest(&map));
        if !provenance.mnsd_sha256.eq_ignore_ascii_case(&checksum) {
            return Err(invalid(path, "provenance MNSD SHA-256 mismatch"));
        }
        let network = header
            .network_checksum()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let teacher = &provenance.teacher;
        if !teacher.network_checksum.eq_ignore_ascii_case(&network)
            || !teacher
                .generation_commit
                .eq_ignore_ascii_case(header.generation_commit())
            || teacher.nodes != header.teacher_nodes()
            || teacher.rule_set != header.rule_set()
        {
            return Err(invalid(
                path,
                "provenance teacher does not match the MNSD header",
            ));
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

/// 複数の自己対局MNSDをmmapで保持し、同じ通し番号で前処理とバッチを参照する。
///
/// 元ファイルはこの型の生存中に変更・切り詰めしないこと。盤面全体は保持せず、教師探索値
/// （f32）と結果（u8）、訓練・検証の通し番号を保持する。
pub struct Dataset {
    files: Vec<MappedFile>,
    offsets: Vec<u64>,
    training_indices: Vec<u64>,
    validation_indices: Vec<u64>,
    classes: Vec<TeacherClass>,
}

impl Dataset {
    /// ファイル長と来歴を検証し、全記録を復号して分割と教師探索値を作る。
    ///
    /// 先読みを指定した場合はファイルごとに対局・手数の順序も検証する。
    /// 来歴のλは読み手の契約で検証するが、学習時のλは指定仕様に従って1とする。
    ///
    /// # Safety
    ///
    /// 呼び手は、この呼び出し中、および返された`Dataset`の生存中に、渡したファイルを
    /// 外部プロセスも含めて変更・切り詰めしないことを保証しなければならない。
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
        };
        let mut seeds = HashSet::new();
        for path in paths {
            let path = path.canonicalize()?;
            // SAFETY: 元ファイルの不変性はDataset::openの呼び手が保証する。
            let mut file = unsafe { MappedFile::open(&path)? };
            if !seeds.insert(file.header.seed()) {
                return Err(invalid(&path, "random-selfplay file seeds must be unique"));
            }
            let start = dataset.record_count();
            let end = start
                .checked_add(file.header.record_count())
                .filter(|&count| count <= i64::MAX as u64)
                .ok_or_else(|| invalid(&path, "combined record count exceeds i64"))?;
            let count = usize::try_from(file.header.record_count())
                .map_err(|_| invalid(&path, "record count exceeds address space"))?;
            let class = TeacherClass {
                network_checksum: *file.header.network_checksum(),
                nodes: file.header.teacher_nodes(),
                lookahead,
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
            let mut games = Vec::with_capacity(count);
            let mut plies = Vec::with_capacity(count);
            let mut scores = Vec::with_capacity(count);
            file.outcomes.reserve(count);
            for local in 0..count {
                let record = file.record(local)?;
                games.push(record.game_number());
                plies.push(record.ply());
                scores.push(f64::from(record.score()));
                file.outcomes.push(outcome_code(record.outcome()));
                let indices = if is_validation(file.header.seed(), record.game_number()) {
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

    /// 分類ごとに訓練集合だけを使って教師用Kを推定する。
    ///
    /// 保持したf32の教師探索値をf64へ昇格して校正する。標本は1分類ずつ確保する。
    pub fn estimate_teacher_ks(&self) -> Result<Vec<f64>> {
        let mut values = Vec::with_capacity(self.classes.len());
        for class in 0..self.classes.len() {
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

pub(super) fn outcome_code(outcome: Outcome) -> u8 {
    match outcome {
        Outcome::Loss => 0,
        Outcome::Draw => 1,
        Outcome::Win => 2,
    }
}
