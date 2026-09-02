//! 対局実行条件と完了ペアの永続化。

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Deserializer, Serialize};

/// 現行の実行記録形式。
pub(super) const FORMAT_VERSION: u32 = 3;

const MANIFEST_FILE: &str = "manifest.json";
const MANIFEST_TEMP_FILE: &str = ".manifest.json.tmp";
const PAIRS_DIRECTORY: &str = "pairs";
const SUMMARY_FILE: &str = "summary.json";
const SUMMARY_TEMP_FILE: &str = ".summary.json.tmp";

/// 実行バイナリの識別情報。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HarnessRecord {
    /// Cargoパッケージの版。
    pub(super) version: String,
    /// 実行ファイル全体のSHA-256。
    pub(super) sha256: String,
}

/// 測定に使うCPUの識別情報。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CpuRecord {
    /// CPUの機種名。
    pub(super) model: String,
    /// 取得できた場合の物理コア数。
    pub(super) physical_cores: Option<usize>,
    /// OSが報告した論理コア数。
    pub(super) logical_cores: usize,
    /// OSが報告した実メモリ容量(byte)。
    pub(super) physical_memory_bytes: Option<u64>,
}

/// エンジンとの通信プロトコル。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredProtocol {
    /// Universal Shogi Interface。
    Usi,
    /// Chess Engine Communication Protocol。
    Cecp,
}

/// エンジン実体の識別方法。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum EngineIdentity {
    /// 同梱のランダムエンジン。
    Random {
        /// 実行ファイル全体のSHA-256。
        sha256: String,
    },
    /// Gitコミットからビルドしたエンジン。
    Commit {
        /// 完全コミットハッシュ。
        hash: String,
        /// 実際に起動するキャッシュ済みバイナリのSHA-256。
        sha256: String,
    },
    /// 任意の起動コマンド。
    Command {
        /// 実行ファイルの指定。
        program: PathBuf,
        /// 起動引数。
        args: Vec<String>,
        /// 通信プロトコル。
        protocol: StoredProtocol,
        /// 相対パスの解釈に使う作業ディレクトリ。
        working_directory: PathBuf,
    },
}

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

/// 再開時に完全一致を要求する実行条件記録。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RunManifest {
    /// 記録形式の版。
    pub(super) format_version: u32,
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
    /// CPUの識別情報。
    pub(super) cpu: CpuRecord,
    /// 対局ハーネス実行バイナリの識別情報。
    pub(super) runner: HarnessRecord,
}

/// 保存用の手番。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredColor {
    /// 先手。
    Black,
    /// 後手。
    White,
}

/// エンジン異常の分類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FailureKind {
    /// 不正着手。
    IllegalMove,
    /// プロセス終了またはパイプ切断。
    Crash,
    /// 応答タイムアウト。
    Timeout,
    /// 持ち時間の超過。
    TimeForfeit,
    /// 相手の合法手の拒否。
    RejectedMove,
}

/// エンジンが返した評価値。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ScoreRecord {
    /// 通常評価値。
    Cp {
        /// エンジン固有尺度の評価値。
        value: i32,
    },
    /// 手番側が詰ませる評価値。
    MateIn {
        /// 詰みまでの手数。手数不明の`+`では`None`。
        moves: Option<u32>,
    },
    /// 手番側が詰む評価値。
    MatedIn {
        /// 詰みまでの手数。手数不明の`-`では`None`。
        moves: Option<u32>,
    },
}

/// 評価値が表す境界。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScoreBound {
    /// 上下界ではない評価値。
    Exact,
    /// 下界。
    Lower,
    /// 上界。
    Upper,
}

/// 1回の思考で最後に得た評価情報。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EvaluationRecord {
    /// 評価値の視点。
    pub(super) perspective: StoredColor,
    /// 評価値と同じ`info`行の探索深さ。
    pub(super) depth: Option<u32>,
    /// 評価値。
    pub(super) score: ScoreRecord,
    /// 上下界の種別。
    pub(super) bound: ScoreBound,
}

/// 探索を停止した条件。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StopReasonRecord {
    /// 指定深さを完了した。
    Depth,
    /// 指定ノード数へ達した。
    Nodes,
    /// 完了イテレーションの境界でsoft limitへ達した。
    Soft,
    /// 探索中にhard limitへ達した。
    Hard,
    /// 呼び出し側から停止を要求された。
    External,
}

/// `null`を認めつつ、JSON欄自体の省略は拒否する。
fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

/// 1回の思考に対するエンジン応答。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum TurnResponse {
    /// エンジンが選んだ着手。
    Move {
        /// 審判層が正規化したUSI表記。
        usi: String,
    },
    /// 投了。
    Resigned,
    /// エンジン異常。
    Failure {
        /// 異常分類。
        reason: FailureKind,
    },
}

/// 1回の思考と応答の記録。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TurnRecord {
    /// 思考した側。
    pub(super) side: StoredColor,
    /// 実測思考時間(ns)。
    pub(super) think_time_ns: u64,
    /// 最後の評価情報。評価値がない場合は`None`。
    pub(super) evaluation: Option<EvaluationRecord>,
    /// エンジンが報告した停止理由。報告がない場合は`None`。
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(super) stop_reason: Option<StopReasonRecord>,
    /// 最後に完了した反復の経過時間(ms)。報告がない場合は`None`。
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(super) completed_time_ms: Option<u64>,
    /// エンジンの応答。
    pub(super) response: TurnResponse,
}

/// 1局の終局理由。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum TerminationRecord {
    /// 審判層による一方の勝利。
    AdjudicatedWin {
        /// 勝者。
        winner: StoredColor,
        /// 審判層の終局理由名。
        reason: String,
    },
    /// 審判層による引き分け。
    AdjudicatedDraw {
        /// 審判層の終局理由名。
        reason: String,
    },
    /// エンジンの投了。
    Resigned {
        /// 投了した側。
        loser: StoredColor,
    },
    /// エンジン異常による反則負け。
    Forfeit {
        /// 異常を起こした側。
        loser: StoredColor,
        /// 異常分類。
        reason: FailureKind,
    },
    /// 手数上限による打ち切り。
    Cutoff,
}

/// 開始手順の記録。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OpeningRecord {
    /// 開始手順の生成に使ったシード。
    pub(super) seed: u64,
    /// 初期局面からのUSI着手列。
    pub(super) moves: Vec<String>,
}

/// 1局分の記録。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GameRecord {
    /// この局で候補を割り当てた手番。
    pub(super) candidate_color: StoredColor,
    /// 候補へ渡した乱数シード。
    pub(super) candidate_seed: u64,
    /// 基準へ渡した乱数シード。
    pub(super) baseline_seed: u64,
    /// エンジン起動から終局記録までの壁時計時間(ns)。
    pub(super) wall_time_ns: u64,
    /// 候補エンジンのプロセスCPU時間(ns)。取得不能なら`None`。
    pub(super) candidate_cpu_time_ns: Option<u64>,
    /// 基準エンジンのプロセスCPU時間(ns)。取得不能なら`None`。
    pub(super) baseline_cpu_time_ns: Option<u64>,
    /// 候補エンジンが記録した最大常駐メモリ(byte)。取得不能なら`None`。
    pub(super) candidate_peak_rss_bytes: Option<u64>,
    /// 基準エンジンが記録した最大常駐メモリ(byte)。取得不能なら`None`。
    pub(super) baseline_peak_rss_bytes: Option<u64>,
    /// 開始手順後の全思考と応答。
    pub(super) turns: Vec<TurnRecord>,
    /// 終局理由。
    pub(super) termination: TerminationRecord,
}

/// 原子的に確定する1ペア分の記録。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PairRecord {
    /// 1起算のペア番号。
    pub(super) pair_number: u64,
    /// 基本シードとペア番号から派生したシード。
    pub(super) pair_seed: u64,
    /// 両局で共有する開始手順。
    pub(super) opening: OpeningRecord,
    /// 候補の先後を入れ替えた2局。
    pub(super) games: [GameRecord; 2],
    /// 候補側ペア得点のペンタノミアル分類。打ち切りを含む場合は`None`。
    pub(super) category: Option<u8>,
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
    root: PathBuf,
    pairs: PathBuf,
    active_wall_time_ns: u64,
    interrupted: bool,
    _lock: RunLock,
}

/// 生存中だけ保持する実行ディレクトリの排他ロック。
#[derive(Debug)]
struct RunLock(File);

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

impl RunStore {
    /// 存在しないパスへ新規実行ディレクトリを作る。
    pub(super) fn create(path: &Path, manifest: RunManifest) -> io::Result<Self> {
        if manifest.format_version != FORMAT_VERSION {
            return Err(invalid_data(format!(
                "unsupported manifest format version {}",
                manifest.format_version
            )));
        }
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
            &path.join(SUMMARY_TEMP_FILE),
            &path.join(SUMMARY_FILE),
            &RunSummary {
                active_wall_time_ns: 0,
                interrupted: false,
                invocation_active: false,
            },
        )?;
        Ok(Self {
            root: path.to_owned(),
            pairs,
            active_wall_time_ns: 0,
            interrupted: false,
            _lock: lock,
        })
    }

    /// 既存実行ディレクトリを検証し、保存済みペアを読み込む。
    pub(super) fn resume(
        path: &Path,
        expected: &RunManifest,
        target_pairs: u64,
    ) -> io::Result<(Self, BTreeMap<u64, PairRecord>)> {
        if target_pairs == 0 {
            return Err(invalid_data("target pair count must be at least 1"));
        }
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

        let manifest: RunManifest = read_json(&path.join(MANIFEST_FILE))?;
        if manifest.format_version != FORMAT_VERSION {
            return Err(invalid_data(format!(
                "unsupported manifest format version {}",
                manifest.format_version
            )));
        }
        if &manifest != expected {
            return Err(invalid_data(
                "run manifest does not match the requested experiment",
            ));
        }

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
                root: path.to_owned(),
                pairs: pairs_directory,
                active_wall_time_ns: summary.active_wall_time_ns,
                interrupted,
                _lock: lock,
            },
            records,
        ))
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

    /// 新しい対局実行の開始を記録する。
    pub(super) fn begin_invocation(&self) -> io::Result<()> {
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

/// 実行ディレクトリをプロセス間で排他的にロックする。
fn lock_run_directory(path: &Path) -> io::Result<RunLock> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.join(".match_runner.lock"))?;
    file.try_lock_exclusive().map_err(|error| {
        io::Error::new(
            io::ErrorKind::WouldBlock,
            format!("run directory is already in use: {error}"),
        )
    })?;
    Ok(RunLock(file))
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

/// 値を同一ディレクトリの一時ファイルへ同期後、renameで確定する。
fn atomic_write_json<T: Serialize>(
    directory: &Path,
    temporary_path: &Path,
    final_path: &Path,
    value: &T,
) -> io::Result<()> {
    let write_result = (|| {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temporary_path)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, value).map_err(json_error)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        if final_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("record already exists: {}", final_path.display()),
            ));
        }
        fs::rename(temporary_path, final_path)?;
        File::open(directory)?.sync_all()
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(temporary_path);
    }
    write_result
}

/// 既存JSONを同一ディレクトリの一時ファイルから原子的に置き換える。
fn atomic_replace_json<T: Serialize>(
    directory: &Path,
    temporary_path: &Path,
    final_path: &Path,
    value: &T,
) -> io::Result<()> {
    remove_stale_temp(temporary_path)?;
    let write_result = (|| {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temporary_path)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, value).map_err(json_error)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        fs::rename(temporary_path, final_path)?;
        File::open(directory)?.sync_all()
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(temporary_path);
    }
    write_result
}

/// 指定JSONファイル全体を読み込む。
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<T> {
    let file = File::open(path)?;
    serde_json::from_reader(file).map_err(json_error)
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
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_directory(test: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "minase-match-storage-{test}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn manifest() -> RunManifest {
        RunManifest {
            format_version: FORMAT_VERSION,
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
                identity: EngineIdentity::Random {
                    sha256: "c".repeat(64),
                },
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
            cpu: CpuRecord {
                model: "test cpu".to_owned(),
                physical_cores: Some(8),
                logical_cores: 8,
                physical_memory_bytes: Some(16 * 1024 * 1024 * 1024),
            },
            runner: HarnessRecord {
                version: "0.1.0".to_owned(),
                sha256: "b".repeat(64),
            },
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
    fn create_save_and_resume_round_trip() {
        let path = temporary_directory("round-trip");
        let expected = manifest();
        let store = RunStore::create(&path, expected.clone()).unwrap();
        store.save_pair(&pair(2)).unwrap();
        store.save_pair(&pair(1)).unwrap();
        drop(store);

        let (store, records) = RunStore::resume(&path, &expected, 2).unwrap();
        assert_eq!(store.path(), path);
        assert_eq!(records.keys().copied().collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(records[&2], pair(2));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn turn_record_round_trip_preserves_stop_data_and_nulls() {
        let with_values = game(StoredColor::Black).turns.remove(0);
        let value = serde_json::to_value(&with_values).unwrap();
        assert_eq!(value["stop_reason"], "hard");
        assert_eq!(value["completed_time_ms"], 40);
        assert_eq!(
            serde_json::from_value::<TurnRecord>(value).unwrap(),
            with_values
        );

        let mut without_values = with_values.clone();
        without_values.stop_reason = None;
        without_values.completed_time_ms = None;
        let value = serde_json::to_value(&without_values).unwrap();
        assert!(value["stop_reason"].is_null());
        assert!(value["completed_time_ms"].is_null());
        assert_eq!(
            serde_json::from_value::<TurnRecord>(value).unwrap(),
            without_values
        );

        let mut missing = serde_json::to_value(&without_values).unwrap();
        missing.as_object_mut().unwrap().remove("stop_reason");
        assert!(serde_json::from_value::<TurnRecord>(missing).is_err());
        let mut missing = serde_json::to_value(&without_values).unwrap();
        missing.as_object_mut().unwrap().remove("completed_time_ms");
        assert!(serde_json::from_value::<TurnRecord>(missing).is_err());
    }

    #[test]
    fn stop_reason_record_uses_fixed_snake_case_words() {
        for (reason, word) in [
            (StopReasonRecord::Depth, "depth"),
            (StopReasonRecord::Nodes, "nodes"),
            (StopReasonRecord::Soft, "soft"),
            (StopReasonRecord::Hard, "hard"),
            (StopReasonRecord::External, "external"),
        ] {
            let value = serde_json::to_value(reason).unwrap();
            assert_eq!(value, word);
            assert_eq!(
                serde_json::from_value::<StopReasonRecord>(value).unwrap(),
                reason
            );
        }
    }

    #[test]
    fn resume_rejects_format_version_two() {
        let path = temporary_directory("version-two");
        let expected = manifest();
        let store = RunStore::create(&path, expected.clone()).unwrap();
        drop(store);
        let manifest_path = path.join(MANIFEST_FILE);
        let mut old_manifest = serde_json::to_value(&expected).unwrap();
        old_manifest["format_version"] = serde_json::json!(2);
        fs::write(&manifest_path, serde_json::to_vec(&old_manifest).unwrap()).unwrap();

        let error = RunStore::resume(&path, &expected, 1).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "unsupported manifest format version 2");
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn checkpoints_accumulate_across_resumed_invocations() {
        let path = temporary_directory("checkpoint");
        let expected = manifest();
        let store = RunStore::create(&path, expected.clone()).unwrap();
        store.begin_invocation().unwrap();
        store.checkpoint(std::time::Duration::from_secs(2)).unwrap();
        drop(store);

        let (store, _) = RunStore::resume(&path, &expected, 1).unwrap();
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
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn create_rejects_an_existing_directory_and_resume_requires_one() {
        let path = temporary_directory("directory-contract");
        fs::create_dir(&path).unwrap();
        assert_eq!(
            RunStore::create(&path, manifest()).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        fs::remove_dir(&path).unwrap();
        assert_eq!(
            RunStore::resume(&path, &manifest(), 1).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn a_run_directory_cannot_be_opened_by_two_process_owners() {
        let path = temporary_directory("exclusive-lock");
        let expected = manifest();
        let store = RunStore::create(&path, expected.clone()).unwrap();
        assert_eq!(
            RunStore::resume(&path, &expected, 1).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(store);
        RunStore::resume(&path, &expected, 1).unwrap();
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn resume_rejects_manifest_mismatch_and_saved_number_above_target() {
        let path = temporary_directory("resume-validation");
        let expected = manifest();
        let store = RunStore::create(&path, expected.clone()).unwrap();
        store.save_pair(&pair(2)).unwrap();
        drop(store);

        let mut different = expected.clone();
        different.seed += 1;
        assert_eq!(
            RunStore::resume(&path, &different, 2).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            RunStore::resume(&path, &expected, 1).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn resume_rejects_corruption_filename_mismatch_and_invalid_category() {
        let path = temporary_directory("pair-validation");
        let expected = manifest();
        let store = RunStore::create(&path, expected.clone()).unwrap();
        store.save_pair(&pair(1)).unwrap();
        drop(store);

        fs::write(path.join(PAIRS_DIRECTORY).join(pair_file_name(2)), b"{").unwrap();
        assert_eq!(
            RunStore::resume(&path, &expected, 2).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_file(path.join(PAIRS_DIRECTORY).join(pair_file_name(2))).unwrap();

        let wrong_name = path.join(PAIRS_DIRECTORY).join(pair_file_name(2));
        fs::write(&wrong_name, serde_json::to_vec(&pair(3)).unwrap()).unwrap();
        assert_eq!(
            RunStore::resume(&path, &expected, 3).unwrap_err().kind(),
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
            RunStore::resume(&path, &expected, 2).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn stale_temporary_pair_is_ignored_and_removed_before_resave() {
        let path = temporary_directory("stale-temp");
        let expected = manifest();
        let store = RunStore::create(&path, expected.clone()).unwrap();
        let temporary = path.join(PAIRS_DIRECTORY).join(pair_temp_file_name(1));
        fs::write(&temporary, b"partial").unwrap();
        drop(store);

        let (store, records) = RunStore::resume(&path, &expected, 1).unwrap();
        assert!(records.is_empty());
        store.save_pair(&pair(1)).unwrap();
        assert!(!temporary.exists());
        assert!(path.join(PAIRS_DIRECTORY).join(pair_file_name(1)).exists());
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn save_rejects_duplicate_pair_without_overwriting_it() {
        let path = temporary_directory("duplicate");
        let store = RunStore::create(&path, manifest()).unwrap();
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
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn unknown_json_field_is_rejected() {
        let mut value = serde_json::to_value(pair(1)).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("unexpected".to_owned(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<PairRecord>(value).is_err());
    }
}
