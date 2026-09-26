//! 対局測定の実行条件と同時対局数の構築。

use crate::cli::RuleSetArgument;
use crate::storage::{
    EngineHashSizes, EngineRecord, EngineThreadCounts, FORMAT_VERSION, ManifestMode, RunManifest,
    StoredSearchLimit,
};
use minase::RuleCode;
use minase::harness::*;
use std::{io, thread, time::Duration};

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

/// CLIから再開時に完全一致させる実行条件記録を構成する。
#[allow(clippy::too_many_arguments)]
pub(super) fn run_manifest(
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
pub(super) fn rules_text(codes: &[RuleCode]) -> String {
    codes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::parse_rule_set_argument;

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

    // D8-HARN-27（ponder.md設計判断「同時対局数」）。
    #[test]
    fn ponder_concurrency_reserves_two_engines_per_game() {
        for (threads, on, off) in [(1, 7, 15), (2, 3, 7)] {
            for (a, b) in [(threads, 1), (1, threads)] {
                assert_eq!(
                    default_concurrency(Some(16), Some(a), Some(b), true),
                    Ok(on)
                );
                assert_eq!(
                    default_concurrency(Some(16), Some(a), Some(b), false),
                    Ok(off)
                );
            }
        }
        for (cores, a, b) in [
            (Some(2), Some(1), Some(1)),
            (None, Some(1), Some(1)),
            (Some(16), None, Some(1)),
            (Some(16), Some(1), None),
        ] {
            assert!(default_concurrency(cores, a, b, true).is_err());
        }
    }
}
