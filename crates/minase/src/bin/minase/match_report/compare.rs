//! 2つの時間制御の校正指標を比較する。

use crate::match_report::report::Report;
use serde::Serialize;
use std::io;

/// 現行条件に対する候補条件の指標比。
#[derive(Serialize)]
pub(super) struct Comparison {
    variance_time_reduction_percent: f64,
    evidence_per_cpu_ratio: f64,
}

/// 比較条件とペア番号列が完全に一致する2実行の主指標を比較する。
///
/// どちらかの主指標が`null`、または基準側の主指標が0なら拒否する。
/// `variance_time_reduction_percent`は`100 * (1 - 候補の分散時間積 / 基準の分散時間積)`である。
/// `evidence_per_cpu_ratio`は候補のCPU秒あたりの証拠を基準の値で割った比である。
pub(super) fn compare(candidate: &Report, current: &Report) -> io::Result<Comparison> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_report::report::{FailureCounts, MissingEngineThreads, MissingResourceCounts};

    #[test]
    fn comparison_computes_metric_ratios_and_rejects_different_manifests() {
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
        assert_eq!(
            compare(&candidate, &current).err().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
        candidate.comparison_manifest = current.comparison_manifest.clone();
        candidate.pair_numbers = vec![2];
        assert_eq!(
            compare(&candidate, &current).err().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
        candidate.pair_numbers = vec![1];
        for current in [make(0.0, 4.0), make(10.0, 0.0)] {
            assert_eq!(
                compare(&candidate, &current).err().unwrap().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }
}
