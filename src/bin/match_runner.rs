//! コミット対コミットの自己対局測定ハーネス。
//!
//! ペア対局のペンタノミアルGSPRTと固定局数Eloを提供する。運用規約と
//! 統計的契約はdocs/sprt.mdを参照。

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{CommandFactory, Parser, Subcommand, error::ErrorKind};
use minase::core::rules::parse_rule_set;
use minase::notation::{cecp, usi};
use minase::rng::{XorShift64, derive_seed};
use minase::search::MAX_PLY;
use minase::stats::{GsprtDecision, estimate_elo, gsprt_decision, gsprt_llr};
use minase::{Color, Game, GameResult, GameStatus, Move, RuleCode, Rules, Square};

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
#[command(name = "match_runner")]
struct Arguments {
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
    /// 同時に実行するペア数。
    #[arg(long, default_value_t = 1, value_parser = parse_positive_usize)]
    concurrency: usize,
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
    parse_rule_set(input).map(|codes| RuleSetArgument {
        source: input.to_owned(),
        codes,
    })
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

impl EngineProcess {
    /// プロセスを起動し、USI初期化列を完了する。
    fn start(config: &PlayerConfig, seed: u64, timeout: Duration) -> Result<Self, EngineFailure> {
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
        let mut process = Self {
            child,
            input: Some(input),
            lines,
            reader: Some(reader),
            timeout,
            protocol: config.protocol,
            sent_moves: 0,
        };
        match config.protocol {
            Protocol::Usi => {
                process.send("usi")?;
                process.wait_for("usiok")?;
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
    ) -> Result<(EngineResponse, Duration), EngineFailure> {
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
    ) -> Result<(EngineResponse, Duration), EngineFailure> {
        if history.is_empty() {
            self.send("position startpos")?;
        } else {
            self.send(&format!("position startpos moves {}", history.join(" ")))?;
        }
        let start = Instant::now();
        self.send(&format!("go {go_text}"))?;
        self.receive_until(|line| line.split_whitespace().next() == Some("bestmove"))
            .map(|line| {
                (
                    EngineResponse::Move(
                        line.split_whitespace()
                            .nth(1)
                            .unwrap_or_default()
                            .to_owned(),
                    ),
                    start.elapsed(),
                )
            })
    }

    /// CECPの差分着手と時計を送り、`pong`までの応答を解釈する。
    fn bestmove_cecp(
        &mut self,
        history: &[Move],
        request: &ThinkRequest,
    ) -> Result<(EngineResponse, Duration), EngineFailure> {
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
        let response = receive_cecp_response(&self.lines, self.timeout, pong_number, start)?;
        if matches!(response.0, EngineResponse::Move(_)) {
            self.send("force")?;
            self.sent_moves += 1;
        }
        Ok(response)
    }

    /// 条件を満たす行を、呼び出し全体の期限まで受信する。
    fn receive_until(&self, predicate: impl FnMut(&str) -> bool) -> Result<String, EngineFailure> {
        receive_until(&self.lines, self.timeout, predicate)
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
    seed: u64,
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
    let (path, args, protocol, is_random) = match spec.kind {
        PlayerKind::Random => {
            let current = std::env::current_exe()?;
            let filename = format!("usi_random{}", std::env::consts::EXE_SUFFIX);
            (
                current.with_file_name(filename),
                Vec::new(),
                Protocol::Usi,
                true,
            )
        }
        PlayerKind::Commit(revision) => (
            resolve_commit(&revision)?,
            vec![
                "--protocol".to_owned(),
                "usi".to_owned(),
                "--rules".to_owned(),
                rules_text.to_owned(),
            ],
            Protocol::Usi,
            false,
        ),
        PlayerKind::Command { program, args } => (program, args, Protocol::Usi, false),
        PlayerKind::Cecp { program, args } => {
            validate_cecp_limit(limit)?;
            (program, args, Protocol::Cecp, false)
        }
    };
    Ok(PlayerConfig {
        text: spec.text,
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
fn resolve_commit(revision: &str) -> io::Result<PathBuf> {
    let repository = std::env::current_dir()?;
    println!("resolving commit {revision}...");
    let hash = normalize_commit(&repository, revision)?;
    let cache_root = repository.join("target/match-cache");
    let binary_name = format!("minase{}", std::env::consts::EXE_SUFFIX);
    let cache_path = cache_root.join(&hash).join(&binary_name);
    if cache_path.exists() {
        println!("cached: {}", cache_path.display());
        return Ok(cache_path);
    }

    println!("building commit {hash}...");
    fs::create_dir_all(&cache_root)?;
    let worktree = cache_root.join(format!("worktree-{hash}"));
    let add_output = Command::new("git")
        .args(["worktree", "add"])
        .arg(&worktree)
        .arg(&hash)
        .current_dir(&repository)
        .output()?;
    if !add_output.status.success() {
        return Err(command_error("git worktree add", &add_output));
    }

    let build_result = (|| {
        let output = Command::new("cargo")
            .args(["build", "--release", "--bin", "minase"])
            .current_dir(&worktree)
            .output()?;
        if !output.status.success() {
            return Err(command_error("cargo build --release --bin minase", &output));
        }
        fs::create_dir_all(cache_path.parent().expect("cache path has a parent"))?;
        fs::copy(
            worktree.join("target/release").join(&binary_name),
            &cache_path,
        )?;
        Ok(())
    })();
    let remove_output = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(&worktree)
        .current_dir(&repository)
        .output()?;
    let remove_result = if remove_output.status.success() {
        Ok(())
    } else {
        Err(command_error("git worktree remove --force", &remove_output))
    };
    build_result?;
    remove_result?;
    println!("cached: {}", cache_path.display());
    Ok(cache_path)
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
fn generate_opening(rules: Rules, pair_seed: u64) -> Opening {
    let mut opening_seed = derive_seed(pair_seed, 0);
    loop {
        let mut game = Game::new(rules);
        let mut rng = XorShift64::new(opening_seed);
        let opening_plies = 8 + rng.index(5);
        let mut moves = Vec::with_capacity(opening_plies);
        let mut usi_moves = Vec::with_capacity(opening_plies);
        let mut finished = false;

        for _ in 0..opening_plies {
            let legal_moves = game.legal_moves();
            assert!(
                !legal_moves.is_empty(),
                "ongoing game must have legal moves"
            );
            let selected = legal_moves[rng.index(legal_moves.len())];
            moves.push(selected);
            usi_moves.push(usi::text(game.position(), selected));
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
        opening_seed = derive_seed(opening_seed, 0);
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
    player_a_seed: u64,
    player_b: &PlayerConfig,
    player_b_seed: u64,
    timeout: Duration,
) -> PlayedGame {
    let forfeit = |plies, loser: Color, reason| PlayedGame::Finished {
        plies,
        outcome: GameOutcome::Forfeit {
            winner: loser.opposite(),
            reason,
        },
    };
    let mut player_a_process = match EngineProcess::start(player_a, player_a_seed, timeout) {
        Ok(process) => process,
        Err(reason) => return forfeit(game.ply_count(), player_a_color, reason),
    };
    let mut player_b_process = match EngineProcess::start(player_b, player_b_seed, timeout) {
        Ok(process) => process,
        Err(reason) => {
            return forfeit(game.ply_count(), player_a_color.opposite(), reason);
        }
    };
    let mut clocks = GameClocks::new(player_a_color, player_a.limit, player_b.limit);

    loop {
        if game.ply_count() >= max_ply {
            return PlayedGame::Cutoff {
                plies: game.ply_count(),
            };
        }

        let side_to_move = game.position().side_to_move();
        let (process, limit) = if side_to_move == player_a_color {
            (&mut player_a_process, player_a.limit)
        } else {
            (&mut player_b_process, player_b.limit)
        };
        let request = clocks.think_request(side_to_move, limit);
        let (response, elapsed) = match process.bestmove(&usi_history, &move_history, &request) {
            Ok(response) => response,
            Err(reason) => return forfeit(game.ply_count(), side_to_move, reason),
        };
        if let Some(clock) = clocks.get_mut(side_to_move)
            && let Err(reason) = clock.update(elapsed)
        {
            return forfeit(game.ply_count(), side_to_move, reason);
        }
        let EngineResponse::Move(response) = response else {
            return PlayedGame::Finished {
                plies: game.ply_count(),
                outcome: GameOutcome::Resigned {
                    winner: side_to_move.opposite(),
                },
            };
        };
        let selected = match validate_bestmove(&game, &response, process.protocol) {
            Ok(selected) => selected,
            Err(reason) => return forfeit(game.ply_count(), side_to_move, reason),
        };
        usi_history.push(usi::text(game.position(), selected));
        move_history.push(selected);
        let status = game
            .play(selected)
            .expect("a move validated against legal_moves must be accepted");
        if let GameStatus::Finished(result) = status {
            return PlayedGame::Finished {
                plies: game.ply_count(),
                outcome: GameOutcome::Adjudicated(result),
            };
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
) -> CompletedPair {
    let pair_seed = derive_seed(base_seed, pair_number);
    let opening = generate_opening(rules, pair_seed);
    let game1_a_seed = derive_seed(pair_seed, 1);
    let game1_b_seed = derive_seed(pair_seed, 2);
    let game2_a_seed = derive_seed(pair_seed, 3);
    let game2_b_seed = derive_seed(pair_seed, 4);

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
    );
    writeln!(
        output,
        "pair {pair_number} game 1: {}",
        played_game_text(game1)
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
    );
    writeln!(
        output,
        "pair {pair_number} game 2: {}",
        played_game_text(game2)
    )
    .expect("writing to String cannot fail");

    let mut failures = FailureCounts::default();
    record_game_failure(game1, &mut failures);
    record_game_failure(game2, &mut failures);
    let (
        PlayedGame::Finished {
            outcome: game1_outcome,
            ..
        },
        PlayedGame::Finished {
            outcome: game2_outcome,
            ..
        },
    ) = (game1, game2)
    else {
        writeln!(output, "pair {pair_number} result: discarded")
            .expect("writing to String cannot fail");
        return CompletedPair {
            number: pair_number,
            output,
            result: PairResult {
                category: None,
                failures,
            },
        };
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
    CompletedPair {
        number: pair_number,
        output,
        result: PairResult {
            category: Some(category),
            failures,
        },
    }
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

/// 引数を検証し、ワーカープールでペア対局を実行して集計を出力する。
fn main() {
    if let Err(error) = minase::eval::network() {
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
    let response_timeout = Duration::from_secs(arguments.response_timeout);
    println!("rules: {rules_text}");
    println!("seed: {base_seed}");
    println!("max_ply: {}", arguments.max_ply);
    println!("candidate: {}", candidate.name());
    println!("baseline: {}", baseline.name());
    println!("response_timeout: {} s", arguments.response_timeout);

    let (target_pairs, use_gsprt) = match arguments.mode {
        Mode::Gsprt { max_pairs } => (max_pairs, true),
        Mode::Elo { pairs } => (pairs, false),
    };
    let start = Instant::now();
    let mut results = [0; 5];
    let mut valid_pairs = 0;
    let mut discarded_pairs = 0;
    let mut failures = FailureCounts::default();
    let mut decision = GsprtDecision::Continue;
    let worker_count = arguments
        .concurrency
        .min(usize::try_from(target_pairs).unwrap_or(usize::MAX));
    let pool_result = thread::scope(|scope| {
        let (job_sender, job_receiver) = mpsc::channel::<u64>();
        let job_receiver = Arc::new(Mutex::new(job_receiver));
        let (result_sender, result_receiver) = mpsc::channel::<Result<CompletedPair, ()>>();
        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let result_sender = result_sender.clone();
            let job_receiver = Arc::clone(&job_receiver);
            let candidate = &candidate;
            let baseline = &baseline;
            let rules_text = &rules_text;
            workers.push(scope.spawn(move || {
                let worker_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    loop {
                        let job = job_receiver
                            .lock()
                            .expect("the job receiver mutex must not be poisoned")
                            .recv();
                        let Ok(pair_number) = job else {
                            break;
                        };
                        let pair = run_pair(
                            rules,
                            rules_text,
                            base_seed,
                            pair_number,
                            arguments.max_ply,
                            candidate,
                            baseline,
                            response_timeout,
                        );
                        if result_sender.send(Ok(pair)).is_err() {
                            break;
                        }
                    }
                }));
                if worker_result.is_err() {
                    let _ = result_sender.send(Err(()));
                }
            }));
        }
        drop(result_sender);

        let initial_jobs = u64::try_from(worker_count).expect("worker count must fit in u64");
        for pair_number in 1..=initial_jobs {
            job_sender
                .send(pair_number)
                .expect("workers must be waiting for initial jobs");
        }
        let mut next_pair = initial_jobs.checked_add(1).expect("pair number overflow");
        let mut next_to_integrate = 1_u64;
        let mut completed = BTreeMap::new();
        let mut pool_error = None;

        while next_to_integrate <= target_pairs
            && (!use_gsprt || decision == GsprtDecision::Continue)
        {
            let pair = match result_receiver.recv() {
                Ok(Ok(pair)) => pair,
                Ok(Err(())) => {
                    pool_error = Some("a match worker panicked".to_owned());
                    break;
                }
                Err(error) => {
                    pool_error = Some(format!("worker result channel disconnected: {error}"));
                    break;
                }
            };
            // 完了ペアを一旦バッファし、ペア番号順に出力とLLR取り込みを行う。
            // これにより判定と出力は並列度に依存しない(docs/sprt.mdの再現契約)。
            completed.insert(pair.number, pair);
            while let Some(pair) = completed.remove(&next_to_integrate) {
                print!("{}", pair.output);
                failures.add(pair.result.failures);
                match pair.result.category {
                    Some(category) => {
                        results[category] += 1;
                        valid_pairs += 1;
                        if use_gsprt {
                            let llr = gsprt_llr(&results);
                            decision = gsprt_decision(llr);
                            println!(
                                "statistics: valid_pairs={valid_pairs} pentanomial={results:?} llr={llr:.10} decision={}",
                                decision_text(decision)
                            );
                        }
                    }
                    None => discarded_pairs += 1,
                }
                next_to_integrate = next_to_integrate
                    .checked_add(1)
                    .expect("pair number overflow");
                if use_gsprt && decision != GsprtDecision::Continue {
                    break;
                }
                if next_pair <= target_pairs {
                    if let Err(error) = job_sender.send(next_pair) {
                        pool_error = Some(format!("worker job channel disconnected: {error}"));
                        break;
                    }
                    next_pair = next_pair.checked_add(1).expect("pair number overflow");
                }
            }
            if pool_error.is_some() {
                break;
            }
        }

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

    // D8-HARN-08/D8-STAT-06(sprt.md): 文書化された既定値
    // (--response-timeout 120秒、--max-ply 4096、--max-pairs 100,000)を
    // 文書の明文値リテラルで固定する。文書が変わらない限り実装定数の変更は
    // 逸脱である。
    #[test]
    fn documented_defaults_match_sprt_md() {
        let arguments = Arguments::try_parse_from(["match_runner", "gsprt"])
            .expect("the documented default invocation must be accepted");
        assert_eq!(arguments.response_timeout, 120);
        assert_eq!(arguments.max_ply, 4096);
        assert!(matches!(arguments.mode, Mode::Gsprt { max_pairs: 100_000 }));
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
        let equal = Arguments::try_parse_from(["match_runner", "--each", "depth=4", "gsprt"])
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
        let default = Arguments::try_parse_from(["match_runner", "gsprt"])
            .expect("omitting --rules must fall back to engine-default");
        assert_eq!(default.rules.source, "engine-default");
        assert_eq!(
            default.rules.codes,
            Vec::<RuleCode>::from(Rules::ENGINE_DEFAULT)
        );

        let named =
            Arguments::try_parse_from(["match_runner", "--rules", "engine-default", "gsprt"])
                .expect("the engine-default preset must be accepted");
        assert_eq!(named.rules.source, "engine-default");
        assert_eq!(
            named.rules.codes,
            Vec::<RuleCode>::from(Rules::ENGINE_DEFAULT)
        );

        let lishogi = Arguments::try_parse_from(["match_runner", "--rules", "LISHOGI", "gsprt"])
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
                Arguments::try_parse_from(["match_runner", "--rules", invalid, "gsprt"]).is_err(),
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
        let legal_text = usi::text(game.position(), legal);
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
        let seeds: Vec<u64> = (1..=100).map(|n| derive_seed(base, n)).collect();
        let replay: Vec<u64> = (1..=100).map(|n| derive_seed(base, n)).collect();
        assert_eq!(seeds, replay);
        assert!(seeds.iter().all(|&seed| seed != 0));
        let mut unique = seeds.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), seeds.len());
        // 派生値が0になる入力でも非ゼロへ置換される(random-play.mdの
        // 仕様式の逆算により、splitmix64の出力0の原像は0x61C8_8646_80B5_83EB)
        assert_ne!(derive_seed(0x61C8_8646_80B5_83EB - 5, 5), 0);
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
