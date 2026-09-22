//! 外部エンジンの解決、通信、および先後入替ペアの対局実行。

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::{Path, PathBuf};
use std::process::{self, Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::notation::{cecp, usi};
use crate::rng::{XorShift64, derive_seed};
use crate::search::MAX_PLY;
use crate::{Color, Game, GameResult, GameStatus, Move, MoveGenerator, Rules, Square};
use fs2::FileExt;
#[cfg(target_os = "linux")]
use procfs::process::Process;
use sha2::{Digest, Sha256};

mod environment;
pub use environment::*;
mod storage;
pub use storage::*;
mod records;
pub use records::*;

/// CECPエンジンへ割り当てる置換表容量の既定値。HaChuは`memory`受信前の`go`で
/// 異常終了するため明示し、256 MBはminaseの`USI_Hash`既定値に合わせる。
const CECP_MEMORY_MB: u64 = 256;
/// 固定制限時にCECPエンジンへ通知する時計残量。時計残量0ではHaChuが
/// 反復深化を即座に打ち切り、HaChuは5倍した残量を32ビット整数で計算するため、
/// 十分大きくかつその範囲に収まる3,000,000センチ秒とする。
const CECP_FIXED_TIME_CS: u64 = 3_000_000;

/// 外部エンジンの指定。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlayerSpec {
    /// 入力された指定の原文。表示に使う。
    pub text: String,
    /// 指定の解釈結果。
    pub kind: PlayerKind,
}

/// ランダムエンジンまたは実行ファイルの起動コマンドを表す。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PlayerKind {
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
pub enum Protocol {
    /// USIプロトコル。
    Usi,
    /// CECP（XBoard）プロトコル。
    Cecp,
}

/// USIの`go`へ渡す思考制限。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SearchLimit {
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
pub struct TimeControl {
    /// 持ち時間(ms)。
    pub base_ms: u64,
    /// 1手ごとの加算時間(ms)。
    pub increment_ms: u64,
    /// 1手ごとの秒読み(ms)。
    pub byoyomi_ms: u64,
}

impl SearchLimit {
    /// CLI表示用の制限文字列を返す。
    pub fn cli_text(self) -> String {
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
pub struct Clock {
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
    pub fn update(&mut self, elapsed: Duration) -> Result<(), EngineFailure> {
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

    /// CECPへ送る残り時間と秒読みの合計をセンチ秒で返す。10 ms未満は切り捨てる。
    fn cecp_time_cs(self) -> u64 {
        Self {
            remaining: self.remaining + self.byoyomi,
            ..self
        }
        .remaining_cs()
    }
}

/// 1局で両色に割り当てた時計。
#[derive(Clone, Copy)]
pub struct GameClocks {
    /// 先手の時計。固定制限の側は`None`。
    black: Option<Clock>,
    /// 後手の時計。固定制限の側は`None`。
    white: Option<Clock>,
}

impl GameClocks {
    /// プレイヤーAとBの制限を対局時の色へ割り当てる。
    pub fn new(player_a_color: Color, player_a: SearchLimit, player_b: SearchLimit) -> Self {
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
    pub fn get_mut(&mut self, color: Color) -> Option<&mut Clock> {
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
                    .cecp_time_cs(),
                opponent_cs: self
                    .get(side_to_move.opposite())
                    .map_or(0, Clock::cecp_time_cs),
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
#[derive(Clone)]
pub struct PlayerConfig {
    /// 入力された指定の原文。表示に使う。
    pub text: String,
    /// 再開時の照合に使う完全コミットハッシュまたは起動指定。
    pub identity: EngineIdentity,
    /// 実行ファイルのパス。
    pub path: PathBuf,
    /// 起動引数。
    pub args: Vec<String>,
    /// エンジンとの通信プロトコル。
    pub protocol: Protocol,
    /// 校正用ランダムエンジンかどうか。真ならシードを設定する。
    pub is_random: bool,
    /// このプレイヤーに適用する思考制限。
    pub limit: SearchLimit,
    /// このプレイヤーに適用する置換表容量(MB)。省略時はエンジンの既定値。
    pub hash_mb: Option<u64>,
    /// 起動引数と規則オプションへ渡す`--rules`入力原文。
    pub rules_source: String,
    /// 握手後、isreadyより前に列順で送るUSIオプションの名前と値。
    pub options: Vec<(String, String)>,
}

impl PlayerConfig {
    /// specと実効制限を組み合わせた表示名を返す。
    pub fn name(&self) -> String {
        format!("{} limit={}", self.text, self.limit.cli_text())
    }
}

/// USIセッションの異常分類。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EngineFailure {
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
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailureCounts {
    /// 不正着手の件数。
    pub illegal_moves: u64,
    /// クラッシュの件数。
    pub crashes: u64,
    /// 応答タイムアウトの件数。
    pub timeouts: u64,
    /// 時間切れの件数。
    pub time_forfeits: u64,
    /// 審判層の合法手を拒否した件数。
    pub rejected_moves: u64,
}

impl FailureCounts {
    /// 1局の異常を加算する。
    pub fn record(&mut self, failure: EngineFailure) {
        match failure {
            EngineFailure::IllegalMove => self.illegal_moves += 1,
            EngineFailure::Crash => self.crashes += 1,
            EngineFailure::Timeout => self.timeouts += 1,
            EngineFailure::TimeForfeit => self.time_forfeits += 1,
            EngineFailure::RejectedMove => self.rejected_moves += 1,
        }
    }

    /// 別の集計値を加算する。
    pub fn add(&mut self, other: Self) {
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
    /// 着手とともに返された予想手。
    ponder: Option<String>,
    /// `go`、`ponderhit`または`stop`から応答までの実測時間。
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
    /// 待機中はNone、先読み中は予想した相手の着手。
    pondering: Option<Move>,
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
pub struct EngineDefaults {
    /// `Threads`の既定値。
    pub threads: Option<u32>,
    /// `USI_Hash`の既定値(MB)。
    pub hash_mb: Option<u64>,
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
            pondering: None,
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
                if !config.is_random
                    && let Some(hash_mb) = config.hash_mb
                {
                    process.send(&format!("setoption name USI_Hash value {hash_mb}"))?;
                }
                // 測定の対局は規則上の終局まで指させる（docs/plans/usi-resignation.md
                // 「棋力測定での無効化」）。オプションを持たないエンジンはこの行を無視する。
                process.send("setoption name ResignValue value 99999")?;
                if config.is_random {
                    process.send(&format!("setoption name Seed value {seed}"))?;
                }
                for (name, value) in &config.options {
                    process.send(&format!("setoption name {name} value {value}"))?;
                }
                process.send("isready")?;
                process.wait_for("readyok")?;
                process.send("usinewgame")?;
            }
            Protocol::Cecp => {
                process.send("xboard")?;
                process.send("protover 2")?;
                process.wait_for("feature done=1")?;
                process.send(&cecp_memory_text(config.hash_mb))?;
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
                    SearchLimit::Time(time) if time.byoyomi_ms > 0 => {
                        process.send(&cecp_fixed_time_text(time))?;
                    }
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

    /// 現局面の応答を得る。外れた先読みの停止も同じ時計と期限に含める。
    fn bestmove(
        &mut self,
        usi_history: &[String],
        move_history: &[Move],
        clocks: &GameClocks,
        side: Color,
        limit: SearchLimit,
    ) -> Result<ThinkResult, EngineFailure> {
        if self.protocol == Protocol::Cecp {
            return self.bestmove_cecp(move_history, &clocks.think_request(side, limit));
        }
        let start;
        if let Some(predicted) = self.pondering.take() {
            start = Instant::now();
            if move_history.last() == Some(&predicted) {
                self.send("ponderhit")?;
            } else {
                self.send("stop")?;
                self.discard_bestmove(start)?;
                self.send_position(usi_history)?;
                let mut adjusted = *clocks;
                if let Some(clock) = adjusted.get_mut(side) {
                    clock.remaining = clock.remaining.saturating_sub(start.elapsed());
                }
                let request = adjusted.think_request(side, limit);
                self.send(&format!("go {}", request.go_text))?;
            }
        } else {
            self.send_position(usi_history)?;
            let request = clocks.think_request(side, limit);
            start = Instant::now();
            self.send(&format!("go {}", request.go_text))?;
        }
        self.receive_bestmove(start)
    }

    /// 初期局面からのUSI着手列を送る。
    fn send_position(&mut self, history: &[String]) -> Result<(), EngineFailure> {
        if history.is_empty() {
            self.send("position startpos")
        } else {
            self.send(&format!("position startpos moves {}", history.join(" ")))
        }
    }

    /// 合法で終局しない予想手だけを使い、次の相手番の間に先読みする。
    fn start_ponder(
        &mut self,
        game: &Game,
        history: &[String],
        prediction: Option<&str>,
        request: &ThinkRequest,
    ) {
        let Some(Ok((predicted, true))) = prediction.map(|text| ponder_move(game, text)) else {
            return;
        };
        let mut history = history.to_vec();
        history.push(
            usi::text(
                game.position(),
                predicted,
                &MoveGenerator::new(game.rules().moves),
            )
            .expect("a legal prediction must be renderable"),
        );
        if self.send_position(&history).is_ok()
            && self.send(&format!("go ponder {}", request.go_text)).is_ok()
        {
            self.pondering = Some(predicted);
        }
    }

    /// 先読みの出力を探索情報も含めて捨てる。
    fn discard_bestmove(&self, start: Instant) -> Result<(), EngineFailure> {
        receive_until_deadline(&self.lines, start, self.timeout, |line| {
            line.split_whitespace().next() == Some("bestmove")
        })
        .map(|_| ())
    }

    /// 終局時の停止は結果を変更せず、資源を読む前に同期する。
    fn stop_ponder(&mut self) {
        if self.pondering.take().is_some() {
            let start = Instant::now();
            if self.send("stop").is_ok() {
                let _ = self.discard_bestmove(start);
            }
        }
    }

    /// 的中時には先読み中のinfoも同じ探索の情報として読む。
    fn receive_bestmove(&self, start: Instant) -> Result<ThinkResult, EngineFailure> {
        let mut evaluation = None;
        let mut stop_reason = None;
        let mut completed_time_ms = None;
        let mut observation_error = None;
        let line = receive_until_deadline(&self.lines, start, self.timeout, |line| {
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
        let tokens = line.split_whitespace().collect::<Vec<_>>();
        let ponder = if tokens.get(2) == Some(&"ponder") {
            tokens.get(3).map(|text| (*text).to_owned())
        } else {
            None
        };
        let elapsed = start.elapsed();
        if elapsed > self.timeout {
            return Err(EngineFailure::Timeout);
        }
        Ok(ThinkResult {
            response,
            ponder,
            elapsed,
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
            ponder: None,
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

/// 1手固定時間をCECPの`st`コマンドへ変換する。
/// HaChuでは`st`が固定時間モードを設定し、毎手の`time`が表す時間の0.4倍を目標、
/// 約0.98倍を強制中断の上限とするため、`time`にも1手分の時間を送る必要がある。
fn cecp_fixed_time_text(time: TimeControl) -> String {
    format!("st {}", time.byoyomi_ms / 1_000)
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
    predicate: impl FnMut(&str) -> bool,
) -> Result<String, EngineFailure> {
    receive_until_deadline(lines, Instant::now(), timeout, predicate)
}

/// 複数の応答を読む場合も、同じ起点から期限を測る。
fn receive_until_deadline(
    lines: &Receiver<io::Result<String>>,
    start: Instant,
    timeout: Duration,
    mut predicate: impl FnMut(&str) -> bool,
) -> Result<String, EngineFailure> {
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
        self.stop_ponder();
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
pub struct Opening {
    /// 開始手順を適用済みの対局。
    pub game: Game,
    /// 開始手順の生成に使ったシード。
    pub seed: NonZeroU64,
    /// 開始手順の指し手列。
    pub moves: Vec<Move>,
    /// 開始手順のUSI表記列。エンジンへの`position`送信に使う。
    pub usi_moves: Vec<String>,
}

/// 審判裁定またはエンジン反則による終局結果。
#[derive(Clone, Copy)]
pub enum GameOutcome {
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
pub enum PlayedGame {
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
pub struct RecordedGame {
    /// 既存の得点計算と表示に使う終局結果。
    pub played: PlayedGame,
    /// 永続化する構造化記録。
    pub record: GameRecord,
}

/// 手番を保存形式へ変換する。
pub const fn stored_color(color: Color) -> StoredColor {
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

/// 保存形式の異常分類を集計用の分類へ戻す。
pub const fn failure_from_stored(reason: FailureKind) -> EngineFailure {
    match reason {
        FailureKind::IllegalMove => EngineFailure::IllegalMove,
        FailureKind::Crash => EngineFailure::Crash,
        FailureKind::Timeout => EngineFailure::Timeout,
        FailureKind::TimeForfeit => EngineFailure::TimeForfeit,
        FailureKind::RejectedMove => EngineFailure::RejectedMove,
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
    mut candidate_process: Option<&mut EngineProcess>,
    mut baseline_process: Option<&mut EngineProcess>,
) -> RecordedGame {
    if let Some(process) = candidate_process.as_mut() {
        process.stop_ponder();
    }
    if let Some(process) = baseline_process.as_mut() {
        process.stop_ponder();
    }
    let candidate_usage = candidate_process.map_or_else(EngineResourceUsage::default, |process| {
        process.resource_usage()
    });
    let baseline_usage = baseline_process.map_or_else(EngineResourceUsage::default, |process| {
        process.resource_usage()
    });
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
pub struct PairResult {
    /// 候補側ペア得点のペンタノミアル分類(0〜4)。打ち切りを含むペアは`None`。
    pub category: Option<usize>,
    /// このペアで発生した異常の件数。
    pub failures: FailureCounts,
}

/// 1ペアの番号、表示内容、集計結果。
pub struct CompletedPair {
    /// 1起算のペア番号。
    pub number: u64,
    /// ペア番号順に出力する表示内容。
    pub output: String,
    /// 集計へ取り込む結果。
    pub result: PairResult,
    /// 原子的に保存する監査記録。
    pub record: PairRecord,
}

/// 外部エンジン指定を解析する。空白区切りの2語目以降は起動引数として渡す。
pub fn parse_player_spec(input: &str) -> Result<PlayerSpec, String> {
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
///
/// `options`はUSIの握手後、`isready`より前に列順で送る名前と値である。
/// CECP指定で列が空でなければ入力エラーを返す。コミット指定は通常ビルドを使う。
pub fn resolve_player(
    spec: PlayerSpec,
    limit: SearchLimit,
    hash_mb: Option<u64>,
    rules_text: &str,
    options: Vec<(String, String)>,
) -> io::Result<PlayerConfig> {
    if matches!(spec.kind, PlayerKind::Cecp { .. }) && !options.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CECP engine does not support USI setoption settings",
        ));
    }
    if let Some(hash_mb) = hash_mb {
        if hash_mb == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "engine hash size must be at least 1 MB",
            ));
        }
        if matches!(spec.kind, PlayerKind::Random) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "random engine does not support a hash size",
            ));
        }
        if matches!(spec.kind, PlayerKind::Cecp { .. }) && !hash_mb.is_power_of_two() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "CECP engine hash size must be a power of two in MB because HaChu rounds its memory allocation",
            ));
        }
    }
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
            let (path, hash, sha256) = resolve_commit(&revision, None)?;
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
        hash_mb,
        rules_source: rules_text.to_owned(),
        options,
    })
}

/// 明示容量または既定容量を設定するCECPコマンドを返す。
fn cecp_memory_text(hash_mb: Option<u64>) -> String {
    format!("memory {}", hash_mb.unwrap_or(CECP_MEMORY_MB))
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
        SearchLimit::Time(time) if time.byoyomi_ms > 0 => {
            if time.base_ms == 0 && time.increment_ms == 0 && time.byoyomi_ms % 1_000 == 0 {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "CECP engine supports byoyomi only as a fixed time per move: base 0, increment 0, whole seconds",
                ))
            }
        }
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

/// コミットをビルドし、実行ファイル、完全ハッシュ、SHA-256を返す。
///
/// `feature`が`None`なら完全ハッシュをキャッシュのキーに使う。
/// 指定する場合は英数字、`_`、`-`からなる1つのfeature名を受け取り、
/// `<完全ハッシュ>-<feature名>`をキーにして通常ビルドと分離する。
pub fn resolve_commit(
    revision: &str,
    feature: Option<&str>,
) -> io::Result<(PathBuf, String, String)> {
    let repository = std::env::current_dir()?;
    let report = |message: String| {
        if feature.is_some() {
            eprintln!("{message}");
        } else {
            println!("{message}");
        }
    };
    report(format!("resolving commit {revision}..."));
    let hash = normalize_commit(&repository, revision)?;
    let cache_root = repository.join("target/match-cache");
    let binary_name = format!("minase{}", std::env::consts::EXE_SUFFIX);
    let key = match feature {
        None => hash.clone(),
        Some(feature) => {
            if feature.is_empty()
                || !feature
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "feature must be a nonempty name containing ASCII letters, digits, underscores, or hyphens",
                ));
            }
            format!("{hash}-{feature}")
        }
    };
    let cache_path = cache_root.join(&key).join(&binary_name);
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
        report(format!("cached: {}", cache_path.display()));
        return Ok((cache_path, hash, sha256));
    }

    report(format!("building commit {hash}..."));
    fs::create_dir_all(&cache_root)?;
    let source_tree = cache_root.join(format!(".source-{key}-{}", process::id()));
    let archive_path = cache_root.join(format!(".source-{key}-{}.tar", process::id()));
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
                let mut command = Command::new("cargo");
                command.args(["build", "--release", "--bin", "minase"]);
                if let Some(feature) = feature {
                    command.args(["--features", feature]);
                }
                let output = command.current_dir(&source_tree).output()?;
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
    report(format!("cached: {}", cache_path.display()));
    let sha256 = sha256_file(&cache_path)?;
    Ok((cache_path, hash, sha256))
}

/// 思考制限を解析する。
pub fn parse_search_limit(input: &str) -> Result<SearchLimit, String> {
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
pub fn parse_positive_u64(text: &str) -> Result<u64, String> {
    let value = text
        .parse::<u64>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if value == 0 {
        return Err("value must be at least 1".to_owned());
    }
    Ok(value)
}

/// 0より大きい`u32`を解析する。
pub fn parse_positive_u32(text: &str) -> Result<u32, String> {
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

/// 1エンジンを事前起動し、実際のプロトコル応答から既定資源を得る。
pub fn probe_engine_defaults(
    player: &PlayerConfig,
    timeout: Duration,
) -> io::Result<EngineDefaults> {
    if player.is_random {
        return Ok(EngineDefaults {
            threads: Some(1),
            hash_mb: None,
        });
    }
    if player.protocol == Protocol::Cecp {
        return Ok(EngineDefaults {
            threads: None,
            hash_mb: Some(CECP_MEMORY_MB),
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

/// ファイル全体のSHA-256を小文字16進数で返す。
pub fn sha256_file(path: &Path) -> io::Result<String> {
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

/// 終局しない8手から12手の開始手順を生成する。
pub fn generate_opening(rules: Rules, pair_seed: NonZeroU64) -> Opening {
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
pub fn validate_bestmove(
    game: &Game,
    text: &str,
    protocol: Protocol,
) -> Result<Move, EngineFailure> {
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

/// 予想手の合法性と、その手で対局が続くかを審判層で判定する。
fn ponder_move(game: &Game, text: &str) -> Result<(Move, bool), EngineFailure> {
    let selected = validate_bestmove(game, text, Protocol::Usi)?;
    let mut predicted = game.clone();
    let status = predicted
        .play(selected)
        .expect("a legal prediction must be accepted");
    Ok((selected, !matches!(status, GameStatus::Finished(_))))
}

/// 保存済みの応答列から再計算するエンジン別の先読み件数。
#[derive(Default, Debug, PartialEq, Eq)]
pub struct PonderCounts {
    /// 予想手を返した回数。
    pub predictions: u64,
    /// 不合法な予想手の回数。
    pub illegal_predictions: u64,
    /// 先読みを開始した回数。
    pub starts: u64,
    /// 予想手が的中した回数。
    pub hits: u64,
    /// 指した着手の数。
    pub moves: u64,
}

/// 候補・基準に同じ審判規則を適用し、通信を再現する。
pub fn count_ponder_game(
    record: &GameRecord,
    mut game: Game,
    ponder: bool,
    max_ply: u32,
    counts: &mut [PonderCounts; 2],
) -> io::Result<()> {
    let mut pending = [None; 2];
    for turn in &record.turns {
        let side = game.position().side_to_move();
        let index = usize::from(turn.side != record.candidate_color);
        counts[index].predictions += u64::from(turn.ponder.is_some());
        let TurnResponse::Move { usi: text } = &turn.response else {
            continue;
        };
        counts[index].moves += 1;
        let selected = validate_bestmove(&game, text, Protocol::Usi).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "cannot replay saved move for ponder counts",
            )
        })?;
        let status = game.play(selected).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "cannot apply saved move for ponder counts",
            )
        })?;
        if matches!(status, GameStatus::Finished(_)) {
            continue;
        }
        if pending[1 - index].take() == Some(selected) && game.ply_count() < max_ply {
            counts[1 - index].hits += 1;
        }
        if let Some(text) = &turn.ponder {
            match ponder_move(&game, text) {
                Err(_) => counts[index].illegal_predictions += 1,
                Ok((predicted, true)) if ponder => {
                    counts[index].starts += 1;
                    pending[index] = Some(predicted);
                }
                Ok(_) => {}
            }
        }
        debug_assert_eq!(game.position().side_to_move(), side.opposite());
    }
    Ok(())
}

/// 1局を既存の対局管理層で進行する。
#[allow(clippy::too_many_arguments)]
pub fn play_game(
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
    ponder: bool,
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
                Some(&mut player_a_process),
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
                Some(&mut player_a_process),
                Some(&mut player_b_process),
            ));
        }

        let side_to_move = game.position().side_to_move();
        let (process, limit) = if side_to_move == player_a_color {
            (&mut player_a_process, player_a.limit)
        } else {
            (&mut player_b_process, player_b.limit)
        };
        let ThinkResult {
            response,
            elapsed,
            evaluation,
            stop_reason,
            completed_time_ms,
            ponder: prediction,
        } = match process.bestmove(&usi_history, &move_history, &clocks, side_to_move, limit) {
            Ok(response) => response,
            Err(reason) => {
                return Some(recorded_game(
                    forfeit(game.ply_count(), side_to_move, reason),
                    player_a_color,
                    player_a_seed,
                    player_b_seed,
                    turns,
                    started,
                    Some(&mut player_a_process),
                    Some(&mut player_b_process),
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
                ponder: prediction.clone(),
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
                Some(&mut player_a_process),
                Some(&mut player_b_process),
            ));
        }
        let EngineResponse::Move(response) = response else {
            turns.push(TurnRecord {
                side: stored_color(side_to_move),
                think_time_ns: duration_ns(elapsed),
                evaluation: evaluation_record(evaluation, side_to_move),
                stop_reason,
                completed_time_ms,
                ponder: prediction.clone(),
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
                Some(&mut player_a_process),
                Some(&mut player_b_process),
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
                    ponder: prediction.clone(),
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
                    Some(&mut player_a_process),
                    Some(&mut player_b_process),
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
            ponder: prediction.clone(),
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
                Some(&mut player_a_process),
                Some(&mut player_b_process),
            ));
        }
        if ponder {
            process.start_ponder(
                &game,
                &usi_history,
                prediction.as_deref(),
                &clocks.think_request(side_to_move, limit),
            );
        }
    }
}

/// 1局の結果を候補Aから見た半点単位の得点へ変換する。
pub fn half_points(outcome: GameOutcome, player_a_color: Color) -> u8 {
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
pub fn record_game_failure(game: PlayedGame, counts: &mut FailureCounts) {
    if let PlayedGame::Finished {
        outcome: GameOutcome::Forfeit { reason, .. },
        ..
    } = game
    {
        counts.record(reason);
    }
}

/// 1ペアを実行し、表示内容とペンタノミアル分類を返す。
#[allow(clippy::too_many_arguments)]
pub fn run_pair(
    rules: Rules,
    rules_text: &str,
    base_seed: u64,
    pair_number: u64,
    max_ply: u32,
    candidate: &PlayerConfig,
    baseline: &PlayerConfig,
    timeout: Duration,
    ponder: bool,
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
        ponder,
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
        ponder,
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

#[cfg(test)]
mod ponder_tests;
#[cfg(test)]
mod tests;

/// USI握手で宣言されたoption行を、期限つきの既存通信経路で取得する。
pub fn probe_usi_options(
    player: &PlayerConfig,
    timeout: Duration,
) -> Result<Vec<String>, EngineFailure> {
    let mut process = EngineProcess::spawn(player, timeout)?;
    process.send("usi")?;
    let mut options = Vec::new();
    process.receive_until(|line| {
        if line.starts_with("option ") {
            options.push(line.to_owned());
        }
        line.trim() == "usiok"
    })?;
    Ok(options)
}

impl std::fmt::Display for EngineFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for EngineFailure {}
