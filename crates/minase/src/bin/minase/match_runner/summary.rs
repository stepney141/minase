//! 逐次統計の取り込みと対局測定の集計表示。

use super::storage::ManifestMode;
use crate::match_runner::scheduler::continue_after_decision;
use minase::harness::{CompletedPair, FailureCounts};
use minase::stats::{GsprtDecision, estimate_elo, gsprt_decision, gsprt_llr_with_hypotheses};
use std::{sync::atomic::AtomicBool, time::Duration};

impl ManifestMode {
    fn llr(&self, results: &[u64; 5]) -> f64 {
        match self {
            Self::Gsprt { h0_elo, h1_elo, .. } => {
                gsprt_llr_with_hypotheses(results, *h0_elo, *h1_elo)
            }
            Self::Elo => 0.0,
        }
    }

    fn decision(&self, llr: f64) -> GsprtDecision {
        match self {
            Self::Gsprt { alpha, beta, .. } => gsprt_decision(llr, *alpha, *beta),
            Self::Elo => GsprtDecision::Continue,
        }
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
pub(super) fn print_gsprt_summary(
    results: &[u64; 5],
    discarded_pairs: u64,
    failures: FailureCounts,
    decision: GsprtDecision,
    mode: &ManifestMode,
    elapsed: Duration,
) {
    println!(
        "summary: mode=gsprt pairs={} valid_pairs={} discarded_pairs={discarded_pairs}",
        results.iter().sum::<u64>() + discarded_pairs,
        results.iter().sum::<u64>()
    );
    println!("pentanomial: {results:?}");
    println!("llr: {:.10}", mode.llr(results));
    println!("decision: {}", decision_text(decision));
    print_failure_summary(failures);
    println!("elapsed: {:.6} s", elapsed.as_secs_f64());
}

/// 固定局数Eloの最終集計を表示する。
pub(super) fn print_elo_summary(
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
pub(super) fn integrate_pair_statistics(
    pair: CompletedPair,
    results: &mut [u64; 5],
    valid_pairs: &mut u64,
    discarded_pairs: &mut u64,
    failures: &mut FailureCounts,
    decision: &mut GsprtDecision,
    mode: &ManifestMode,
    stop: &AtomicBool,
) -> bool {
    print!("{}", pair.output);
    failures.add(pair.result.failures);
    match pair.result.category {
        Some(category) => {
            results[category] += 1;
            *valid_pairs += 1;
            if matches!(mode, ManifestMode::Gsprt { .. }) {
                let llr = mode.llr(results);
                *decision = mode.decision(llr);
                println!(
                    "statistics: valid_pairs={valid_pairs} pentanomial={results:?} llr={llr:.10} decision={}",
                    decision_text(*decision)
                );
            }
        }
        None => *discarded_pairs += 1,
    }
    continue_after_decision(matches!(mode, ManifestMode::Gsprt { .. }), *decision, stop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_gsprt_hypotheses_and_error_rates_control_statistics() {
        use crate::match_runner::cli::Arguments;
        use clap::Parser;

        let boundary = 19.0_f64.ln();
        // 対称な度数と±5 EloではLLR=0。0/5 Eloの値はstats.rsと同じfishtest参照値。
        // 非対称な度数の±1.213888438138128は、多項尤度の制約付き最大化から独立に計算した。
        // p_i=n_i/(λ+μs_i)、Σp_i=1、Σs_i p_i=s(Elo)の2方程式を65桁の十進演算で解き、
        // Σn_i ln(p_i(H1)/p_i(H0))を求めた。度数を反転すると符号も反転する。
        for (alpha, beta, hypotheses, counts, expected_llr, llr, expected) in [
            (
                0.05,
                0.05,
                ["-5", "5"],
                [10, 20, 40, 20, 10],
                0.0,
                boundary + 1e-10,
                GsprtDecision::AcceptH1,
            ),
            (
                0.05,
                0.05,
                ["-5", "5"],
                [10, 20, 40, 20, 10],
                0.0,
                boundary - 1e-10,
                GsprtDecision::Continue,
            ),
            (
                0.05,
                0.05,
                ["-5", "5"],
                [10, 20, 40, 20, 10],
                0.0,
                -boundary - 1e-10,
                GsprtDecision::AcceptH0,
            ),
            (
                0.05,
                0.05,
                ["-5", "5"],
                [10, 20, 40, 20, 10],
                0.0,
                -boundary + 1e-10,
                GsprtDecision::Continue,
            ),
            (
                0.05,
                0.05,
                ["-5", "5"],
                [10, 20, 40, 30, 20],
                1.213_888_438_138_128,
                1.213_888_438_138_128,
                GsprtDecision::Continue,
            ),
            (
                0.05,
                0.05,
                ["-5", "5"],
                [20, 30, 40, 20, 10],
                -1.213_888_438_138_128,
                -1.213_888_438_138_128,
                GsprtDecision::Continue,
            ),
            (
                0.1,
                0.2,
                ["-5", "5"],
                [10, 20, 40, 20, 10],
                0.0,
                2.1,
                GsprtDecision::AcceptH1,
            ),
            (
                0.1,
                0.2,
                ["-5", "5"],
                [10, 20, 40, 20, 10],
                0.0,
                -1.6,
                GsprtDecision::AcceptH0,
            ),
            (
                0.1,
                0.2,
                ["-5", "5"],
                [10, 20, 40, 20, 10],
                0.0,
                0.0,
                GsprtDecision::Continue,
            ),
            (
                0.05,
                0.05,
                ["0", "5"],
                [30, 120, 240, 180, 30],
                1.622_809_170_6,
                0.0,
                GsprtDecision::Continue,
            ),
        ] {
            let arguments = Arguments::try_parse_from([
                "match_runner",
                "--run-dir",
                "run",
                "gsprt",
                "--elo0",
                hypotheses[0],
                "--elo1",
                hypotheses[1],
            ])
            .unwrap();
            let (_, mode) = arguments.mode.unwrap().new_run_settings().unwrap();
            let mut value = serde_json::to_value(mode).unwrap();
            value["alpha"] = serde_json::json!(alpha);
            value["beta"] = serde_json::json!(beta);
            let recorded: ManifestMode = serde_json::from_value(value).unwrap();
            let case = format!("α={alpha}, β={beta}, hypotheses={hypotheses:?}, LLR={llr}");
            let tolerance = if expected_llr == 0.0 { 1e-10 } else { 1e-6 };
            assert!(
                (recorded.llr(&counts) - expected_llr).abs() < tolerance,
                "{case}"
            );
            assert!(
                (recorded.llr(&counts) - minase::stats::gsprt_llr(&counts)).abs() > 1e-6,
                "{case}"
            );
            assert_eq!(recorded.decision(llr), expected, "{case}");
        }
    }

    // docs/guides/sprt.md測定の種類と標準コマンド節に従い、判定の表示語彙は
    // `decision: H1`(採用)・`decision: H0`(不採用)・`decision: pending`(保留)。
    #[test]
    fn decision_labels_match_the_sprt_md_vocabulary() {
        assert_eq!(decision_text(GsprtDecision::AcceptH1), "H1");
        assert_eq!(decision_text(GsprtDecision::AcceptH0), "H0");
        assert_eq!(decision_text(GsprtDecision::Continue), "pending");
    }
}
