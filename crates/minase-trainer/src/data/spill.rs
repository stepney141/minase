//! 実行専用の一時ファイルによる局面の保存とRao–Sandelius方式の供給。
//! 版番号やマジックを持たず、別の実行から読み込む入口は設けない。
//!
//! | ファイル | 内容 |
//! | --- | --- |
//! | `lock` | 所有するプロセスのpidを10進で記録する。 |
//! | `train-<f>.bin` | 入力fの訓練局面をファイル順で保存する。 |
//! | `validation-<f>.bin` | 入力fの検証局面をファイル順で保存する。 |
//! | `scale-<f>.bin` | 訓練局面の探索値f32と結果u8を5バイトで保存する。 |
//! | `epoch/bucket-<k>.bin` | そのエポックのbucket kの局面を保存する。 |
//!
//! | 固定長記録の欄 | バイト数 |
//! | --- | --- |
//! | 占有集合、手番、先獅子の升、麒麟成りの印 | 18 + 3 |
//! | 升順の駒コードと末尾の0埋め | 92 |
//! | 先読み後の探索値f32、元の探索値i16、結果u8 | 4 + 2 + 1 |
//! | 手数u16、対局番号u32、入力ファイル番号u32、ファイル内番号u64 | 2 + 4 + 4 + 8 |
//!
//! 数値はリトルエンディアンで保存する。結果は負け0、引き分け1、勝ち2。
//! bucketのメモリ上限は`bucket_positions × RECORD_LEN`バイトと同数のusize順列である。
//! 振り分けバッファはB個の各64 KiB。読み込み1ファイルと、吐き出す間だけの書き込み1ファイルを開く。
//! バッチの残りは1バッチ未満であり、供給側は最大`workers × batch`件の復元記録を先に取り出す。

use super::{
    BucketAssigner, CpPerLogitEstimate, Error, Lookahead, Outcome, Result, Sample, TeacherClass,
    bucket_permutation, floyd_sample, outcome_code,
    sample::Metadata,
    stream::{FileSummary, read_file},
    teacher::estimate_cp_per_logit_by_passes,
    validate_lambda,
};
use crate::nn::{Sha256Hasher, hex};
use minase::training::records::{Header, Origin};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter, Read, Write},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// 圧縮局面と教師情報を合わせた固定長記録のバイト数。
pub const RECORD_LEN: usize = 18 + 3 + 92 + 4 + 2 + 1 + 2 + 4 + 4 + 8;
/// bucketごとの書き出しバッファの上限。開くファイル数には比例させない。
const WRITE_BUFFER: usize = 64 * 1024;
/// 圧縮局面と0埋めを合わせた長さ。
const COMPACT_LEN: usize = 18 + 3 + 92;
/// 教師の勝率尺度の推定用記録の長さ。
const SCALE_LEN: usize = 4 + 1;
/// 同一プロセス内の実行ディレクトリを区別する連番。
static NEXT_RUN: AtomicU64 = AtomicU64::new(0);

/// 作成直後から所有し、途中の失敗でも作業ディレクトリを削除する。
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!(
                "warning: cannot remove scratch directory {}: {error}",
                self.0.display()
            );
        }
    }
}

/// 入力ファイルごとの由来、分類、件数と通し番号の開始位置。
struct SpillFile {
    summary: FileSummary,
    training: u64,
    validation: u64,
    start: u64,
    class: u32,
}

/// 読み戻した局面と、学習に使う教師情報。
#[derive(Debug)]
pub struct SpillRecord {
    /// 入力順に振った全適格局面の通し番号。
    pub index: u64,
    /// 元の探索値、結果と対局情報を含む局面。
    pub sample: Sample,
    /// 先読み後にf32へ丸めた探索値。
    pub teacher_score: f32,
    /// 入力ファイルに割り当てた教師の分類番号。
    pub teacher_class: u32,
}

/// 全局面をメモリへ保持せず、実行専用の一時ファイルから供給する。
/// 破棄時には正常終了、エラーのどちらでも所有するディレクトリを削除する。
pub struct SpillStore {
    directory: Directory,
    files: Vec<SpillFile>,
    classes: Vec<TeacherClass>,
    lambdas: Vec<f64>,
    buckets: u32,
    bucket_positions: u64,
    training_hash: String,
    validation_hash: String,
}

/// パスを付けた一時ファイルの不整合を作る。
fn invalid(path: &Path, reason: impl Into<String>) -> Error {
    Error::Spill {
        path: path.to_owned(),
        reason: reason.into(),
    }
}

/// 期待件数との完全一致を検査し、記録境界での切断も拒否する。
fn checked_reader(path: &Path, count: u64, record_len: usize) -> Result<BufReader<File>> {
    let file = File::open(path)?;
    let expected = count
        .checked_mul(record_len as u64)
        .ok_or_else(|| invalid(path, "record length overflow"))?;
    let actual = file.metadata()?.len();
    if actual != expected {
        return Err(invalid(
            path,
            format!("length {actual} differs from expected {expected} ({count} records)"),
        ));
    }
    Ok(BufReader::new(file))
}

/// 検査後に短くなったファイルも一時ファイルのエラーとして報告する。
fn read_record<const N: usize>(reader: &mut impl Read, path: &Path) -> Result<[u8; N]> {
    let mut bytes = [0; N];
    reader
        .read_exact(&mut bytes)
        .map_err(|error| invalid(path, error.to_string()))?;
    Ok(bytes)
}

/// 標本を固定長記録へ詰める。駒数が上限を超えれば明示的に拒否する。
fn encode(sample: &Sample, score: f32, file: u32, local: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(RECORD_LEN);
    sample.append_compact(&mut bytes);
    if bytes.len() > COMPACT_LEN {
        return Err(Error::Invalid("spill record exceeds 92 pieces"));
    }
    bytes.resize(COMPACT_LEN, 0);
    bytes.extend(score.to_le_bytes());
    bytes.extend(sample.score().to_le_bytes());
    bytes.push(outcome_code(sample.outcome()));
    bytes.extend(sample.ply().to_le_bytes());
    bytes.extend(sample.game_number().to_le_bytes());
    bytes.extend(file.to_le_bytes());
    bytes.extend(local.to_le_bytes());
    Ok(bytes)
}

/// 1入力の受け手は3つの書き出しバッファと件数だけを保持する。
fn write_input(
    path: &Path,
    directory: &Path,
    f: usize,
    lookahead: Option<Lookahead>,
) -> Result<SpillFile> {
    let file_number =
        u32::try_from(f).map_err(|_| Error::Invalid("input file count exceeds u32"))?;
    let mut train = BufWriter::new(File::create(directory.join(format!("train-{f}.bin")))?);
    let mut validation =
        BufWriter::new(File::create(directory.join(format!("validation-{f}.bin")))?);
    let mut scale = BufWriter::new(File::create(directory.join(format!("scale-{f}.bin")))?);
    let (mut training_count, mut validation_count) = (0, 0);
    let summary = read_file(path, lookahead, &mut |emitted| {
        let bytes = encode(
            &emitted.sample,
            emitted.teacher_score,
            file_number,
            training_count + validation_count,
        )?;
        if emitted.validation {
            validation.write_all(&bytes)?;
            validation_count += 1;
        } else {
            train.write_all(&bytes)?;
            scale.write_all(&emitted.teacher_score.to_le_bytes())?;
            scale.write_all(&[outcome_code(emitted.sample.outcome())])?;
            training_count += 1;
        }
        Ok(())
    })?;
    train.flush()?;
    validation.flush()?;
    scale.flush()?;
    Ok(SpillFile {
        summary,
        training: training_count,
        validation: validation_count,
        start: 0,
        class: 0,
    })
}

impl SpillStore {
    /// MNSDを1局ずつ再生し、適格な局面の一時表現を作業ディレクトリへ書く。
    pub fn create(
        paths: &[PathBuf],
        lookahead: Option<Lookahead>,
        parallelism: NonZeroUsize,
        scratch_dir: &Path,
        buckets: u32,
        bucket_positions: u64,
    ) -> Result<Self> {
        if paths.is_empty() {
            return Err(Error::Dataset {
                path: PathBuf::new(),
                reason: "at least one MNSD file is required",
            });
        }
        if !(2..=4096).contains(&buckets) || bucket_positions == 0 || !scratch_dir.is_dir() {
            return Err(Error::Invalid(
                "spill requires B in 2..=4096, positive bucket positions and an existing scratch directory",
            ));
        }
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| invalid(scratch_dir, error.to_string()))?
            .as_secs();
        let directory = scratch_dir.join(format!(
            "nnue-{seconds}-{}-{}",
            std::process::id(),
            NEXT_RUN.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory)?;
        let directory = Directory(directory);
        let mut lock = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.0.join("lock"))?;
        writeln!(lock, "{}", std::process::id())?;
        drop(lock);
        let mut store = Self {
            directory,
            files: Vec::new(),
            classes: Vec::new(),
            lambdas: Vec::new(),
            buckets,
            bucket_positions,
            training_hash: String::new(),
            validation_hash: String::new(),
        };
        let mut unique_paths = HashSet::new();
        let mut seeds = HashSet::new();
        for (group, paths) in paths.chunks(parallelism.get()).enumerate() {
            let loaded = thread::scope(|scope| {
                let handles: Vec<_> = paths
                    .iter()
                    .enumerate()
                    .map(|(i, path)| {
                        let directory = &store.directory.0;
                        thread::Builder::new().spawn_scoped(scope, move || {
                            let path = path.canonicalize()?;
                            let result = write_input(
                                &path,
                                directory,
                                group * parallelism.get() + i,
                                lookahead,
                            );
                            Ok::<_, Error>((path, result))
                        })
                    })
                    .collect();
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
                    return Err(Error::Dataset {
                        path,
                        reason: "the same MNSD file was specified more than once",
                    });
                }
                let mut file = file?;
                file.start = store.record_count();
                file.start
                    .checked_add(file.summary.count)
                    .filter(|&n| n <= i64::MAX as u64)
                    .ok_or_else(|| Error::Dataset {
                        path: path.clone(),
                        reason: "combined record count exceeds i64",
                    })?;
                if file.summary.origin == Some(Origin::RandomSelfplay)
                    && !seeds.insert(file.summary.header.seed())
                {
                    return Err(Error::Dataset {
                        path,
                        reason: "random-selfplay file seeds must be unique",
                    });
                }
                if let Some(origin) = file.summary.origin.filter(|_| file.summary.count > 0) {
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
                    let index = match store.classes.iter().position(|item| item == &class) {
                        Some(index) => index,
                        None => {
                            store.classes.push(class);
                            store.classes.len() - 1
                        }
                    };
                    file.class = u32::try_from(index).map_err(|_| Error::Dataset {
                        path,
                        reason: "teacher class count exceeds u32",
                    })?;
                }
                store.files.push(file);
            }
        }
        for (f, file) in store.files.iter().enumerate() {
            checked_reader(&store.path("scale", f), file.training, SCALE_LEN)?;
        }
        let mut training = Sha256Hasher::default();
        for record in store.training_records() {
            training.update(&record?.index.to_le_bytes());
        }
        let mut validation = Sha256Hasher::default();
        for record in store.records(true) {
            validation.update(&record?.index.to_le_bytes());
        }
        store.training_hash = hex(&training.finalize());
        store.validation_hash = hex(&validation.finalize());
        Ok(store)
    }

    /// 自己対局には指定したλを、実戦対局には0を設定する。
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
    /// 訓練局面の総数を返す。
    pub fn training_count(&self) -> u64 {
        self.files.iter().map(|f| f.training).sum()
    }
    /// 検証局面の総数を返す。
    pub fn validation_count(&self) -> u64 {
        self.files.iter().map(|f| f.validation).sum()
    }
    /// 適格な全局面の総数を返す。
    pub fn record_count(&self) -> u64 {
        self.files.iter().map(|f| f.summary.count).sum()
    }
    /// 入力順の正規化済みパスを返す。
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(|f| f.summary.path.as_path())
    }
    /// 入力順のヘッダを返す。
    pub fn headers(&self) -> impl Iterator<Item = &Header> {
        self.files.iter().map(|f| &f.summary.header)
    }
    /// 分類番号順の教師情報を返す。
    pub fn teacher_classes(&self) -> &[TeacherClass] {
        &self.classes
    }
    /// 分類番号順の教師の混合比を返す。
    pub fn teacher_lambdas(&self) -> &[f64] {
        &self.lambdas
    }
    /// ファイル順の訓練通し番号のu64 LE列のSHA-256を返す。
    pub fn training_indices_sha256(&self) -> &str {
        &self.training_hash
    }
    /// ファイル順の検証通し番号のu64 LE列のSHA-256を返す。
    pub fn validation_indices_sha256(&self) -> &str {
        &self.validation_hash
    }
    /// bucket数を返す。
    pub fn buckets(&self) -> u32 {
        self.buckets
    }
    /// 1つのbucketの局面数の上限を返す。
    pub fn bucket_positions(&self) -> u64 {
        self.bucket_positions
    }
    /// このストアが所有する作業ディレクトリを返す。
    pub fn directory(&self) -> &Path {
        &self.directory.0
    }
    /// ファイル番号に対応する一時ファイルのパスを作る。
    fn path(&self, kind: &str, f: usize) -> PathBuf {
        self.directory.0.join(format!("{kind}-{f}.bin"))
    }

    /// 固定長記録の索引と駒数を検査してから復元する。
    fn decode(&self, bytes: &[u8; RECORD_LEN], path: &Path) -> Result<SpillRecord> {
        let f = u32::from_le_bytes(bytes[126..130].try_into().expect("fixed field")) as usize;
        let local = u64::from_le_bytes(bytes[130..138].try_into().expect("fixed field"));
        let file = self
            .files
            .get(f)
            .ok_or_else(|| invalid(path, "unknown input file"))?;
        let origin = file
            .summary
            .origin
            .ok_or_else(|| invalid(path, "sample has no game origin"))?;
        if local >= file.summary.count
            || bytes[..18].iter().map(|v| v.count_ones()).sum::<u32>() > 92
        {
            return Err(invalid(path, "invalid local index or piece count"));
        }
        let outcome = match bytes[119] {
            0 => Outcome::Loss,
            1 => Outcome::Draw,
            2 => Outcome::Win,
            _ => return Err(invalid(path, "invalid outcome")),
        };
        let metadata = Metadata {
            score: i16::from_le_bytes(bytes[117..119].try_into().expect("fixed field")),
            outcome,
            ply: u16::from_le_bytes(bytes[120..122].try_into().expect("fixed field")),
            game: u32::from_le_bytes(bytes[122..126].try_into().expect("fixed field")),
        };
        Ok(SpillRecord {
            index: file.start + local,
            sample: Sample::from_compact(&bytes[..COMPACT_LEN], metadata, origin),
            teacher_score: f32::from_le_bytes(bytes[113..117].try_into().expect("fixed field")),
            teacher_class: file.class,
        })
    }

    /// ファイル順の記録を、読み込み1ファイルだけでなぞる。
    fn records(&self, validation: bool) -> impl Iterator<Item = Result<SpillRecord>> {
        RawRecords::new(self, validation).map(move |item| {
            let (bytes, f) = item?;
            self.decode(
                &bytes,
                &self.path(if validation { "validation" } else { "train" }, f),
            )
        })
    }
    /// 訓練局面をファイル順に返す。局面数に比例する配列を作らない。
    pub fn training_records(&self) -> impl Iterator<Item = Result<SpillRecord>> {
        self.records(false)
    }
    /// 訓練局面をファイル順に1つずつ受け手へ渡す。
    pub fn for_each_training(&self, mut f: impl FnMut(SpillRecord) -> Result<()>) -> Result<()> {
        for record in self.training_records() {
            f(record?)?;
        }
        Ok(())
    }
    /// 検証局面をファイル順に最大chunk件ずつ渡す。0件の指定はエラーを返す。
    pub fn validation_chunks(
        &self,
        chunk: usize,
    ) -> impl Iterator<Item = Result<Vec<SpillRecord>>> {
        let mut records = self.records(true);
        let mut done = false;
        std::iter::from_fn(move || {
            if done {
                return None;
            }
            if chunk == 0 {
                done = true;
                return Some(Err(Error::Invalid("validation chunk must be positive")));
            }
            let result: Result<Vec<_>> = records.by_ref().take(chunk).collect();
            match result {
                Ok(records) if records.is_empty() => {
                    done = true;
                    None
                }
                Err(error) => {
                    done = true;
                    Some(Err(error))
                }
                other => Some(other),
            }
        })
    }
    /// 分類ごとの訓練探索値を同じ順に反復して読み、勝率尺度を推定する。
    pub fn estimate_teacher_cp_per_logit(&self) -> Result<Vec<Option<CpPerLogitEstimate>>> {
        if self.classes.len() != self.lambdas.len() {
            return Err(Error::TeacherLambda);
        }
        self.lambdas
            .iter()
            .enumerate()
            .map(|(class, &lambda)| {
                if lambda == 0.0 {
                    return Ok(None);
                }
                estimate_cp_per_logit_by_passes(|visit| {
                    let mut count = 0;
                    for (f, file) in self
                        .files
                        .iter()
                        .enumerate()
                        .filter(|(_, f)| f.class as usize == class)
                    {
                        let path = self.path("scale", f);
                        let mut reader = checked_reader(&path, file.training, SCALE_LEN)?;
                        for _ in 0..file.training {
                            let bytes = read_record::<SCALE_LEN>(&mut reader, &path)?;
                            visit(
                                f64::from(f32::from_le_bytes(
                                    bytes[..4].try_into().expect("fixed field"),
                                )),
                                bytes[4],
                            );
                        }
                        count += file.training;
                    }
                    if count == 0 {
                        return Err(Error::EmptyTeacherClass { class });
                    }
                    Ok(())
                })
                .map(Some)
            })
            .collect()
    }
    /// 訓練局面をbucketへ振り分け、期限内ならエポックの読み手を返す。
    pub fn epoch(
        &self,
        seed: u64,
        epoch: u64,
        deadline: Instant,
    ) -> Result<Option<EpochReader<'_>>> {
        let directory = self.directory.0.join("epoch");
        if directory.exists() {
            fs::remove_dir_all(&directory)?;
        }
        fs::create_dir(&directory)?;
        let mut buffers: Vec<_> = (0..self.buckets)
            .map(|_| Vec::with_capacity(WRITE_BUFFER))
            .collect();
        let mut counts = vec![0_u64; self.buckets as usize];
        for k in 0..self.buckets {
            File::create(directory.join(format!("bucket-{k}.bin")))?;
        }
        let mut assigner = BucketAssigner::new(seed, epoch, self.buckets);
        for (i, record) in RawRecords::new(self, false).enumerate() {
            if i % 65_536 == 0 && Instant::now() >= deadline {
                return Ok(None);
            }
            let (bytes, _) = record?;
            let bucket = assigner.next() as usize;
            if buffers[bucket].len() + RECORD_LEN > WRITE_BUFFER {
                flush_bucket(&directory, bucket, &mut buffers[bucket])?;
            }
            buffers[bucket].extend(bytes);
            counts[bucket] += 1;
        }
        for (bucket, buffer) in buffers.iter_mut().enumerate() {
            flush_bucket(&directory, bucket, buffer)?;
        }
        for (bucket, &positions) in counts.iter().enumerate() {
            if positions > self.bucket_positions {
                return Err(Error::BucketOverflow {
                    bucket: bucket as u32,
                    positions,
                    limit: self.bucket_positions,
                    buckets: self.buckets,
                });
            }
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        Ok(Some(EpochReader {
            store: self,
            counts,
            seed,
            epoch,
            next_bucket: 0,
            records: Vec::new(),
            order: Vec::new(),
            cursor: 0,
        }))
    }
    /// Floydの方法で検証標本を選び、通し番号の昇順に読み戻す。
    /// 要求数が検証集合より大きければ全件を返し、0件の要求は拒否する。
    pub fn validation_sample(&self, k: usize, seed: u64) -> Result<Vec<SpillRecord>> {
        if k == 0 {
            return Err(Error::Invalid("validation sample must be positive"));
        }
        let selected = floyd_sample(
            self.validation_count(),
            (k as u64).min(self.validation_count()) as usize,
            seed,
        )?;
        let mut selected = selected.into_iter().peekable();
        let mut result = Vec::new();
        for (index, record) in self.records(true).enumerate() {
            let record = record?;
            if selected.peek() == Some(&(index as u64)) {
                result.push(record);
                selected.next();
            }
        }
        Ok(result)
    }
}

/// bucketのバッファを追記し、ファイルを閉じてから呼び手へ戻る。
fn flush_bucket(directory: &Path, bucket: usize, buffer: &mut Vec<u8>) -> Result<()> {
    if !buffer.is_empty() {
        OpenOptions::new()
            .append(true)
            .open(directory.join(format!("bucket-{bucket}.bin")))?
            .write_all(buffer)?;
        buffer.clear();
    }
    Ok(())
}

/// 入力順に1ファイルずつ開き、長さと短い読み込みを検査する反復子。
struct RawRecords<'a> {
    store: &'a SpillStore,
    validation: bool,
    file: usize,
    reader: Option<BufReader<File>>,
    remaining: u64,
    failed: bool,
}
impl<'a> RawRecords<'a> {
    /// 開くファイルと残件数を初期化する。
    fn new(store: &'a SpillStore, validation: bool) -> Self {
        Self {
            store,
            validation,
            file: 0,
            reader: None,
            remaining: 0,
            failed: false,
        }
    }
    /// 空ファイルも長さを検査してから次の入力へ進む。
    fn read_next(&mut self) -> Result<Option<([u8; RECORD_LEN], usize)>> {
        while self.file < self.store.files.len() {
            let path = self.store.path(
                if self.validation {
                    "validation"
                } else {
                    "train"
                },
                self.file,
            );
            if self.reader.is_none() {
                let file = &self.store.files[self.file];
                self.remaining = if self.validation {
                    file.validation
                } else {
                    file.training
                };
                self.reader = Some(checked_reader(&path, self.remaining, RECORD_LEN)?);
            }
            if self.remaining == 0 {
                self.reader = None;
                self.file += 1;
                continue;
            }
            let bytes = read_record(self.reader.as_mut().expect("reader opened"), &path)?;
            self.remaining -= 1;
            return Ok(Some((bytes, self.file)));
        }
        Ok(None)
    }
}
impl Iterator for RawRecords<'_> {
    type Item = Result<([u8; RECORD_LEN], usize)>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        match self.read_next() {
            Ok(value) => value.map(Ok),
            Err(error) => {
                self.failed = true;
                Some(Err(error))
            }
        }
    }
}

/// 1つのbucketの固定長記録と順列だけを保持するエポックの読み手。
pub struct EpochReader<'a> {
    store: &'a SpillStore,
    counts: Vec<u64>,
    seed: u64,
    epoch: u64,
    next_bucket: usize,
    records: Vec<[u8; RECORD_LEN]>,
    order: Vec<usize>,
    cursor: usize,
}
impl EpochReader<'_> {
    /// bucketの境界をまたいで最大n件を渡す。全局面を渡したらNoneを返す。
    pub fn next_chunk(&mut self, n: usize) -> Result<Option<Vec<SpillRecord>>> {
        if n == 0 {
            return Err(Error::Invalid("epoch chunk must be positive"));
        }
        let mut result = Vec::with_capacity(n);
        while result.len() < n {
            if self.cursor == self.order.len() {
                // 次のbucketを確保する前に、前の記録と順列のメモリを解放する。
                self.records = Vec::new();
                self.order = Vec::new();
                self.cursor = 0;
                if self.next_bucket == self.counts.len() {
                    break;
                }
                let k = self.next_bucket;
                let path = self.store.directory.0.join(format!("epoch/bucket-{k}.bin"));
                let mut reader = checked_reader(&path, self.counts[k], RECORD_LEN)?;
                let count = usize::try_from(self.counts[k])
                    .map_err(|_| invalid(&path, "bucket exceeds address space"))?;
                self.records
                    .try_reserve_exact(count)
                    .map_err(|error| invalid(&path, error.to_string()))?;
                for _ in 0..count {
                    self.records.push(read_record(&mut reader, &path)?);
                }
                self.order = bucket_permutation(count, self.seed, self.epoch, k as u32);
                self.cursor = 0;
                self.next_bucket += 1;
                if count == 0 {
                    continue;
                }
            }
            let path = self
                .store
                .directory
                .0
                .join(format!("epoch/bucket-{}.bin", self.next_bucket - 1));
            result.push(
                self.store
                    .decode(&self.records[self.order[self.cursor]], &path)?,
            );
            self.cursor += 1;
        }
        Ok((!result.is_empty()).then_some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Dataset, rao_sandelius_order};
    use crate::nnue::data::{BatchSlot, InputFeatureSet, Observations};
    use minase::training::records::{
        Entry, Exclusion, GameRecord, Outcome as GameOutcome, Policy, TeacherKind, Value, Writer,
    };
    use minase_core::{Color, GameResult, Position, WinReason};
    use std::time::Duration;

    /// 独立した実行ディレクトリと、仕様の訓練・検証分割を持つ小さな入力。
    fn fixture() -> (Directory, Vec<PathBuf>) {
        let path = std::env::temp_dir().join(format!(
            "minase-spill-test-{}-{}",
            std::process::id(),
            NEXT_RUN.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let directory = Directory(path);
        let mut paths = Vec::new();
        for (f, seed, origin) in [(0, 11, Origin::RandomSelfplay), (1, 22, Origin::HumanGame)] {
            let path = directory.0.join(format!("input-{f}.mnsd"));
            let header = Header::new(
                "L0,P0,R1,E0".into(),
                "1".repeat(40),
                [1; 32],
                TeacherKind::AlphaBeta,
                1000,
                seed,
            )
            .unwrap();
            let mut writer = Writer::new(File::create(&path).unwrap(), header).unwrap();
            for game in 0..24 {
                let record = GameRecord::new(
                    game,
                    origin,
                    GameOutcome::Game(GameResult::Win {
                        winner: Color::Black,
                        reason: WinReason::Resignation,
                    }),
                    0,
                    Position::initial(),
                    vec![
                        Entry::new(
                            None,
                            Value::AlphaBeta(100 + game as i16),
                            Exclusion::None,
                            Policy::None,
                            None,
                        )
                        .unwrap(),
                    ],
                )
                .unwrap();
                writer.write_game(&record).unwrap();
            }
            writer.finish().unwrap();
            paths.push(path);
        }
        (directory, paths)
    }

    /// テスト用のストアを、指定されたbucket数と上限で作る。
    fn open(directory: &Directory, paths: &[PathBuf], buckets: u32, limit: u64) -> SpillStore {
        SpillStore::create(
            paths,
            None,
            NonZeroUsize::new(2).unwrap(),
            &directory.0,
            buckets,
            limit,
        )
        .unwrap()
        .with_lambda(0.75)
        .unwrap()
    }

    #[test]
    fn epoch_records_match_reference_permutation_across_bucket_and_batch_boundaries() {
        // 仕様2.5と3.1: 参照順列、通し番号、教師情報、満杯バッチ、空bucketを検査する。
        let (directory, paths) = fixture();
        let memory = Dataset::open(&paths, None, NonZeroUsize::new(1).unwrap())
            .unwrap()
            .with_lambda(0.75)
            .unwrap();
        for buckets in [3, 64] {
            let spill = open(&directory, &paths, buckets, 100);
            for epoch in 1..=3 {
                for batch in [1, 7, 100] {
                    let mut reader = spill
                        .epoch(9, epoch, Instant::now() + Duration::from_secs(60))
                        .unwrap()
                        .unwrap();
                    let mut indices = Vec::new();
                    while let Some(records) = reader.next_chunk(batch).unwrap() {
                        let remaining = memory.training_indices().len() - indices.len();
                        assert_eq!(records.len(), batch.min(remaining));
                        let slot =
                            BatchSlot::from_spill_records(InputFeatureSet::P, &records).unwrap();
                        for (record, &index) in records.iter().zip(slot.record_indices()) {
                            assert_eq!(record.index, index);
                            assert_eq!(record.sample, memory.record(index).unwrap());
                            assert_eq!(
                                record.teacher_score.to_bits(),
                                memory.teacher_score(index).unwrap().to_bits()
                            );
                            assert_eq!(record.teacher_class, memory.teacher_class(index).unwrap());
                            indices.push(index);
                        }
                    }
                    let expected: Vec<_> =
                        rao_sandelius_order(memory.training_indices().len(), buckets, 9, epoch)
                            .into_iter()
                            .map(|i| memory.training_indices()[i])
                            .collect();
                    assert_eq!(indices, expected);
                }
            }
        }
    }

    #[test]
    fn preprocessing_and_validation_match_memory_and_cleanup_runs_on_drop() {
        // 仕様2.3、2.4: 検証集合、尺度、観測数と検査和はB=1と同じ。
        let (directory, paths) = fixture();
        let memory = Dataset::open(&paths, None, NonZeroUsize::new(1).unwrap())
            .unwrap()
            .with_lambda(0.75)
            .unwrap();
        let spill = open(&directory, &paths, 3, 100);
        assert_eq!(spill.teacher_classes(), memory.teacher_classes());
        assert_eq!(spill.teacher_lambdas(), memory.teacher_lambdas());
        assert_eq!(spill.record_count(), memory.record_count());
        assert_eq!(
            spill.estimate_teacher_cp_per_logit().unwrap(),
            memory.estimate_teacher_cp_per_logit().unwrap()
        );
        for feature_set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
            let expected = Observations::compute(&memory, feature_set).unwrap();
            let actual = Observations::from_positions(
                spill
                    .training_records()
                    .map(|r| Ok(r?.sample.to_position()?)),
                feature_set,
            )
            .unwrap();
            assert_eq!(actual.canonical_counts(), expected.canonical_counts());
            assert_eq!(actual.input_counts(), expected.input_counts());
            assert_eq!(actual.virtual_mask(), expected.virtual_mask());
        }
        let validation: Vec<_> = spill
            .validation_chunks(3)
            .collect::<Result<Vec<_>>>()
            .unwrap()
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(
            validation.iter().map(|r| r.index).collect::<Vec<_>>(),
            memory.validation_indices()
        );
        for record in validation {
            assert_eq!(record.sample, memory.record(record.index).unwrap());
        }
        for (indices, hash) in [
            (memory.training_indices(), spill.training_indices_sha256()),
            (
                memory.validation_indices(),
                spill.validation_indices_sha256(),
            ),
        ] {
            let mut expected = Sha256Hasher::default();
            for index in indices {
                expected.update(&index.to_le_bytes());
            }
            assert_eq!(hash, hex(&expected.finalize()));
        }
        let selected = spill.validation_sample(3, 7).unwrap();
        assert_eq!(
            selected.iter().map(|r| r.index).collect::<Vec<_>>(),
            floyd_sample(memory.validation_indices().len() as u64, 3, 7)
                .unwrap()
                .into_iter()
                .map(|i| memory.validation_indices()[i as usize])
                .collect::<Vec<_>>()
        );
        assert_eq!(
            spill.validation_sample(100, 7).unwrap().len(),
            memory.validation_indices().len()
        );
        assert!(matches!(
            spill.validation_sample(0, 7),
            Err(Error::Invalid(_))
        ));
        let path = spill.directory().to_owned();
        assert_eq!(
            fs::read_to_string(path.join("lock")).unwrap().trim(),
            std::process::id().to_string()
        );
        drop(spill);
        assert!(!path.exists());
    }

    #[test]
    fn truncation_at_record_boundary_and_inside_record_is_spill_error() {
        // 仕様2.2、検証4: 件数照合は完全な記録を失った場合も検出する。
        let (directory, paths) = fixture();
        for kind in ["train", "validation", "scale", "bucket"] {
            for boundary in [true, false] {
                let spill = open(&directory, &paths, 3, 100);
                let mut epoch = spill
                    .epoch(3, 1, Instant::now() + Duration::from_secs(60))
                    .unwrap()
                    .unwrap();
                let path = if kind == "bucket" {
                    spill.directory().join("epoch/bucket-0.bin")
                } else {
                    spill.path(kind, 0)
                };
                let len = fs::metadata(&path).unwrap().len();
                let record_len = if kind == "scale" {
                    SCALE_LEN
                } else {
                    RECORD_LEN
                } as u64;
                assert!(len >= record_len);
                OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_len(len - if boundary { record_len } else { 1 })
                    .unwrap();
                let error = match kind {
                    "train" => spill.for_each_training(|_| Ok(())).unwrap_err(),
                    "validation" => spill.validation_chunks(100).next().unwrap().unwrap_err(),
                    "scale" => spill.estimate_teacher_cp_per_logit().unwrap_err(),
                    "bucket" => epoch.next_chunk(100).unwrap_err(),
                    _ => unreachable!(),
                };
                assert!(matches!(error, Error::Spill { .. }), "{kind}: {error}");
            }
        }
    }

    #[test]
    fn overflow_and_deadline_never_return_partial_epochs() {
        // 仕様2.4: 上限超過を引き直さず報告し、期限超過は未採用として返す。
        let (directory, paths) = fixture();
        let spill = open(&directory, &paths, 3, 1);
        let error = match spill.epoch(1, 1, Instant::now() + Duration::from_secs(60)) {
            Err(error) => error,
            _ => panic!("overflow was not rejected"),
        };
        assert!(
            matches!(error, Error::BucketOverflow { positions, limit: 1, buckets: 3, .. } if positions > 1)
        );
        assert!(spill.epoch(1, 2, Instant::now()).unwrap().is_none());
        let path = spill.directory().to_owned();
        drop(spill);
        assert!(!path.exists());
        let bad = vec![directory.0.join("missing.mnsd")];
        assert!(
            SpillStore::create(
                &bad,
                None,
                NonZeroUsize::new(1).unwrap(),
                &directory.0,
                3,
                10
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
    }
}
