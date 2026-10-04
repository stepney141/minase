//! 逐次統計の取り込みと対局測定の集計表示。

use crate::scheduler::continue_after_decision;
use minase::harness::{CompletedPair, FailureCounts};
use minase::stats::{GsprtDecision, estimate_elo, gsprt_decision, gsprt_llr};
use std::{sync::atomic::AtomicBool, time::Duration};

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

#[cfg(test)]
mod tests {
    use super::*;

    // D8-HARN-13(sprt.md測定の種類と標準コマンド節): 判定の表示語彙は
    // `decision: H1`(採用)・`decision: H0`(不採用)・`decision: pending`(保留)。
    #[test]
    fn decision_labels_match_the_sprt_md_vocabulary() {
        assert_eq!(decision_text(GsprtDecision::AcceptH1), "H1");
        assert_eq!(decision_text(GsprtDecision::AcceptH0), "H0");
        assert_eq!(decision_text(GsprtDecision::Continue), "pending");
    }
}
