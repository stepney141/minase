//! 保存済み対局から時間制御校正の指標を再計算する。

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use clap::Parser;
use fs2::FileExt;
use minase::stats::estimate_elo;
use serde::{Deserialize, Serialize};

/// 集計対象の実行記録形式。
const FORMAT_VERSION: u32 = 3;

/// 校正指標集計器のコマンドライン引数。
#[derive(Parser)]
#[command(name = "match_report")]
struct Arguments {
    /// 集計するmatch_runner実行ディレクトリ。
    #[arg(long)]
    run_dir: PathBuf,
    /// 第1、第2主指標の比較基準とする実行ディレクトリ。
    #[arg(long)]
    compare_to: Option<PathBuf>,
}

/// 実行条件から集計に必要な部分。
#[derive(Deserialize)]
struct Manifest {
    format_version: u32,
    candidate: EngineRecord,
    baseline: EngineRecord,
    mode: Mode,
    concurrency: usize,
    engine_threads: ThreadCounts,
    cpu: CpuRecord,
}

/// 集計対象エンジンの探索制限。
#[derive(Deserialize)]
struct EngineRecord {
    limit: SearchLimit,
}

/// 校正で許可する固定ペア数Eloモード。
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Mode {
    Elo,
    Gsprt,
}

/// 保存された探索制限。
#[derive(Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SearchLimit {
    Fixed {
        depth: Option<u32>,
        nodes: Option<u64>,
    },
    Time {
        base_ms: u64,
        increment_ms: u64,
        byoyomi_ms: u64,
    },
}

/// 両エンジンの探索ワーカー数。
#[derive(Deserialize)]
struct ThreadCounts {
    candidate: Option<u32>,
    baseline: Option<u32>,
}

/// 測定機の資源量。
#[derive(Deserialize)]
struct CpuRecord {
    physical_cores: Option<usize>,
    physical_memory_bytes: Option<u64>,
}

/// 再開を含む有効実行時間。
#[derive(Deserialize)]
struct RunSummary {
    active_wall_time_ns: u64,
    interrupted: bool,
    invocation_active: bool,
}

/// 1ペアから集計に必要な部分。
#[derive(Deserialize)]
struct PairRecord {
    pair_number: u64,
    category: Option<u8>,
    games: [GameRecord; 2],
}

/// 1局から集計に必要な部分。
#[derive(Deserialize)]
struct GameRecord {
    candidate_color: StoredColor,
    wall_time_ns: u64,
    candidate_cpu_time_ns: Option<u64>,
    baseline_cpu_time_ns: Option<u64>,
    candidate_peak_rss_bytes: Option<u64>,
    baseline_peak_rss_bytes: Option<u64>,
    termination: Termination,
}

/// 終局理由から異常分類に必要な部分。
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Termination {
    AdjudicatedWin {
        #[serde(rename = "winner")]
        winner: StoredColor,
        #[serde(rename = "reason")]
        _reason: String,
    },
    AdjudicatedDraw {
        #[serde(rename = "reason")]
        _reason: String,
    },
    Resigned {
        #[serde(rename = "loser")]
        loser: StoredColor,
    },
    Forfeit {
        #[serde(rename = "loser")]
        loser: StoredColor,
        reason: FailureKind,
    },
    Cutoff,
}

/// 保存された手番。
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StoredColor {
    Black,
    White,
}

/// 保存されたエンジン異常の分類。
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum FailureKind {
    IllegalMove,
    Crash,
    Timeout,
    TimeForfeit,
    RejectedMove,
}

/// 主指標と監査用の補助指標。
#[derive(Serialize)]
struct Report {
    pentanomial: [u64; 5],
    valid_pairs: u64,
    discarded_pairs: u64,
    normalized_mean_score: f64,
    normalized_pair_variance: f64,
    standard_error: f64,
    elo: String,
    elo_ci95: [String; 2],
    total_cpu_time_ns: Option<u64>,
    average_cpu_time_per_valid_pair_ns: Option<f64>,
    variance_time_product: Option<f64>,
    evidence_per_cpu_second: Option<f64>,
    active_wall_time_ns: u64,
    valid_pairs_per_hour: f64,
    game_wall_time_median_ns: u64,
    game_wall_time_p95_ns: u64,
    cutoffs: u64,
    engine_failures: FailureCounts,
    missing_resource_observations: MissingResourceCounts,
    maximum_game_peak_rss_bytes: Option<u64>,
    conservative_concurrent_peak_rss_bytes: Option<u64>,
    physical_memory_bytes: u64,
    conservative_memory_fraction: Option<f64>,
    physical_cores: usize,
    missing_engine_threads: MissingEngineThreads,
    maximum_engine_threads: Option<u32>,
    leaves_one_physical_core: Option<bool>,
    #[serde(skip)]
    comparison_manifest: serde_json::Value,
    #[serde(skip)]
    pair_numbers: Vec<u64>,
}

/// 異常理由別の件数。
#[derive(Default, Serialize)]
struct FailureCounts {
    illegal_moves: u64,
    crashes: u64,
    timeouts: u64,
    time_forfeits: u64,
    rejected_moves: u64,
}

/// エンジン別、資源別の欠測数。
#[derive(Default, Serialize)]
struct MissingResourceCounts {
    candidate_cpu_time: u64,
    baseline_cpu_time: u64,
    candidate_peak_rss: u64,
    baseline_peak_rss: u64,
}

/// エンジン別の探索ワーカー数の欠測状態。
#[derive(Serialize)]
struct MissingEngineThreads {
    candidate: bool,
    baseline: bool,
}

/// 現行条件に対する候補条件の指標比。
#[derive(Serialize)]
struct Comparison {
    variance_time_reduction_percent: f64,
    evidence_per_cpu_ratio: f64,
}

/// JSONファイルを型付きで読む。
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<T> {
    serde_json::from_reader(File::open(path)?).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid JSON in {}: {error}", path.display()),
        )
    })
}

/// 最近傍順位によるパーセンタイルを返す。
fn nearest_rank(sorted: &[u64], numerator: usize, denominator: usize) -> u64 {
    let rank = sorted
        .len()
        .checked_mul(numerator)
        .expect("sample count must fit usize")
        .div_ceil(denominator);
    sorted[rank.saturating_sub(1)]
}

/// 整列済み標本の中央値を整数ナノ秒で返す。
fn median(sorted: &[u64]) -> u64 {
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[middle]
    } else {
        let sum = u128::from(sorted[middle - 1]) + u128::from(sorted[middle]);
        u64::try_from(sum / 2).expect("the average of two u64 values fits u64")
    }
}

/// 反則負けの理由を1件加算する。
fn record_failure(counts: &mut FailureCounts, reason: FailureKind) {
    match reason {
        FailureKind::IllegalMove => counts.illegal_moves += 1,
        FailureKind::Crash => counts.crashes += 1,
        FailureKind::Timeout => counts.timeouts += 1,
        FailureKind::TimeForfeit => counts.time_forfeits += 1,
        FailureKind::RejectedMove => counts.rejected_moves += 1,
    }
}

/// 無限大を含むEloをJSONで失われない文字列へ変換する。
fn elo_text(value: f64) -> String {
    if value == f64::INFINITY {
        "+inf".to_owned()
    } else if value == f64::NEG_INFINITY {
        "-inf".to_owned()
    } else {
        format!("{value:.12}")
    }
}

/// 候補側から見た1局の得点を半点単位で返す。
fn candidate_half_points(game: &GameRecord) -> Option<u8> {
    let winner = match game.termination {
        Termination::AdjudicatedWin { winner, .. } => Some(winner),
        Termination::AdjudicatedDraw { .. } => return Some(1),
        Termination::Resigned { loser } | Termination::Forfeit { loser, .. } => Some(match loser {
            StoredColor::Black => StoredColor::White,
            StoredColor::White => StoredColor::Black,
        }),
        Termination::Cutoff => return None,
    };
    Some(u8::from(winner == Some(game.candidate_color)) * 2)
}

/// 1つの実行ディレクトリを校正指標へ集計する。
fn report(run_dir: &Path) -> io::Result<Report> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(run_dir.join(".match_runner.lock"))?;
    FileExt::try_lock_shared(&lock).map_err(|error| {
        io::Error::new(
            io::ErrorKind::WouldBlock,
            format!("run directory is still active: {error}"),
        )
    })?;
    let mut comparison_manifest: serde_json::Value = read_json(&run_dir.join("manifest.json"))?;
    let manifest: Manifest =
        serde_json::from_value(comparison_manifest.clone()).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid calibration manifest: {error}"),
            )
        })?;
    if manifest.format_version != FORMAT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "match_report requires format version {FORMAT_VERSION}, found {}",
                manifest.format_version
            ),
        ));
    }
    for pointer in [
        "/candidate/identity",
        "/baseline/identity",
        "/rules_source",
        "/canonical_rules",
        "/seed",
        "/max_ply",
        "/response_timeout_secs",
        "/engine_threads",
        "/hash_mb",
        "/concurrency",
        "/cpu/model",
        "/cpu/logical_cores",
        "/runner",
    ] {
        if comparison_manifest.pointer(pointer).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("calibration manifest is missing {pointer}"),
            ));
        }
    }
    if !matches!(manifest.mode, Mode::Elo) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "calibration report requires fixed-pair Elo mode",
        ));
    }
    if manifest.candidate.limit != manifest.baseline.limit
        || !matches!(manifest.candidate.limit, SearchLimit::Time { .. })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "calibration report requires equal time controls for both engines",
        ));
    }
    comparison_manifest
        .pointer_mut("/candidate")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|candidate| candidate.remove("limit"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "candidate limit is missing"))?;
    comparison_manifest
        .pointer_mut("/baseline")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|baseline| baseline.remove("limit"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "baseline limit is missing"))?;
    let summary: RunSummary = read_json(&run_dir.join("summary.json"))?;
    if summary.invocation_active || summary.interrupted {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "an interrupted or active run cannot calibrate throughput",
        ));
    }
    if summary.active_wall_time_ns == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "active wall time is zero",
        ));
    }
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(run_dir.join("pairs"))? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "pair filename is not UTF-8")
        })?;
        let pair_number = name.strip_suffix(".json").and_then(|digits| {
            (digits.len() == 20 && digits.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| digits.parse::<u64>().ok())
                .flatten()
        });
        let is_temporary = name
            .strip_prefix('.')
            .and_then(|name| name.strip_suffix(".json.tmp"))
            .is_some_and(|digits| {
                digits.len() == 20 && digits.bytes().all(|byte| byte.is_ascii_digit())
            });
        if pair_number.is_some_and(|number| number != 0) && entry.file_type()?.is_file() {
            paths.push((
                pair_number.expect("the condition requires a number"),
                entry.path(),
            ));
        } else if !is_temporary {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected entry in pair record directory: {name}"),
            ));
        }
    }
    paths.sort_by_key(|(number, _)| *number);
    if paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "run contains no pair records",
        ));
    }

    let mut pentanomial = [0_u64; 5];
    let mut game_times = Vec::with_capacity(paths.len() * 2);
    let mut total_cpu_time_ns = Some(0_u128);
    let mut maximum_game_peak_rss_bytes = Some(0_u64);
    let mut cutoffs = 0_u64;
    let mut engine_failures = FailureCounts::default();
    let mut missing_resource_observations = MissingResourceCounts::default();
    let mut pair_numbers = Vec::with_capacity(paths.len());
    for (index, (file_number, path)) in paths.iter().enumerate() {
        let expected_number = u64::try_from(index + 1).expect("pair count fits u64");
        if *file_number != expected_number {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("pair records are not continuous at {expected_number}"),
            ));
        }
        let pair: PairRecord = read_json(path)?;
        if pair.pair_number != *file_number {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "pair filename {file_number} contains record {}",
                    pair.pair_number
                ),
            ));
        }
        pair_numbers.push(*file_number);
        if pair.games[0].candidate_color == pair.games[1].candidate_color {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("pair {file_number} does not swap candidate colors"),
            ));
        }
        let expected_category = pair.games.iter().try_fold(0_u8, |score, game| {
            candidate_half_points(game).map(|points| score + points)
        });
        if pair.category != expected_category {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("pair {file_number} category contradicts its game results"),
            ));
        }
        if let Some(category) = pair.category {
            let count = pentanomial.get_mut(usize::from(category)).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "pair category is outside 0..=4")
            })?;
            *count += 1;
        }
        for game in pair.games {
            game_times.push(game.wall_time_ns);
            if game.candidate_cpu_time_ns.is_none() {
                missing_resource_observations.candidate_cpu_time += 1;
            }
            if game.baseline_cpu_time_ns.is_none() {
                missing_resource_observations.baseline_cpu_time += 1;
            }
            if let (Some(candidate), Some(baseline), Some(total)) = (
                game.candidate_cpu_time_ns,
                game.baseline_cpu_time_ns,
                total_cpu_time_ns.as_mut(),
            ) {
                *total = total
                    .checked_add(u128::from(candidate) + u128::from(baseline))
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "total CPU time overflow")
                    })?;
            } else {
                total_cpu_time_ns = None;
            }

            if game.candidate_peak_rss_bytes.is_none() {
                missing_resource_observations.candidate_peak_rss += 1;
            }
            if game.baseline_peak_rss_bytes.is_none() {
                missing_resource_observations.baseline_peak_rss += 1;
            }
            if let (Some(candidate), Some(baseline)) =
                (game.candidate_peak_rss_bytes, game.baseline_peak_rss_bytes)
            {
                let game_peak = candidate.checked_add(baseline).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "game peak RSS overflow")
                })?;
                if let Some(maximum) = maximum_game_peak_rss_bytes.as_mut() {
                    *maximum = (*maximum).max(game_peak);
                }
            } else {
                maximum_game_peak_rss_bytes = None;
            }
            match game.termination {
                Termination::Cutoff => cutoffs += 1,
                Termination::Forfeit { reason, .. } => {
                    record_failure(&mut engine_failures, reason);
                }
                _ => {}
            }
        }
    }

    let valid_pairs = pentanomial.iter().sum::<u64>();
    if valid_pairs == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "run has no valid pairs",
        ));
    }
    let total_cpu_time_ns = total_cpu_time_ns
        .map(u64::try_from)
        .transpose()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "total CPU time overflow"))?;
    if total_cpu_time_ns == Some(0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "total CPU time is zero",
        ));
    }
    let count = valid_pairs as f64;
    let normalized_mean_score = pentanomial
        .iter()
        .enumerate()
        .map(|(category, &frequency)| category as f64 / 4.0 * frequency as f64)
        .sum::<f64>()
        / count;
    let normalized_pair_variance = pentanomial
        .iter()
        .enumerate()
        .map(|(category, &frequency)| {
            let difference = category as f64 / 4.0 - normalized_mean_score;
            difference * difference * frequency as f64
        })
        .sum::<f64>()
        / count;
    if normalized_pair_variance == 0.0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pair score variance is zero",
        ));
    }
    let standard_error = (normalized_pair_variance / count).sqrt();
    let average_cpu_time_per_valid_pair_ns = total_cpu_time_ns.map(|total| total as f64 / count);
    let variance_time_product =
        average_cpu_time_per_valid_pair_ns.map(|average| normalized_pair_variance * average);
    let z = (normalized_mean_score - 0.5) / standard_error;
    let evidence_per_cpu_second =
        total_cpu_time_ns.map(|total| z * z / (total as f64 / 1_000_000_000.0));
    let estimate = estimate_elo(&pentanomial);
    game_times.sort_unstable();

    let missing_engine_threads = MissingEngineThreads {
        candidate: manifest.engine_threads.candidate.is_none(),
        baseline: manifest.engine_threads.baseline.is_none(),
    };
    let maximum_engine_threads = manifest
        .engine_threads
        .candidate
        .zip(manifest.engine_threads.baseline)
        .map(|(candidate, baseline)| candidate.max(baseline));
    let physical_cores = manifest.cpu.physical_cores.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "physical core count is missing")
    })?;
    let physical_memory_bytes = manifest.cpu.physical_memory_bytes.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "physical memory size is missing",
        )
    })?;
    let concurrency = u64::try_from(manifest.concurrency)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "concurrency does not fit u64"))?;
    let conservative_concurrent_peak_rss_bytes = maximum_game_peak_rss_bytes
        .map(|maximum| {
            maximum.checked_mul(concurrency).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "concurrent RSS overflow")
            })
        })
        .transpose()?;
    let used_search_cores = maximum_engine_threads
        .map(|threads| {
            let threads = usize::try_from(threads).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "thread count does not fit usize",
                )
            })?;
            manifest.concurrency.checked_mul(threads).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "search core count overflow")
            })
        })
        .transpose()?;

    Ok(Report {
        pentanomial,
        valid_pairs,
        discarded_pairs: u64::try_from(paths.len()).expect("path count fits u64") - valid_pairs,
        normalized_mean_score,
        normalized_pair_variance,
        standard_error,
        elo: elo_text(estimate.elo),
        elo_ci95: [elo_text(estimate.lower), elo_text(estimate.upper)],
        total_cpu_time_ns,
        average_cpu_time_per_valid_pair_ns,
        variance_time_product,
        evidence_per_cpu_second,
        active_wall_time_ns: summary.active_wall_time_ns,
        valid_pairs_per_hour: count * 3_600_000_000_000.0 / summary.active_wall_time_ns as f64,
        game_wall_time_median_ns: median(&game_times),
        game_wall_time_p95_ns: nearest_rank(&game_times, 95, 100),
        cutoffs,
        engine_failures,
        missing_resource_observations,
        maximum_game_peak_rss_bytes,
        conservative_concurrent_peak_rss_bytes,
        physical_memory_bytes,
        conservative_memory_fraction: conservative_concurrent_peak_rss_bytes
            .map(|bytes| bytes as f64 / physical_memory_bytes as f64),
        physical_cores,
        missing_engine_threads,
        maximum_engine_threads,
        leaves_one_physical_core: used_search_cores.map(|cores| cores < physical_cores),
        comparison_manifest,
        pair_numbers,
    })
}

/// 2条件の主指標を比較する。
fn compare(candidate: &Report, current: &Report) -> io::Result<Comparison> {
    if candidate.comparison_manifest != current.comparison_manifest {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "comparison manifests differ outside the time control",
        ));
    }
    if candidate.pair_numbers != current.pair_numbers {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "comparison runs do not contain the same pair numbers",
        ));
    }
    let candidate_variance_time_product = candidate.variance_time_product.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "candidate variance-time product is unavailable",
        )
    })?;
    let current_variance_time_product = current.variance_time_product.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "current variance-time product is unavailable",
        )
    })?;
    let candidate_evidence_per_cpu_second = candidate.evidence_per_cpu_second.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "candidate evidence per CPU second is unavailable",
        )
    })?;
    let current_evidence_per_cpu_second = current.evidence_per_cpu_second.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "current evidence per CPU second is unavailable",
        )
    })?;
    if current_variance_time_product == 0.0 || current_evidence_per_cpu_second == 0.0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "current condition has an indeterminate metric ratio",
        ));
    }
    Ok(Comparison {
        variance_time_reduction_percent: 100.0
            * (1.0 - candidate_variance_time_product / current_variance_time_product),
        evidence_per_cpu_ratio: candidate_evidence_per_cpu_second / current_evidence_per_cpu_second,
    })
}

fn main() {
    let arguments = Arguments::parse();
    let run_report = report(&arguments.run_dir).unwrap_or_else(|error| {
        eprintln!("failed to report {}: {error}", arguments.run_dir.display());
        std::process::exit(1);
    });
    let comparison = arguments.compare_to.as_deref().map(|path| {
        let current = report(path).unwrap_or_else(|error| {
            eprintln!(
                "failed to report comparison run {}: {error}",
                path.display()
            );
            std::process::exit(1);
        });
        compare(&run_report, &current).unwrap_or_else(|error| {
            eprintln!("failed to compare calibration metrics: {error}");
            std::process::exit(1);
        })
    });
    let output = serde_json::json!({
        "run": run_report,
        "comparison_to_current": comparison,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&output).expect("finite report values serialize as JSON")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn percentile_and_median_follow_fixed_rank_rules() {
        assert_eq!(median(&[10, 20, 30]), 20);
        assert_eq!(median(&[10, 20, 30, 40]), 25);
        assert_eq!(
            nearest_rank(
                &std::array::from_fn::<_, 100, _>(|index| index as u64),
                95,
                100
            ),
            94
        );
    }

    #[test]
    fn comparison_accepts_exact_threshold_boundaries() {
        let make = |m1, m2| Report {
            pentanomial: [0; 5],
            valid_pairs: 1,
            discarded_pairs: 0,
            normalized_mean_score: 0.5,
            normalized_pair_variance: 0.1,
            standard_error: 0.1,
            elo: "0".to_owned(),
            elo_ci95: ["-1".to_owned(), "1".to_owned()],
            total_cpu_time_ns: Some(1),
            average_cpu_time_per_valid_pair_ns: Some(1.0),
            variance_time_product: Some(m1),
            evidence_per_cpu_second: Some(m2),
            active_wall_time_ns: 1,
            valid_pairs_per_hour: 1.0,
            game_wall_time_median_ns: 1,
            game_wall_time_p95_ns: 1,
            cutoffs: 0,
            engine_failures: FailureCounts::default(),
            missing_resource_observations: MissingResourceCounts::default(),
            maximum_game_peak_rss_bytes: Some(1),
            conservative_concurrent_peak_rss_bytes: Some(1),
            physical_memory_bytes: 10,
            conservative_memory_fraction: Some(0.1),
            physical_cores: 2,
            missing_engine_threads: MissingEngineThreads {
                candidate: false,
                baseline: false,
            },
            maximum_engine_threads: Some(1),
            leaves_one_physical_core: Some(true),
            comparison_manifest: serde_json::json!({"experiment": 1}),
            pair_numbers: vec![1],
        };
        let current = make(10.0, 4.0);
        let mut candidate = make(7.0, 4.0);
        let comparison = compare(&candidate, &current).unwrap();
        assert!((comparison.variance_time_reduction_percent - 30.0).abs() < 1e-12);
        assert_eq!(comparison.evidence_per_cpu_ratio, 1.0);
        candidate.comparison_manifest = serde_json::json!({"experiment": 2});
        assert!(compare(&candidate, &current).is_err());
    }

    #[test]
    fn report_rejects_format_version_two() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let run_dir = std::env::temp_dir().join(format!(
            "minase-match-report-version-two-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&run_dir).unwrap();
        File::create(run_dir.join(".match_runner.lock")).unwrap();
        std::fs::write(
            run_dir.join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "format_version": 2,
                "candidate": {
                    "limit": {"kind": "time", "base_ms": 1, "increment_ms": 0, "byoyomi_ms": 0}
                },
                "baseline": {
                    "limit": {"kind": "time", "base_ms": 1, "increment_ms": 0, "byoyomi_ms": 0}
                },
                "mode": {"kind": "elo"},
                "concurrency": 1,
                "engine_threads": {"candidate": 1, "baseline": 1},
                "cpu": {"physical_cores": 1, "physical_memory_bytes": 1}
            }))
            .unwrap(),
        )
        .unwrap();

        let error = match report(&run_dir) {
            Ok(_) => panic!("version 2 must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            "match_report requires format version 3, found 2"
        );
        std::fs::remove_dir_all(run_dir).unwrap();
    }

    #[test]
    fn report_recomputes_the_documented_metrics_from_pair_records() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let run_dir = std::env::temp_dir().join(format!(
            "minase-match-report-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(run_dir.join("pairs")).unwrap();
        File::create(run_dir.join(".match_runner.lock")).unwrap();
        std::fs::write(
            run_dir.join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "format_version": FORMAT_VERSION,
                "candidate": {
                    "identity": {"kind": "commit", "hash": "a", "sha256": "b"},
                    "limit": {"kind": "time", "base_ms": 10_000, "increment_ms": 100, "byoyomi_ms": 0}
                },
                "baseline": {
                    "identity": {"kind": "commit", "hash": "c", "sha256": "d"},
                    "limit": {"kind": "time", "base_ms": 10_000, "increment_ms": 100, "byoyomi_ms": 0}
                },
                "mode": {"kind": "elo"},
                "rules_source": "engine-default",
                "canonical_rules": ["L0", "P0", "R1", "E0"],
                "seed": 20260828_u64,
                "max_ply": 4096,
                "response_timeout_secs": 120,
                "concurrency": 8,
                "engine_threads": {"candidate": 1, "baseline": 1},
                "hash_mb": {"candidate": 256, "baseline": 256},
                "cpu": {
                    "model": "test",
                    "physical_cores": 20,
                    "logical_cores": 20,
                    "physical_memory_bytes": 10_000
                },
                "runner": {"version": "0.1.0", "sha256": "e"}
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            run_dir.join("summary.json"),
            serde_json::to_vec(&serde_json::json!({
                "active_wall_time_ns": 20_000_000_000_u64,
                "interrupted": false,
                "invocation_active": false
            }))
            .unwrap(),
        )
        .unwrap();
        for (number, category) in [2_u8, 2, 2, 2, 2, 2, 2, 2, 4, 4].into_iter().enumerate() {
            let game = |candidate_color: &str, loser: &str| {
                serde_json::json!({
                    "candidate_color": candidate_color,
                    "wall_time_ns": 2_000_000_000_u64,
                    "candidate_cpu_time_ns": 250_000_000_u64,
                    "baseline_cpu_time_ns": 250_000_000_u64,
                    "candidate_peak_rss_bytes": 100_u64,
                    "baseline_peak_rss_bytes": 200_u64,
                    "termination": {"kind": "resigned", "loser": loser}
                })
            };
            let games = if category == 2 {
                [game("black", "black"), game("white", "black")]
            } else {
                [game("black", "white"), game("white", "black")]
            };
            std::fs::write(
                run_dir
                    .join("pairs")
                    .join(format!("{:020}.json", number + 1)),
                serde_json::to_vec(&serde_json::json!({
                    "pair_number": number + 1,
                    "category": category,
                    "games": games
                }))
                .unwrap(),
            )
            .unwrap();
        }

        let complete_report = report(&run_dir).unwrap();
        assert_eq!(complete_report.pentanomial, [0, 0, 8, 0, 2]);
        assert_eq!(complete_report.valid_pairs, 10);
        assert!((complete_report.normalized_mean_score - 0.6).abs() < 1e-12);
        assert!((complete_report.normalized_pair_variance - 0.04).abs() < 1e-12);
        assert!((complete_report.standard_error - 0.04_f64.sqrt() / 10.0_f64.sqrt()).abs() < 1e-12);
        assert_eq!(complete_report.total_cpu_time_ns, Some(10_000_000_000));
        assert!((complete_report.variance_time_product.unwrap() - 40_000_000.0).abs() < 1e-6);
        assert!((complete_report.evidence_per_cpu_second.unwrap() - 0.25).abs() < 1e-12);
        assert!((complete_report.valid_pairs_per_hour - 1_800.0).abs() < 1e-12);
        assert_eq!(
            complete_report.conservative_concurrent_peak_rss_bytes,
            Some(2_400)
        );
        assert_eq!(complete_report.leaves_one_physical_core, Some(true));

        // docs/sprt.md「異常時の裁定」「測定結果の記録」: クラッシュは反則負けとして
        // Eloと異常件数へ算入する。終了済みプロセスの資源が欠測しても、その統計を
        // 失わず、欠測値へ依存する集計だけをnullとして明示する。
        let pair_path = run_dir.join("pairs/00000000000000000001.json");
        let complete_pair: serde_json::Value = read_json(&pair_path).unwrap();
        let mut missing_rss_pair = complete_pair.clone();
        missing_rss_pair["games"][0]["termination"] = serde_json::json!({
            "kind": "forfeit",
            "loser": "black",
            "reason": "crash"
        });
        missing_rss_pair["games"][0]["baseline_peak_rss_bytes"] = serde_json::Value::Null;
        std::fs::write(&pair_path, serde_json::to_vec(&missing_rss_pair).unwrap()).unwrap();

        let missing_rss_report = report(&run_dir).unwrap();
        assert_eq!(missing_rss_report.pentanomial, [0, 0, 8, 0, 2]);
        assert_eq!(missing_rss_report.engine_failures.crashes, 1);
        assert_eq!(
            missing_rss_report
                .missing_resource_observations
                .baseline_peak_rss,
            1
        );
        assert_eq!(missing_rss_report.total_cpu_time_ns, Some(10_000_000_000));
        assert!(missing_rss_report.variance_time_product.is_some());
        assert_eq!(missing_rss_report.maximum_game_peak_rss_bytes, None);
        assert_eq!(
            missing_rss_report.conservative_concurrent_peak_rss_bytes,
            None
        );
        assert_eq!(missing_rss_report.conservative_memory_fraction, None);
        let output = serde_json::to_value(&missing_rss_report).unwrap();
        assert!(output["maximum_game_peak_rss_bytes"].is_null());
        assert_eq!(
            output["missing_resource_observations"]["baseline_peak_rss"],
            1
        );

        // 同じ契約をCPU時間の欠測にも適用する。主指標の比較だけは、算出不能な値を
        // 推測せずに明示エラーとする。
        missing_rss_pair["games"][0]["baseline_cpu_time_ns"] = serde_json::Value::Null;
        std::fs::write(&pair_path, serde_json::to_vec(&missing_rss_pair).unwrap()).unwrap();
        let missing_cpu_report = report(&run_dir).unwrap();
        assert_eq!(missing_cpu_report.engine_failures.crashes, 1);
        assert_eq!(
            missing_cpu_report
                .missing_resource_observations
                .baseline_cpu_time,
            1
        );
        assert_eq!(missing_cpu_report.total_cpu_time_ns, None);
        assert_eq!(missing_cpu_report.average_cpu_time_per_valid_pair_ns, None);
        assert_eq!(missing_cpu_report.variance_time_product, None);
        assert_eq!(missing_cpu_report.evidence_per_cpu_second, None);
        let error = match compare(&missing_cpu_report, &complete_report) {
            Ok(_) => panic!("comparison must reject unavailable primary metrics"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("unavailable"));

        std::fs::write(&pair_path, serde_json::to_vec(&complete_pair).unwrap()).unwrap();

        std::fs::rename(
            run_dir.join("pairs/00000000000000000010.json"),
            run_dir.join("pairs/00000000000000000011.json"),
        )
        .unwrap();
        assert!(report(&run_dir).is_err());
        std::fs::rename(
            run_dir.join("pairs/00000000000000000011.json"),
            run_dir.join("pairs/00000000000000000010.json"),
        )
        .unwrap();

        let mut manifest: serde_json::Value = read_json(&run_dir.join("manifest.json")).unwrap();
        // CECPエンジンはUSIのThreads既定値を報告しない。1ワーカーと推測せず、
        // ワーカー数に依存する補助指標だけをnullとして他の集計を維持する。
        manifest["engine_threads"]["baseline"] = serde_json::Value::Null;
        std::fs::write(
            run_dir.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let missing_threads_report = report(&run_dir).unwrap();
        assert_eq!(missing_threads_report.pentanomial, [0, 0, 8, 0, 2]);
        assert_eq!(
            missing_threads_report.total_cpu_time_ns,
            Some(10_000_000_000)
        );
        assert_eq!(missing_threads_report.maximum_engine_threads, None);
        assert_eq!(missing_threads_report.leaves_one_physical_core, None);
        assert!(!missing_threads_report.missing_engine_threads.candidate);
        assert!(missing_threads_report.missing_engine_threads.baseline);
        let output = serde_json::to_value(&missing_threads_report).unwrap();
        assert!(output["maximum_engine_threads"].is_null());
        assert_eq!(output["missing_engine_threads"]["baseline"], true);

        manifest["engine_threads"]["baseline"] = serde_json::json!(1);
        manifest["mode"] = serde_json::json!({
            "kind": "gsprt",
            "h0_elo": 0.0,
            "h1_elo": 5.0,
            "alpha": 0.05,
            "beta": 0.05
        });
        std::fs::write(
            run_dir.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(report(&run_dir).is_err());
        std::fs::remove_dir_all(run_dir).unwrap();
    }
}
