//! コミット対コミットの自己対局測定ハーネス。
//!
//! ペア対局のペンタノミアルGSPRTと固定局数Eloを提供する。運用規約と
//! 統計的契約はdocs/guides/sprt.mdを参照。

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{ArgGroup, CommandFactory, Parser, Subcommand, error::ErrorKind};
use minase::core::rules::parse_rule_set;
use minase::harness::*;
use minase::notation::usi;
use minase::rng::derive_seed;
use minase::stats::{GSPRT_H1_ELO, GsprtDecision, estimate_elo, gsprt_decision, gsprt_llr};
use minase::{Color, Game, GameResult, GameStatus, MoveGenerator, RuleCode, Rules};

#[path = "match_runner/storage.rs"]
mod storage;
use storage::{
    CpuRecord, EngineHashSizes, EngineRecord, EngineThreadCounts, FORMAT_VERSION, HarnessRecord,
    ManifestMode, RunManifest, RunStore, StoredSearchLimit,
};

/// 1局を打ち切る手数上限の既定値。
const DEFAULT_MAX_PLY: u32 = 4096;
/// GSPRTの暴走保険となる実行ペア数上限の既定値。
const DEFAULT_MAX_PAIRS: u64 = 100_000;
/// 1回のエンジン応答を待つ秒数の既定値。
const DEFAULT_RESPONSE_TIMEOUT_SECONDS: u64 = 120;
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
    /// 候補側だけに適用する置換表容量(MB)。省略時はエンジンの既定値。
    #[arg(long, value_name = "MB")]
    candidate_hash: Option<u64>,
    /// 基準側だけに適用する思考制限。
    #[arg(long, value_parser = parse_search_limit)]
    baseline_limit: Option<SearchLimit>,
    /// 基準側だけに適用する置換表容量(MB)。省略時はエンジンの既定値。
    #[arg(long, value_name = "MB")]
    baseline_hash: Option<u64>,
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
    /// USIの予想手に従って両エンジンの先読みを進行する。
    #[arg(long)]
    ponder: bool,
    /// 実行する統計モード。
    #[command(subcommand)]
    mode: Mode,
}

impl Arguments {
    /// 先読みを利用できない対局条件を、プロセス起動前に拒否する。
    fn validate_ponder(&self) -> Result<(), String> {
        if self.ponder {
            if !matches!(
                self.candidate_limit.unwrap_or(self.each),
                SearchLimit::Time(_)
            ) || !matches!(
                self.baseline_limit.unwrap_or(self.each),
                SearchLimit::Time(_)
            ) {
                return Err("--ponder requires time= limits for both engines".to_owned());
            }
            if matches!(self.candidate.kind, PlayerKind::Cecp { .. })
                || matches!(self.baseline.kind, PlayerKind::Cecp { .. })
            {
                return Err("--ponder does not support cecp: engines".to_owned());
            }
        }
        Ok(())
    }
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
    ponder: bool,
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
    let engines = if ponder { 2 } else { 1 };
    let concurrency = available_cores / engine_threads / engines;
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
    ponder: bool,
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
            ponder,
        )
        .map_err(io::Error::other)?,
    };
    Ok(RunManifest {
        format_version: FORMAT_VERSION,
        ponder,
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
            candidate: candidate.hash_mb.or(candidate_defaults.hash_mb),
            baseline: baseline.hash_mb.or(baseline_defaults.hash_mb),
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

/// 規則コード列をカンマ区切りで返す。
fn rules_text(codes: &[RuleCode]) -> String {
    codes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// 再開以前の確定ペアも含め、保存記録の全件から件数を得る。
fn ponder_summary(
    store: &RunStore,
    rules: Rules,
    ponder: bool,
    max_ply: u32,
    target_pairs: u64,
) -> io::Result<[PonderCounts; 2]> {
    let mut counts = [PonderCounts::default(), PonderCounts::default()];
    for record in store.records(target_pairs)?.values() {
        let mut opening = Game::new(rules);
        for text in &record.opening.moves {
            let selected = validate_bestmove(&opening, text, Protocol::Usi)
                .map_err(|_| invalid_pair_record("cannot replay saved opening"))?;
            opening
                .play(selected)
                .map_err(|_| invalid_pair_record("cannot apply saved opening"))?;
        }
        for game in &record.games {
            count_ponder_game(game, opening.clone(), ponder, max_ply, &mut counts)?;
        }
    }
    Ok(counts)
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
                || turn.completed_time_ms.is_some()
                || turn.ponder.is_some())
        {
            return Err(invalid_pair_record(
                "CECP turn must not contain USI search information",
            ));
        }
        if turn.ponder.as_ref().is_some_and(|text| {
            text.is_empty() || text.split_whitespace().count() != 1 || text.trim() != text
        }) {
            return Err(invalid_pair_record(
                "saved prediction must be one USI token",
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
    if let Err(error) = arguments.validate_ponder() {
        Arguments::command()
            .error(ErrorKind::ValueValidation, error)
            .exit();
    }
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
        arguments.candidate_hash,
        &arguments.rules.source,
        Vec::new(),
    ) {
        Ok(player) => player,
        Err(error) => {
            eprintln!("failed to resolve candidate engine: {error}");
            process::exit(1);
        }
    };
    let baseline = match resolve_player(
        arguments.baseline,
        baseline_limit,
        arguments.baseline_hash,
        &arguments.rules.source,
        Vec::new(),
    ) {
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
        arguments.ponder,
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
                            arguments.ponder,
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

    let ponder_counts = match ponder_summary(
        &store,
        rules,
        arguments.ponder,
        arguments.max_ply,
        target_pairs,
    ) {
        Ok(counts) => counts,
        Err(error) => {
            eprintln!("failed to replay ponder statistics: {error}");
            process::exit(1);
        }
    };
    for (name, counts) in ["candidate", "baseline"].into_iter().zip(ponder_counts) {
        println!(
            "ponder {name}: predictions={} illegal_predictions={} go_ponder={} hits={} moves={}",
            counts.predictions,
            counts.illegal_predictions,
            counts.starts,
            counts.hits,
            counts.moves
        );
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
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_host_resource_probe_reports_cores_and_memory() {
        assert!(physical_core_count().is_some_and(|cores| cores > 0));
        assert!(physical_memory_bytes().is_some_and(|bytes| bytes > 0));
    }

    use super::*;

    #[test]
    fn default_concurrency_uses_available_cores_and_larger_thread_count() {
        assert_eq!(
            default_concurrency(Some(20), Some(1), Some(1), false),
            Ok(19)
        );
        assert_eq!(
            default_concurrency(Some(20), Some(4), Some(2), false),
            Ok(4)
        );
    }

    #[test]
    fn default_concurrency_requires_resource_counts() {
        let error = default_concurrency(None, Some(1), Some(1), false).unwrap_err();
        assert!(error.contains("physical core count"));
        assert!(error.contains("--concurrency"));

        let error = default_concurrency(Some(20), None, Some(1), false).unwrap_err();
        assert!(error.contains("candidate engine Threads"));
        assert!(error.contains("--concurrency"));

        let error = default_concurrency(Some(20), Some(1), None, false).unwrap_err();
        assert!(error.contains("baseline engine Threads"));
        assert!(error.contains("--concurrency"));
    }

    #[test]
    fn default_concurrency_rejects_values_below_one() {
        let error = default_concurrency(Some(1), Some(1), Some(1), false).unwrap_err();
        assert!(error.contains("--concurrency"));

        let error = default_concurrency(Some(20), Some(0), Some(1), false).unwrap_err();
        assert!(error.contains("Threads must be at least 1"));
        assert!(error.contains("--concurrency"));

        let error = default_concurrency(Some(20), Some(1), Some(0), false).unwrap_err();
        assert!(error.contains("Threads must be at least 1"));
        assert!(error.contains("--concurrency"));
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
                ponder: None,
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
                ponder: None,
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
        assert_eq!(arguments.candidate_hash, None);
        assert_eq!(arguments.baseline_hash, None);
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

    // 置換表容量の指定: 両側の容量は独立に指定でき、省略時はNoneを保つ。
    #[test]
    fn hash_options_resolve_independently_for_each_engine() {
        let arguments = Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "run",
            "--candidate",
            "engine",
            "--baseline",
            "cecp:engine",
            "--candidate-hash",
            "300",
            "--baseline-hash",
            "512",
            "gsprt",
        ])
        .unwrap();
        for (spec, hash_mb, expected) in [
            (arguments.candidate, arguments.candidate_hash, 300),
            (arguments.baseline, arguments.baseline_hash, 512),
        ] {
            let player = resolve_player(spec, arguments.each, hash_mb, "R1", Vec::new()).unwrap();
            assert_eq!(player.hash_mb, Some(expected));
        }
    }

    // manifestの`hash_mb`: manifestは明示容量を優先し、省略側は既定値を記録する。
    #[test]
    fn manifest_hash_sizes_prefer_each_players_explicit_value() {
        for (candidate_hash, baseline_hash, expected_candidate, expected_baseline) in [
            (Some(512), None, Some(512), Some(256)),
            (None, Some(128), Some(256), Some(128)),
        ] {
            let player = |hash_mb| {
                resolve_player(
                    parse_player_spec("cecp:engine").unwrap(),
                    parse_search_limit("depth=1").unwrap(),
                    hash_mb,
                    "R1",
                    Vec::new(),
                )
                .unwrap()
            };
            let manifest = run_manifest(
                &player(candidate_hash),
                &player(baseline_hash),
                &parse_rule_set_argument("R1").unwrap(),
                ManifestMode::Elo,
                1,
                4096,
                120,
                Some(1),
                false,
            )
            .unwrap();
            assert_eq!(manifest.hash_mb.candidate, expected_candidate);
            assert_eq!(manifest.hash_mb.baseline, expected_baseline);
        }
    }

    // RULES.md第33条: 共通parserへの接続と指定原文の保持を検査する。
    // sprt.md: --rules省略時にはengine-defaultを使う。
    #[test]
    fn rules_presets_resolve_per_article_33() {
        let default = Arguments::try_parse_from(["match_runner", "--run-dir", "run", "gsprt"])
            .expect("omitting --rules must fall back to engine-default");
        assert_eq!(default.rules.source, "engine-default");
        assert_eq!(
            default.rules.codes,
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
        assert_eq!(lishogi.rules.source, "LISHOGI");
        assert_eq!(lishogi.rules.codes, Vec::<RuleCode>::from(Rules::LISHOGI));
        assert!(
            Arguments::try_parse_from([
                "match_runner",
                "--run-dir",
                "run",
                "--rules",
                "lishogi,P1",
                "gsprt",
            ])
            .is_err()
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
}

#[cfg(test)]
#[path = "match_runner/ponder_tests.rs"]
mod ponder_tests;
