//! 対局実行条件と完了ペアの永続化。
//!
//! # 対局測定の実行ディレクトリ
//!
//! 全JSONはUTF-8で、各構造体のフィールド名をそのままキーにする。
//! 任意値の欠測は`null`として書き、列挙型の名前は小文字のスネークケースとする。
//!
//! `manifest.json`は[`RunManifest`]を1個保存し、`format_version`は5である。
//! `ponder`は先読みの真偽、`candidate`と`baseline`は`identity`と`limit`を持つエンジン設定である。
//! `identity`は[`EngineIdentity`]の`kind`つきオブジェクトである。
//! `limit`は`kind="fixed"`なら`depth`と`nodes`、`kind="time"`なら
//! `base_ms`、`increment_ms`、`byoyomi_ms`、`kind="movetime"`なら
//! `movetime_ms`と`timemargin_ms`を持ち、時間の単位はミリ秒である。
//! `rules_source`は起動時の規則指定、`canonical_rules`は審判用の規則コードの文字列配列である。
//! `mode`は`kind="elo"`、または`kind="gsprt"`と数値の`h0_elo`、`h1_elo`、`alpha`、`beta`である。
//! `seed`はu64の基本シード、`max_ply`はu32の手数上限、`response_timeout_secs`はu64の応答待ち秒数である。
//! `engine_threads`と`hash_mb`はそれぞれ`candidate`と`baseline`を持ち、探索ワーカー数と置換表容量MBを表す。
//! `concurrency`は同時に実行するペア数である。
//!
//! `invocations.json`は[`Invocation`]の配列で、各要素は`runner`、`cpu`、`target_pairs`を持つ。
//! `runner`の`version`はパッケージ版、`sha256`は実行ファイル全体の検査和の文字列である。
//! `cpu`は機種名`model`、物理コア数`physical_cores`、論理コア数`logical_cores`、
//! 実メモリ容量バイト数`physical_memory_bytes`を持ち、物理コア数とメモリ容量の欠測は`null`である。
//! `target_pairs`はその起動の目標ペア数u64である。
//!
//! `summary.json`は[`RunSummary`]を1個保存する。
//! `active_wall_time_ns`は完了ペアを待つ有効実行時間の累計u64で、単位はナノ秒である。
//! `interrupted`は正常終了しなかった起動があるか、`invocation_active`は対局を進める起動が進行中かを表す真偽値である。
//! 勝敗やEloの統計はこのファイルには含めない。
//!
//! `pairs/<20桁のペア番号>.json`は1起算の番号ごとに[`PairRecord`]を保存する。
//! 欄と入れ子の構造は[`minase::harness::PairRecord`]に定める。
//! `.manifest.json.tmp`、`.invocations.json.tmp`、`.summary.json.tmp`、
//! `pairs/.<20桁のペア番号>.json.tmp`は確定前の一時ファイルである。
//! ロックファイル`.match_runner.lock`と原子的保存は[`minase::harness::lock_run_directory`]に定める。

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use minase::harness::{
    CpuRecord, EngineIdentity, HarnessRecord, PairRecord, ResumeError, RunLock,
    atomic_replace_json, atomic_write_json, lock_run_directory,
};
use serde::{Deserialize, Serialize};

/// 再開で受け付ける実行記録形式の版5。
/// 旧版は、新しい欄を型として読み取る前に版番号で拒否する。
pub(super) const FORMAT_VERSION: u32 = 5;

const MANIFEST_FILE: &str = "manifest.json";
const MANIFEST_TEMP_FILE: &str = ".manifest.json.tmp";
const PAIRS_DIRECTORY: &str = "pairs";
const SUMMARY_FILE: &str = "summary.json";
const SUMMARY_TEMP_FILE: &str = ".summary.json.tmp";

/// 1エンジンの実効設定。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EngineRecord {
    /// エンジン実体の識別情報。
    pub(super) identity: EngineIdentity,
    /// 実効探索制限。
    pub(super) limit: StoredSearchLimit,
}

/// 保存用の探索制限。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum StoredSearchLimit {
    /// 深さまたはノード数による固定制限。
    Fixed {
        /// 探索深さの上限。
        depth: Option<u32>,
        /// 探索ノード数の上限。
        nodes: Option<u64>,
    },
    /// 持ち時間による制限。
    Time {
        /// 初期持ち時間(ms)。
        base_ms: u64,
        /// 1手ごとの加算時間(ms)。
        increment_ms: u64,
        /// 1手ごとの秒読み(ms)。
        byoyomi_ms: u64,
    },
    /// 1手ごとの固定思考時間。
    Movetime {
        /// エンジンへ送る思考時間(ms)。
        movetime_ms: u64,
        /// 時間切れ判定に加える許容時間(ms)。
        timemargin_ms: u64,
    },
}

/// 候補と基準の探索ワーカー数。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EngineThreadCounts {
    /// 候補側。報告されない場合は`None`。
    pub(super) candidate: Option<u32>,
    /// 基準側。報告されない場合は`None`。
    pub(super) baseline: Option<u32>,
}

/// 候補と基準の置換表容量。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EngineHashSizes {
    /// 候補側の容量(MB)。報告されない場合は`None`。
    pub(super) candidate: Option<u64>,
    /// 基準側の容量(MB)。報告されない場合は`None`。
    pub(super) baseline: Option<u64>,
}

/// 統計モードの実験同一性を決める部分。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ManifestMode {
    /// ペンタノミアルGSPRT。
    Gsprt {
        /// 帰無仮説のElo。
        h0_elo: f64,
        /// 対立仮説のElo。
        h1_elo: f64,
        /// 第1種過誤率。
        alpha: f64,
        /// 第2種過誤率。
        beta: f64,
    },
    /// 固定ペア数Elo推定。
    Elo,
}

/// 新規実行時に確定する実行条件記録。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RunManifest {
    /// 記録形式の版。
    pub(super) format_version: u32,
    /// 両エンジンに適用する先読みの対局条件。欄は省略できない。
    pub(super) ponder: bool,
    /// 候補エンジンの設定。
    pub(super) candidate: EngineRecord,
    /// 基準エンジンの設定。
    pub(super) baseline: EngineRecord,
    /// エンジンへ渡した規則指定。
    pub(super) rules_source: String,
    /// 審判層が使う正準規則コード列。
    pub(super) canonical_rules: Vec<String>,
    /// 統計モードとGSPRTの仮説。
    pub(super) mode: ManifestMode,
    /// ペアシードを派生させる基本シード。
    pub(super) seed: u64,
    /// 1局の手数上限。
    pub(super) max_ply: u32,
    /// 1回のエンジン応答を待つ秒数。
    pub(super) response_timeout_secs: u64,
    /// 両エンジンの探索ワーカー数。
    pub(super) engine_threads: EngineThreadCounts,
    /// 両エンジンの置換表容量。
    pub(super) hash_mb: EngineHashSizes,
    /// 同時に実行するペア数。
    pub(super) concurrency: usize,
}

/// 実際に対局を進めた起動の記録。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Invocation {
    pub(super) runner: HarnessRecord,
    pub(super) cpu: CpuRecord,
    pub(super) target_pairs: u64,
}

impl Invocation {
    fn current(target_pairs: u64) -> io::Result<Self> {
        use minase::harness::{cpu_record, harness_record};
        Ok(Self {
            runner: harness_record()?,
            cpu: cpu_record()?,
            target_pairs,
        })
    }
}

/// 再開を含む実験の有効実行時間。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunSummary {
    /// 完了ペアを待っていた実行時間の累計(ns)。
    active_wall_time_ns: u64,
    /// 過去の起動が正常終了しなかった場合は真。
    interrupted: bool,
    /// 対局を実行する起動が進行中なら真。
    invocation_active: bool,
}

/// 1つの実行ディレクトリを所有する永続化層。
#[derive(Debug)]
pub(super) struct RunStore {
    pub(super) manifest: RunManifest,
    pub(super) target_pairs: u64,
    invocations: Vec<Invocation>,
    resumed: bool,
    root: PathBuf,
    pairs: PathBuf,
    active_wall_time_ns: u64,
    interrupted: bool,
    _lock: RunLock,
}

impl RunStore {
    /// 存在しないパスへ新規実行ディレクトリを作る。
    ///
    /// `manifest.json`、`invocations.json`、`summary.json`、および`pairs/`を作る。
    /// 3つのJSONはそれぞれ原子的に書き、実行条件の`manifest.json`は以後変更しない。
    /// 初回の起動記録はここで作り、実行開始の状態は[`Self::begin_invocation`]で記録する。
    pub(super) fn create(
        path: &Path,
        manifest: RunManifest,
        target_pairs: u64,
    ) -> io::Result<Self> {
        if manifest.format_version != FORMAT_VERSION {
            return Err(invalid_data(format!(
                "unsupported manifest format version {}",
                manifest.format_version
            )));
        }
        if target_pairs == 0 {
            return Err(ResumeError::InvalidTarget.into());
        }
        let invocations = vec![Invocation::current(target_pairs)?];
        fs::create_dir(path)?;
        let lock = lock_run_directory(path)?;
        let pairs = path.join(PAIRS_DIRECTORY);
        fs::create_dir(&pairs)?;
        atomic_write_json(
            path,
            &path.join(MANIFEST_TEMP_FILE),
            &path.join(MANIFEST_FILE),
            &manifest,
        )?;
        atomic_write_json(
            path,
            &path.join(".invocations.json.tmp"),
            &path.join("invocations.json"),
            &invocations,
        )?;
        atomic_write_json(
            path,
            &path.join(SUMMARY_TEMP_FILE),
            &path.join(SUMMARY_FILE),
            &RunSummary {
                active_wall_time_ns: 0,
                interrupted: false,
                invocation_active: false,
            },
        )?;
        Ok(Self {
            manifest,
            target_pairs,
            invocations,
            resumed: false,
            root: path.to_owned(),
            pairs,
            active_wall_time_ns: 0,
            interrupted: false,
            _lock: lock,
        })
    }

    /// 版5の実行ディレクトリを検証し、保存済みペアを読み込む。
    ///
    /// `requested_target`を指定する場合は直前の起動の目標より大きい値を要求し、
    /// 省略時は直前の目標を使う。条件は保存済みの`manifest.json`から読む。
    /// 未実行のペアがある場合だけ、呼び出し側が[`Self::begin_invocation`]へ進む。
    /// 理由は`docs/adr/resume-from-the-manifest-and-accept-rebuilt-tuning-commits.md`にある。
    pub(super) fn resume(
        path: &Path,
        requested_target: Option<u64>,
    ) -> io::Result<(Self, BTreeMap<u64, PairRecord>)> {
        if !path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("run directory does not exist: {}", path.display()),
            ));
        }
        if !path.is_dir() {
            return Err(invalid_data(format!(
                "run directory is not a directory: {}",
                path.display()
            )));
        }
        let lock = lock_run_directory(path)?;

        let value: serde_json::Value = read_json(&path.join(MANIFEST_FILE))?;
        if value["format_version"].as_u64() != Some(u64::from(FORMAT_VERSION)) {
            return Err(invalid_data(format!(
                "unsupported manifest format version {}",
                value["format_version"]
            )));
        }
        let manifest: RunManifest = serde_json::from_value(value).map_err(json_error)?;
        let invocations: Vec<Invocation> = read_json(&path.join("invocations.json"))?;
        let last = invocations.last().ok_or(ResumeError::EmptyInvocations)?;
        if invocations.iter().any(|entry| entry.target_pairs == 0)
            || invocations
                .windows(2)
                .any(|entries| entries[0].target_pairs > entries[1].target_pairs)
        {
            return Err(ResumeError::InvalidTarget.into());
        }
        let target_pairs = match requested_target {
            Some(target) if target <= last.target_pairs => {
                return Err(ResumeError::TargetNotIncreased.into());
            }
            Some(target) => target,
            None => last.target_pairs,
        };

        let pairs_directory = path.join(PAIRS_DIRECTORY);
        if !pairs_directory.is_dir() {
            return Err(invalid_data(format!(
                "pair record directory is missing: {}",
                pairs_directory.display()
            )));
        }
        let records = load_pairs(&pairs_directory, target_pairs)?;
        let summary: RunSummary = read_json(&path.join(SUMMARY_FILE))?;
        let interrupted = summary.interrupted || summary.invocation_active;
        Ok((
            Self {
                manifest,
                target_pairs,
                invocations,
                resumed: true,
                root: path.to_owned(),
                pairs: pairs_directory,
                active_wall_time_ns: summary.active_wall_time_ns,
                interrupted,
                _lock: lock,
            },
            records,
        ))
    }

    /// ロック保持中の確定ペアを読み直す。
    pub(super) fn records(&self, target_pairs: u64) -> io::Result<BTreeMap<u64, PairRecord>> {
        load_pairs(&self.pairs, target_pairs)
    }

    /// 実行ディレクトリを返す。
    pub(super) fn path(&self) -> &Path {
        &self.root
    }

    /// 完了ペアを一時ファイルからrenameして確定する。
    pub(super) fn save_pair(&self, record: &PairRecord) -> io::Result<()> {
        validate_record_header(record, u64::MAX)?;
        let final_path = self.pairs.join(pair_file_name(record.pair_number));
        if final_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("pair {} is already recorded", record.pair_number),
            ));
        }
        let temporary_path = self.pairs.join(pair_temp_file_name(record.pair_number));
        remove_stale_temp(&temporary_path)?;
        atomic_write_json(&self.pairs, &temporary_path, &final_path, record)
    }

    /// 未実行のペアがある起動について、対局実行の開始を記録する。
    ///
    /// 再開時は実行プログラムと測定機を調べ、前回と異なれば標準エラーへ1行通知する。
    /// 続けて`invocations.json`へ起動記録と目標を原子的に追記し、
    /// その後に`summary.json`へ実行中の状態を書く。
    /// 対局の開始より先に起動記録を確定するので、ペアだけが記録済みの目標を超えて保存されることはない。
    pub(super) fn begin_invocation(&self) -> io::Result<()> {
        if self.resumed {
            let current = Invocation::current(self.target_pairs)?;
            let previous = self
                .invocations
                .last()
                .ok_or(ResumeError::EmptyInvocations)?;
            minase::harness::notify_invocation_change(
                &current.runner,
                &current.cpu,
                &previous.runner,
                &previous.cpu,
            );
            let mut invocations = self.invocations.clone();
            invocations.push(current);
            atomic_replace_json(
                &self.root,
                &self.root.join(".invocations.json.tmp"),
                &self.root.join("invocations.json"),
                &invocations,
            )?;
        }
        self.write_summary(self.active_wall_time_ns, true)
    }

    /// 現在の起動時間を累計実行時間へ反映する。
    pub(super) fn checkpoint(&self, invocation_elapsed: std::time::Duration) -> io::Result<()> {
        let elapsed = u64::try_from(invocation_elapsed.as_nanos())
            .map_err(|_| invalid_data("invocation elapsed time exceeds u64 nanoseconds"))?;
        let active_wall_time_ns = self
            .active_wall_time_ns
            .checked_add(elapsed)
            .ok_or_else(|| invalid_data("cumulative active wall time overflow"))?;
        self.write_summary(active_wall_time_ns, true)
    }

    /// 現在の対局実行を正常終了として確定する。
    pub(super) fn finish_invocation(
        &self,
        invocation_elapsed: std::time::Duration,
    ) -> io::Result<()> {
        let elapsed = u64::try_from(invocation_elapsed.as_nanos())
            .map_err(|_| invalid_data("invocation elapsed time exceeds u64 nanoseconds"))?;
        let active_wall_time_ns = self
            .active_wall_time_ns
            .checked_add(elapsed)
            .ok_or_else(|| invalid_data("cumulative active wall time overflow"))?;
        self.write_summary(active_wall_time_ns, false)
    }

    /// 実行時間と起動状態を原子的に置き換える。
    fn write_summary(&self, active_wall_time_ns: u64, invocation_active: bool) -> io::Result<()> {
        atomic_replace_json(
            &self.root,
            &self.root.join(SUMMARY_TEMP_FILE),
            &self.root.join(SUMMARY_FILE),
            &RunSummary {
                active_wall_time_ns,
                interrupted: self.interrupted,
                invocation_active,
            },
        )
    }
}

/// ディレクトリ内の確定済みペアを厳密に読み込む。
fn load_pairs(directory: &Path, target_pairs: u64) -> io::Result<BTreeMap<u64, PairRecord>> {
    let mut records = BTreeMap::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid_data("pair record filename is not UTF-8"))?;
        if is_pair_temp_file_name(&name) {
            if !file_type.is_file() {
                return Err(invalid_data(format!(
                    "pair temporary path is not a file: {}",
                    entry.path().display()
                )));
            }
            continue;
        }
        if !file_type.is_file() {
            return Err(invalid_data(format!(
                "unexpected entry in pair record directory: {}",
                entry.path().display()
            )));
        }
        let number = parse_pair_file_name(&name)
            .ok_or_else(|| invalid_data(format!("invalid pair record filename: {name}")))?;
        let record: PairRecord = read_json(&entry.path())?;
        validate_record_header(&record, target_pairs)?;
        if record.pair_number != number {
            return Err(invalid_data(format!(
                "pair filename {number} contains record {}",
                record.pair_number
            )));
        }
        if records.insert(number, record).is_some() {
            return Err(invalid_data(format!("duplicate pair record {number}")));
        }
    }
    Ok(records)
}

/// JSONとして解釈できるペア記録の局面非依存条件を検証する。
fn validate_record_header(record: &PairRecord, target_pairs: u64) -> io::Result<()> {
    if record.pair_number == 0 {
        return Err(invalid_data("pair number must be at least 1"));
    }
    if record.pair_number > target_pairs {
        return Err(invalid_data(format!(
            "saved pair {} exceeds requested target {target_pairs}",
            record.pair_number
        )));
    }
    if record.category.is_some_and(|category| category > 4) {
        return Err(invalid_data(format!(
            "pair {} has category outside 0..=4",
            record.pair_number
        )));
    }
    Ok(())
}

/// 指定JSONファイル全体を読み込む。
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<T> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(json_error)
}

/// 前回の未確定一時ファイルだけを除去する。
fn remove_stale_temp(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// ペア番号を辞書順と数値順が一致するファイル名へ変換する。
fn pair_file_name(number: u64) -> String {
    format!("{number:020}.json")
}

/// ペア番号に対応する一時ファイル名を返す。
fn pair_temp_file_name(number: u64) -> String {
    format!(".{number:020}.json.tmp")
}

/// 正準ペアファイル名なら番号を返す。
fn parse_pair_file_name(name: &str) -> Option<u64> {
    let digits = name.strip_suffix(".json")?;
    if digits.len() != 20 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let number = digits.parse().ok()?;
    (number != 0 && pair_file_name(number) == name).then_some(number)
}

/// ハーネスが作るペア一時ファイル名かどうかを返す。
fn is_pair_temp_file_name(name: &str) -> bool {
    let Some(digits) = name
        .strip_prefix('.')
        .and_then(|name| name.strip_suffix(".json.tmp"))
    else {
        return false;
    };
    digits.len() == 20 && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// JSONエラーを破損記録として返す。
fn json_error(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

/// 破損または条件不一致を表すエラーを返す。
fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase::harness::{
        EvaluationRecord, GameRecord, OpeningRecord, ScoreBound, ScoreRecord, StopReasonRecord,
        StoredColor, TerminationRecord, TurnRecord, TurnResponse,
    };

    struct Directory(std::path::PathBuf);
    impl std::ops::Deref for Directory {
        type Target = std::path::Path;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }
    impl AsRef<std::path::Path> for Directory {
        fn as_ref(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            match std::fs::remove_dir_all(&self.0) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("cannot remove {}: {error}", self.0.display()),
            }
        }
    }
    fn temporary_directory(test: &str) -> Directory {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Directory(std::env::temp_dir().join(format!(
            "minase-match-storage-{test}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }

    fn manifest() -> RunManifest {
        RunManifest {
            format_version: FORMAT_VERSION,
            ponder: false,
            candidate: EngineRecord {
                identity: EngineIdentity::Commit {
                    hash: "a".repeat(40),
                    sha256: "d".repeat(64),
                },
                limit: StoredSearchLimit::Fixed {
                    depth: None,
                    nodes: Some(100_000),
                },
            },
            baseline: EngineRecord {
                identity: EngineIdentity::Random,
                limit: StoredSearchLimit::Time {
                    base_ms: 10_000,
                    increment_ms: 100,
                    byoyomi_ms: 0,
                },
            },
            rules_source: "engine-default".to_owned(),
            canonical_rules: vec!["L1".to_owned(), "P0".to_owned()],
            mode: ManifestMode::Gsprt {
                h0_elo: 0.0,
                h1_elo: 5.0,
                alpha: 0.05,
                beta: 0.05,
            },
            seed: 20260828,
            max_ply: 4096,
            response_timeout_secs: 120,
            engine_threads: EngineThreadCounts {
                candidate: Some(1),
                baseline: Some(1),
            },
            hash_mb: EngineHashSizes {
                candidate: Some(256),
                baseline: Some(256),
            },
            concurrency: 8,
        }
    }

    fn game(candidate_color: StoredColor) -> GameRecord {
        GameRecord {
            candidate_color,
            candidate_seed: 2,
            baseline_seed: 3,
            wall_time_ns: 84,
            candidate_cpu_time_ns: Some(30),
            baseline_cpu_time_ns: Some(40),
            candidate_peak_rss_bytes: Some(1024),
            baseline_peak_rss_bytes: Some(2048),
            turns: vec![TurnRecord {
                side: StoredColor::Black,
                think_time_ns: 42,
                evaluation: Some(EvaluationRecord {
                    perspective: StoredColor::Black,
                    depth: Some(3),
                    score: ScoreRecord::Cp { value: 12 },
                    bound: ScoreBound::Exact,
                }),
                stop_reason: Some(StopReasonRecord::Hard),
                completed_time_ms: Some(40),
                ponder: None,
                response: TurnResponse::Resigned,
            }],
            termination: TerminationRecord::Resigned {
                loser: StoredColor::Black,
            },
        }
    }

    fn pair(number: u64) -> PairRecord {
        PairRecord {
            pair_number: number,
            pair_seed: 1,
            opening: OpeningRecord {
                seed: 4,
                moves: vec!["1a1b".to_owned()],
            },
            games: [game(StoredColor::Black), game(StoredColor::White)],
            category: Some(2),
        }
    }

    #[test]
    fn resume_validates_invocation_history_before_writing() {
        let path = temporary_directory("invalid-history");
        drop(RunStore::create(&path, manifest(), 2).unwrap());
        let original: Vec<Invocation> = read_json(&path.join("invocations.json")).unwrap();
        for (history, expected) in [
            (Vec::new(), ResumeError::EmptyInvocations),
            (
                vec![Invocation {
                    target_pairs: 0,
                    ..original[0].clone()
                }],
                ResumeError::InvalidTarget,
            ),
            (
                vec![
                    original[0].clone(),
                    Invocation {
                        target_pairs: 1,
                        ..original[0].clone()
                    },
                ],
                ResumeError::InvalidTarget,
            ),
        ] {
            fs::write(
                path.join("invocations.json"),
                serde_json::to_vec(&history).unwrap(),
            )
            .unwrap();
            let error = RunStore::resume(&path, None).unwrap_err();
            assert_eq!(
                error.get_ref().unwrap().downcast_ref::<ResumeError>(),
                Some(&expected)
            );
        }
    }

    #[test]
    fn invocation_target_is_persisted_before_active_summary() {
        let path = temporary_directory("invocation-order");
        drop(RunStore::create(&path, manifest(), 1).unwrap());
        let (store, _) = RunStore::resume(&path, Some(2)).unwrap();
        fs::remove_file(path.join(SUMMARY_FILE)).unwrap();
        fs::create_dir(path.join(SUMMARY_FILE)).unwrap();
        assert!(store.begin_invocation().is_err());
        let history: Vec<Invocation> = read_json(&path.join("invocations.json")).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].target_pairs, 2);
        assert!(!path.join(".invocations.json.tmp").exists());
    }

    #[test]
    fn create_save_and_resume_round_trip() {
        let path = temporary_directory("round-trip");
        let expected = manifest();
        let store = RunStore::create(&path, expected.clone(), 1).unwrap();
        store.save_pair(&pair(2)).unwrap();
        store.save_pair(&pair(1)).unwrap();
        drop(store);

        let (store, records) = RunStore::resume(&path, Some(2)).unwrap();
        assert_eq!(store.path(), &*path);
        assert_eq!(records.keys().copied().collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(records[&2], pair(2));
    }

    #[test]
    fn checkpoints_accumulate_across_resumed_invocations() {
        let path = temporary_directory("checkpoint");
        let expected = manifest();
        let store = RunStore::create(&path, expected.clone(), 1).unwrap();
        store.begin_invocation().unwrap();
        store.checkpoint(std::time::Duration::from_secs(2)).unwrap();
        drop(store);

        let (store, _) = RunStore::resume(&path, None).unwrap();
        assert_eq!(store.active_wall_time_ns, 2_000_000_000);
        assert!(store.interrupted);
        store.begin_invocation().unwrap();
        store
            .finish_invocation(std::time::Duration::from_secs(3))
            .unwrap();
        drop(store);

        let summary: RunSummary = read_json(&path.join(SUMMARY_FILE)).unwrap();
        assert_eq!(summary.active_wall_time_ns, 5_000_000_000);
        assert!(summary.interrupted);
        assert!(!summary.invocation_active);
    }

    #[test]
    fn create_rejects_an_existing_directory_and_resume_requires_one() {
        let path = temporary_directory("directory-contract");
        fs::create_dir(&path).unwrap();
        assert_eq!(
            RunStore::create(&path, manifest(), 1).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        fs::remove_dir(&path).unwrap();
        assert_eq!(
            RunStore::resume(&path, None).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn a_run_directory_cannot_be_opened_by_two_process_owners() {
        let path = temporary_directory("exclusive-lock");
        let expected = manifest();
        let store = RunStore::create(&path, expected.clone(), 1).unwrap();
        assert_eq!(
            RunStore::resume(&path, None).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(store);
        RunStore::resume(&path, None).unwrap();
    }

    #[test]
    fn resume_rejects_nonincreasing_target_and_saved_number_above_target() {
        let path = temporary_directory("resume-validation");
        let expected = manifest();
        let store = RunStore::create(&path, expected.clone(), 1).unwrap();
        store.save_pair(&pair(1)).unwrap();
        drop(store);
        let error = RunStore::resume(&path, Some(1)).unwrap_err();
        assert_eq!(
            error.get_ref().unwrap().downcast_ref::<ResumeError>(),
            Some(&ResumeError::TargetNotIncreased)
        );
        let (store, _) = RunStore::resume(&path, None).unwrap();
        store.save_pair(&pair(2)).unwrap();
        drop(store);
        assert_eq!(
            RunStore::resume(&path, None).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn resume_rejects_corruption_filename_mismatch_and_invalid_category() {
        let path = temporary_directory("pair-validation");
        let expected = manifest();
        let store = RunStore::create(&path, expected.clone(), 1).unwrap();
        store.save_pair(&pair(1)).unwrap();
        drop(store);

        fs::write(path.join(PAIRS_DIRECTORY).join(pair_file_name(2)), b"{").unwrap();
        assert_eq!(
            RunStore::resume(&path, Some(2)).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_file(path.join(PAIRS_DIRECTORY).join(pair_file_name(2))).unwrap();

        let wrong_name = path.join(PAIRS_DIRECTORY).join(pair_file_name(2));
        fs::write(&wrong_name, serde_json::to_vec(&pair(3)).unwrap()).unwrap();
        assert_eq!(
            RunStore::resume(&path, Some(3)).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_file(&wrong_name).unwrap();

        let mut invalid = pair(2);
        invalid.category = Some(5);
        fs::write(
            path.join(PAIRS_DIRECTORY).join(pair_file_name(2)),
            serde_json::to_vec(&invalid).unwrap(),
        )
        .unwrap();
        assert_eq!(
            RunStore::resume(&path, Some(2)).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn stale_temporary_pair_is_ignored_and_removed_before_resave() {
        let path = temporary_directory("stale-temp");
        let expected = manifest();
        let store = RunStore::create(&path, expected.clone(), 1).unwrap();
        let temporary = path.join(PAIRS_DIRECTORY).join(pair_temp_file_name(1));
        fs::write(&temporary, b"partial").unwrap();
        drop(store);

        let (store, records) = RunStore::resume(&path, None).unwrap();
        assert!(records.is_empty());
        store.save_pair(&pair(1)).unwrap();
        assert!(!temporary.exists());
        assert!(path.join(PAIRS_DIRECTORY).join(pair_file_name(1)).exists());
    }

    #[test]
    fn save_rejects_duplicate_pair_without_overwriting_it() {
        let path = temporary_directory("duplicate");
        let store = RunStore::create(&path, manifest(), 1).unwrap();
        let original = pair(1);
        store.save_pair(&original).unwrap();
        let mut replacement = original.clone();
        replacement.category = Some(4);
        assert_eq!(
            store.save_pair(&replacement).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        let saved: PairRecord = read_json(
            &path
                .join(PAIRS_DIRECTORY)
                .join(pair_file_name(original.pair_number)),
        )
        .unwrap();
        assert_eq!(saved, original);
    }

    // [`RunManifest::ponder`]と[`minase::harness::TurnRecord::ponder`]の欄は省略できない。
    #[test]
    fn ponder_manifest_and_prediction_fields_are_required() {
        let turn = game(StoredColor::Black).turns.remove(0);
        let mut value = serde_json::to_value(&turn).unwrap();
        value.as_object_mut().unwrap().remove("ponder");
        assert!(serde_json::from_value::<TurnRecord>(value).is_err());
        let mut value = serde_json::to_value(manifest()).unwrap();
        value.as_object_mut().unwrap().remove("ponder");
        assert!(serde_json::from_value::<RunManifest>(value).is_err());
    }

    // [`FORMAT_VERSION`]に従い、新しい欄の有無を調べる前に旧版を拒否する。
    #[test]
    fn resume_rejects_old_versions_before_decoding_new_fields_without_writes() {
        for version in [3, 4] {
            let path = temporary_directory("old-version");
            drop(RunStore::create(&path, manifest(), 1).unwrap());
            fs::write(
                path.join(MANIFEST_FILE),
                format!("{{\"format_version\":{version}}}"),
            )
            .unwrap();
            let before: Vec<_> = [MANIFEST_FILE, "invocations.json", "summary.json"]
                .into_iter()
                .map(|name| (name, fs::read(path.join(name)).unwrap()))
                .collect();
            let error = RunStore::resume(&path, None).unwrap_err();
            assert_eq!(
                error.kind(),
                io::ErrorKind::InvalidData,
                "version {version}"
            );
            assert!(
                error
                    .to_string()
                    .contains(&format!("unsupported manifest format version {version}")),
                "version {version}: {error}"
            );
            for (name, bytes) in before {
                assert_eq!(
                    fs::read(path.join(name)).unwrap(),
                    bytes,
                    "version {version}: {name}"
                );
            }
            fs::remove_dir_all(path).unwrap();
        }
    }
}
