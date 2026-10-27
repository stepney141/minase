//! 対局測定の実行条件と同時対局数の構築。

use crate::match_runner::cli::RuleSetArgument;
use crate::match_runner::storage::{
    EngineHashSizes, EngineRecord, EngineThreadCounts, FORMAT_VERSION, ManifestMode, RunManifest,
    StoredSearchLimit,
};
use minase::harness::*;
use std::{io, time::Duration};

/// 探索制限を実行条件記録へ変換する。
const fn stored_search_limit(limit: SearchLimit) -> StoredSearchLimit {
    match limit {
        SearchLimit::Fixed { depth, nodes } => StoredSearchLimit::Fixed { depth, nodes },
        SearchLimit::Time(time) => StoredSearchLimit::Time {
            base_ms: time.base_ms,
            increment_ms: time.increment_ms,
            byoyomi_ms: time.byoyomi_ms,
        },
        SearchLimit::Movetime {
            movetime_ms,
            timemargin_ms,
        } => StoredSearchLimit::Movetime {
            movetime_ms: movetime_ms.get(),
            timemargin_ms,
        },
    }
}

/// 同時対局数の既定値を物理コア数と両エンジンの`Threads`から求める。
///
/// 物理コア数から1を引き、両側の`Threads`の大きい方で整数除算する。
/// 先読みを含む場合はさらに2で割る。
/// コア数かいずれかの`Threads`が不明、`Threads`が0または`usize`に収まらない、
/// あるいは商が1未満の場合は、`--concurrency`の明示を求めるエラーを返す。
/// 解決値は`manifest.json`に保存し、再開でもその値を使う。
/// 理由は`docs/adr/derive-default-concurrency-from-physical-cores.md`にある。
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

/// 新規実行の条件を構成する。
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
            options: candidate.options.clone(),
        },
        baseline: EngineRecord {
            identity: baseline.identity.clone(),
            limit: stored_search_limit(baseline.limit),
            options: baseline.options.clone(),
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_runner::cli::parse_rule_set_argument;

    // docs/plans/archived/movetime-deadline.md: 固定時間は版を変えずに専用のkindと2値で保存する。
    #[test]
    fn movetime_manifest_serializes_explicit_duration_and_margin() {
        let limit = parse_search_limit("movetime=300,timemargin=100").unwrap();
        let stored = stored_search_limit(limit);
        let json = serde_json::to_value(&stored).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "movetime", "movetime_ms": 300, "timemargin_ms": 100})
        );
        assert_eq!(
            serde_json::from_value::<StoredSearchLimit>(json).unwrap(),
            stored
        );
    }

    // default_concurrencyのdocコメントと物理コア数からの既定値に関するADRに従う。
    #[test]
    fn default_concurrency_uses_available_resources_or_requires_an_override() {
        for (cores, candidate, baseline, ponder, expected) in [
            (Some(20), Some(1), Some(1), false, Ok(19)),
            (Some(20), Some(4), Some(2), false, Ok(4)),
            (None, Some(1), Some(1), false, Err("physical core count")),
            (
                Some(20),
                None,
                Some(1),
                false,
                Err("candidate engine Threads"),
            ),
            (
                Some(20),
                Some(1),
                None,
                false,
                Err("baseline engine Threads"),
            ),
            (
                Some(1),
                Some(1),
                Some(1),
                false,
                Err("automatic concurrency is less than 1"),
            ),
            (
                Some(20),
                Some(0),
                Some(1),
                false,
                Err("Threads must be at least 1"),
            ),
            (
                Some(20),
                Some(1),
                Some(0),
                false,
                Err("Threads must be at least 1"),
            ),
            (Some(16), Some(1), Some(1), true, Ok(7)),
            (Some(16), Some(1), Some(1), false, Ok(15)),
            (Some(16), Some(2), Some(1), true, Ok(3)),
            (Some(16), Some(1), Some(2), true, Ok(3)),
            (Some(16), Some(2), Some(1), false, Ok(7)),
            (Some(16), Some(1), Some(2), false, Ok(7)),
            (
                Some(2),
                Some(1),
                Some(1),
                true,
                Err("automatic concurrency is less than 1"),
            ),
        ] {
            let case =
                format!("cores={cores:?}, Threads=({candidate:?}, {baseline:?}), ponder={ponder}");
            let actual = default_concurrency(cores, candidate, baseline, ponder);
            match expected {
                Ok(value) => assert_eq!(actual, Ok(value), "{case}"),
                Err(message) => {
                    let error = actual.unwrap_err();
                    assert!(error.contains(message), "{case}: {error}");
                    assert!(error.contains("--concurrency"), "{case}: {error}");
                }
            }
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
                    "engine-default",
                    Vec::new(),
                    None,
                )
                .unwrap()
            };
            let manifest = run_manifest(
                &player(candidate_hash),
                &player(baseline_hash),
                &parse_rule_set_argument("engine-default").unwrap(),
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
}
