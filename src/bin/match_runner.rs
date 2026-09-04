//! コミット対コミットの自己対局測定ハーネス。
//!
//! ペア対局のペンタノミアルGSPRTと固定局数Eloを提供する。運用規約と
//! 統計的契約はdocs/sprt.mdを参照。

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::{Path, PathBuf};
use std::process::{self, Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{ArgGroup, CommandFactory, Parser, Subcommand, error::ErrorKind};
use fs2::FileExt;
use minase::core::rules::parse_rule_set;
use minase::notation::{cecp, usi};
use minase::rng::{XorShift64, derive_seed};
use minase::search::MAX_PLY;
use minase::stats::{GSPRT_H1_ELO, GsprtDecision, estimate_elo, gsprt_decision, gsprt_llr};
use minase::{Color, Game, GameResult, GameStatus, Move, MoveGenerator, RuleCode, Rules, Square};
use sha2::{Digest, Sha256};

#[cfg(target_os = "linux")]
use procfs::process::Process;

#[path = "match_runner/storage.rs"]
mod storage;

use storage::{
    CpuRecord, EngineHashSizes, EngineIdentity, EngineRecord, EngineThreadCounts, EvaluationRecord,
    FORMAT_VERSION, FailureKind, GameRecord, HarnessRecord, ManifestMode, OpeningRecord,
    PairRecord, RunManifest, RunStore, ScoreBound, ScoreRecord, StopReasonRecord, StoredColor,
    StoredProtocol, StoredSearchLimit, TerminationRecord, TurnRecord, TurnResponse,
};

/// 1局を打ち切る手数上限の既定値。
const DEFAULT_MAX_PLY: u32 = 4096;
/// GSPRTの暴走保険となる実行ペア数上限の既定値。
const DEFAULT_MAX_PAIRS: u64 = 100_000;
/// 1回のエンジン応答を待つ秒数の既定値。
const DEFAULT_RESPONSE_TIMEOUT_SECONDS: u64 = 120;
/// CECPエンジンへ割り当てる置換表容量。HaChuは`memory`受信前の`go`で
/// 異常終了するため明示し、256 MBはminaseの`USI_Hash`既定値に合わせる。
const CECP_MEMORY_MB: u32 = 256;
/// 固定制限時にCECPエンジンへ通知する時計残量。時計残量0ではHaChuが
/// 反復深化を即座に打ち切り、HaChuは5倍した残量を32ビット整数で計算するため、
/// 十分大きくかつその範囲に収まる3,000,000センチ秒とする。
const CECP_FIXED_TIME_CS: u64 = 3_000_000;

/// バイナリ対戦ハーネスのコマンドライン引数。
#[derive(Parser)]
#[command(
    name = "match_runner",
    group(
        ArgGroup::new("run_operation")
            .required(true)
            .multiple(false)
            .args(["run_dir", "resume"])
    )
)]
struct Arguments {
    /// 新しい実験を作成する、まだ存在しない実行ディレクトリ。
    #[arg(long)]
    run_dir: Option<PathBuf>,
    /// 保存済みの実験を再開する実行ディレクトリ。
    #[arg(long)]
    resume: Option<PathBuf>,
    /// 全ペアの乱数列を派生させる基本シード。
    #[arg(long)]
    seed: Option<u64>,
    /// 採用するローカルルールコード列または規則セット名。
    #[arg(
        long,
        default_value = "engine-default",
        value_parser = parse_rule_set_argument
    )]
    rules: RuleSetArgument,
    /// 1局を打ち切る手数上限。
    #[arg(long, default_value_t = DEFAULT_MAX_PLY, value_parser = parse_positive_u32)]
    max_ply: u32,
    /// 候補側の起動コマンド（パスと空白区切りの引数）または`random`。
    #[arg(long, default_value = "random", value_parser = parse_player_spec)]
    candidate: PlayerSpec,
    /// 基準側の起動コマンド（パスと空白区切りの引数）または`random`。
    #[arg(long, default_value = "random", value_parser = parse_player_spec)]
    baseline: PlayerSpec,
    /// 両エンジンに適用する既定の思考制限。
    #[arg(long, default_value = "depth=1", value_parser = parse_search_limit)]
    each: SearchLimit,
    /// 候補側だけに適用する思考制限。
    #[arg(long, value_parser = parse_search_limit)]
    candidate_limit: Option<SearchLimit>,
    /// 基準側だけに適用する思考制限。
    #[arg(long, value_parser = parse_search_limit)]
    baseline_limit: Option<SearchLimit>,
    /// 1回のエンジン応答を待つ秒数。
    #[arg(
        long,
        default_value_t = DEFAULT_RESPONSE_TIMEOUT_SECONDS,
        value_parser = parse_positive_u64
    )]
    response_timeout: u64,
    /// 同時に実行するペア数。省略時は物理コア数から自動計算する。
    #[arg(long, value_parser = parse_positive_usize)]
    concurrency: Option<usize>,
    /// 実行する統計モード。
    #[command(subcommand)]
    mode: Mode,
}

/// 対局結果の集計方法。
#[derive(Subcommand)]
enum Mode {
    /// ペンタノミアルGSPRTでH0またはH1を逐次判定する。
    Gsprt {
        /// 判定を保留して停止する実行ペア数の上限。
        #[arg(long, default_value_t = DEFAULT_MAX_PAIRS, value_parser = parse_positive_u64)]
        max_pairs: u64,
    },
    /// 固定ペア数からEloと95%信頼区間を推定する。
    Elo {
        /// 実行するペア数。
        #[arg(long, value_parser = parse_positive_u64)]
        pairs: u64,
    },
}

/// `--rules`の入力原文と解析済みコード列。
#[derive(Clone)]
struct RuleSetArgument {
    /// 両エンジンへ渡す入力原文。
    source: String,
    /// 審判層と測定記録に使う解析済みコード列。
    codes: Vec<RuleCode>,
}

/// `--rules`の値を規則セット名またはコード列として解析する。
fn parse_rule_set_argument(input: &str) -> Result<RuleSetArgument, String> {
    parse_rule_set(input)
        .map(|codes| RuleSetArgument {
            source: input.to_owned(),
            codes,
        })
        .map_err(|error| error.to_string())
}

/// 外部エンジンの指定。
#[derive(Clone, PartialEq, Eq, Debug)]
struct PlayerSpec {
    /// 入力された指定の原文。表示に使う。
    text: String,
    /// 指定の解釈結果。
    kind: PlayerKind,
}

/// ランダムエンジンまたは実行ファイルの起動コマンドを表す。
#[derive(Clone, PartialEq, Eq, Debug)]
enum PlayerKind {
    /// 同梱の校正用ランダムエンジン。
    Random,
    /// ビルドして使うgitリビジョン。
    Commit(String),
    /// 任意のUSIエンジンの起動コマンド。
    Command {
        /// 実行ファイルのパス。
        program: PathBuf,
        /// 起動引数。
        args: Vec<String>,
    },
    /// 任意のCECPエンジンの起動コマンド。
    Cecp {
        /// 実行ファイルのパス。
        program: PathBuf,
        /// 起動引数。
        args: Vec<String>,
    },
}

/// 外部エンジンとの通信プロトコル。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Protocol {
    /// USIプロトコル。
    Usi,
    /// CECP（XBoard）プロトコル。
    Cecp,
}

/// USIの`go`へ渡す思考制限。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SearchLimit {
    /// 深さまたはノード数による固定制限。
    Fixed {
        /// 探索深さの上限。
        depth: Option<u32>,
        /// 探索ノード数の上限。
        nodes: Option<u64>,
    },
    /// 持ち時間による時間制御。
    Time(TimeControl),
}

/// ミリ秒単位の持ち時間、加算時間、秒読み。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct TimeControl {
    /// 持ち時間(ms)。
    base_ms: u64,
    /// 1手ごとの加算時間(ms)。
    increment_ms: u64,
    /// 1手ごとの秒読み(ms)。
    byoyomi_ms: u64,
}

impl SearchLimit {
    /// CLI表示用の制限文字列を返す。
    fn cli_text(self) -> String {
        match self {
            Self::Fixed {
                depth: Some(depth),
                nodes: Some(nodes),
            } => format!("depth={depth},nodes={nodes}"),
            Self::Fixed {
                depth: Some(depth),
                nodes: None,
            } => format!("depth={depth}"),
            Self::Fixed {
                depth: None,
                nodes: Some(nodes),
            } => format!("nodes={nodes}"),
            Self::Fixed {
                depth: None,
                nodes: None,
            } => unreachable!("a validated fixed limit contains depth or nodes"),
            Self::Time(time) if time.byoyomi_ms == 0 => {
                format!("time={}+{}", time.base_ms, time.increment_ms)
            }
            Self::Time(time) => format!(
                "time={}+{},byoyomi={}",
                time.base_ms, time.increment_ms, time.byoyomi_ms
            ),
        }
    }

    /// 固定制限のUSI `go`引数を返す。
    fn fixed_go_text(self) -> Option<String> {
        match self {
            Self::Fixed {
                depth: Some(depth),
                nodes: Some(nodes),
            } => Some(format!("depth {depth} nodes {nodes}")),
            Self::Fixed {
                depth: Some(depth),
                nodes: None,
            } => Some(format!("depth {depth}")),
            Self::Fixed {
                depth: None,
                nodes: Some(nodes),
            } => Some(format!("nodes {nodes}")),
            Self::Fixed {
                depth: None,
                nodes: None,
            } => unreachable!("a validated fixed limit contains depth or nodes"),
            Self::Time(_) => None,
        }
    }

    /// 時間制御なら時計の初期状態を返す。
    fn clock(self) -> Option<Clock> {
        match self {
            Self::Fixed { .. } => None,
            Self::Time(time) => Some(Clock::new(time)),
        }
    }
}

/// 1エンジンの現在の時計。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Clock {
    /// 残り時間。
    remaining: Duration,
    /// 1手ごとの加算時間。
    increment: Duration,
    /// 1手ごとの秒読み。
    byoyomi: Duration,
}

impl Clock {
    /// 時間制御から時計を初期化する。
    fn new(time: TimeControl) -> Self {
        Self {
            remaining: Duration::from_millis(time.base_ms),
            increment: Duration::from_millis(time.increment_ms),
            byoyomi: Duration::from_millis(time.byoyomi_ms),
        }
    }

    /// 実測思考時間を反映し、時間切れかどうかを返す。
    fn update(&mut self, elapsed: Duration) -> Result<(), EngineFailure> {
        if elapsed > self.remaining + self.byoyomi {
            return Err(EngineFailure::TimeForfeit);
        }
        self.remaining = self.remaining.saturating_sub(elapsed) + self.increment;
        Ok(())
    }

    /// USIへ送るミリ秒単位の残り時間を返す。
    fn remaining_ms(self) -> u128 {
        self.remaining.as_millis()
    }

    /// USIへ送るミリ秒単位の加算時間を返す。
    fn increment_ms(self) -> u128 {
        self.increment.as_millis()
    }

    /// USIへ送るミリ秒単位の秒読みを返す。
    fn byoyomi_ms(self) -> u128 {
        self.byoyomi.as_millis()
    }

    /// CECPへ送るセンチ秒単位の残り時間を返す。10 ms未満は切り捨てる。
    fn remaining_cs(self) -> u64 {
        u64::try_from(self.remaining.as_millis() / 10)
            .expect("a clock created from u64 milliseconds must fit in u64 centiseconds")
    }
}

/// 1局で両色に割り当てた時計。
struct GameClocks {
    /// 先手の時計。固定制限の側は`None`。
    black: Option<Clock>,
    /// 後手の時計。固定制限の側は`None`。
    white: Option<Clock>,
}

impl GameClocks {
    /// プレイヤーAとBの制限を対局時の色へ割り当てる。
    fn new(player_a_color: Color, player_a: SearchLimit, player_b: SearchLimit) -> Self {
        let (black, white) = if player_a_color == Color::Black {
            (player_a.clock(), player_b.clock())
        } else {
            (player_b.clock(), player_a.clock())
        };
        Self { black, white }
    }

    /// 指定色の時計を返す。
    fn get(&self, color: Color) -> Option<Clock> {
        match color {
            Color::Black => self.black,
            Color::White => self.white,
        }
    }

    /// 指定色の時計を可変参照で返す。
    fn get_mut(&mut self, color: Color) -> Option<&mut Clock> {
        match color {
            Color::Black => self.black.as_mut(),
            Color::White => self.white.as_mut(),
        }
    }

    /// 現在の両時計から時間制御用のUSI `go`引数を返す。
    fn go_text(&self, side_to_move: Color) -> String {
        let black = self.black.unwrap_or_else(zero_clock);
        let white = self.white.unwrap_or_else(zero_clock);
        let byoyomi = self
            .get(side_to_move)
            .expect("a time-controlled player must have a clock");
        format!(
            "btime {} wtime {} binc {} winc {} byoyomi {}",
            black.remaining_ms(),
            white.remaining_ms(),
            black.increment_ms(),
            white.increment_ms(),
            byoyomi.byoyomi_ms()
        )
    }

    /// 現在の制限と両時計からプロトコル共通の思考要求を作る。
    fn think_request(&self, side_to_move: Color, limit: SearchLimit) -> ThinkRequest {
        match limit {
            SearchLimit::Fixed { .. } => ThinkRequest {
                go_text: limit
                    .fixed_go_text()
                    .expect("a fixed limit must have USI go text"),
                own_cs: CECP_FIXED_TIME_CS,
                opponent_cs: CECP_FIXED_TIME_CS,
            },
            SearchLimit::Time(_) => ThinkRequest {
                go_text: self.go_text(side_to_move),
                own_cs: self
                    .get(side_to_move)
                    .expect("a time-controlled player must have a clock")
                    .remaining_cs(),
                opponent_cs: self
                    .get(side_to_move.opposite())
                    .map_or(0, Clock::remaining_cs),
            },
        }
    }
}

/// 時間制御を使わない側をUSI時間引数へ表す0値の時計を返す。
fn zero_clock() -> Clock {
    Clock {
        remaining: Duration::ZERO,
        increment: Duration::ZERO,
        byoyomi: Duration::ZERO,
    }
}

/// 起動に必要な解決済みプレイヤー設定。
struct PlayerConfig {
    /// 入力された指定の原文。表示に使う。
    text: String,
    /// 再開時の照合に使う完全コミットハッシュまたは起動指定。
    identity: EngineIdentity,
    /// 実行ファイルのパス。
    path: PathBuf,
    /// 起動引数。
    args: Vec<String>,
    /// エンジンとの通信プロトコル。
    protocol: Protocol,
    /// 校正用ランダムエンジンかどうか。真ならシードを設定する。
    is_random: bool,
    /// このプレイヤーに適用する思考制限。
    limit: SearchLimit,
    /// 起動引数と規則オプションへ渡す`--rules`入力原文。
    rules_source: String,
}

impl PlayerConfig {
    /// specと実効制限を組み合わせた表示名を返す。
    fn name(&self) -> String {
        format!("{} limit={}", self.text, self.limit.cli_text())
    }
}

/// USIセッションの異常分類。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EngineFailure {
    /// `bestmove`が審判層の合法手にない。
    IllegalMove,
    /// プロセス終了またはパイプ切断。
    Crash,
    /// 応答期限までに応答がない。
    Timeout,
    /// 時間制御対局での時間切れ。
    TimeForfeit,
    /// 審判層が合法とした相手の着手を`Illegal move`で拒否した。
    RejectedMove,
}

/// 異常理由別の発生件数。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct FailureCounts {
    /// 不正着手の件数。
    illegal_moves: u64,
    /// クラッシュの件数。
    crashes: u64,
    /// 応答タイムアウトの件数。
    timeouts: u64,
    /// 時間切れの件数。
    time_forfeits: u64,
    /// 審判層の合法手を拒否した件数。
    rejected_moves: u64,
}

impl FailureCounts {
    /// 1局の異常を加算する。
    fn record(&mut self, failure: EngineFailure) {
        match failure {
            EngineFailure::IllegalMove => self.illegal_moves += 1,
            EngineFailure::Crash => self.crashes += 1,
            EngineFailure::Timeout => self.timeouts += 1,
            EngineFailure::TimeForfeit => self.time_forfeits += 1,
            EngineFailure::RejectedMove => self.rejected_moves += 1,
        }
    }

    /// 別の集計値を加算する。
    fn add(&mut self, other: Self) {
        self.illegal_moves += other.illegal_moves;
        self.crashes += other.crashes;
        self.timeouts += other.timeouts;
        self.time_forfeits += other.time_forfeits;
        self.rejected_moves += other.rejected_moves;
    }
}

/// 1回の思考に必要なプロトコル別の制限値。
struct ThinkRequest {
    /// USIの`go`へ渡す引数。
    go_text: String,
    /// CECPの`time`へ渡す手番側の残り時間（センチ秒）。
    own_cs: u64,
    /// CECPの`otim`へ渡す相手側の残り時間（センチ秒）。
    opponent_cs: u64,
}

/// エンジンが返した対局上の応答。
#[derive(PartialEq, Eq, Debug)]
enum EngineResponse {
    /// エンジンが選んだ指し手表記。
    Move(String),
    /// エンジンが着手を返さず投了した。
    Resigned,
}

/// エンジンが最後に報告した評価値。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct EngineEvaluation {
    /// 報告された探索深さ。省略された場合は`None`。
    depth: Option<u32>,
    /// センチポーンまたは詰み手数による評価値。
    score: EngineScore,
    /// 評価値が上下界ならその種別。
    bound: ScoreBound,
}

/// USI `info score`の評価値。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EngineScore {
    /// センチポーン単位の評価値。
    Cp(i32),
    /// 手番側が詰ませるまでの手数。手数不明なら`None`。
    MateIn(Option<u32>),
    /// 手番側が詰むまでの手数。手数不明なら`None`。
    MatedIn(Option<u32>),
}

/// 1回の思考で得た応答、所要時間、および探索情報。
struct ThinkResult {
    /// エンジンが返した着手または投了。
    response: EngineResponse,
    /// `go`から応答までの実測時間。
    elapsed: Duration,
    /// 最後の有効な`info score`。報告がなければ`None`。
    evaluation: Option<EngineEvaluation>,
    /// エンジンが報告した停止理由。報告がなければ`None`。
    stop_reason: Option<StopReasonRecord>,
    /// 最後の`info`行が報告した経過時間(ms)。報告がなければ`None`。
    completed_time_ms: Option<u64>,
}

/// USI `info`行から評価値を解析する。
fn parse_usi_evaluation(line: &str) -> Result<Option<EngineEvaluation>, EngineFailure> {
    let tokens: Vec<_> = line.split_whitespace().collect();
    if tokens.first() != Some(&"info") {
        return Ok(None);
    }
    if tokens.get(1) == Some(&"string") {
        return Ok(None);
    }
    let Some(score_index) = tokens.iter().position(|token| *token == "score") else {
        return Ok(None);
    };
    let kind = tokens.get(score_index + 1).ok_or(EngineFailure::Crash)?;
    let value = *tokens.get(score_index + 2).ok_or(EngineFailure::Crash)?;
    let score = match *kind {
        "cp" => EngineScore::Cp(value.parse().map_err(|_| EngineFailure::Crash)?),
        "mate" => match value {
            "+" => EngineScore::MateIn(None),
            "-" => EngineScore::MatedIn(None),
            value => {
                let moves = value.parse::<i32>().map_err(|_| EngineFailure::Crash)?;
                if moves >= 0 {
                    EngineScore::MateIn(Some(moves.unsigned_abs()))
                } else {
                    EngineScore::MatedIn(Some(moves.unsigned_abs()))
                }
            }
        },
        _ => return Err(EngineFailure::Crash),
    };
    let depth = tokens
        .iter()
        .position(|token| *token == "depth")
        .map(|index| {
            tokens
                .get(index + 1)
                .ok_or(EngineFailure::Crash)?
                .parse::<u32>()
                .map_err(|_| EngineFailure::Crash)
        })
        .transpose()?;
    let lower = tokens[score_index + 3..].contains(&"lowerbound");
    let upper = tokens[score_index + 3..].contains(&"upperbound");
    let bound = match (lower, upper) {
        (false, false) => ScoreBound::Exact,
        (true, false) => ScoreBound::Lower,
        (false, true) => ScoreBound::Upper,
        (true, true) => return Err(EngineFailure::Crash),
    };
    Ok(Some(EngineEvaluation {
        depth,
        score,
        bound,
    }))
}

/// 有効な評価行だけで監査値を更新し、通信結果には影響させない。
fn observe_usi_evaluation(current: &mut Option<EngineEvaluation>, line: &str) {
    if let Ok(Some(parsed)) = parse_usi_evaluation(line) {
        *current = Some(parsed);
    }
}

/// USI `info string stop`行から停止理由を解析する。
fn parse_usi_stop_reason(line: &str) -> Result<Option<StopReasonRecord>, EngineFailure> {
    let tokens: Vec<_> = line.split_whitespace().collect();
    if tokens.get(..3) != Some(["info", "string", "stop"].as_slice()) {
        return Ok(None);
    }
    let reason = match tokens.as_slice() {
        ["info", "string", "stop", "depth"] => StopReasonRecord::Depth,
        ["info", "string", "stop", "nodes"] => StopReasonRecord::Nodes,
        ["info", "string", "stop", "soft"] => StopReasonRecord::Soft,
        ["info", "string", "stop", "hard"] => StopReasonRecord::Hard,
        ["info", "string", "stop", "external"] => StopReasonRecord::External,
        _ => return Err(EngineFailure::Crash),
    };
    Ok(Some(reason))
}

/// USI `info`行から探索開始後の経過時間を解析する。
fn parse_usi_time(line: &str) -> Result<Option<u64>, EngineFailure> {
    let tokens: Vec<_> = line.split_whitespace().collect();
    if tokens.first() != Some(&"info") || tokens.get(1) == Some(&"string") {
        return Ok(None);
    }
    let Some(index) = tokens.iter().position(|token| *token == "time") else {
        return Ok(None);
    };
    let value = tokens
        .get(index + 1)
        .ok_or(EngineFailure::Crash)?
        .parse()
        .map_err(|_| EngineFailure::Crash)?;
    Ok(Some(value))
}

/// 停止理由と完了反復の経過時間を最新の`info`行で更新する。
fn observe_usi_search_data(
    stop_reason: &mut Option<StopReasonRecord>,
    completed_time_ms: &mut Option<u64>,
    line: &str,
) -> Result<(), EngineFailure> {
    if let Some(reason) = parse_usi_stop_reason(line)? {
        *stop_reason = Some(reason);
    }
    if let Some(time) = parse_usi_time(line)? {
        *completed_time_ms = Some(time);
    }
    Ok(())
}

/// 読み取りスレッドからプロトコル出力を受け取る外部エンジン。
struct EngineProcess {
    /// エンジンの子プロセス。
    child: Child,
    /// エンジンの標準入力。dropで読み取りスレッドの回収前に閉じる。
    input: Option<ChildStdin>,
    /// 読み取りスレッドが送るUSI出力行。
    lines: Receiver<io::Result<String>>,
    /// 読み取りスレッドのハンドル。dropで回収する。
    reader: Option<JoinHandle<()>>,
    /// 1回の応答を待つ期限。
    timeout: Duration,
    /// エンジンとの通信プロトコル。
    protocol: Protocol,
    /// CECPエンジンへ送信済みとして扱う着手数。
    sent_moves: usize,
}

/// 終局時に読み取る1エンジンの資源使用量。
#[derive(Clone, Copy, Default)]
struct EngineResourceUsage {
    /// プロセス全体のユーザー時間とシステム時間の合計(ns)。
    cpu_time_ns: Option<u64>,
    /// プロセスが記録した最大常駐メモリ(byte)。
    peak_rss_bytes: Option<u64>,
}

/// USI初期化応答が報告する既定の探索資源。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct EngineDefaults {
    /// `Threads`の既定値。
    threads: Option<u32>,
    /// `USI_Hash`の既定値(MB)。
    hash_mb: Option<u64>,
}

/// USI `option`行から指定したspin optionの既定値を得る。
fn parse_usi_spin_default(line: &str, expected_name: &str) -> Option<u64> {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    if tokens.first() != Some(&"option") || tokens.get(1) != Some(&"name") {
        return None;
    }
    let type_index = tokens.iter().position(|token| *token == "type")?;
    if tokens.get(type_index + 1) != Some(&"spin")
        || tokens[2..type_index].join(" ") != expected_name
    {
        return None;
    }
    let default_index = tokens[type_index + 2..]
        .iter()
        .position(|token| *token == "default")?
        + type_index
        + 2;
    tokens.get(default_index + 1)?.parse().ok()
}

/// 資源に関係する有効なUSI `option`行を記録する。
fn observe_usi_default(defaults: &mut EngineDefaults, line: &str) {
    if let Some(value) =
        parse_usi_spin_default(line, "Threads").and_then(|value| value.try_into().ok())
    {
        defaults.threads = Some(value);
    }
    if let Some(value) = parse_usi_spin_default(line, "USI_Hash") {
        defaults.hash_mb = Some(value);
    }
}

impl EngineProcess {
    /// プロセスと標準出力読み取りスレッドを起動する。
    fn spawn(config: &PlayerConfig, timeout: Duration) -> Result<Self, EngineFailure> {
        let mut child = Command::new(&config.path)
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|_| EngineFailure::Crash)?;
        let input = child.stdin.take().ok_or(EngineFailure::Crash)?;
        let output = child.stdout.take().ok_or(EngineFailure::Crash)?;
        let (sender, lines) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                let mut line = String::new();
                match output.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        while matches!(line.as_bytes().last(), Some(b'\n' | b'\r')) {
                            line.pop();
                        }
                        if sender.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            input: Some(input),
            lines,
            reader: Some(reader),
            timeout,
            protocol: config.protocol,
            sent_moves: 0,
        })
    }

    /// プロセスを起動し、プロトコル初期化列を完了する。
    fn start(config: &PlayerConfig, seed: u64, timeout: Duration) -> Result<Self, EngineFailure> {
        let mut process = Self::spawn(config, timeout)?;
        match config.protocol {
            Protocol::Usi => {
                process.read_usi_defaults()?;
                process.send(&format!(
                    "setoption name RuleSet value {}",
                    config.rules_source
                ))?;
                if config.is_random {
                    process.send(&format!("setoption name Seed value {seed}"))?;
                }
                process.send("isready")?;
                process.wait_for("readyok")?;
                process.send("usinewgame")?;
            }
            Protocol::Cecp => {
                process.send("xboard")?;
                process.send("protover 2")?;
                process.wait_for("feature done=1")?;
                process.send(&format!("memory {CECP_MEMORY_MB}"))?;
                process.send("new")?;
                process.send("variant chu")?;
                process.send("easy")?;
                process.send("nopost")?;
                process.send("force")?;
                match config.limit {
                    SearchLimit::Fixed {
                        depth: Some(depth),
                        nodes: None,
                    } => process.send(&format!("sd {depth}"))?,
                    SearchLimit::Time(time) => {
                        process.send(&cecp_level_text(time))?;
                    }
                    SearchLimit::Fixed { .. } => {
                        unreachable!("CECP limits are validated during player resolution")
                    }
                }
            }
        }
        Ok(process)
    }

    /// USI `option`行を`usiok`まで読み、既定資源を返す。
    fn read_usi_defaults(&mut self) -> Result<EngineDefaults, EngineFailure> {
        self.send("usi")?;
        let mut defaults = EngineDefaults::default();
        self.receive_until(|line| {
            observe_usi_default(&mut defaults, line);
            line.trim() == "usiok"
        })?;
        Ok(defaults)
    }

    /// エンジンへ1行を送る。
    fn send(&mut self, line: &str) -> Result<(), EngineFailure> {
        let input = self.input.as_mut().ok_or(EngineFailure::Crash)?;
        writeln!(input, "{line}").map_err(|_| EngineFailure::Crash)?;
        input.flush().map_err(|_| EngineFailure::Crash)
    }

    /// 指定行を期限まで読み進める。
    fn wait_for(&self, expected: &str) -> Result<(), EngineFailure> {
        self.receive_until(|line| line.trim() == expected)
            .map(|_| ())
    }

    /// 現局面と思考指示を送り、着手または投了と実測思考時間を受け取る。
    fn bestmove(
        &mut self,
        usi_history: &[String],
        move_history: &[Move],
        request: &ThinkRequest,
    ) -> Result<ThinkResult, EngineFailure> {
        match self.protocol {
            Protocol::Usi => self.bestmove_usi(usi_history, &request.go_text),
            Protocol::Cecp => self.bestmove_cecp(move_history, request),
        }
    }

    /// USIの既存送信列で`bestmove`を受け取る。
    fn bestmove_usi(
        &mut self,
        history: &[String],
        go_text: &str,
    ) -> Result<ThinkResult, EngineFailure> {
        if history.is_empty() {
            self.send("position startpos")?;
        } else {
            self.send(&format!("position startpos moves {}", history.join(" ")))?;
        }
        let start = Instant::now();
        self.send(&format!("go {go_text}"))?;
        let mut evaluation = None;
        let mut stop_reason = None;
        let mut completed_time_ms = None;
        let mut observation_error = None;
        let line = self.receive_until(|line| {
            observe_usi_evaluation(&mut evaluation, line);
            if observation_error.is_none()
                && let Err(reason) =
                    observe_usi_search_data(&mut stop_reason, &mut completed_time_ms, line)
            {
                observation_error = Some(reason);
            }
            observation_error.is_some() || line.split_whitespace().next() == Some("bestmove")
        })?;
        if let Some(reason) = observation_error {
            return Err(reason);
        }
        let response = match line.split_whitespace().nth(1).unwrap_or_default() {
            "resign" => EngineResponse::Resigned,
            bestmove => EngineResponse::Move(bestmove.to_owned()),
        };
        Ok(ThinkResult {
            response,
            elapsed: start.elapsed(),
            evaluation,
            stop_reason,
            completed_time_ms,
        })
    }

    /// CECPの差分着手と時計を送り、`pong`までの応答を解釈する。
    fn bestmove_cecp(
        &mut self,
        history: &[Move],
        request: &ThinkRequest,
    ) -> Result<ThinkResult, EngineFailure> {
        for &mv in &history[self.sent_moves..] {
            self.send(&format!("usermove {}", cecp::legs(mv).concat()))?;
        }
        self.sent_moves = history.len();
        self.send(&format!("time {}", request.own_cs))?;
        self.send(&format!("otim {}", request.opponent_cs))?;
        let start = Instant::now();
        self.send("go")?;
        let pong_number = history.len();
        self.send(&format!("ping {pong_number}"))?;
        let (response, elapsed) =
            receive_cecp_response(&self.lines, self.timeout, pong_number, start)?;
        if matches!(response, EngineResponse::Move(_)) {
            self.send("force")?;
            self.sent_moves += 1;
        }
        Ok(ThinkResult {
            response,
            elapsed,
            evaluation: None,
            stop_reason: None,
            completed_time_ms: None,
        })
    }

    /// 条件を満たす行を、呼び出し全体の期限まで受信する。
    fn receive_until(&self, predicate: impl FnMut(&str) -> bool) -> Result<String, EngineFailure> {
        receive_until(&self.lines, self.timeout, predicate)
    }

    /// 子プロセスが終了する前にCPU時間と最大常駐メモリを読み取る。
    fn resource_usage(&self) -> EngineResourceUsage {
        process_resource_usage(self.child.id())
    }
}

/// Linuxのprocfsからプロセス全体の資源使用量を読み取る。
#[cfg(target_os = "linux")]
fn process_resource_usage(pid: u32) -> EngineResourceUsage {
    let Ok(pid) = i32::try_from(pid) else {
        return EngineResourceUsage::default();
    };
    let Ok(process) = Process::new(pid) else {
        return EngineResourceUsage::default();
    };
    let cpu_time_ns = process.stat().ok().and_then(|stat| {
        let ticks = u128::from(stat.utime) + u128::from(stat.stime);
        let nanos = ticks
            .checked_mul(1_000_000_000)?
            .checked_div(u128::from(procfs::ticks_per_second()))?;
        u64::try_from(nanos).ok()
    });
    let peak_rss_bytes = process
        .status()
        .ok()
        .and_then(|status| status.vmhwm)
        .and_then(|kib| kib.checked_mul(1024));
    EngineResourceUsage {
        cpu_time_ns,
        peak_rss_bytes,
    }
}

/// procfsがないOSでは欠測を明示する。
#[cfg(not(target_os = "linux"))]
const fn process_resource_usage(_pid: u32) -> EngineResourceUsage {
    EngineResourceUsage {
        cpu_time_ns: None,
        peak_rss_bytes: None,
    }
}

/// 時間制御をCECPの`level`コマンドへ変換する。
fn cecp_level_text(time: TimeControl) -> String {
    let total_seconds = time.base_ms / 1_000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    let base = if seconds == 0 {
        minutes.to_string()
    } else {
        format!("{minutes}:{seconds:02}")
    };
    format!("level 0 {base} {}", time.increment_ms / 1_000)
}

/// CECPの着手・結果・拒否のいずれかを示す行かどうかを返す。
fn is_cecp_response_line(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("move ")
        || line.starts_with("Illegal move")
        || ["1-0", "0-1", "1/2-1/2", "resign"]
            .iter()
            .any(|prefix| line.starts_with(prefix))
}

/// `pong`までのCECP応答行を着手または投了へ解釈する。
///
/// 応答行がない`pong`はプロトコル違反であり、クラッシュ相当に分類する。
fn interpret_cecp_response(lines: &[String]) -> Result<EngineResponse, EngineFailure> {
    if lines
        .iter()
        .any(|line| line.trim_start().starts_with("Illegal move"))
    {
        return Err(EngineFailure::RejectedMove);
    }

    let mut moves = lines
        .iter()
        .filter_map(|line| line.trim_start().strip_prefix("move ").map(str::trim));
    if let Some(first) = moves.next() {
        let mut text = first.to_owned();
        if text.ends_with(',')
            && let Some(second) = moves.next()
        {
            text.push_str(second);
        }
        return Ok(EngineResponse::Move(text));
    }

    if lines.iter().any(|line| {
        let line = line.trim_start();
        ["1-0", "0-1", "1/2-1/2", "resign"]
            .iter()
            .any(|prefix| line.starts_with(prefix))
    }) {
        Ok(EngineResponse::Resigned)
    } else {
        Err(EngineFailure::Crash)
    }
}

/// CECP応答を最初の着手または結果から`pong`まで読み切る。
fn receive_cecp_response(
    lines: &Receiver<io::Result<String>>,
    timeout: Duration,
    pong_number: usize,
    start: Instant,
) -> Result<(EngineResponse, Duration), EngineFailure> {
    let expected_pong = format!("pong {pong_number}");
    let mut received = Vec::new();
    let mut response_elapsed = None;
    let first = receive_until(lines, timeout, |line| {
        received.push(line.to_owned());
        if is_cecp_response_line(line) {
            response_elapsed = Some(start.elapsed());
            true
        } else {
            line.trim() == expected_pong
        }
    })?;
    if first.trim() != expected_pong {
        receive_until(lines, timeout, |line| {
            received.push(line.to_owned());
            line.trim() == expected_pong
        })?;
    }
    let response = interpret_cecp_response(&received)?;
    let elapsed = response_elapsed.expect("a valid CECP response has a response line");
    Ok((response, elapsed))
}

/// USI出力を条件一致、切断、または期限切れまで受信する。
fn receive_until(
    lines: &Receiver<io::Result<String>>,
    timeout: Duration,
    mut predicate: impl FnMut(&str) -> bool,
) -> Result<String, EngineFailure> {
    let start = Instant::now();
    loop {
        let remaining = timeout
            .checked_sub(start.elapsed())
            .ok_or(EngineFailure::Timeout)?;
        match lines.recv_timeout(remaining) {
            Ok(Ok(line)) if predicate(&line) => return Ok(line),
            Ok(Ok(_)) => {}
            Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => {
                return Err(EngineFailure::Crash);
            }
            Err(RecvTimeoutError::Timeout) => return Err(EngineFailure::Timeout),
        }
    }
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        // 標準入力を先に閉じる。ラッパースクリプト経由で起動したエンジンは
        // killでは止まらず、入力のEOFで終了して初めて出力パイプが閉じるため、
        // この順序でないと読み取りスレッドの回収が止まる。
        drop(self.input.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// ペア対局で共有する開始状態。
struct Opening {
    /// 開始手順を適用済みの対局。
    game: Game,
    /// 開始手順の生成に使ったシード。
    seed: NonZeroU64,
    /// 開始手順の指し手列。
    moves: Vec<Move>,
    /// 開始手順のUSI表記列。エンジンへの`position`送信に使う。
    usi_moves: Vec<String>,
}

/// 審判裁定またはエンジン反則による終局結果。
#[derive(Clone, Copy)]
enum GameOutcome {
    /// 審判層の裁定による終局。
    Adjudicated(GameResult),
    /// エンジンの反則による負け。
    Forfeit {
        /// 反則をしなかった側。
        winner: Color,
        /// 反則の分類。
        reason: EngineFailure,
    },
    /// エンジンの投了による勝敗。
    Resigned {
        /// 投了しなかった側。
        winner: Color,
    },
}

/// 1局の完走または手数上限による打ち切り。
#[derive(Clone, Copy)]
enum PlayedGame {
    /// 終局した1局。
    Finished {
        /// 終了時点の手数。
        plies: u32,
        /// 終局結果。
        outcome: GameOutcome,
    },
    /// 手数上限による打ち切り。
    Cutoff {
        /// 打ち切り時点の手数。
        plies: u32,
    },
}

/// 1局の集計結果と監査記録。
struct RecordedGame {
    /// 既存の得点計算と表示に使う終局結果。
    played: PlayedGame,
    /// 永続化する構造化記録。
    record: GameRecord,
}

/// 手番を保存形式へ変換する。
const fn stored_color(color: Color) -> StoredColor {
    match color {
        Color::Black => StoredColor::Black,
        Color::White => StoredColor::White,
    }
}

/// エンジン異常を保存形式へ変換する。
const fn stored_failure(failure: EngineFailure) -> FailureKind {
    match failure {
        EngineFailure::IllegalMove => FailureKind::IllegalMove,
        EngineFailure::Crash => FailureKind::Crash,
        EngineFailure::Timeout => FailureKind::Timeout,
        EngineFailure::TimeForfeit => FailureKind::TimeForfeit,
        EngineFailure::RejectedMove => FailureKind::RejectedMove,
    }
}

/// エンジン評価値へ視点を付けて保存形式へ変換する。
fn evaluation_record(
    evaluation: Option<EngineEvaluation>,
    perspective: Color,
) -> Option<EvaluationRecord> {
    evaluation.map(|evaluation| EvaluationRecord {
        perspective: stored_color(perspective),
        depth: evaluation.depth,
        score: match evaluation.score {
            EngineScore::Cp(value) => ScoreRecord::Cp { value },
            EngineScore::MateIn(moves) => ScoreRecord::MateIn { moves },
            EngineScore::MatedIn(moves) => ScoreRecord::MatedIn { moves },
        },
        bound: evaluation.bound,
    })
}

/// `Duration`を保存形式のナノ秒へ変換する。
fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).expect("an engine response duration must fit in u64 ns")
}

/// 終局結果を保存形式へ変換する。
fn termination_record(game: PlayedGame) -> TerminationRecord {
    match game {
        PlayedGame::Finished {
            outcome: GameOutcome::Adjudicated(GameResult::Win { winner, reason }),
            ..
        } => TerminationRecord::AdjudicatedWin {
            winner: stored_color(winner),
            reason: format!("{reason:?}"),
        },
        PlayedGame::Finished {
            outcome: GameOutcome::Adjudicated(GameResult::Draw { reason }),
            ..
        } => TerminationRecord::AdjudicatedDraw {
            reason: format!("{reason:?}"),
        },
        PlayedGame::Finished {
            outcome: GameOutcome::Forfeit { winner, reason },
            ..
        } => TerminationRecord::Forfeit {
            loser: stored_color(winner.opposite()),
            reason: stored_failure(reason),
        },
        PlayedGame::Finished {
            outcome: GameOutcome::Resigned { winner },
            ..
        } => TerminationRecord::Resigned {
            loser: stored_color(winner.opposite()),
        },
        PlayedGame::Cutoff { .. } => TerminationRecord::Cutoff,
    }
}

/// 1局の終局結果と収集済み着手をまとめる。
#[allow(clippy::too_many_arguments)]
fn recorded_game(
    played: PlayedGame,
    candidate_color: Color,
    candidate_seed: NonZeroU64,
    baseline_seed: NonZeroU64,
    turns: Vec<TurnRecord>,
    started: Instant,
    candidate_process: Option<&EngineProcess>,
    baseline_process: Option<&EngineProcess>,
) -> RecordedGame {
    let candidate_usage =
        candidate_process.map_or_else(EngineResourceUsage::default, EngineProcess::resource_usage);
    let baseline_usage =
        baseline_process.map_or_else(EngineResourceUsage::default, EngineProcess::resource_usage);
    RecordedGame {
        played,
        record: GameRecord {
            candidate_color: stored_color(candidate_color),
            candidate_seed: candidate_seed.get(),
            baseline_seed: baseline_seed.get(),
            wall_time_ns: duration_ns(started.elapsed()),
            candidate_cpu_time_ns: candidate_usage.cpu_time_ns,
            baseline_cpu_time_ns: baseline_usage.cpu_time_ns,
            candidate_peak_rss_bytes: candidate_usage.peak_rss_bytes,
            baseline_peak_rss_bytes: baseline_usage.peak_rss_bytes,
            turns,
            termination: termination_record(played),
        },
    }
}

/// 1ペアの集計結果。
struct PairResult {
    /// 候補側ペア得点のペンタノミアル分類(0〜4)。打ち切りを含むペアは`None`。
    category: Option<usize>,
    /// このペアで発生した異常の件数。
    failures: FailureCounts,
}

/// 1ペアの番号、表示内容、集計結果。
struct CompletedPair {
    /// 1起算のペア番号。
    number: u64,
    /// ペア番号順に出力する表示内容。
    output: String,
    /// 集計へ取り込む結果。
    result: PairResult,
    /// 原子的に保存する監査記録。
    record: PairRecord,
}

/// 完了したペアを番号順に取り込み、実験を続ける場合は次の1件を返す。
///
/// ジョブの補充は統計へ取り込めた件数ではなく、完了結果を1件受信した事実に
/// 対応させる。これにより、若い番号のペアが遅れても空いた並列枠を維持する。
fn accept_completed_pair(
    pair: CompletedPair,
    completed: &mut BTreeMap<u64, CompletedPair>,
    next_to_integrate: &mut u64,
    pending_jobs: &mut VecDeque<u64>,
    mut integrate: impl FnMut(CompletedPair) -> bool,
) -> Option<u64> {
    assert!(
        completed.insert(pair.number, pair).is_none(),
        "a pair number must be completed at most once"
    );
    while let Some(pair) = completed.remove(next_to_integrate) {
        *next_to_integrate = next_to_integrate
            .checked_add(1)
            .expect("pair number overflow");
        if !integrate(pair) {
            return None;
        }
    }

    pending_jobs.pop_front()
}

/// GSPRTの判定に応じて実験を続けるかを返し、停止時は全ワーカーへ通知する。
fn continue_after_decision(use_gsprt: bool, decision: GsprtDecision, stop: &AtomicBool) -> bool {
    let keep_running = !use_gsprt || decision == GsprtDecision::Continue;
    if !keep_running {
        stop.store(true, Ordering::Release);
    }
    keep_running
}

/// ジョブを受信してペアを実行し、停止で打ち切られなかった結果だけを送る。
fn run_worker_loop(
    job_receiver: &Mutex<Receiver<u64>>,
    result_sender: &mpsc::Sender<Result<CompletedPair, String>>,
    stop: &AtomicBool,
    mut run: impl FnMut(u64, &AtomicBool) -> Result<Option<CompletedPair>, String>,
) {
    loop {
        let job = job_receiver
            .lock()
            .expect("the job receiver mutex must not be poisoned")
            .recv();
        let Ok(pair_number) = job else {
            break;
        };
        let pair = match run(pair_number, stop) {
            Ok(Some(pair)) => pair,
            Ok(None) => break,
            Err(error) => {
                let _ = result_sender.send(Err(error));
                break;
            }
        };
        if result_sender.send(Ok(pair)).is_err() {
            break;
        }
    }
}

/// 外部エンジン指定を解析する。空白区切りの2語目以降は起動引数として渡す。
fn parse_player_spec(input: &str) -> Result<PlayerSpec, String> {
    if input == "random" {
        return Ok(PlayerSpec {
            text: input.to_owned(),
            kind: PlayerKind::Random,
        });
    }
    let mut tokens = input.split_whitespace();
    let Some(program) = tokens.next() else {
        return Err("engine spec must be a command line or 'random'".to_owned());
    };
    if let Some(program) = program.strip_prefix("cecp:") {
        if program.is_empty() {
            return Err("cecp: engine spec requires a startup command".to_owned());
        }
        return Ok(PlayerSpec {
            text: input.to_owned(),
            kind: PlayerKind::Cecp {
                program: PathBuf::from(program),
                args: tokens.map(str::to_owned).collect(),
            },
        });
    }
    if program.starts_with("depth=") {
        return Err("the legacy depth=N player spec is not supported; use --each".to_owned());
    }
    if let Some(revision) = program.strip_prefix("commit:") {
        if revision.is_empty() {
            return Err("commit: engine spec requires a revision".to_owned());
        }
        if tokens.next().is_some() {
            return Err("commit: engine spec does not accept startup arguments".to_owned());
        }
        return Ok(PlayerSpec {
            text: input.to_owned(),
            kind: PlayerKind::Commit(revision.to_owned()),
        });
    }
    Ok(PlayerSpec {
        text: input.to_owned(),
        kind: PlayerKind::Command {
            program: PathBuf::from(program),
            args: tokens.map(str::to_owned).collect(),
        },
    })
}

/// specを起動コマンドと実効制限へ解決する。
fn resolve_player(
    spec: PlayerSpec,
    limit: SearchLimit,
    rules_text: &str,
) -> io::Result<PlayerConfig> {
    let working_directory = std::env::current_dir()?;
    let (path, args, protocol, is_random, identity) = match spec.kind {
        PlayerKind::Random => {
            let current = std::env::current_exe()?;
            let filename = format!("usi_random{}", std::env::consts::EXE_SUFFIX);
            let path = current.with_file_name(filename);
            (
                path.clone(),
                Vec::new(),
                Protocol::Usi,
                true,
                EngineIdentity::Random {
                    sha256: sha256_file(&path)?,
                },
            )
        }
        PlayerKind::Commit(revision) => {
            let (path, hash, sha256) = resolve_commit(&revision)?;
            (
                path,
                vec![
                    "--protocol".to_owned(),
                    "usi".to_owned(),
                    "--rules".to_owned(),
                    rules_text.to_owned(),
                ],
                Protocol::Usi,
                false,
                EngineIdentity::Commit { hash, sha256 },
            )
        }
        PlayerKind::Command { program, args } => (
            program.clone(),
            args.clone(),
            Protocol::Usi,
            false,
            EngineIdentity::Command {
                program,
                args,
                protocol: StoredProtocol::Usi,
                working_directory,
            },
        ),
        PlayerKind::Cecp { program, args } => {
            validate_cecp_limit(limit)?;
            (
                program.clone(),
                args.clone(),
                Protocol::Cecp,
                false,
                EngineIdentity::Command {
                    program,
                    args,
                    protocol: StoredProtocol::Cecp,
                    working_directory,
                },
            )
        }
    };
    Ok(PlayerConfig {
        text: spec.text,
        identity,
        path,
        args,
        protocol,
        is_random,
        limit,
        rules_source: rules_text.to_owned(),
    })
}

/// CECPで表現できる思考制限かどうかを検証する。
fn validate_cecp_limit(limit: SearchLimit) -> io::Result<()> {
    match limit {
        SearchLimit::Fixed { nodes: Some(_), .. } => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CECP engine does not support a fixed node limit",
        )),
        SearchLimit::Fixed {
            depth: Some(_),
            nodes: None,
        } => Ok(()),
        SearchLimit::Fixed {
            depth: None,
            nodes: None,
        } => unreachable!("a validated fixed limit contains depth or nodes"),
        SearchLimit::Time(time) if time.byoyomi_ms > 0 => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CECP engine does not support byoyomi",
        )),
        SearchLimit::Time(time) if time.base_ms % 1_000 != 0 || time.increment_ms % 1_000 != 0 => {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "CECP time control requires base time and increment in whole seconds",
            ))
        }
        SearchLimit::Time(_) => Ok(()),
    }
}

/// Gitコマンドの失敗内容を標準出力と標準エラーを含めて返す。
fn command_error(action: &str, output: &process::Output) -> io::Error {
    let mut message = format!("{action} failed with {}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.trim().is_empty() {
        write!(message, "\nstdout:\n{}", stdout.trim_end()).expect("writing to String cannot fail");
    }
    if !stderr.trim().is_empty() {
        write!(message, "\nstderr:\n{}", stderr.trim_end()).expect("writing to String cannot fail");
    }
    io::Error::other(message)
}

/// リビジョンをコミットの完全ハッシュへ正規化する。
fn normalize_commit(repository: &Path, revision: &str) -> io::Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", &format!("{revision}^{{commit}}")])
        .current_dir(repository)
        .output()?;
    if !output.status.success() {
        return Err(command_error("git rev-parse --verify", &output));
    }
    let hash = String::from_utf8(output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(hash.trim().to_owned())
}

/// コミットをビルドし、完全ハッシュ単位のキャッシュへ解決する。
fn resolve_commit(revision: &str) -> io::Result<(PathBuf, String, String)> {
    let repository = std::env::current_dir()?;
    println!("resolving commit {revision}...");
    let hash = normalize_commit(&repository, revision)?;
    let cache_root = repository.join("target/match-cache");
    let binary_name = format!("minase{}", std::env::consts::EXE_SUFFIX);
    let cache_path = cache_root.join(&hash).join(&binary_name);
    let cache_directory = cache_path.parent().expect("cache path has a parent");
    fs::create_dir_all(cache_directory)?;
    let cache_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(cache_directory.join(".build.lock"))?;
    FileExt::lock_exclusive(&cache_lock)?;
    let digest_path = cache_path.with_extension("sha256");
    let cached_digest = fs::read_to_string(&digest_path).ok();
    if cache_path.is_file()
        && cached_digest.as_deref().is_some_and(|digest| {
            sha256_file(&cache_path).is_ok_and(|actual| actual == digest.trim())
        })
    {
        let sha256 = cached_digest
            .expect("the preceding condition requires a digest")
            .trim()
            .to_owned();
        println!("cached: {}", cache_path.display());
        return Ok((cache_path, hash, sha256));
    }

    println!("building commit {hash}...");
    fs::create_dir_all(&cache_root)?;
    let source_tree = cache_root.join(format!(".source-{hash}-{}", process::id()));
    let archive_path = cache_root.join(format!(".source-{hash}-{}.tar", process::id()));
    fs::create_dir(&source_tree)?;
    let archive_output = Command::new("git")
        .args(["archive", "--format=tar", "--output"])
        .arg(&archive_path)
        .arg(&hash)
        .current_dir(&repository)
        .output()?;
    let build_result = if !archive_output.status.success() {
        Err(command_error("git archive", &archive_output))
    } else {
        let extract_output = Command::new("tar")
            .args(["-xf"])
            .arg(&archive_path)
            .arg("-C")
            .arg(&source_tree)
            .output()?;
        if !extract_output.status.success() {
            Err(command_error("tar -xf", &extract_output))
        } else {
            (|| {
                let output = Command::new("cargo")
                    .args(["build", "--release", "--bin", "minase"])
                    .current_dir(&source_tree)
                    .output()?;
                if !output.status.success() {
                    return Err(command_error("cargo build --release --bin minase", &output));
                }
                let temporary_binary =
                    cache_directory.join(format!(".{binary_name}.{}.tmp", process::id()));
                let temporary_digest =
                    cache_directory.join(format!(".{binary_name}.sha256.{}.tmp", process::id()));
                let source = source_tree.join("target/release").join(&binary_name);
                let install_result = (|| {
                    fs::copy(&source, &temporary_binary)?;
                    File::open(&temporary_binary)?.sync_all()?;
                    let digest = sha256_file(&temporary_binary)?;
                    fs::write(&temporary_digest, format!("{digest}\n"))?;
                    File::open(&temporary_digest)?.sync_all()?;
                    fs::rename(&temporary_binary, &cache_path)?;
                    fs::rename(&temporary_digest, &digest_path)?;
                    File::open(cache_directory)?.sync_all()?;
                    Ok::<_, io::Error>(())
                })();
                if install_result.is_err() {
                    let _ = fs::remove_file(&temporary_binary);
                    let _ = fs::remove_file(&temporary_digest);
                }
                install_result?;
                Ok(())
            })()
        }
    };
    let archive_remove_result = match fs::remove_file(&archive_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    };
    let source_remove_result = fs::remove_dir_all(&source_tree);
    build_result?;
    archive_remove_result?;
    source_remove_result?;
    println!("cached: {}", cache_path.display());
    let sha256 = sha256_file(&cache_path)?;
    Ok((cache_path, hash, sha256))
}

/// 思考制限を解析する。
fn parse_search_limit(input: &str) -> Result<SearchLimit, String> {
    let mut fields = input.split(',');
    let first = fields.next().expect("split always returns one field");
    if let Some(value) = first.strip_prefix("time=") {
        let (base_ms, increment_ms) = value
            .split_once('+')
            .ok_or_else(|| "time limit must be 'time=<base_ms>+<inc_ms>'".to_owned())?;
        if increment_ms.contains('+') {
            return Err("time limit must contain exactly one '+'".to_owned());
        }
        let base_ms = parse_nonnegative_u64(base_ms)?;
        let increment_ms = parse_nonnegative_u64(increment_ms)?;
        let byoyomi_ms = fields
            .next()
            .map(|field| {
                field
                    .strip_prefix("byoyomi=")
                    .ok_or_else(|| "time limit may only be followed by 'byoyomi=<ms>'".to_owned())
                    .and_then(parse_nonnegative_u64)
            })
            .transpose()?
            .unwrap_or(0);
        if fields.next().is_some() {
            return Err("limit has too many comma-separated fields".to_owned());
        }
        return Ok(SearchLimit::Time(TimeControl {
            base_ms,
            increment_ms,
            byoyomi_ms,
        }));
    }

    let (depth, nodes) = if let Some(value) = first.strip_prefix("depth=") {
        let depth = parse_search_depth(value)?;
        let nodes = fields
            .next()
            .map(|field| {
                field
                    .strip_prefix("nodes=")
                    .ok_or_else(|| "the second limit field must be 'nodes=M'".to_owned())
                    .and_then(parse_positive_u64)
            })
            .transpose()?;
        (Some(depth), nodes)
    } else if let Some(value) = first.strip_prefix("nodes=") {
        (None, Some(parse_positive_u64(value)?))
    } else {
        return Err(
            "limit must be 'depth=N', 'nodes=M', 'depth=N,nodes=M', or 'time=B+I'".to_owned(),
        );
    };
    if fields.next().is_some() {
        return Err("limit has too many comma-separated fields".to_owned());
    }
    Ok(SearchLimit::Fixed { depth, nodes })
}

/// 0以上の`u64`を解析する。
fn parse_nonnegative_u64(text: &str) -> Result<u64, String> {
    text.parse::<u64>()
        .map_err(|error| format!("invalid nonnegative integer '{text}': {error}"))
}

/// 0より大きい`u64`を解析する。
fn parse_positive_u64(text: &str) -> Result<u64, String> {
    let value = text
        .parse::<u64>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if value == 0 {
        return Err("value must be at least 1".to_owned());
    }
    Ok(value)
}

/// 0より大きい`usize`を解析する。
fn parse_positive_usize(text: &str) -> Result<usize, String> {
    let value = text
        .parse::<usize>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if value == 0 {
        return Err("value must be at least 1".to_owned());
    }
    Ok(value)
}

/// 0より大きい`u32`を解析する。
fn parse_positive_u32(text: &str) -> Result<u32, String> {
    let value = text
        .parse::<u32>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if value == 0 {
        return Err("value must be at least 1".to_owned());
    }
    Ok(value)
}

/// 探索が扱える範囲の深さを解析する。
fn parse_search_depth(text: &str) -> Result<u32, String> {
    let depth = parse_positive_u32(text)?;
    if depth > MAX_PLY {
        return Err(format!("search depth must not exceed {MAX_PLY}"));
    }
    Ok(depth)
}

/// 現在時刻から基本シードを生成する。
fn time_seed() -> Result<u64, std::time::SystemTimeError> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(nanos as u64 ^ (nanos >> 64) as u64)
}

/// 探索制限を実行条件記録へ変換する。
const fn stored_search_limit(limit: SearchLimit) -> StoredSearchLimit {
    match limit {
        SearchLimit::Fixed { depth, nodes } => StoredSearchLimit::Fixed { depth, nodes },
        SearchLimit::Time(time) => StoredSearchLimit::Time {
            base_ms: time.base_ms,
            increment_ms: time.increment_ms,
            byoyomi_ms: time.byoyomi_ms,
        },
    }
}

/// 1エンジンを事前起動し、実際のプロトコル応答から既定資源を得る。
fn probe_engine_defaults(player: &PlayerConfig, timeout: Duration) -> io::Result<EngineDefaults> {
    if player.is_random {
        return Ok(EngineDefaults {
            threads: Some(1),
            hash_mb: None,
        });
    }
    if player.protocol == Protocol::Cecp {
        return Ok(EngineDefaults {
            threads: None,
            hash_mb: Some(u64::from(CECP_MEMORY_MB)),
        });
    }
    let mut process = EngineProcess::spawn(player, timeout).map_err(|_| {
        io::Error::other(format!(
            "failed to start {} for USI resource probe",
            player.text
        ))
    })?;
    let defaults = process.read_usi_defaults().map_err(|failure| {
        io::Error::other(format!(
            "failed to read USI resource defaults from {}: {failure:?}",
            player.text
        ))
    })?;
    if matches!(player.identity, EngineIdentity::Commit { .. })
        && (defaults.threads.is_none() || defaults.hash_mb.is_none())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "commit engine {} did not report Threads and USI_Hash defaults",
                player.text
            ),
        ));
    }
    Ok(defaults)
}

/// Linuxで取得できるCPU機種名を返す。
fn cpu_model() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("model name")
                    .and_then(|line| line.split_once(':'))
                    .map(|(_, value)| value.trim().to_owned())
            })
        })
        .unwrap_or_else(|| "unreported".to_owned())
}

/// Linuxがオンラインと報告する論理CPU番号を展開する。
#[cfg(target_os = "linux")]
fn online_cpu_indices() -> Option<BTreeSet<usize>> {
    let text = fs::read_to_string("/sys/devices/system/cpu/online").ok()?;
    let mut indices = BTreeSet::new();
    for range in text.trim().split(',') {
        let (start, end) = range
            .split_once('-')
            .map_or((range, range), |(start, end)| (start, end));
        let start = start.parse::<usize>().ok()?;
        let end = end.parse::<usize>().ok()?;
        if start > end {
            return None;
        }
        indices.extend(start..=end);
    }
    Some(indices)
}

/// LinuxのCPUトポロジーからオンライン物理コア数を得る。
#[cfg(target_os = "linux")]
fn physical_core_count() -> Option<usize> {
    let online = online_cpu_indices()?;
    let mut cores = BTreeSet::new();
    for cpu in online {
        let topology = PathBuf::from(format!("/sys/devices/system/cpu/cpu{cpu}/topology"));
        let package = fs::read_to_string(topology.join("physical_package_id"))
            .ok()?
            .trim()
            .parse::<u32>()
            .ok()?;
        let core = fs::read_to_string(topology.join("core_id"))
            .ok()?
            .trim()
            .parse::<u32>()
            .ok()?;
        cores.insert((package, core));
    }
    (!cores.is_empty()).then_some(cores.len())
}

/// CPUトポロジーを提供しないOSでは物理コア数を欠測とする。
#[cfg(not(target_os = "linux"))]
const fn physical_core_count() -> Option<usize> {
    None
}

/// 省略時の同時対局数を物理コア数と両エンジンのスレッド数から計算する。
fn default_concurrency(
    physical_cores: Option<usize>,
    candidate_threads: Option<u32>,
    baseline_threads: Option<u32>,
) -> Result<usize, String> {
    let physical_cores = physical_cores.ok_or_else(|| {
        "physical core count is unavailable; specify --concurrency explicitly".to_owned()
    })?;
    let candidate_threads = candidate_threads.ok_or_else(|| {
        "candidate engine Threads is unavailable; specify --concurrency explicitly".to_owned()
    })?;
    let baseline_threads = baseline_threads.ok_or_else(|| {
        "baseline engine Threads is unavailable; specify --concurrency explicitly".to_owned()
    })?;
    if candidate_threads == 0 || baseline_threads == 0 {
        return Err(
            "engine Threads must be at least 1; specify --concurrency explicitly".to_owned(),
        );
    }
    let engine_threads = candidate_threads.max(baseline_threads);
    let available_cores = physical_cores.checked_sub(1).ok_or_else(|| {
        "automatic concurrency is less than 1; specify --concurrency explicitly".to_owned()
    })?;
    let engine_threads = usize::try_from(engine_threads)
        .map_err(|_| "engine Threads is too large; specify --concurrency explicitly".to_owned())?;
    let concurrency = available_cores / engine_threads;
    if concurrency == 0 {
        return Err(
            "automatic concurrency is less than 1; specify --concurrency explicitly".to_owned(),
        );
    }
    Ok(concurrency)
}

/// Linuxの`MemTotal`から実メモリ容量を得る。
#[cfg(target_os = "linux")]
fn physical_memory_bytes() -> Option<u64> {
    fs::read_to_string("/proc/meminfo")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

/// 実メモリ容量を提供しないOSでは欠測とする。
#[cfg(not(target_os = "linux"))]
const fn physical_memory_bytes() -> Option<u64> {
    None
}

/// 現在の対局ハーネス実行ファイルをSHA-256で識別する。
fn harness_record() -> io::Result<HarnessRecord> {
    Ok(HarnessRecord {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        sha256: sha256_file(&std::env::current_exe()?)?,
    })
}

/// ファイル全体のSHA-256を小文字16進数で返す。
fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// CLIから再開時に完全一致させる実行条件記録を構成する。
#[allow(clippy::too_many_arguments)]
fn run_manifest(
    candidate: &PlayerConfig,
    baseline: &PlayerConfig,
    rules: &RuleSetArgument,
    mode: ManifestMode,
    seed: u64,
    max_ply: u32,
    response_timeout_secs: u64,
    concurrency: Option<usize>,
) -> io::Result<RunManifest> {
    let timeout = Duration::from_secs(response_timeout_secs);
    let candidate_defaults = probe_engine_defaults(candidate, timeout)?;
    let baseline_defaults = probe_engine_defaults(baseline, timeout)?;
    let physical_cores = physical_core_count();
    let concurrency = match concurrency {
        Some(concurrency) => concurrency,
        None => default_concurrency(
            physical_cores,
            candidate_defaults.threads,
            baseline_defaults.threads,
        )
        .map_err(io::Error::other)?,
    };
    Ok(RunManifest {
        format_version: FORMAT_VERSION,
        candidate: EngineRecord {
            identity: candidate.identity.clone(),
            limit: stored_search_limit(candidate.limit),
        },
        baseline: EngineRecord {
            identity: baseline.identity.clone(),
            limit: stored_search_limit(baseline.limit),
        },
        rules_source: rules.source.clone(),
        canonical_rules: rules.codes.iter().map(ToString::to_string).collect(),
        mode,
        seed,
        max_ply,
        response_timeout_secs,
        engine_threads: EngineThreadCounts {
            candidate: candidate_defaults.threads,
            baseline: baseline_defaults.threads,
        },
        hash_mb: EngineHashSizes {
            candidate: candidate_defaults.hash_mb,
            baseline: baseline_defaults.hash_mb,
        },
        concurrency,
        cpu: CpuRecord {
            model: cpu_model(),
            physical_cores,
            logical_cores: thread::available_parallelism()?.get(),
            physical_memory_bytes: physical_memory_bytes(),
        },
        runner: harness_record()?,
    })
}

/// 升をperftと同じ0起算座標で表記する。
fn square_text(square: Square) -> String {
    format!("({},{})", square.file(), square.rank())
}

/// 着手をperftの`move_text`と同じ形式で表記する。
fn move_text(mv: Move) -> String {
    if let Some(mid) = mv.mid {
        format!(
            "double {}->{}->{}{}",
            square_text(mv.from),
            square_text(mid),
            square_text(mv.to),
            if mv.promote { "+" } else { "" }
        )
    } else {
        format!(
            "move {}->{}{}",
            square_text(mv.from),
            square_text(mv.to),
            if mv.promote { "+" } else { "" }
        )
    }
}

/// 規則コード列をカンマ区切りで返す。
fn rules_text(codes: &[RuleCode]) -> String {
    codes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// 終局しない8手から12手の開始手順を生成する。
fn generate_opening(rules: Rules, pair_seed: NonZeroU64) -> Opening {
    let mut opening_seed = derive_seed(pair_seed.get(), 0);
    loop {
        let mut game = Game::new(rules);
        let mut rng = XorShift64::new(opening_seed);
        let opening_plies = 8 + rng.index(NonZeroUsize::new(5).unwrap());
        let mut moves = Vec::with_capacity(opening_plies);
        let mut usi_moves = Vec::with_capacity(opening_plies);
        let mut finished = false;

        for _ in 0..opening_plies {
            let legal_moves = game.legal_moves();
            assert!(
                !legal_moves.is_empty(),
                "ongoing game must have legal moves"
            );
            let selected = legal_moves
                [rng.index(NonZeroUsize::new(legal_moves.len()).expect("moves are non-empty"))];
            moves.push(selected);
            usi_moves.push(
                usi::text(
                    game.position(),
                    selected,
                    &MoveGenerator::new(game.rules().moves),
                )
                .expect("a move returned by legal_moves must be renderable"),
            );
            let status = game
                .play(selected)
                .expect("a move returned by legal_moves must be accepted");
            if matches!(status, GameStatus::Finished(_)) {
                finished = true;
                break;
            }
        }

        if !finished {
            return Opening {
                game,
                seed: opening_seed,
                moves,
                usi_moves,
            };
        }
        opening_seed = derive_seed(opening_seed.get(), 0);
    }
}

/// 合法手集合から移動元の内部升番号が最小の正準じっとを選ぶ。
fn canonical_jitto(legal_moves: &[Move]) -> Option<Move> {
    legal_moves
        .iter()
        .copied()
        .filter(|mv| mv.mid.is_none() && mv.from == mv.to)
        .min_by_key(|mv| mv.from.raw_index())
}

/// エンジンの着手表記を解析し、審判の合法手集合と照合する。
fn validate_bestmove(game: &Game, text: &str, protocol: Protocol) -> Result<Move, EngineFailure> {
    let legal_moves = game.legal_moves();
    let selected = match protocol {
        Protocol::Usi => {
            usi::parse(game.position(), text).map_err(|_| EngineFailure::IllegalMove)?
        }
        Protocol::Cecp if text == "@@@@" => {
            canonical_jitto(&legal_moves).ok_or(EngineFailure::IllegalMove)?
        }
        Protocol::Cecp => {
            cecp::parse(game.position(), text).map_err(|_| EngineFailure::IllegalMove)?
        }
    };
    if legal_moves.contains(&selected) {
        Ok(selected)
    } else {
        Err(EngineFailure::IllegalMove)
    }
}

/// 1局を既存の対局管理層で進行する。
#[allow(clippy::too_many_arguments)]
fn play_game(
    mut game: Game,
    mut usi_history: Vec<String>,
    mut move_history: Vec<Move>,
    max_ply: u32,
    player_a_color: Color,
    player_a: &PlayerConfig,
    player_a_seed: NonZeroU64,
    player_b: &PlayerConfig,
    player_b_seed: NonZeroU64,
    timeout: Duration,
    stop: &AtomicBool,
) -> Option<RecordedGame> {
    let started = Instant::now();
    let forfeit = |plies, loser: Color, reason| PlayedGame::Finished {
        plies,
        outcome: GameOutcome::Forfeit {
            winner: loser.opposite(),
            reason,
        },
    };
    let mut turns = Vec::new();
    if game.ply_count() >= max_ply {
        return Some(recorded_game(
            PlayedGame::Cutoff {
                plies: game.ply_count(),
            },
            player_a_color,
            player_a_seed,
            player_b_seed,
            turns,
            started,
            None,
            None,
        ));
    }
    let mut player_a_process = match EngineProcess::start(player_a, player_a_seed.get(), timeout) {
        Ok(process) => process,
        Err(reason) => {
            return Some(recorded_game(
                forfeit(game.ply_count(), player_a_color, reason),
                player_a_color,
                player_a_seed,
                player_b_seed,
                turns,
                started,
                None,
                None,
            ));
        }
    };
    let mut player_b_process = match EngineProcess::start(player_b, player_b_seed.get(), timeout) {
        Ok(process) => process,
        Err(reason) => {
            return Some(recorded_game(
                forfeit(game.ply_count(), player_a_color.opposite(), reason),
                player_a_color,
                player_a_seed,
                player_b_seed,
                turns,
                started,
                Some(&player_a_process),
                None,
            ));
        }
    };
    let mut clocks = GameClocks::new(player_a_color, player_a.limit, player_b.limit);

    loop {
        if stop.load(Ordering::Acquire) {
            return None;
        }
        if game.ply_count() >= max_ply {
            return Some(recorded_game(
                PlayedGame::Cutoff {
                    plies: game.ply_count(),
                },
                player_a_color,
                player_a_seed,
                player_b_seed,
                turns,
                started,
                Some(&player_a_process),
                Some(&player_b_process),
            ));
        }

        let side_to_move = game.position().side_to_move();
        let (process, limit) = if side_to_move == player_a_color {
            (&mut player_a_process, player_a.limit)
        } else {
            (&mut player_b_process, player_b.limit)
        };
        let request = clocks.think_request(side_to_move, limit);
        let ThinkResult {
            response,
            elapsed,
            evaluation,
            stop_reason,
            completed_time_ms,
        } = match process.bestmove(&usi_history, &move_history, &request) {
            Ok(response) => response,
            Err(reason) => {
                return Some(recorded_game(
                    forfeit(game.ply_count(), side_to_move, reason),
                    player_a_color,
                    player_a_seed,
                    player_b_seed,
                    turns,
                    started,
                    Some(&player_a_process),
                    Some(&player_b_process),
                ));
            }
        };
        if stop.load(Ordering::Acquire) {
            return None;
        }
        if let Some(clock) = clocks.get_mut(side_to_move)
            && let Err(reason) = clock.update(elapsed)
        {
            turns.push(TurnRecord {
                side: stored_color(side_to_move),
                think_time_ns: duration_ns(elapsed),
                evaluation: evaluation_record(evaluation, side_to_move),
                stop_reason,
                completed_time_ms,
                response: TurnResponse::Failure {
                    reason: stored_failure(reason),
                },
            });
            return Some(recorded_game(
                forfeit(game.ply_count(), side_to_move, reason),
                player_a_color,
                player_a_seed,
                player_b_seed,
                turns,
                started,
                Some(&player_a_process),
                Some(&player_b_process),
            ));
        }
        let EngineResponse::Move(response) = response else {
            turns.push(TurnRecord {
                side: stored_color(side_to_move),
                think_time_ns: duration_ns(elapsed),
                evaluation: evaluation_record(evaluation, side_to_move),
                stop_reason,
                completed_time_ms,
                response: TurnResponse::Resigned,
            });
            return Some(recorded_game(
                PlayedGame::Finished {
                    plies: game.ply_count(),
                    outcome: GameOutcome::Resigned {
                        winner: side_to_move.opposite(),
                    },
                },
                player_a_color,
                player_a_seed,
                player_b_seed,
                turns,
                started,
                Some(&player_a_process),
                Some(&player_b_process),
            ));
        };
        let selected = match validate_bestmove(&game, &response, process.protocol) {
            Ok(selected) => selected,
            Err(reason) => {
                turns.push(TurnRecord {
                    side: stored_color(side_to_move),
                    think_time_ns: duration_ns(elapsed),
                    evaluation: evaluation_record(evaluation, side_to_move),
                    stop_reason,
                    completed_time_ms,
                    response: TurnResponse::Failure {
                        reason: stored_failure(reason),
                    },
                });
                return Some(recorded_game(
                    forfeit(game.ply_count(), side_to_move, reason),
                    player_a_color,
                    player_a_seed,
                    player_b_seed,
                    turns,
                    started,
                    Some(&player_a_process),
                    Some(&player_b_process),
                ));
            }
        };
        let canonical = usi::text(
            game.position(),
            selected,
            &MoveGenerator::new(game.rules().moves),
        )
        .expect("a move validated against legal_moves must be renderable");
        turns.push(TurnRecord {
            side: stored_color(side_to_move),
            think_time_ns: duration_ns(elapsed),
            evaluation: evaluation_record(evaluation, side_to_move),
            stop_reason,
            completed_time_ms,
            response: TurnResponse::Move {
                usi: canonical.clone(),
            },
        });
        usi_history.push(canonical);
        move_history.push(selected);
        let status = game
            .play(selected)
            .expect("a move validated against legal_moves must be accepted");
        if let GameStatus::Finished(result) = status {
            return Some(recorded_game(
                PlayedGame::Finished {
                    plies: game.ply_count(),
                    outcome: GameOutcome::Adjudicated(result),
                },
                player_a_color,
                player_a_seed,
                player_b_seed,
                turns,
                started,
                Some(&player_a_process),
                Some(&player_b_process),
            ));
        }
    }
}

/// 1局の結果を候補Aから見た半点単位の得点へ変換する。
fn half_points(outcome: GameOutcome, player_a_color: Color) -> u8 {
    let winner = match outcome {
        GameOutcome::Adjudicated(GameResult::Win { winner, .. })
        | GameOutcome::Forfeit { winner, .. }
        | GameOutcome::Resigned { winner } => Some(winner),
        GameOutcome::Adjudicated(GameResult::Draw { .. }) => None,
    };
    match winner {
        Some(winner) if winner == player_a_color => 2,
        Some(_) => 0,
        None => 1,
    }
}

/// 1局の結果を表示文字列へ変換する。
fn played_game_text(game: PlayedGame) -> String {
    match game {
        PlayedGame::Finished {
            plies,
            outcome: GameOutcome::Adjudicated(GameResult::Win { winner, reason }),
        } => format!("plies={plies} result=win winner={winner:?} reason={reason:?}"),
        PlayedGame::Finished {
            plies,
            outcome: GameOutcome::Adjudicated(GameResult::Draw { reason }),
        } => format!("plies={plies} result=draw reason={reason:?}"),
        PlayedGame::Finished {
            plies,
            outcome: GameOutcome::Forfeit { winner, reason },
        } => format!("plies={plies} result=win winner={winner:?} reason={reason:?}"),
        PlayedGame::Finished {
            plies,
            outcome: GameOutcome::Resigned { winner },
        } => format!("plies={plies} result=win winner={winner:?} reason=Resigned"),
        PlayedGame::Cutoff { plies } => format!("plies={plies} result=cutoff"),
    }
}

/// 1局の異常理由を集計する。
fn record_game_failure(game: PlayedGame, counts: &mut FailureCounts) {
    if let PlayedGame::Finished {
        outcome: GameOutcome::Forfeit { reason, .. },
        ..
    } = game
    {
        counts.record(reason);
    }
}

/// 保存形式の手番を対局管理層の手番へ戻す。
const fn color_from_stored(color: StoredColor) -> Color {
    match color {
        StoredColor::Black => Color::Black,
        StoredColor::White => Color::White,
    }
}

/// 保存形式の異常分類を集計用の分類へ戻す。
const fn failure_from_stored(reason: FailureKind) -> EngineFailure {
    match reason {
        FailureKind::IllegalMove => EngineFailure::IllegalMove,
        FailureKind::Crash => EngineFailure::Crash,
        FailureKind::Timeout => EngineFailure::Timeout,
        FailureKind::TimeForfeit => EngineFailure::TimeForfeit,
        FailureKind::RejectedMove => EngineFailure::RejectedMove,
    }
}

/// 審判層の終局結果が保存済みの終局理由と一致するかを返す。
fn adjudication_matches(result: GameResult, termination: &TerminationRecord) -> bool {
    match (result, termination) {
        (
            GameResult::Win { winner, reason },
            TerminationRecord::AdjudicatedWin {
                winner: saved_winner,
                reason: saved_reason,
            },
        ) => stored_color(winner) == *saved_winner && format!("{reason:?}") == *saved_reason,
        (
            GameResult::Draw { reason },
            TerminationRecord::AdjudicatedDraw {
                reason: saved_reason,
            },
        ) => format!("{reason:?}") == *saved_reason,
        _ => false,
    }
}

/// 保存済み棋譜のプロトコルと時計を検証するための実効条件。
#[derive(Clone, Copy)]
struct SavedGameConditions {
    candidate_protocol: Protocol,
    candidate_limit: SearchLimit,
    baseline_protocol: Protocol,
    baseline_limit: SearchLimit,
    response_timeout: Duration,
}

/// 保存済み1局を開始局面から再生し、集計可能な終局結果へ戻す。
fn validate_saved_game(
    record: &GameRecord,
    opening: &Opening,
    max_ply: u32,
    conditions: SavedGameConditions,
) -> io::Result<PlayedGame> {
    if record.wall_time_ns == 0 {
        return Err(invalid_pair_record("saved game wall time is zero"));
    }
    let total_think_time_ns = record.turns.iter().try_fold(0_u128, |total, turn| {
        total.checked_add(u128::from(turn.think_time_ns))
    });
    if total_think_time_ns.is_none_or(|total| total > u128::from(record.wall_time_ns)) {
        return Err(invalid_pair_record(
            "saved think times exceed the game wall time",
        ));
    }
    let mut game = opening.game.clone();
    let candidate_color = color_from_stored(record.candidate_color);
    let mut clocks = GameClocks::new(
        candidate_color,
        conditions.candidate_limit,
        conditions.baseline_limit,
    );
    for (index, turn) in record.turns.iter().enumerate() {
        if game.ply_count() >= max_ply {
            return Err(invalid_pair_record("saved turn exceeds the ply limit"));
        }
        if Duration::from_nanos(turn.think_time_ns) > conditions.response_timeout {
            return Err(invalid_pair_record(
                "saved think time exceeds the response timeout",
            ));
        }
        let side = game.position().side_to_move();
        if turn.side != stored_color(side)
            || turn
                .evaluation
                .as_ref()
                .is_some_and(|evaluation| evaluation.perspective != turn.side)
        {
            return Err(invalid_pair_record(
                "turn side or evaluation perspective is inconsistent",
            ));
        }
        let protocol = if side == candidate_color {
            conditions.candidate_protocol
        } else {
            conditions.baseline_protocol
        };
        if protocol == Protocol::Cecp
            && (turn.evaluation.is_some()
                || turn.stop_reason.is_some()
                || turn.completed_time_ms.is_some())
        {
            return Err(invalid_pair_record(
                "CECP turn must not contain USI search information",
            ));
        }
        let expects_time_forfeit = matches!(
            turn.response,
            TurnResponse::Failure {
                reason: FailureKind::TimeForfeit
            }
        );
        match clocks.get_mut(side) {
            Some(clock) => {
                let timed_out = clock
                    .update(Duration::from_nanos(turn.think_time_ns))
                    .is_err();
                if timed_out != expects_time_forfeit {
                    return Err(invalid_pair_record(
                        "saved think time does not match the clock result",
                    ));
                }
            }
            None if expects_time_forfeit => {
                return Err(invalid_pair_record(
                    "fixed-limit engine cannot lose on time",
                ));
            }
            _ => {}
        }
        let is_last = index + 1 == record.turns.len();
        match &turn.response {
            TurnResponse::Move { usi: text } => {
                let selected = usi::parse(game.position(), text)
                    .map_err(|_| invalid_pair_record("saved move is not valid USI"))?;
                if !game.legal_moves().contains(&selected) {
                    return Err(invalid_pair_record("saved move is illegal"));
                }
                let canonical = usi::text(
                    game.position(),
                    selected,
                    &MoveGenerator::new(game.rules().moves),
                )
                .map_err(|_| invalid_pair_record("saved move cannot be rendered"))?;
                if canonical != *text {
                    return Err(invalid_pair_record("saved move is not canonical USI"));
                }
                if let GameStatus::Finished(result) = game
                    .play(selected)
                    .map_err(|_| invalid_pair_record("saved move was rejected"))?
                {
                    if !is_last || !adjudication_matches(result, &record.termination) {
                        return Err(invalid_pair_record(
                            "adjudicated result does not match moves",
                        ));
                    }
                    return Ok(PlayedGame::Finished {
                        plies: game.ply_count(),
                        outcome: GameOutcome::Adjudicated(result),
                    });
                }
            }
            TurnResponse::Resigned => {
                if !is_last
                    || record.termination
                        != (TerminationRecord::Resigned {
                            loser: stored_color(side),
                        })
                {
                    return Err(invalid_pair_record(
                        "resignation does not match termination",
                    ));
                }
                return Ok(PlayedGame::Finished {
                    plies: game.ply_count(),
                    outcome: GameOutcome::Resigned {
                        winner: side.opposite(),
                    },
                });
            }
            TurnResponse::Failure { reason } => {
                if !matches!(reason, FailureKind::IllegalMove | FailureKind::TimeForfeit) {
                    return Err(invalid_pair_record(
                        "failure without an engine response must not be a turn",
                    ));
                }
                if !is_last
                    || record.termination
                        != (TerminationRecord::Forfeit {
                            loser: stored_color(side),
                            reason: *reason,
                        })
                {
                    return Err(invalid_pair_record(
                        "engine failure does not match termination",
                    ));
                }
                return Ok(PlayedGame::Finished {
                    plies: game.ply_count(),
                    outcome: GameOutcome::Forfeit {
                        winner: side.opposite(),
                        reason: failure_from_stored(*reason),
                    },
                });
            }
        }
    }

    match record.termination {
        TerminationRecord::Cutoff if game.ply_count() >= max_ply => Ok(PlayedGame::Cutoff {
            plies: game.ply_count(),
        }),
        TerminationRecord::Forfeit { loser, reason }
            if game.ply_count() < max_ply
                && ((record.turns.is_empty()
                    && matches!(reason, FailureKind::Crash | FailureKind::Timeout))
                    || (color_from_stored(loser) == game.position().side_to_move()
                        && matches!(
                            reason,
                            FailureKind::Crash | FailureKind::Timeout | FailureKind::RejectedMove
                        )
                        && (record.turns.is_empty()
                            || matches!(
                                record.turns.last().map(|turn| &turn.response),
                                Some(TurnResponse::Move { .. })
                            )))) =>
        {
            Ok(PlayedGame::Finished {
                plies: game.ply_count(),
                outcome: GameOutcome::Forfeit {
                    winner: color_from_stored(loser).opposite(),
                    reason: failure_from_stored(reason),
                },
            })
        }
        _ => Err(invalid_pair_record(
            "termination is not explained by the saved turns",
        )),
    }
}

/// 破損した対局記録を表すエラーを作る。
fn invalid_pair_record(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// 保存済みペアを決定的に検証し、番号順集計へ戻す。
fn completed_pair_from_record(
    record: PairRecord,
    rules: Rules,
    base_seed: u64,
    max_ply: u32,
    response_timeout: Duration,
    candidate: &PlayerConfig,
    baseline: &PlayerConfig,
) -> io::Result<CompletedPair> {
    let pair_seed = derive_seed(base_seed, record.pair_number);
    if record.pair_seed != pair_seed.get() {
        return Err(invalid_pair_record("pair seed does not match pair number"));
    }
    let opening = generate_opening(rules, pair_seed);
    if record.opening.seed != opening.seed.get() || record.opening.moves != opening.usi_moves {
        return Err(invalid_pair_record("opening does not match the pair seed"));
    }
    if record.games[0].candidate_color != StoredColor::Black
        || record.games[1].candidate_color != StoredColor::White
    {
        return Err(invalid_pair_record(
            "candidate colors are not a swapped pair",
        ));
    }
    let expected_seeds = [
        (
            derive_seed(pair_seed.get(), 1),
            derive_seed(pair_seed.get(), 2),
        ),
        (
            derive_seed(pair_seed.get(), 3),
            derive_seed(pair_seed.get(), 4),
        ),
    ];
    for (index, game) in record.games.iter().enumerate() {
        let (candidate, baseline) = expected_seeds[index];
        if game.candidate_seed != candidate.get() || game.baseline_seed != baseline.get() {
            return Err(invalid_pair_record(
                "engine seed does not match the pair seed",
            ));
        }
    }
    let conditions = SavedGameConditions {
        candidate_protocol: candidate.protocol,
        candidate_limit: candidate.limit,
        baseline_protocol: baseline.protocol,
        baseline_limit: baseline.limit,
        response_timeout,
    };
    let game1 = validate_saved_game(&record.games[0], &opening, max_ply, conditions)?;
    let game2 = validate_saved_game(&record.games[1], &opening, max_ply, conditions)?;
    let mut failures = FailureCounts::default();
    record_game_failure(game1, &mut failures);
    record_game_failure(game2, &mut failures);
    let category = match (game1, game2) {
        (
            PlayedGame::Finished { outcome: first, .. },
            PlayedGame::Finished {
                outcome: second, ..
            },
        ) => Some(usize::from(
            half_points(first, Color::Black) + half_points(second, Color::White),
        )),
        _ => None,
    };
    if record.category.map(usize::from) != category {
        return Err(invalid_pair_record(
            "pair category does not match the game results",
        ));
    }
    Ok(CompletedPair {
        number: record.pair_number,
        output: format!("pair {}: loaded from saved record\n", record.pair_number),
        result: PairResult { category, failures },
        record,
    })
}

/// 1ペアを実行し、表示内容とペンタノミアル分類を返す。
#[allow(clippy::too_many_arguments)]
fn run_pair(
    rules: Rules,
    rules_text: &str,
    base_seed: u64,
    pair_number: u64,
    max_ply: u32,
    candidate: &PlayerConfig,
    baseline: &PlayerConfig,
    timeout: Duration,
    stop: &AtomicBool,
) -> Option<CompletedPair> {
    if stop.load(Ordering::Acquire) {
        return None;
    }
    let pair_seed = derive_seed(base_seed, pair_number);
    let opening = generate_opening(rules, pair_seed);
    let opening_record = OpeningRecord {
        seed: opening.seed.get(),
        moves: opening.usi_moves.clone(),
    };
    let game1_a_seed = derive_seed(pair_seed.get(), 1);
    let game1_b_seed = derive_seed(pair_seed.get(), 2);
    let game2_a_seed = derive_seed(pair_seed.get(), 3);
    let game2_b_seed = derive_seed(pair_seed.get(), 4);

    let mut output = String::new();
    writeln!(
        output,
        "pair {pair_number}: pair_seed={pair_seed} opening_seed={} player_a={} player_b={} rules={rules_text} max_ply={max_ply}",
        opening.seed,
        candidate.name(),
        baseline.name()
    )
    .expect("writing to String cannot fail");
    writeln!(
        output,
        "pair {pair_number} opening: plies={}",
        opening.moves.len()
    )
    .expect("writing to String cannot fail");
    for (index, &mv) in opening.moves.iter().enumerate() {
        writeln!(output, "  {}: {}", index + 1, move_text(mv))
            .expect("writing to String cannot fail");
    }

    writeln!(
        output,
        "pair {pair_number} game 1 settings: A=Black seed_a={game1_a_seed} B=White seed_b={game1_b_seed}"
    )
    .expect("writing to String cannot fail");
    let game1 = play_game(
        opening.game.clone(),
        opening.usi_moves.clone(),
        opening.moves.clone(),
        max_ply,
        Color::Black,
        candidate,
        game1_a_seed,
        baseline,
        game1_b_seed,
        timeout,
        stop,
    )?;
    writeln!(
        output,
        "pair {pair_number} game 1: {}",
        played_game_text(game1.played)
    )
    .expect("writing to String cannot fail");

    writeln!(
        output,
        "pair {pair_number} game 2 settings: B=Black seed_b={game2_b_seed} A=White seed_a={game2_a_seed}"
    )
    .expect("writing to String cannot fail");
    let game2 = play_game(
        opening.game,
        opening.usi_moves,
        opening.moves,
        max_ply,
        Color::White,
        candidate,
        game2_a_seed,
        baseline,
        game2_b_seed,
        timeout,
        stop,
    )?;
    writeln!(
        output,
        "pair {pair_number} game 2: {}",
        played_game_text(game2.played)
    )
    .expect("writing to String cannot fail");

    let mut failures = FailureCounts::default();
    record_game_failure(game1.played, &mut failures);
    record_game_failure(game2.played, &mut failures);
    let pair_record = |category| PairRecord {
        pair_number,
        pair_seed: pair_seed.get(),
        opening: opening_record.clone(),
        games: [game1.record.clone(), game2.record.clone()],
        category,
    };
    let (
        PlayedGame::Finished {
            outcome: game1_outcome,
            ..
        },
        PlayedGame::Finished {
            outcome: game2_outcome,
            ..
        },
    ) = (game1.played, game2.played)
    else {
        writeln!(output, "pair {pair_number} result: discarded")
            .expect("writing to String cannot fail");
        return Some(CompletedPair {
            number: pair_number,
            output,
            result: PairResult {
                category: None,
                failures,
            },
            record: pair_record(None),
        });
    };
    let category = usize::from(
        half_points(game1_outcome, Color::Black) + half_points(game2_outcome, Color::White),
    );
    writeln!(
        output,
        "pair {pair_number} result: score_a={:.1} category={category}",
        category as f64 / 2.0
    )
    .expect("writing to String cannot fail");
    Some(CompletedPair {
        number: pair_number,
        output,
        result: PairResult {
            category: Some(category),
            failures,
        },
        record: pair_record(Some(u8::try_from(category).expect("category is in 0..=4"))),
    })
}

/// GSPRTの判定を表示文字列へ変換する。
const fn decision_text(decision: GsprtDecision) -> &'static str {
    match decision {
        GsprtDecision::AcceptH0 => "H0",
        GsprtDecision::Continue => "pending",
        GsprtDecision::AcceptH1 => "H1",
    }
}

/// 無限大を含むEloを表示する。
fn elo_text(elo: f64) -> String {
    if elo == f64::INFINITY {
        "+inf".to_owned()
    } else if elo == f64::NEG_INFINITY {
        "-inf".to_owned()
    } else {
        format!("{elo:.6}")
    }
}

/// 異常理由別の件数を表示する。
fn print_failure_summary(failures: FailureCounts) {
    println!(
        "engine_failures: illegal_moves={} crashes={} timeouts={} time_forfeits={} rejected_moves={}",
        failures.illegal_moves,
        failures.crashes,
        failures.timeouts,
        failures.time_forfeits,
        failures.rejected_moves
    );
}

/// GSPRTの最終集計を表示する。
fn print_gsprt_summary(
    results: &[u64; 5],
    discarded_pairs: u64,
    failures: FailureCounts,
    decision: GsprtDecision,
    elapsed: Duration,
) {
    println!(
        "summary: mode=gsprt pairs={} valid_pairs={} discarded_pairs={discarded_pairs}",
        results.iter().sum::<u64>() + discarded_pairs,
        results.iter().sum::<u64>()
    );
    println!("pentanomial: {results:?}");
    println!("llr: {:.10}", gsprt_llr(results));
    println!("decision: {}", decision_text(decision));
    print_failure_summary(failures);
    println!("elapsed: {:.6} s", elapsed.as_secs_f64());
}

/// 固定局数Eloの最終集計を表示する。
fn print_elo_summary(
    results: &[u64; 5],
    discarded_pairs: u64,
    failures: FailureCounts,
    elapsed: Duration,
) {
    println!(
        "summary: mode=elo pairs={} valid_pairs={} discarded_pairs={discarded_pairs}",
        results.iter().sum::<u64>() + discarded_pairs,
        results.iter().sum::<u64>()
    );
    println!("pentanomial: {results:?}");
    if results.iter().sum::<u64>() == 0 {
        println!("elo: unavailable ci95=unavailable");
    } else {
        let estimate = estimate_elo(results);
        println!(
            "elo: estimate={} ci95=[{}, {}]",
            elo_text(estimate.elo),
            elo_text(estimate.lower),
            elo_text(estimate.upper)
        );
    }
    print_failure_summary(failures);
    println!("elapsed: {:.6} s", elapsed.as_secs_f64());
}

/// 1ペアを番号順の統計へ取り込み、逐次検定を継続するかを返す。
#[allow(clippy::too_many_arguments)]
fn integrate_pair_statistics(
    pair: CompletedPair,
    results: &mut [u64; 5],
    valid_pairs: &mut u64,
    discarded_pairs: &mut u64,
    failures: &mut FailureCounts,
    decision: &mut GsprtDecision,
    use_gsprt: bool,
    stop: &AtomicBool,
) -> bool {
    print!("{}", pair.output);
    failures.add(pair.result.failures);
    match pair.result.category {
        Some(category) => {
            results[category] += 1;
            *valid_pairs += 1;
            if use_gsprt {
                let llr = gsprt_llr(results);
                *decision = gsprt_decision(llr);
                println!(
                    "statistics: valid_pairs={valid_pairs} pentanomial={results:?} llr={llr:.10} decision={}",
                    decision_text(*decision)
                );
            }
        }
        None => *discarded_pairs += 1,
    }
    continue_after_decision(use_gsprt, *decision, stop)
}

/// 引数を検証し、ワーカープールでペア対局を実行して集計を出力する。
fn main() {
    if let Err(error) = minase::eval::weights() {
        eprintln!("error: embedded evaluation weights are invalid: {error}");
        process::exit(1);
    }
    let arguments = Arguments::parse();
    let rules = match Rules::from_codes(&arguments.rules.codes) {
        Ok(rules) => rules,
        Err(error) => Arguments::command()
            .error(ErrorKind::ValueValidation, error.to_string())
            .exit(),
    };
    if arguments.resume.is_some() && arguments.seed.is_none() {
        Arguments::command()
            .error(
                ErrorKind::MissingRequiredArgument,
                "--seed is required with --resume so the experiment can be verified",
            )
            .exit();
    }
    let base_seed = match arguments.seed {
        Some(seed) => seed,
        None => match time_seed() {
            Ok(seed) => seed,
            Err(error) => {
                eprintln!("failed to generate a seed from the current time: {error}");
                process::exit(1);
            }
        },
    };
    let (target_pairs, use_gsprt, manifest_mode) = match arguments.mode {
        Mode::Gsprt { max_pairs } => (
            max_pairs,
            true,
            ManifestMode::Gsprt {
                h0_elo: 0.0,
                h1_elo: GSPRT_H1_ELO,
                alpha: 0.05,
                beta: 0.05,
            },
        ),
        Mode::Elo { pairs } => (pairs, false, ManifestMode::Elo),
    };
    let candidate_limit = arguments.candidate_limit.unwrap_or(arguments.each);
    let baseline_limit = arguments.baseline_limit.unwrap_or(arguments.each);
    let rules_text = rules_text(&arguments.rules.codes);
    let candidate = match resolve_player(
        arguments.candidate,
        candidate_limit,
        &arguments.rules.source,
    ) {
        Ok(player) => player,
        Err(error) => {
            eprintln!("failed to resolve candidate engine: {error}");
            process::exit(1);
        }
    };
    let baseline = match resolve_player(arguments.baseline, baseline_limit, &arguments.rules.source)
    {
        Ok(player) => player,
        Err(error) => {
            eprintln!("failed to resolve baseline engine: {error}");
            process::exit(1);
        }
    };
    let manifest = match run_manifest(
        &candidate,
        &baseline,
        &arguments.rules,
        manifest_mode,
        base_seed,
        arguments.max_ply,
        arguments.response_timeout,
        arguments.concurrency,
    ) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("failed to identify the experiment environment: {error}");
            process::exit(1);
        }
    };
    let concurrency = manifest.concurrency;
    let (store, saved_records) = match (&arguments.run_dir, &arguments.resume) {
        (Some(path), None) => match RunStore::create(path, manifest) {
            Ok(store) => (store, BTreeMap::new()),
            Err(error) => {
                eprintln!("failed to create run directory {}: {error}", path.display());
                process::exit(1);
            }
        },
        (None, Some(path)) => match RunStore::resume(path, &manifest, target_pairs) {
            Ok(result) => result,
            Err(error) => {
                eprintln!("failed to resume run directory {}: {error}", path.display());
                process::exit(1);
            }
        },
        _ => unreachable!("clap requires exactly one run operation"),
    };
    let response_timeout = Duration::from_secs(arguments.response_timeout);
    println!("run_dir: {}", store.path().display());
    println!("rules: {rules_text}");
    println!("seed: {base_seed}");
    println!("max_ply: {}", arguments.max_ply);
    println!("candidate: {}", candidate.name());
    println!("baseline: {}", baseline.name());
    println!("response_timeout: {} s", arguments.response_timeout);

    let mut results = [0; 5];
    let mut valid_pairs = 0;
    let mut discarded_pairs = 0;
    let mut failures = FailureCounts::default();
    let mut decision = GsprtDecision::Continue;
    let stop = Arc::new(AtomicBool::new(false));
    let saved_numbers = saved_records.keys().copied().collect::<BTreeSet<_>>();
    let mut completed = BTreeMap::new();
    for (number, record) in saved_records {
        let pair = match completed_pair_from_record(
            record,
            rules,
            base_seed,
            arguments.max_ply,
            response_timeout,
            &candidate,
            &baseline,
        ) {
            Ok(pair) => pair,
            Err(error) => {
                eprintln!("saved pair {number} is invalid: {error}");
                process::exit(1);
            }
        };
        completed.insert(number, pair);
    }
    let mut next_to_integrate = 1_u64;
    while let Some(pair) = completed.remove(&next_to_integrate) {
        next_to_integrate = next_to_integrate
            .checked_add(1)
            .expect("pair number overflow");
        if !integrate_pair_statistics(
            pair,
            &mut results,
            &mut valid_pairs,
            &mut discarded_pairs,
            &mut failures,
            &mut decision,
            use_gsprt,
            &stop,
        ) {
            break;
        }
    }
    let mut pending_jobs = (1..=target_pairs)
        .filter(|number| !saved_numbers.contains(number))
        .collect::<VecDeque<_>>();
    let has_new_work = !stop.load(Ordering::Acquire) && !pending_jobs.is_empty();
    if has_new_work && let Err(error) = store.begin_invocation() {
        eprintln!("failed to begin active wall time recording: {error}");
        process::exit(1);
    }
    let start = Instant::now();
    let worker_count = if stop.load(Ordering::Acquire) {
        0
    } else {
        concurrency.min(pending_jobs.len())
    };
    let pool_result = thread::scope(|scope| {
        let (job_sender, job_receiver) = mpsc::channel::<u64>();
        let job_receiver = Arc::new(Mutex::new(job_receiver));
        let (result_sender, result_receiver) = mpsc::channel::<Result<CompletedPair, String>>();
        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let result_sender = result_sender.clone();
            let job_receiver = Arc::clone(&job_receiver);
            let stop = Arc::clone(&stop);
            let candidate = &candidate;
            let baseline = &baseline;
            let rules_text = &rules_text;
            let store = &store;
            workers.push(scope.spawn(move || {
                let worker_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_worker_loop(&job_receiver, &result_sender, &stop, |pair_number, stop| {
                        let pair = run_pair(
                            rules,
                            rules_text,
                            base_seed,
                            pair_number,
                            arguments.max_ply,
                            candidate,
                            baseline,
                            response_timeout,
                            stop,
                        );
                        let Some(pair) = pair else {
                            return Ok(None);
                        };
                        store.save_pair(&pair.record).map_err(|error| {
                            format!("failed to save pair {pair_number}: {error}")
                        })?;
                        Ok(Some(pair))
                    });
                }));
                if worker_result.is_err() {
                    let _ = result_sender.send(Err("a match worker panicked".to_owned()));
                }
            }));
        }
        drop(result_sender);

        for _ in 0..worker_count {
            let pair_number = pending_jobs
                .pop_front()
                .expect("worker count is bounded by pending jobs");
            job_sender
                .send(pair_number)
                .expect("workers must be waiting for initial jobs");
        }
        let mut pool_error = None;

        while next_to_integrate <= target_pairs
            && (!use_gsprt || decision == GsprtDecision::Continue)
        {
            let pair = match result_receiver.recv() {
                Ok(Ok(pair)) => pair,
                Ok(Err(error)) => {
                    pool_error = Some(error);
                    break;
                }
                Err(error) => {
                    pool_error = Some(format!("worker result channel disconnected: {error}"));
                    break;
                }
            };
            // 完了ペアを一旦バッファし、ペア番号順に出力とLLR取り込みを行う。
            // 補充は受信1件につき1件なので、若い番号の完了を待つ間も枠が空かない。
            let replacement = accept_completed_pair(
                pair,
                &mut completed,
                &mut next_to_integrate,
                &mut pending_jobs,
                |pair| {
                    integrate_pair_statistics(
                        pair,
                        &mut results,
                        &mut valid_pairs,
                        &mut discarded_pairs,
                        &mut failures,
                        &mut decision,
                        use_gsprt,
                        &stop,
                    )
                },
            );
            if let Some(job) = replacement
                && let Err(error) = job_sender.send(job)
            {
                pool_error = Some(format!("worker job channel disconnected: {error}"));
            }
            if pool_error.is_none()
                && let Err(error) = store.checkpoint(start.elapsed())
            {
                pool_error = Some(format!("failed to checkpoint active wall time: {error}"));
            }
            if pool_error.is_some() {
                break;
            }
        }

        stop.store(true, Ordering::Release);
        drop(job_sender);
        let mut worker_panicked = false;
        for worker in workers {
            worker_panicked |= worker.join().is_err();
        }
        worker_panicked |= result_receiver.try_iter().any(|result| result.is_err());
        if worker_panicked {
            Err("a match worker panicked".to_owned())
        } else if let Some(error) = pool_error {
            Err(error)
        } else {
            Ok(())
        }
    });
    if let Err(error) = pool_result {
        eprintln!("match execution failed: {error}");
        process::exit(1);
    }
    if has_new_work && let Err(error) = store.finish_invocation(start.elapsed()) {
        eprintln!("failed to finalize active wall time: {error}");
        process::exit(1);
    }

    if use_gsprt {
        print_gsprt_summary(
            &results,
            discarded_pairs,
            failures,
            decision,
            start.elapsed(),
        );
    } else {
        print_elo_summary(&results, discarded_pairs, failures, start.elapsed());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase::{DrawReason, WinReason};

    #[test]
    fn default_concurrency_uses_available_cores_and_larger_thread_count() {
        assert_eq!(default_concurrency(Some(20), Some(1), Some(1)), Ok(19));
        assert_eq!(default_concurrency(Some(20), Some(4), Some(2)), Ok(4));
    }

    #[test]
    fn default_concurrency_requires_resource_counts() {
        let error = default_concurrency(None, Some(1), Some(1)).unwrap_err();
        assert!(error.contains("physical core count"));
        assert!(error.contains("--concurrency"));

        let error = default_concurrency(Some(20), None, Some(1)).unwrap_err();
        assert!(error.contains("candidate engine Threads"));
        assert!(error.contains("--concurrency"));

        let error = default_concurrency(Some(20), Some(1), None).unwrap_err();
        assert!(error.contains("baseline engine Threads"));
        assert!(error.contains("--concurrency"));
    }

    #[test]
    fn default_concurrency_rejects_values_below_one() {
        let error = default_concurrency(Some(1), Some(1), Some(1)).unwrap_err();
        assert!(error.contains("--concurrency"));

        let error = default_concurrency(Some(20), Some(0), Some(1)).unwrap_err();
        assert!(error.contains("Threads must be at least 1"));
        assert!(error.contains("--concurrency"));

        let error = default_concurrency(Some(20), Some(1), Some(0)).unwrap_err();
        assert!(error.contains("Threads must be at least 1"));
        assert!(error.contains("--concurrency"));
    }

    #[test]
    fn usi_resource_probe_parses_exact_spin_option_defaults() {
        let mut defaults = EngineDefaults::default();
        observe_usi_default(
            &mut defaults,
            "option name Threads type spin default 2 min 1 max 20",
        );
        observe_usi_default(
            &mut defaults,
            "option name USI_Hash type spin default 256 min 1 max 65536",
        );
        observe_usi_default(
            &mut defaults,
            "option name Other Threads type spin default 99 min 1 max 100",
        );
        assert_eq!(
            defaults,
            EngineDefaults {
                threads: Some(2),
                hash_mb: Some(256),
            }
        );
        assert_eq!(
            parse_usi_spin_default("option name Threads type check default true", "Threads"),
            None
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_resource_probe_reports_current_process_cpu_and_memory() {
        let usage = process_resource_usage(process::id());
        assert!(usage.cpu_time_ns.is_some());
        assert!(usage.peak_rss_bytes.is_some_and(|bytes| bytes > 0));
        assert!(physical_core_count().is_some_and(|cores| cores > 0));
        assert!(physical_memory_bytes().is_some_and(|bytes| bytes > 0));
    }

    fn completed_pair(number: u64, category: usize) -> CompletedPair {
        let game = GameRecord {
            candidate_color: StoredColor::Black,
            candidate_seed: 1,
            baseline_seed: 2,
            wall_time_ns: 1,
            candidate_cpu_time_ns: Some(1),
            baseline_cpu_time_ns: Some(1),
            candidate_peak_rss_bytes: Some(1),
            baseline_peak_rss_bytes: Some(1),
            turns: Vec::new(),
            termination: TerminationRecord::Cutoff,
        };
        CompletedPair {
            number,
            output: String::new(),
            result: PairResult {
                category: Some(category),
                failures: FailureCounts::default(),
            },
            record: PairRecord {
                pair_number: number,
                pair_seed: 1,
                opening: OpeningRecord {
                    seed: 1,
                    moves: Vec::new(),
                },
                games: [game.clone(), game],
                category: Some(u8::try_from(category).unwrap()),
            },
        }
    }

    struct SetAtomicOnDrop(Arc<AtomicBool>);

    impl Drop for SetAtomicOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    // match-harness-efficiency.md「並列枠の維持」: ペア1が遅れても、後続結果を
    // 1件受信するたびに空いた枠へ次のペアを1件だけ補充する。統計への取り込みは
    // ペア番号順を維持する。
    #[test]
    fn out_of_order_completions_replenish_one_slot_each() {
        let mut completed = BTreeMap::new();
        let mut next_to_integrate = 1;
        let mut pending_jobs = VecDeque::from([3, 4, 5]);
        let mut integrated = Vec::new();

        let replacement = accept_completed_pair(
            completed_pair(2, 2),
            &mut completed,
            &mut next_to_integrate,
            &mut pending_jobs,
            |pair| {
                integrated.push(pair.number);
                true
            },
        );
        assert_eq!(replacement, Some(3));
        assert!(integrated.is_empty());

        let replacement = accept_completed_pair(
            completed_pair(3, 2),
            &mut completed,
            &mut next_to_integrate,
            &mut pending_jobs,
            |pair| {
                integrated.push(pair.number);
                true
            },
        );
        assert_eq!(replacement, Some(4));
        assert!(integrated.is_empty());

        let replacement = accept_completed_pair(
            completed_pair(1, 2),
            &mut completed,
            &mut next_to_integrate,
            &mut pending_jobs,
            |pair| {
                integrated.push(pair.number);
                true
            },
        );
        assert_eq!(replacement, Some(5));
        assert_eq!(integrated, [1, 2, 3]);
        assert_eq!(next_to_integrate, 4);
        assert!(completed.is_empty());
    }

    // match-harness-efficiency.md「並列枠の維持」: 実際の2ワーカーでペア1を
    // 停止させても、ペア2の完了を受信した枠にはペア3が投入される。
    #[test]
    fn delayed_head_pair_does_not_leave_a_worker_idle() {
        let release_head = Arc::new(AtomicBool::new(false));
        thread::scope(|scope| {
            let (job_sender, job_receiver) = mpsc::channel::<u64>();
            let job_receiver = Arc::new(Mutex::new(job_receiver));
            let (result_sender, result_receiver) = mpsc::channel::<CompletedPair>();
            let (started_sender, started_receiver) = mpsc::channel::<u64>();
            let mut workers = Vec::new();

            for _ in 0..2 {
                let job_receiver = Arc::clone(&job_receiver);
                let result_sender = result_sender.clone();
                let started_sender = started_sender.clone();
                let release_head = Arc::clone(&release_head);
                workers.push(scope.spawn(move || {
                    loop {
                        let job = job_receiver.lock().unwrap().recv();
                        let Ok(number) = job else {
                            break;
                        };
                        started_sender.send(number).unwrap();
                        if number == 1 {
                            while !release_head.load(Ordering::Acquire) {
                                thread::yield_now();
                            }
                        }
                        result_sender.send(completed_pair(number, 2)).unwrap();
                    }
                }));
            }
            drop(result_sender);
            drop(started_sender);

            job_sender.send(1).unwrap();
            job_sender.send(2).unwrap();
            let _release_on_unwind = SetAtomicOnDrop(Arc::clone(&release_head));
            let mut initially_started = [
                started_receiver
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap(),
                started_receiver
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap(),
            ];
            initially_started.sort_unstable();
            assert_eq!(initially_started, [1, 2]);

            let pair = result_receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            assert_eq!(pair.number, 2);
            let mut completed = BTreeMap::new();
            let mut next_to_integrate = 1;
            let mut pending_jobs = VecDeque::from([3]);
            let replacement = accept_completed_pair(
                pair,
                &mut completed,
                &mut next_to_integrate,
                &mut pending_jobs,
                |_| true,
            );
            job_sender.send(replacement.unwrap()).unwrap();

            assert_eq!(
                started_receiver
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap(),
                3
            );
            assert!(!release_head.load(Ordering::Acquire));
            release_head.store(true, Ordering::Release);
            drop(job_sender);
            for worker in workers {
                worker.join().unwrap();
            }
        });
    }

    // match-harness-efficiency.md「並列枠の維持」: GSPRT境界は停止フラグを
    // 設定し、実行中のワーカーが打ち切った未完了ペアを結果へ送らない。
    #[test]
    fn decision_stop_discards_an_unfinished_worker_result() {
        let stop = Arc::new(AtomicBool::new(false));
        thread::scope(|scope| {
            let (job_sender, job_receiver) = mpsc::channel();
            let job_receiver = Mutex::new(job_receiver);
            let (result_sender, result_receiver) = mpsc::channel();
            let (started_sender, started_receiver) = mpsc::channel();
            let worker_stop = Arc::clone(&stop);
            let worker = scope.spawn(move || {
                run_worker_loop(
                    &job_receiver,
                    &result_sender,
                    &worker_stop,
                    |number, stop| {
                        started_sender.send(number).unwrap();
                        while !stop.load(Ordering::Acquire) {
                            thread::yield_now();
                        }
                        Ok(None)
                    },
                );
            });
            let _stop_on_unwind = SetAtomicOnDrop(Arc::clone(&stop));

            job_sender.send(1).unwrap();
            assert_eq!(
                started_receiver
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap(),
                1
            );
            assert!(!continue_after_decision(
                true,
                GsprtDecision::AcceptH1,
                &stop
            ));
            drop(job_sender);
            worker.join().unwrap();
            assert!(matches!(
                result_receiver.try_recv(),
                Err(mpsc::TryRecvError::Disconnected)
            ));
        });
    }

    // match-harness-efficiency.md「実行記録と再開」: 1手以上進んだ後に応答が
    // 途絶えた局は最終着手の次の手番側の反則負けとして再検証できる。
    #[test]
    fn saved_timeout_after_a_move_is_valid_without_a_failure_turn() {
        let opening = generate_opening(Rules::ENGINE_DEFAULT, derive_seed(7, 1));
        let mut game = opening.game.clone();
        let selected = game.legal_moves()[0];
        let text = usi::text(
            game.position(),
            selected,
            &MoveGenerator::new(game.rules().moves),
        )
        .unwrap();
        let mover = stored_color(game.position().side_to_move());
        assert!(matches!(game.play(selected).unwrap(), GameStatus::Ongoing));
        let loser = stored_color(game.position().side_to_move());
        let record = GameRecord {
            candidate_color: StoredColor::Black,
            candidate_seed: 1,
            baseline_seed: 2,
            wall_time_ns: 1,
            candidate_cpu_time_ns: Some(1),
            baseline_cpu_time_ns: Some(1),
            candidate_peak_rss_bytes: Some(1),
            baseline_peak_rss_bytes: Some(1),
            turns: vec![TurnRecord {
                side: mover,
                think_time_ns: 1,
                evaluation: None,
                stop_reason: None,
                completed_time_ms: None,
                response: TurnResponse::Move { usi: text },
            }],
            termination: TerminationRecord::Forfeit {
                loser,
                reason: FailureKind::Timeout,
            },
        };
        assert!(matches!(
            validate_saved_game(
                &record,
                &opening,
                u32::MAX,
                SavedGameConditions {
                    candidate_protocol: Protocol::Usi,
                    candidate_limit: SearchLimit::Fixed {
                        depth: None,
                        nodes: Some(1),
                    },
                    baseline_protocol: Protocol::Usi,
                    baseline_limit: SearchLimit::Fixed {
                        depth: None,
                        nodes: Some(1),
                    },
                    response_timeout: Duration::from_secs(1),
                },
            )
            .unwrap(),
            PlayedGame::Finished {
                outcome: GameOutcome::Forfeit {
                    reason: EngineFailure::Timeout,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn saved_game_validation_rejects_protocol_clock_and_ply_contradictions() {
        let opening = generate_opening(Rules::ENGINE_DEFAULT, derive_seed(8, 1));
        let side = stored_color(opening.game.position().side_to_move());
        let fixed_conditions = SavedGameConditions {
            candidate_protocol: Protocol::Cecp,
            candidate_limit: SearchLimit::Fixed {
                depth: Some(1),
                nodes: None,
            },
            baseline_protocol: Protocol::Usi,
            baseline_limit: SearchLimit::Fixed {
                depth: Some(1),
                nodes: None,
            },
            response_timeout: Duration::from_secs(1),
        };
        let mut record = GameRecord {
            candidate_color: side,
            candidate_seed: 1,
            baseline_seed: 2,
            wall_time_ns: 1,
            candidate_cpu_time_ns: Some(1),
            baseline_cpu_time_ns: Some(1),
            candidate_peak_rss_bytes: Some(1),
            baseline_peak_rss_bytes: Some(1),
            turns: vec![TurnRecord {
                side,
                think_time_ns: 1,
                evaluation: Some(EvaluationRecord {
                    perspective: side,
                    depth: Some(1),
                    score: ScoreRecord::Cp { value: 0 },
                    bound: ScoreBound::Exact,
                }),
                stop_reason: None,
                completed_time_ms: None,
                response: TurnResponse::Resigned,
            }],
            termination: TerminationRecord::Resigned { loser: side },
        };
        assert!(validate_saved_game(&record, &opening, u32::MAX, fixed_conditions).is_err());

        record.turns[0].evaluation = None;
        record.turns[0].think_time_ns = u64::MAX;
        assert!(validate_saved_game(&record, &opening, u32::MAX, fixed_conditions).is_err());

        record.turns[0].think_time_ns = 1;
        record.turns[0].response = TurnResponse::Failure {
            reason: FailureKind::TimeForfeit,
        };
        record.termination = TerminationRecord::Forfeit {
            loser: side,
            reason: FailureKind::TimeForfeit,
        };
        assert!(validate_saved_game(&record, &opening, u32::MAX, fixed_conditions).is_err());

        record.turns.clear();
        record.termination = TerminationRecord::Forfeit {
            loser: side,
            reason: FailureKind::Timeout,
        };
        assert!(
            validate_saved_game(
                &record,
                &opening,
                opening.game.ply_count(),
                fixed_conditions
            )
            .is_err()
        );
    }

    // match-harness-efficiency.md「並列枠の維持」: 番号順の取り込みでGSPRT境界へ
    // 到達した場合は、同じ受信によって空いた枠へ新しいペアを投入しない。
    #[test]
    fn decision_boundary_prevents_replenishment() {
        let mut completed = BTreeMap::new();
        let mut next_to_integrate = 1;
        let mut pending_jobs = (2..=10).collect::<VecDeque<_>>();
        let mut integrated = Vec::new();

        let replacement = accept_completed_pair(
            completed_pair(1, 4),
            &mut completed,
            &mut next_to_integrate,
            &mut pending_jobs,
            |pair| {
                integrated.push(pair.number);
                false
            },
        );
        assert_eq!(replacement, None);
        assert_eq!(integrated, [1]);
        assert_eq!(pending_jobs.front(), Some(&2));
    }

    // match-harness-efficiency.md「並列枠の維持」: 同じ固定結果列は、完了順が
    // 入れ替わってもペンタノミアル度数、LLR、判定、停止ペア番号が一致する。
    #[test]
    fn completion_order_does_not_change_gsprt_stopping_result() {
        fn integrate_arrivals(
            arrivals: impl IntoIterator<Item = u64>,
        ) -> ([u64; 5], f64, GsprtDecision, u64) {
            let mut completed = BTreeMap::new();
            let mut next_to_integrate = 1;
            let mut pending_jobs = VecDeque::new();
            let mut results = [0_u64; 5];
            let mut decision = GsprtDecision::Continue;
            let mut stop_pair = 0;

            for number in arrivals {
                if decision != GsprtDecision::Continue {
                    break;
                }
                let replacement = accept_completed_pair(
                    completed_pair(number, 4),
                    &mut completed,
                    &mut next_to_integrate,
                    &mut pending_jobs,
                    |pair| {
                        let category = pair
                            .result
                            .category
                            .expect("the synthetic result must be valid");
                        results[category] += 1;
                        stop_pair = pair.number;
                        decision = gsprt_decision(gsprt_llr(&results));
                        decision == GsprtDecision::Continue
                    },
                );
                assert_eq!(replacement, None);
            }

            assert_ne!(decision, GsprtDecision::Continue);
            (results, gsprt_llr(&results), decision, stop_pair)
        }

        let sequential = integrate_arrivals(1..=1_000);
        let delayed_head = integrate_arrivals((2..=1_000).chain(std::iter::once(1)));
        assert_eq!(sequential.0, delayed_head.0);
        assert_eq!(sequential.1, delayed_head.1);
        assert_eq!(sequential.2, delayed_head.2);
        assert_eq!(sequential.3, delayed_head.3);
    }

    // D8-HARN-08/D8-STAT-06(sprt.md): 文書化された既定値
    // (--response-timeout 120秒、--max-ply 4096、--max-pairs 100,000)を
    // 文書の明文値リテラルで固定する。文書が変わらない限り実装定数の変更は
    // 逸脱である。
    #[test]
    fn documented_defaults_match_sprt_md() {
        let arguments = Arguments::try_parse_from(["match_runner", "--run-dir", "run", "gsprt"])
            .expect("the documented default invocation must be accepted");
        assert_eq!(arguments.response_timeout, 120);
        assert_eq!(arguments.max_ply, 4096);
        assert_eq!(arguments.concurrency, None);
        assert!(matches!(arguments.mode, Mode::Gsprt { max_pairs: 100_000 }));
    }

    #[test]
    fn explicit_concurrency_remains_a_positive_override() {
        let arguments = Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "run",
            "--concurrency",
            "4",
            "gsprt",
        ])
        .unwrap();
        assert_eq!(arguments.concurrency, Some(4));
        assert!(
            Arguments::try_parse_from([
                "match_runner",
                "--run-dir",
                "run",
                "--concurrency",
                "0",
                "gsprt",
            ])
            .is_err()
        );
    }

    // match-harness-efficiency.md「実行記録と再開」: 記録なし実行を許さず、
    // 新規作成と再開を同時に指定させない。
    #[test]
    fn run_directory_operation_is_required_and_exclusive() {
        assert!(Arguments::try_parse_from(["match_runner", "gsprt"]).is_err());
        assert!(
            Arguments::try_parse_from([
                "match_runner",
                "--run-dir",
                "new",
                "--resume",
                "old",
                "gsprt",
            ])
            .is_err()
        );
        assert!(Arguments::try_parse_from(["match_runner", "--resume", "old", "gsprt"]).is_ok());
    }

    // D8-HARN-01（sprt.md「エンジンの指定方法」、match-harness.md「エンジン指定と
    // 解決」）: specは`commit:<hash>`・USI起動コマンド・`random`・CECP起動コマンドの
    // 4形式であり、起動コマンドの2語目以降は引数になる。
    #[test]
    fn engine_specs_cover_the_documented_four_forms() {
        let random = parse_player_spec("random").expect("the reserved spec must be accepted");
        assert_eq!(random.kind, PlayerKind::Random);

        let commit = parse_player_spec("commit:0045833")
            .expect("a commit spec must be accepted without resolving it");
        assert_eq!(commit.kind, PlayerKind::Commit("0045833".to_owned()));

        // 起動コマンド形式: 2語目以降は起動引数として渡す
        let command = parse_player_spec("target/release/minase --protocol usi --rules R1")
            .expect("a command line spec must be accepted");
        assert_eq!(
            command.kind,
            PlayerKind::Command {
                program: PathBuf::from("target/release/minase"),
                args: vec![
                    "--protocol".to_owned(),
                    "usi".to_owned(),
                    "--rules".to_owned(),
                    "R1".to_owned(),
                ],
            }
        );

        let hachu = parse_player_spec("cecp:../hachu-debian/hachu")
            .expect("a CECP command without arguments must be accepted");
        assert_eq!(hachu.text, "cecp:../hachu-debian/hachu");
        assert_eq!(
            hachu.kind,
            PlayerKind::Cecp {
                program: PathBuf::from("../hachu-debian/hachu"),
                args: Vec::new(),
            }
        );
        let minase =
            parse_player_spec("cecp:target/release/minase --protocol cecp --rules engine-default")
                .expect("a CECP command with arguments must be accepted");
        assert_eq!(
            minase.kind,
            PlayerKind::Cecp {
                program: PathBuf::from("target/release/minase"),
                args: vec![
                    "--protocol".to_owned(),
                    "cecp".to_owned(),
                    "--rules".to_owned(),
                    "engine-default".to_owned(),
                ],
            }
        );

        // 4形式に含まれない入力は拒否される。旧`depth=N` specの削除は
        // match-harness.md適用範囲に明文がある。空リビジョンとcommit形式への
        // 起動引数付与の拒否は[実装契約](SPEC_UNCLEAR-05関連)。
        for invalid in [
            "",
            "depth=1",
            "depth=2,nodes=1000",
            "commit:",
            "commit:abc --x",
            "cecp:",
            "cecp:   ",
        ] {
            assert!(
                parse_player_spec(invalid).is_err(),
                "spec {invalid:?} must be rejected"
            );
        }
    }

    // D8-HARN-01境界（sprt.md「エンジンの指定方法」、match-harness.md「CECP
    // セッション管理」）: CECPに写せないnodes、秒読み、秒未満の時間単位は解決時に
    // InvalidInputとして拒否する。
    #[test]
    fn cecp_resolution_rejects_unsupported_search_limits() {
        for limit in [
            parse_search_limit("nodes=1").unwrap(),
            parse_search_limit("depth=1,nodes=1").unwrap(),
            parse_search_limit("time=1000+0,byoyomi=1000").unwrap(),
            parse_search_limit("time=1500+0").unwrap(),
            parse_search_limit("time=1000+1500").unwrap(),
        ] {
            let spec = parse_player_spec("cecp:engine").unwrap();
            let error = resolve_player(spec, limit, "R1")
                .err()
                .expect("the unsupported CECP limit must fail resolution");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert!(!error.to_string().is_empty());
        }
    }

    // D8-HARN-01/20（sprt.md「エンジンの指定方法」、match-harness.md「CECP
    // セッション管理」）: 対応する深さ固定と秒単位時間制御はCECP設定へ解決できる。
    #[test]
    fn cecp_resolution_accepts_supported_limits_and_marks_the_protocol() {
        let depth = resolve_player(
            parse_player_spec("cecp:engine --option value").unwrap(),
            parse_search_limit("depth=4").unwrap(),
            "R1",
        )
        .unwrap();
        assert_eq!(depth.protocol, Protocol::Cecp);
        assert_eq!(depth.path, PathBuf::from("engine"));
        assert_eq!(depth.args, ["--option", "value"]);
        assert_eq!(
            probe_engine_defaults(&depth, Duration::from_secs(1)).unwrap(),
            EngineDefaults {
                threads: None,
                hash_mb: Some(256),
            }
        );

        let time = resolve_player(
            parse_player_spec("cecp:engine").unwrap(),
            parse_search_limit("time=61000+2000").unwrap(),
            "R1",
        )
        .unwrap();
        assert_eq!(time.protocol, Protocol::Cecp);
    }

    // D8-HARN-20（match-harness.md「CECPセッション管理」）: `level`の持ち時間は
    // 60秒の倍数なら分だけ、それ以外は分:秒の2桁表記になり、加算は秒で送る。
    #[test]
    fn cecp_level_formats_minutes_seconds_and_increment() {
        assert_eq!(
            cecp_level_text(TimeControl {
                base_ms: 60_000,
                increment_ms: 1_000,
                byoyomi_ms: 0,
            }),
            "level 0 1 1"
        );
        assert_eq!(
            cecp_level_text(TimeControl {
                base_ms: 61_000,
                increment_ms: 2_000,
                byoyomi_ms: 0,
            }),
            "level 0 1:01 2"
        );
    }

    // SPEC_UNCLEAR-05 [実装契約]: 不明リビジョンの解決は対局実行前に失敗する。
    // 診断文言は契約ではないため種別(Err)だけを検証する。
    #[test]
    fn unknown_commit_revision_fails_resolution() {
        assert!(
            normalize_commit(
                Path::new(env!("CARGO_MANIFEST_DIR")),
                "definitely-not-a-minase-commit",
            )
            .is_err()
        );
    }

    // D8-HARN-09(sprt.md・search.md実施状況): 思考制限は
    // `depth=N|nodes=M|time=<base_ms>+<inc_ms>[,byoyomi=<ms>]`。
    // depthとnodesの併記受理はSPEC_UNCLEAR-08につき[実装契約]。
    #[test]
    fn search_limits_accept_the_documented_grammar() {
        assert_eq!(
            parse_search_limit("depth=4"),
            Ok(SearchLimit::Fixed {
                depth: Some(4),
                nodes: None
            })
        );
        assert_eq!(
            parse_search_limit("nodes=100000"),
            Ok(SearchLimit::Fixed {
                depth: None,
                nodes: Some(100_000)
            })
        );
        assert_eq!(
            parse_search_limit("time=10000+100"),
            Ok(SearchLimit::Time(TimeControl {
                base_ms: 10_000,
                increment_ms: 100,
                byoyomi_ms: 0,
            }))
        );
        assert_eq!(
            parse_search_limit("time=0+0,byoyomi=1000"),
            Ok(SearchLimit::Time(TimeControl {
                base_ms: 0,
                increment_ms: 0,
                byoyomi_ms: 1_000,
            }))
        );
        // [実装契約] 併記形
        assert_eq!(
            parse_search_limit("depth=3,nodes=400"),
            Ok(SearchLimit::Fixed {
                depth: Some(3),
                nodes: Some(400)
            })
        );
    }

    // SPEC_UNCLEAR-08 [実装契約]: 文法に合致しない制限は拒否される。
    // エラー文言は契約ではない。
    #[test]
    fn search_limits_reject_malformed_inputs() {
        for invalid in [
            "",
            "byoyomi=1000",
            "time=1000",
            "time=1000+10+5",
            "time=1000+10,depth=1",
            "depth=1,time=1000+10",
            "nodes=1,depth=1",
            "depth=0",
            "nodes=0",
            "depth=1,nodes=2,x=3",
        ] {
            assert!(
                parse_search_limit(invalid).is_err(),
                "limit {invalid:?} must be rejected"
            );
        }
    }

    // D8-HARN-09(sprt.md測定の種類と標準コマンド節): `--each`がマッチ共通既定、
    // `--candidate-limit`・`--baseline-limit`が当該エンジンだけを上書きする。
    // 対等条件と完了基準ゲートの非対称条件を同じCLIで表現できる。
    #[test]
    fn each_limit_is_shared_and_per_engine_overrides_are_optional() {
        let equal = Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "run",
            "--each",
            "depth=4",
            "gsprt",
        ])
        .expect("the equal-condition invocation must be accepted");
        assert_eq!(
            equal.each,
            SearchLimit::Fixed {
                depth: Some(4),
                nodes: None
            }
        );
        assert_eq!(equal.candidate_limit, None);
        assert_eq!(equal.baseline_limit, None);

        // 完了基準ゲートの形: ベースライン側だけdepth=1へ上書き
        let gate = Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "run",
            "--each",
            "depth=4",
            "--baseline-limit",
            "depth=1",
            "gsprt",
        ])
        .expect("the asymmetric gate invocation must be accepted");
        assert_eq!(
            gate.baseline_limit,
            Some(SearchLimit::Fixed {
                depth: Some(1),
                nodes: None
            })
        );
        assert_eq!(gate.candidate_limit, None);

        // 等価表現: `--each X`と`--candidate-limit X --baseline-limit X`は
        // 同一の制限値を与える
        let explicit = Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "run",
            "--candidate-limit",
            "depth=4",
            "--baseline-limit",
            "depth=4",
            "gsprt",
        ])
        .expect("explicit overrides must be accepted");
        assert_eq!(explicit.candidate_limit, Some(equal.each));
        assert_eq!(explicit.baseline_limit, Some(equal.each));
    }

    // D8-HARN-10(search.md実施状況): 実測思考時間が`残り時間 + byoyomi`を
    // 「超えた」場合だけ時間切れ失権となる。同値は超過ではない。
    #[test]
    fn time_forfeit_requires_exceeding_remaining_plus_byoyomi() {
        let mut clock = Clock::new(TimeControl {
            base_ms: 1_000,
            increment_ms: 0,
            byoyomi_ms: 50,
        });
        // ちょうど残り+秒読みの消費は失権ではない
        assert_eq!(clock.update(Duration::from_millis(1_050)), Ok(()));
        // 残りは消費に応じて減少する(現在値契約。加算時間0で会計の曖昧さを避ける)
        assert_eq!(clock.remaining_ms(), 0);
        assert_eq!(clock.update(Duration::from_millis(50)), Ok(()));
        assert_eq!(
            clock.update(Duration::from_millis(51)),
            Err(EngineFailure::TimeForfeit)
        );
    }

    #[test]
    fn clock_update_adds_the_increment_after_each_move() {
        // sprt.md時間制御対局のフィッシャー式加算の会計: base 1000ms・加算100msで
        // 250ms消費すると残りは1000-250+100=850msになる。変異検証(フェーズ4)で
        // 検出した加算会計の無検証を補強する。
        let mut clock = Clock::new(TimeControl {
            base_ms: 1_000,
            increment_ms: 100,
            byoyomi_ms: 0,
        });
        assert_eq!(clock.update(Duration::from_millis(250)), Ok(()));
        assert_eq!(clock.remaining_ms(), 850);
    }

    // D8-HARN-10(search.md実施状況＋sprt.mdペア対局): 毎手のgoは両者の時計の
    // 現在値をbtime/wtime/binc/winc/byoyomiで送る。ペア内の先後入替で同一
    // エンジンの時計がbtime側とwtime側を交差する。
    #[test]
    fn go_time_arguments_reflect_current_clocks_and_cross_colors() {
        let player_a = SearchLimit::Time(TimeControl {
            base_ms: 10_000,
            increment_ms: 100,
            byoyomi_ms: 1_000,
        });
        let player_b = SearchLimit::Time(TimeControl {
            base_ms: 20_000,
            increment_ms: 200,
            byoyomi_ms: 2_000,
        });
        // プレイヤーAが後手の局では、Aの時計がwtime/winc側に載る
        let clocks = GameClocks::new(Color::White, player_a, player_b);
        assert_eq!(
            clocks.go_text(Color::Black),
            "btime 20000 wtime 10000 binc 200 winc 100 byoyomi 2000"
        );
        // byoyomiは手番側の値を送る
        assert_eq!(
            clocks.go_text(Color::White),
            "btime 20000 wtime 10000 binc 200 winc 100 byoyomi 1000"
        );

        // 現在値契約: 消費後のgoは減少した残り時間を反映する(加算時間0)
        let simple = SearchLimit::Time(TimeControl {
            base_ms: 10_000,
            increment_ms: 0,
            byoyomi_ms: 0,
        });
        let mut clocks = GameClocks::new(Color::Black, simple, simple);
        clocks
            .get_mut(Color::Black)
            .expect("a time-controlled player must have a clock")
            .update(Duration::from_millis(250))
            .expect("a small consumption must not forfeit");
        assert_eq!(
            clocks.go_text(Color::White),
            "btime 9750 wtime 10000 binc 0 winc 0 byoyomi 0"
        );

        // 固定制限側のgo引数(sprt.mdの`--each depth=4`等に対応)
        assert_eq!(
            SearchLimit::Fixed {
                depth: Some(3),
                nodes: Some(400),
            }
            .fixed_go_text(),
            Some("depth 3 nodes 400".to_owned())
        );

        // D8-HARN-20（match-harness.md「CECPセッション管理」）: CECPへは固定制限で
        // 十分大きい固定時計を、時間制御で現在値を10 ms単位へ切り捨てて送る。
        let fixed = clocks.think_request(
            Color::Black,
            SearchLimit::Fixed {
                depth: Some(1),
                nodes: None,
            },
        );
        assert_eq!(fixed.own_cs, 3_000_000);
        assert_eq!(fixed.opponent_cs, 3_000_000);
        let timed = clocks.think_request(Color::Black, simple);
        assert_eq!(timed.own_cs, 975);
        assert_eq!(timed.opponent_cs, 1_000);
    }

    // D8-HARN-12(RULES.md第33条): `engine-default`はR1、`lishogi`は
    // L1+L2+P3+R1+E1+E3へ解決される。照合は大文字小文字を区別せず、
    // 規則コードとの併記とR0の指定は拒否される。`--rules`省略時の既定R1は
    // search.md自己対局既定へ接地する(SPEC_UNCLEAR-10の文書補修待ち)。
    #[test]
    fn rules_presets_resolve_per_article_33() {
        let default = Arguments::try_parse_from(["match_runner", "--run-dir", "run", "gsprt"])
            .expect("omitting --rules must fall back to engine-default");
        assert_eq!(default.rules.source, "engine-default");
        assert_eq!(
            default.rules.codes,
            Vec::<RuleCode>::from(Rules::ENGINE_DEFAULT)
        );

        let named = Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "run",
            "--rules",
            "engine-default",
            "gsprt",
        ])
        .expect("the engine-default preset must be accepted");
        assert_eq!(named.rules.source, "engine-default");
        assert_eq!(
            named.rules.codes,
            Vec::<RuleCode>::from(Rules::ENGINE_DEFAULT)
        );

        let lishogi = Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "run",
            "--rules",
            "LISHOGI",
            "gsprt",
        ])
        .expect("preset names must match case-insensitively");
        assert_eq!(
            lishogi.rules.codes,
            [
                RuleCode::L1,
                RuleCode::L2,
                RuleCode::P0,
                RuleCode::P3,
                RuleCode::R1,
                RuleCode::E1,
                RuleCode::E3,
            ]
        );

        for invalid in ["lishogi,P1", "engine-default,lishogi", "R0", "R0,R1"] {
            assert!(
                Arguments::try_parse_from([
                    "match_runner",
                    "--run-dir",
                    "run",
                    "--rules",
                    invalid,
                    "gsprt",
                ])
                .is_err(),
                "rules {invalid:?} must be rejected"
            );
        }
    }

    // D8-HARN-06(1)/D8-HARN-11(sprt.md異常時の裁定節・match-harness.md): 審判層が
    // 対局進行の正であり、合法手リストにない`bestmove`は不正着手として分類する。
    #[test]
    fn referee_rejects_bestmove_outside_the_legal_move_list() {
        let game = Game::new(Rules::ENGINE_DEFAULT);
        let legal = game.legal_moves()[0];
        let legal_text = usi::text(
            game.position(),
            legal,
            &MoveGenerator::new(game.rules().moves),
        )
        .unwrap();
        assert_eq!(
            validate_bestmove(&game, &legal_text, Protocol::Usi),
            Ok(legal)
        );
        // 表記として解釈できない応答
        assert_eq!(
            validate_bestmove(&game, "not-a-move", Protocol::Usi),
            Err(EngineFailure::IllegalMove)
        );
        // 表記としては読めるが初期局面では指せない着手(空升からの移動)
        assert_eq!(
            validate_bestmove(&game, "6f6g", Protocol::Usi),
            Err(EngineFailure::IllegalMove)
        );
    }

    // D8-HARN-20（match-harness.md「CECPセッション管理」、hachu.md第8節）:
    // `@@@@`は合法手中の正準じっとから移動元の内部升番号が最小の手へ一意に割り当てる。
    #[test]
    fn cecp_null_move_selects_the_lowest_origin_canonical_jitto() {
        let low = Move {
            from: Square::new(2, 3).unwrap(),
            mid: None,
            to: Square::new(2, 3).unwrap(),
            promote: false,
        };
        let high = Move {
            from: Square::new(9, 8).unwrap(),
            mid: None,
            to: Square::new(9, 8).unwrap(),
            promote: false,
        };
        let ordinary = Move {
            from: Square::new(0, 0).unwrap(),
            mid: None,
            to: Square::new(0, 1).unwrap(),
            promote: false,
        };
        assert_eq!(canonical_jitto(&[high, ordinary, low]), Some(low));
        assert_eq!(canonical_jitto(&[ordinary]), None);
    }

    // D8-HARN-06(2)(3)(sprt.md異常時の裁定節): プロセス終了・パイプ切断は
    // クラッシュ、応答期限超過は応答タイムアウトとして分類する。
    #[test]
    fn response_channel_classifies_disconnect_and_timeout() {
        let (sender, lines) = mpsc::channel();
        drop(sender);
        assert_eq!(
            receive_until(&lines, Duration::from_secs(1), |_| false),
            Err(EngineFailure::Crash)
        );

        let (_sender, lines) = mpsc::channel();
        assert_eq!(
            receive_until(&lines, Duration::from_millis(1), |_| false),
            Err(EngineFailure::Timeout)
        );
    }

    // D8-HARN-11(match-harness.md USIセッション管理節): ハーネスは`bestmove`を
    // 受けて着手を適用する。それ以外の行(infoなど)は応答待ちで読み飛ばす。
    #[test]
    fn response_channel_waits_for_bestmove_ignoring_other_lines() {
        let (sender, lines) = mpsc::channel();
        sender
            .send(Ok("info depth 1 score cp 0".to_owned()))
            .expect("the receiver must be alive");
        sender
            .send(Ok("bestmove 1a1b".to_owned()))
            .expect("the receiver must be alive");
        assert_eq!(
            receive_until(&lines, Duration::from_secs(1), |line| {
                line.split_whitespace().next() == Some("bestmove")
            }),
            Ok("bestmove 1a1b".to_owned())
        );
    }

    // match-harness-efficiency.md「実行記録と再開」「早期投了の検証」:
    // bestmove前の最後のscore付きinfoを保存し、後続の非評価infoでは消去しない。
    #[test]
    fn usi_evaluation_uses_the_last_score_bearing_info_line() {
        assert_eq!(
            parse_usi_evaluation("info depth 1 score cp 12"),
            Ok(Some(EngineEvaluation {
                depth: Some(1),
                score: EngineScore::Cp(12),
                bound: ScoreBound::Exact,
            }))
        );
        assert_eq!(
            parse_usi_evaluation("info score mate -3 depth 5"),
            Ok(Some(EngineEvaluation {
                depth: Some(5),
                score: EngineScore::MatedIn(Some(3)),
                bound: ScoreBound::Exact,
            }))
        );
        assert_eq!(
            parse_usi_evaluation("info score mate +"),
            Ok(Some(EngineEvaluation {
                depth: None,
                score: EngineScore::MateIn(None),
                bound: ScoreBound::Exact,
            }))
        );
        assert_eq!(
            parse_usi_evaluation("info score mate -"),
            Ok(Some(EngineEvaluation {
                depth: None,
                score: EngineScore::MatedIn(None),
                bound: ScoreBound::Exact,
            }))
        );
        assert_eq!(parse_usi_evaluation("info string searching"), Ok(None));
        assert_eq!(
            parse_usi_evaluation("info score cp broken"),
            Err(EngineFailure::Crash)
        );
        let mut observed = None;
        observe_usi_evaluation(&mut observed, "info score cp broken");
        assert_eq!(observed, None);
        observe_usi_evaluation(&mut observed, "info score mate +");
        assert_eq!(observed.unwrap().score, EngineScore::MateIn(None));
    }

    #[test]
    fn usi_search_data_uses_the_last_time_and_known_stop_reason() {
        let mut stop_reason = None;
        let mut completed_time_ms = None;
        observe_usi_search_data(
            &mut stop_reason,
            &mut completed_time_ms,
            "info depth 3 time 12 score cp 0",
        )
        .unwrap();
        observe_usi_search_data(
            &mut stop_reason,
            &mut completed_time_ms,
            "info depth 4 score cp 1 time 34",
        )
        .unwrap();
        observe_usi_search_data(
            &mut stop_reason,
            &mut completed_time_ms,
            "info string stop hard",
        )
        .unwrap();
        assert_eq!(stop_reason, Some(StopReasonRecord::Hard));
        assert_eq!(completed_time_ms, Some(34));

        for (word, expected) in [
            ("depth", StopReasonRecord::Depth),
            ("nodes", StopReasonRecord::Nodes),
            ("soft", StopReasonRecord::Soft),
            ("hard", StopReasonRecord::Hard),
            ("external", StopReasonRecord::External),
        ] {
            assert_eq!(
                parse_usi_stop_reason(&format!("info string stop {word}")),
                Ok(Some(expected))
            );
        }
    }

    #[test]
    fn invalid_usi_search_data_is_a_crash() {
        assert_eq!(
            parse_usi_stop_reason("info string stop unknown"),
            Err(EngineFailure::Crash)
        );
        assert_eq!(
            parse_usi_stop_reason("info string stop"),
            Err(EngineFailure::Crash)
        );
        assert_eq!(
            parse_usi_time("info depth 1 time invalid"),
            Err(EngineFailure::Crash)
        );
        assert_eq!(parse_usi_time("info string time invalid"), Ok(None));
    }

    // D8-HARN-06/20（match-harness.md「CECPセッション管理」「異常時裁定」）:
    // CECP応答はpongまで読み切り、分割レグを連結し、拒否を反則へ、結果行だけを投了へ
    // 分類する。着手後の結果行はHaChuの王駒捕獲出力なので着手を優先する。
    #[test]
    fn cecp_response_lines_are_interpreted_through_pong() {
        fn receive(lines_to_send: &[&str]) -> Result<EngineResponse, EngineFailure> {
            let (sender, lines) = mpsc::channel();
            for &line in lines_to_send {
                sender.send(Ok(line.to_owned())).unwrap();
            }
            sender.send(Ok("pong 12".to_owned())).unwrap();
            receive_cecp_response(&lines, Duration::from_secs(1), 12, Instant::now())
                .map(|(response, _)| response)
        }

        assert_eq!(
            receive(&["# debug", "move e7d8,", "move d8d7"]),
            Ok(EngineResponse::Move("e7d8,d8d7".to_owned()))
        );
        assert_eq!(
            receive(&["Illegal move (repetition): e7d8"]),
            Err(EngineFailure::RejectedMove)
        );
        assert_eq!(receive(&["0-1 {resign}"]), Ok(EngineResponse::Resigned));
        assert_eq!(
            receive(&["move c9i3+", "1-0 {royal capture}"]),
            Ok(EngineResponse::Move("c9i3+".to_owned()))
        );
    }

    // D8-HARN-20（match-harness.md「CECPセッション管理」）: 応答なしのpongは
    // プロトコル違反であり、クラッシュ相当に分類する。
    #[test]
    fn cecp_pong_without_a_move_or_result_is_a_crash() {
        let (sender, lines) = mpsc::channel();
        sender.send(Ok("pong 3".to_owned())).unwrap();
        assert_eq!(
            receive_cecp_response(&lines, Duration::from_secs(1), 3, Instant::now())
                .map(|(response, _)| response),
            Err(EngineFailure::Crash)
        );
    }

    // D8-HARN-14（sprt.md「異常時の裁定」）: `engine_failures:`は不正着手・
    // クラッシュ・応答タイムアウト・時間切れ・着手拒否の理由別件数を報告する。
    // 理由別件数の合計は反則負けとして算入された局数と一致する(保存則)。
    #[test]
    fn failure_reasons_are_counted_separately_and_conserved() {
        let mut counts = FailureCounts::default();
        counts.record(EngineFailure::IllegalMove);
        counts.record(EngineFailure::Crash);
        counts.record(EngineFailure::Timeout);
        counts.record(EngineFailure::TimeForfeit);
        counts.record(EngineFailure::RejectedMove);
        assert_eq!(
            counts,
            FailureCounts {
                illegal_moves: 1,
                crashes: 1,
                timeouts: 1,
                time_forfeits: 1,
                rejected_moves: 1,
            }
        );
        // 集計の合成でも件数は保存される
        let mut total = FailureCounts::default();
        total.add(counts);
        total.add(counts);
        assert_eq!(
            total.illegal_moves
                + total.crashes
                + total.timeouts
                + total.time_forfeits
                + total.rejected_moves,
            10
        );
    }

    // D8-STAT-04/D8-HARN-06(sprt.md統計的手続き節): 観測単位はペアであり、
    // 候補側ペア得点合計{0, 0.5, 1, 1.5, 2}の5分類で集計する。反則負けは
    // 当該局の敗北として算入される(ペア破棄ではない)。
    #[test]
    fn game_outcomes_map_to_candidate_pair_score_categories() {
        let win_black = GameOutcome::Adjudicated(GameResult::Win {
            winner: Color::Black,
            reason: WinReason::RoyalCapture,
        });
        let win_white = GameOutcome::Adjudicated(GameResult::Win {
            winner: Color::White,
            reason: WinReason::RoyalCapture,
        });
        let draw = GameOutcome::Adjudicated(GameResult::Draw {
            reason: DrawReason::Repetition,
        });
        let forfeit_win_black = GameOutcome::Forfeit {
            winner: Color::Black,
            reason: EngineFailure::Crash,
        };
        let resignation_win_black = GameOutcome::Resigned {
            winner: Color::Black,
        };

        // 1局の得点は半点単位: 勝ち2、引き分け1、負け0(候補の色に依存)
        assert_eq!(half_points(win_black, Color::Black), 2);
        assert_eq!(half_points(win_black, Color::White), 0);
        assert_eq!(half_points(draw, Color::Black), 1);
        assert_eq!(half_points(draw, Color::White), 1);
        // 反則負けも通常の勝敗として得点化される
        assert_eq!(half_points(forfeit_win_black, Color::Black), 2);
        assert_eq!(half_points(forfeit_win_black, Color::White), 0);
        assert_eq!(half_points(resignation_win_black, Color::Black), 2);
        assert_eq!(half_points(resignation_win_black, Color::White), 0);

        // D8-HARN-06（match-harness.md「異常時裁定」）: 投了は通常の敗北であり、
        // engine_failuresには算入しない。
        let resigned = PlayedGame::Finished {
            plies: 12,
            outcome: resignation_win_black,
        };
        assert_eq!(
            played_game_text(resigned),
            "plies=12 result=win winner=Black reason=Resigned"
        );
        let mut resignation_failures = FailureCounts::default();
        record_game_failure(resigned, &mut resignation_failures);
        assert_eq!(resignation_failures, FailureCounts::default());

        // ペア分類 = 第1局(候補が先手) + 第2局(候補が後手)の半点合計
        // 2局とも勝ち → 得点2.0のセル4
        assert_eq!(
            half_points(win_black, Color::Black) + half_points(win_white, Color::White),
            4
        );
        // 1勝1敗 → 得点1.0のセル2
        assert_eq!(
            half_points(win_black, Color::Black) + half_points(win_black, Color::White),
            2
        );
        // 候補が2局とも反則負け → 得点0のセル0(相手のペア得点2相当)
        assert_eq!(
            half_points(forfeit_win_black, Color::White)
                + half_points(
                    GameOutcome::Forfeit {
                        winner: Color::White,
                        reason: EngineFailure::Timeout,
                    },
                    Color::Black
                ),
            0
        );
    }

    // D8-HARN-13(sprt.md測定の種類と標準コマンド節): 判定の表示語彙は
    // `decision: H1`(採用)・`decision: H0`(不採用)・`decision: pending`(保留)。
    #[test]
    fn decision_labels_match_the_sprt_md_vocabulary() {
        assert_eq!(decision_text(GsprtDecision::AcceptH1), "H1");
        assert_eq!(decision_text(GsprtDecision::AcceptH0), "H0");
        assert_eq!(decision_text(GsprtDecision::Continue), "pending");
    }

    // D8-HARN-03(search.md自己対局ハーネス節・random-play.mdシード派生節):
    // ペアシードは基本シードとペア番号から決定的に派生し、0にならず、
    // ペア番号間で相異なる(ペア間独立の前提)。厳密な合成式は
    // SPEC_UNCLEAR-06につき固定しない。
    #[test]
    fn pair_seed_derivation_is_deterministic_nonzero_and_distinct() {
        let base = 0xACE1_u64;
        let seeds: Vec<NonZeroU64> = (1..=100).map(|n| derive_seed(base, n)).collect();
        let replay: Vec<NonZeroU64> = (1..=100).map(|n| derive_seed(base, n)).collect();
        assert_eq!(seeds, replay);
        let mut unique = seeds.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), seeds.len());
        // 派生値が0になる入力でも非ゼロへ置換される(random-play.mdの
        // 仕様式の逆算により、splitmix64の出力0の原像は0x61C8_8646_80B5_83EB)
        assert_eq!(
            derive_seed(0x61C8_8646_80B5_83EB - 5, 5).get(),
            0x9E37_79B9_7F4A_7C15
        );
    }

    // D8-HARN-02(sprt.mdペア対局と再現性節): 開始局面は初期局面から8〜12手
    // だけランダムに進めて作り、ペアシードから決定的に再現される。
    #[test]
    fn openings_stay_within_8_to_12_plies_and_derive_deterministically() {
        let rules = Rules::ENGINE_DEFAULT;
        let base_seed = 20_260_814_u64;
        let mut all_moves = Vec::new();
        for pair_number in 1..=4 {
            let pair_seed = derive_seed(base_seed, pair_number);
            let opening = generate_opening(rules, pair_seed);
            assert!(
                (8..=12).contains(&opening.moves.len()),
                "opening length {} is outside the documented 8..=12 range",
                opening.moves.len()
            );
            // エンジンへ送るUSI表記列は開始手順と同数
            assert_eq!(opening.usi_moves.len(), opening.moves.len());
            let replay = generate_opening(rules, pair_seed);
            assert_eq!(replay.moves, opening.moves);
            assert_eq!(replay.usi_moves, opening.usi_moves);
            all_moves.push(opening.moves);
        }
        // ペア間独立: 異なるペア番号がすべて同じ開始手順なら派生が退化している
        assert!(all_moves.windows(2).any(|pair| pair[0] != pair[1]));
    }
}
