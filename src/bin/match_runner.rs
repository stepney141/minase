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
use minase::notation::usi;
use minase::rng::{XorShift64, derive_seed};
use minase::search::MAX_PLY;
use minase::stats::{GsprtDecision, estimate_elo, gsprt_decision, gsprt_llr};
use minase::{Color, Game, GameResult, GameStatus, Move, RuleCode, Rules, Square};

const DEFAULT_MAX_PLY: u32 = 4096;
const DEFAULT_MAX_PAIRS: u64 = 100_000;
const DEFAULT_RESPONSE_TIMEOUT_SECONDS: u64 = 120;

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

#[derive(Clone)]
struct RuleSetArgument(Vec<RuleCode>);

fn parse_rule_set_argument(input: &str) -> Result<RuleSetArgument, String> {
    parse_rule_set(input).map(RuleSetArgument)
}

/// 外部エンジンの指定。
#[derive(Clone, PartialEq, Eq, Debug)]
struct PlayerSpec {
    text: String,
    kind: PlayerKind,
}

/// ランダムエンジンまたは実行ファイルの起動コマンドを表す。
#[derive(Clone, PartialEq, Eq, Debug)]
enum PlayerKind {
    Random,
    Commit(String),
    Command { program: PathBuf, args: Vec<String> },
}

/// USIの`go`へ渡す思考制限。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SearchLimit {
    Fixed {
        depth: Option<u32>,
        nodes: Option<u64>,
    },
    Time(TimeControl),
}

/// ミリ秒単位の持ち時間、加算時間、秒読み。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct TimeControl {
    base_ms: u64,
    increment_ms: u64,
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
    remaining: Duration,
    increment: Duration,
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
}

/// 1局で両色に割り当てた時計。
struct GameClocks {
    black: Option<Clock>,
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
    text: String,
    path: PathBuf,
    args: Vec<String>,
    is_random: bool,
    limit: SearchLimit,
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
    IllegalMove,
    Crash,
    Timeout,
    TimeForfeit,
}

/// 異常理由別の発生件数。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct FailureCounts {
    illegal_moves: u64,
    crashes: u64,
    timeouts: u64,
    time_forfeits: u64,
}

impl FailureCounts {
    /// 1局の異常を加算する。
    fn record(&mut self, failure: EngineFailure) {
        match failure {
            EngineFailure::IllegalMove => self.illegal_moves += 1,
            EngineFailure::Crash => self.crashes += 1,
            EngineFailure::Timeout => self.timeouts += 1,
            EngineFailure::TimeForfeit => self.time_forfeits += 1,
        }
    }

    /// 別の集計値を加算する。
    fn add(&mut self, other: Self) {
        self.illegal_moves += other.illegal_moves;
        self.crashes += other.crashes;
        self.timeouts += other.timeouts;
        self.time_forfeits += other.time_forfeits;
    }
}

/// 読み取りスレッドからUSI出力を受け取る外部エンジン。
struct EngineProcess {
    child: Child,
    input: ChildStdin,
    lines: Receiver<io::Result<String>>,
    reader: Option<JoinHandle<()>>,
    timeout: Duration,
}

impl EngineProcess {
    /// プロセスを起動し、USI初期化列を完了する。
    fn start(
        config: &PlayerConfig,
        rules_text: &str,
        seed: u64,
        timeout: Duration,
    ) -> Result<Self, EngineFailure> {
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
            input,
            lines,
            reader: Some(reader),
            timeout,
        };
        process.send("usi")?;
        process.wait_for("usiok")?;
        process.send(&format!("setoption name RuleSet value {rules_text}"))?;
        if config.is_random {
            process.send(&format!("setoption name Seed value {seed}"))?;
        }
        process.send("isready")?;
        process.wait_for("readyok")?;
        process.send("usinewgame")?;
        Ok(process)
    }

    /// エンジンへ1行を送る。
    fn send(&mut self, line: &str) -> Result<(), EngineFailure> {
        writeln!(self.input, "{line}").map_err(|_| EngineFailure::Crash)?;
        self.input.flush().map_err(|_| EngineFailure::Crash)
    }

    /// 指定行を期限まで読み進める。
    fn wait_for(&self, expected: &str) -> Result<(), EngineFailure> {
        self.receive_until(|line| line.trim() == expected)
            .map(|_| ())
    }

    /// 現局面と`go`を送り、`bestmove`と実測思考時間を受け取る。
    fn bestmove(
        &mut self,
        history: &[String],
        go_text: &str,
    ) -> Result<(String, Duration), EngineFailure> {
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
                    line.split_whitespace()
                        .nth(1)
                        .unwrap_or_default()
                        .to_owned(),
                    start.elapsed(),
                )
            })
    }

    /// 条件を満たす行を、呼び出し全体の期限まで受信する。
    fn receive_until(&self, predicate: impl FnMut(&str) -> bool) -> Result<String, EngineFailure> {
        receive_until(&self.lines, self.timeout, predicate)
    }
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
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// ペア対局で共有する開始状態。
struct Opening {
    game: Game,
    seed: u64,
    moves: Vec<Move>,
    usi_moves: Vec<String>,
}

/// 審判裁定またはエンジン反則による終局結果。
#[derive(Clone, Copy)]
enum GameOutcome {
    Adjudicated(GameResult),
    Forfeit {
        winner: Color,
        reason: EngineFailure,
    },
}

/// 1局の完走または手数上限による打ち切り。
#[derive(Clone, Copy)]
enum PlayedGame {
    Finished { plies: u32, outcome: GameOutcome },
    Cutoff { plies: u32 },
}

/// 1ペアの集計結果。
struct PairResult {
    category: Option<usize>,
    failures: FailureCounts,
}

/// 1ペアの番号、表示内容、集計結果。
struct CompletedPair {
    number: u64,
    output: String,
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
    let (path, args, is_random) = match spec.kind {
        PlayerKind::Random => {
            let current = std::env::current_exe()?;
            let filename = format!("usi_random{}", std::env::consts::EXE_SUFFIX);
            (current.with_file_name(filename), Vec::new(), true)
        }
        PlayerKind::Commit(revision) => (
            resolve_commit(&revision)?,
            vec![
                "--protocol".to_owned(),
                "usi".to_owned(),
                "--rules".to_owned(),
                rules_text.to_owned(),
            ],
            false,
        ),
        PlayerKind::Command { program, args } => (program, args, false),
    };
    Ok(PlayerConfig {
        text: spec.text,
        path,
        args,
        is_random,
        limit,
    })
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
        let mut game = Game::new(rules).expect("validated rules must contain a repetition rule");
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

/// `bestmove`を解析し、審判の合法手集合と照合する。
fn validate_bestmove(game: &Game, text: &str) -> Result<Move, EngineFailure> {
    let selected = usi::parse(game.position(), text).map_err(|_| EngineFailure::IllegalMove)?;
    if game.legal_moves().contains(&selected) {
        Ok(selected)
    } else {
        Err(EngineFailure::IllegalMove)
    }
}

/// 1局を既存の対局管理層で進行する。
#[allow(clippy::too_many_arguments)]
fn play_game(
    mut game: Game,
    mut history: Vec<String>,
    max_ply: u32,
    player_a_color: Color,
    player_a: &PlayerConfig,
    player_a_seed: u64,
    player_b: &PlayerConfig,
    player_b_seed: u64,
    rules_text: &str,
    timeout: Duration,
) -> PlayedGame {
    let forfeit = |plies, loser: Color, reason| PlayedGame::Finished {
        plies,
        outcome: GameOutcome::Forfeit {
            winner: loser.opposite(),
            reason,
        },
    };
    let mut player_a_process =
        match EngineProcess::start(player_a, rules_text, player_a_seed, timeout) {
            Ok(process) => process,
            Err(reason) => return forfeit(game.ply_count(), player_a_color, reason),
        };
    let mut player_b_process =
        match EngineProcess::start(player_b, rules_text, player_b_seed, timeout) {
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
        let go_text = match limit.fixed_go_text() {
            Some(go_text) => go_text,
            None => clocks.go_text(side_to_move),
        };
        let (response, elapsed) = match process.bestmove(&history, &go_text) {
            Ok(response) => response,
            Err(reason) => return forfeit(game.ply_count(), side_to_move, reason),
        };
        if let Some(clock) = clocks.get_mut(side_to_move)
            && let Err(reason) = clock.update(elapsed)
        {
            return forfeit(game.ply_count(), side_to_move, reason);
        }
        let selected = match validate_bestmove(&game, &response) {
            Ok(selected) => selected,
            Err(reason) => return forfeit(game.ply_count(), side_to_move, reason),
        };
        history.push(usi::text(game.position(), selected));
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
        | GameOutcome::Forfeit { winner, .. } => Some(winner),
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
        max_ply,
        Color::Black,
        candidate,
        game1_a_seed,
        baseline,
        game1_b_seed,
        rules_text,
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
        max_ply,
        Color::White,
        candidate,
        game2_a_seed,
        baseline,
        game2_b_seed,
        rules_text,
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
        "engine_failures: illegal_moves={} crashes={} timeouts={} time_forfeits={}",
        failures.illegal_moves, failures.crashes, failures.timeouts, failures.time_forfeits
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

fn main() {
    let arguments = Arguments::parse();
    let rules = match Rules::from_codes(&arguments.rules.0) {
        Ok(rules) => rules,
        Err(error) => Arguments::command()
            .error(ErrorKind::ValueValidation, error.to_string())
            .exit(),
    };
    if let Err(error) = Game::new(rules) {
        Arguments::command()
            .error(ErrorKind::ValueValidation, error.to_string())
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
    let candidate_limit = arguments.candidate_limit.unwrap_or(arguments.each);
    let baseline_limit = arguments.baseline_limit.unwrap_or(arguments.each);
    let rules_text = rules_text(&arguments.rules.0);
    let candidate = match resolve_player(arguments.candidate, candidate_limit, &rules_text) {
        Ok(player) => player,
        Err(error) => {
            eprintln!("failed to resolve candidate engine: {error}");
            process::exit(1);
        }
    };
    let baseline = match resolve_player(arguments.baseline, baseline_limit, &rules_text) {
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

    #[test]
    fn arguments_accept_modes_specs_limits_and_defaults() {
        let gsprt = Arguments::try_parse_from(["match_runner", "gsprt"])
            .expect("the GSPRT mode must be accepted");
        assert_eq!(gsprt.rules.0, [RuleCode::R1]);
        assert_eq!(gsprt.candidate.kind, PlayerKind::Random);
        assert_eq!(gsprt.baseline.kind, PlayerKind::Random);
        assert_eq!(
            gsprt.each,
            SearchLimit::Fixed {
                depth: Some(1),
                nodes: None
            }
        );
        assert_eq!(gsprt.response_timeout, DEFAULT_RESPONSE_TIMEOUT_SECONDS);
        assert_eq!(gsprt.concurrency, 1);
        assert!(matches!(
            gsprt.mode,
            Mode::Gsprt {
                max_pairs: DEFAULT_MAX_PAIRS
            }
        ));

        let elo = Arguments::try_parse_from([
            "match_runner",
            "--candidate",
            "/tmp/candidate",
            "--baseline",
            "random",
            "--each",
            "depth=2,nodes=1000",
            "--baseline-limit",
            "nodes=25",
            "--response-timeout",
            "9",
            "--concurrency",
            "4",
            "elo",
            "--pairs",
            "20",
        ])
        .expect("path specs and limit overrides must be accepted");
        assert_eq!(
            elo.candidate.kind,
            PlayerKind::Command {
                program: PathBuf::from("/tmp/candidate"),
                args: Vec::new()
            }
        );
        assert_eq!(
            elo.each,
            SearchLimit::Fixed {
                depth: Some(2),
                nodes: Some(1000)
            }
        );
        assert_eq!(
            elo.baseline_limit,
            Some(SearchLimit::Fixed {
                depth: None,
                nodes: Some(25)
            })
        );
        assert_eq!(elo.response_timeout, 9);
        assert_eq!(elo.concurrency, 4);
        assert!(matches!(elo.mode, Mode::Elo { pairs: 20 }));
    }

    #[test]
    fn player_spec_accepts_commit_revision() {
        let spec = parse_player_spec("commit:0045833")
            .expect("a commit engine spec must be accepted without resolving it");
        assert_eq!(spec.kind, PlayerKind::Commit("0045833".to_owned()));
    }

    #[test]
    fn player_spec_splits_program_and_startup_arguments() {
        let spec = parse_player_spec("target/release/minase --protocol usi --rules engine-default")
            .expect("a command line spec must be accepted");
        assert_eq!(
            spec.kind,
            PlayerKind::Command {
                program: PathBuf::from("target/release/minase"),
                args: vec![
                    "--protocol".to_owned(),
                    "usi".to_owned(),
                    "--rules".to_owned(),
                    "engine-default".to_owned(),
                ],
            }
        );
    }

    #[test]
    fn arguments_reject_legacy_player_specs() {
        for spec in ["depth=1", "depth=2,nodes=1000"] {
            assert!(
                Arguments::try_parse_from([
                    "match_runner",
                    "--candidate",
                    spec,
                    "elo",
                    "--pairs",
                    "1"
                ])
                .is_err(),
                "unsupported spec {spec:?} must be rejected"
            );
        }
    }

    #[test]
    fn invalid_commit_revision_fails_before_match_execution() {
        let error = normalize_commit(
            Path::new(env!("CARGO_MANIFEST_DIR")),
            "definitely-not-a-minase-commit",
        )
        .expect_err("an unknown revision must fail normalization");
        assert!(error.to_string().contains("git rev-parse --verify"));
    }

    #[test]
    fn search_limits_accept_only_supported_forms() {
        assert_eq!(
            parse_search_limit("depth=3"),
            Ok(SearchLimit::Fixed {
                depth: Some(3),
                nodes: None
            })
        );
        assert_eq!(
            parse_search_limit("nodes=400"),
            Ok(SearchLimit::Fixed {
                depth: None,
                nodes: Some(400)
            })
        );
        assert_eq!(
            parse_search_limit("depth=3,nodes=400"),
            Ok(SearchLimit::Fixed {
                depth: Some(3),
                nodes: Some(400)
            })
        );
        for limit in [
            "",
            "depth=0",
            "nodes=0",
            "nodes=1,depth=1",
            "depth=1,foo=2",
            "depth=1,nodes=2,x=3",
        ] {
            assert!(
                parse_search_limit(limit).is_err(),
                "invalid limit {limit:?} must be rejected"
            );
        }
    }

    #[test]
    fn search_limits_accept_time_and_optional_byoyomi() {
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
    }

    #[test]
    fn search_limits_reject_time_mixed_with_fixed_limits() {
        for limit in [
            "time=1000+10,depth=1",
            "time=1000+10,nodes=100",
            "depth=1,time=1000+10",
            "nodes=100,time=1000+10",
            "byoyomi=1000",
        ] {
            assert!(
                parse_search_limit(limit).is_err(),
                "mixed or incomplete limit {limit:?} must be rejected"
            );
        }
    }

    #[test]
    fn clocks_update_remaining_time_and_detect_time_forfeits() {
        let mut clock = Clock::new(TimeControl {
            base_ms: 1_000,
            increment_ms: 100,
            byoyomi_ms: 0,
        });
        assert_eq!(clock.update(Duration::from_millis(250)), Ok(()));
        assert_eq!(clock.remaining, Duration::from_millis(850));
        assert_eq!(
            clock.update(Duration::from_millis(851)),
            Err(EngineFailure::TimeForfeit)
        );
        assert_eq!(clock.remaining, Duration::from_millis(850));

        let mut byoyomi_clock = Clock::new(TimeControl {
            base_ms: 100,
            increment_ms: 20,
            byoyomi_ms: 50,
        });
        assert_eq!(byoyomi_clock.update(Duration::from_millis(150)), Ok(()));
        assert_eq!(byoyomi_clock.remaining, Duration::from_millis(20));
        assert_eq!(
            byoyomi_clock.update(Duration::from_millis(71)),
            Err(EngineFailure::TimeForfeit)
        );
        assert_eq!(byoyomi_clock.remaining, Duration::from_millis(20));
    }

    #[test]
    fn time_go_text_uses_current_clocks_for_both_colors() {
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
        let clocks = GameClocks::new(Color::White, player_a, player_b);
        assert_eq!(
            clocks.go_text(Color::Black),
            "btime 20000 wtime 10000 binc 200 winc 100 byoyomi 2000"
        );
        assert_eq!(
            SearchLimit::Fixed {
                depth: Some(3),
                nodes: Some(400),
            }
            .fixed_go_text(),
            Some("depth 3 nodes 400".to_owned())
        );
    }

    #[test]
    fn rules_argument_rejects_preset_combined_with_code() {
        let error =
            match Arguments::try_parse_from(["match_runner", "--rules", "lishogi,P1", "gsprt"]) {
                Ok(_) => panic!("a preset combined with a rule code must be rejected"),
                Err(error) => error,
            };
        assert!(
            error
                .to_string()
                .contains("preset 'lishogi' must be specified alone")
        );
    }

    #[test]
    fn bestmove_validation_classifies_illegal_responses() {
        let game = Game::new(Rules::engine_default()).unwrap();
        let legal = game.legal_moves()[0];
        let legal_text = usi::text(game.position(), legal);
        assert_eq!(validate_bestmove(&game, &legal_text), Ok(legal));
        assert_eq!(
            validate_bestmove(&game, "not-a-move"),
            Err(EngineFailure::IllegalMove)
        );
        assert_eq!(
            validate_bestmove(&game, "1a1b"),
            Err(EngineFailure::IllegalMove)
        );
    }

    #[test]
    fn failure_counts_classify_each_reason() {
        let mut counts = FailureCounts::default();
        counts.record(EngineFailure::IllegalMove);
        counts.record(EngineFailure::Crash);
        counts.record(EngineFailure::Timeout);
        counts.record(EngineFailure::TimeForfeit);
        assert_eq!(
            counts,
            FailureCounts {
                illegal_moves: 1,
                crashes: 1,
                timeouts: 1,
                time_forfeits: 1
            }
        );
    }

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

    #[test]
    fn response_channel_ignores_info_before_bestmove() {
        let (sender, lines) = mpsc::channel();
        sender
            .send(Ok("info depth 1 score cp 0".to_owned()))
            .unwrap();
        sender.send(Ok("bestmove 1a1b".to_owned())).unwrap();
        assert_eq!(
            receive_until(&lines, Duration::from_secs(1), |line| {
                line.split_whitespace().next() == Some("bestmove")
            }),
            Ok("bestmove 1a1b".to_owned())
        );
    }

    #[test]
    fn game_results_convert_to_candidate_half_points() {
        let win = GameOutcome::Adjudicated(GameResult::Win {
            winner: Color::Black,
            reason: WinReason::RoyalCapture,
        });
        let loss = GameOutcome::Adjudicated(GameResult::Win {
            winner: Color::White,
            reason: WinReason::RoyalCapture,
        });
        let draw = GameOutcome::Adjudicated(GameResult::Draw {
            reason: DrawReason::Repetition,
        });
        let forfeit = GameOutcome::Forfeit {
            winner: Color::Black,
            reason: EngineFailure::Crash,
        };
        assert_eq!(half_points(win, Color::Black), 2);
        assert_eq!(half_points(loss, Color::Black), 0);
        assert_eq!(half_points(draw, Color::Black), 1);
        assert_eq!(half_points(forfeit, Color::Black), 2);
    }
}
