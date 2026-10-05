//! 逐次統計の取り込みと対局測定の集計表示。

use super::storage::ManifestMode;
use crate::match_runner::scheduler::continue_after_decision;
use minase::harness::{CompletedPair, FailureCounts};
use minase::stats::{GsprtDecision, estimate_elo, gsprt_llr_with_hypotheses};
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
        if let Self::Gsprt { alpha, beta, .. } = self {
            if llr >= ((1.0 - beta) / alpha).ln() {
                return GsprtDecision::AcceptH1;
            }
            if llr <= (beta / (1.0 - alpha)).ln() {
                return GsprtDecision::AcceptH0;
            }
        }
        GsprtDecision::Continue
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
        let mode: ManifestMode = serde_json::from_value(serde_json::json!({
            "kind": "gsprt", "h0_elo": -5.0, "h1_elo": 5.0, "alpha": 0.1, "beta": 0.2
        }))
        .unwrap();
        // 対称な度数と±5 Eloの仮説では尤度が等しく、LLRは0になる。
        assert!(mode.llr(&[10, 20, 40, 20, 10]).abs() < 1e-10);
        assert_eq!(mode.decision(2.1), GsprtDecision::AcceptH1);
        assert_eq!(mode.decision(-1.6), GsprtDecision::AcceptH0);
        assert_eq!(mode.decision(0.0), GsprtDecision::Continue);
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
