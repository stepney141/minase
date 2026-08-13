//! 自己対局結果の統計処理。
//!
//! LLRの計算はfishtestの`LLR_logistic`(statistic="expectation"のMLE法)と
//! 同一アルゴリズムであり、fishtest本家の参照値5件との一致を単体テストで
//! 固定している。統計的契約の全体はdocs/sprt.mdを参照。

/// 度数0の分類へ与える擬似度数。
const REGULARIZATION: f64 = 1e-3;
/// 永年方程式の探索区間を特異点から離す余白。
const SECULAR_MARGIN: f64 = 1e-9;
/// 永年方程式の二分法の停止幅。
const SECULAR_TOLERANCE: f64 = 1e-14;
/// ペンタノミアル5分類の正規化ペア得点。
const SCORES: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

/// GSPRTの下側判定境界。
pub const GSPRT_LOWER_BOUND: f64 = -2.944_438_979_166_440_3;
/// GSPRTの上側判定境界。
pub const GSPRT_UPPER_BOUND: f64 = 2.944_438_979_166_440_3;

/// (正規化ペア得点, 確率)の5分類分布。
type Pdf = [(f64, f64); 5];

/// GSPRTの判定。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GsprtDecision {
    /// 帰無仮説H0を採用する。
    AcceptH0,
    /// いずれの仮説も採用せず、標本を追加する。
    Continue,
    /// 対立仮説H1を採用する。
    AcceptH1,
}

/// 固定局数で推定したEloと95%信頼区間。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct EloEstimate {
    /// Eloの点推定値。
    pub elo: f64,
    /// 95%信頼区間の下端。
    pub lower: f64,
    /// 95%信頼区間の上端。
    pub upper: f64,
}

/// ロジスティックEloを期待スコアへ変換する。
fn logistic_score(elo: f64) -> f64 {
    1.0 / (1.0 + 10.0_f64.powf(-elo / 400.0))
}

/// 永年方程式を二分法で解く。
fn secular(pdf: &Pdf, expected: f64) -> f64 {
    let min = SCORES[0] - expected;
    let max = SCORES[4] - expected;
    let mut lower = -1.0 / max + SECULAR_MARGIN;
    let mut upper = -1.0 / min - SECULAR_MARGIN;

    while upper - lower > SECULAR_TOLERANCE {
        let middle = (lower + upper) / 2.0;
        let value = pdf
            .iter()
            .map(|&(score, probability)| {
                let shifted = score - expected;
                probability * shifted / (1.0 + middle * shifted)
            })
            .sum::<f64>();
        if value > 0.0 {
            lower = middle;
        } else if value < 0.0 {
            upper = middle;
        } else {
            return middle;
        }
    }
    (lower + upper) / 2.0
}

/// 観測分布から、指定した期待値を持つ最尤分布を求める。
fn mle_expected(pdf: &Pdf, expected: f64) -> Pdf {
    let multiplier = secular(pdf, expected);
    std::array::from_fn(|index| {
        let (score, probability) = pdf[index];
        (score, probability / (1.0 + multiplier * (score - expected)))
    })
}

/// ペンタノミアル度数を正則化して確率分布へ変換する。
fn results_to_pdf(results: &[u64; 5]) -> (f64, Pdf) {
    let frequencies = results.map(|count| {
        if count == 0 {
            REGULARIZATION
        } else {
            count as f64
        }
    });
    let count = frequencies.iter().sum::<f64>();
    let pdf = std::array::from_fn(|index| (SCORES[index], frequencies[index] / count));
    (count, pdf)
}

/// H0をelo=0、H1をelo=5とするペンタノミアルGSPRTのLLRを返す。
pub fn gsprt_llr(results: &[u64; 5]) -> f64 {
    let (count, observed) = results_to_pdf(results);
    let null_pdf = mle_expected(&observed, logistic_score(0.0));
    let alternative_pdf = mle_expected(&observed, logistic_score(5.0));
    count
        * observed
            .iter()
            .zip(null_pdf.iter().zip(alternative_pdf.iter()))
            .map(|((_, probability), ((_, null), (_, alternative)))| {
                probability * (alternative.ln() - null.ln())
            })
            .sum::<f64>()
}

/// LLRを固定境界と比較してGSPRTの判定を返す。
pub fn gsprt_decision(llr: f64) -> GsprtDecision {
    if llr >= GSPRT_UPPER_BOUND {
        GsprtDecision::AcceptH1
    } else if llr <= GSPRT_LOWER_BOUND {
        GsprtDecision::AcceptH0
    } else {
        GsprtDecision::Continue
    }
}

/// 期待スコアをロジスティックEloへ変換する。
fn score_to_elo(score: f64) -> f64 {
    if score <= 0.0 {
        f64::NEG_INFINITY
    } else if score >= 1.0 {
        f64::INFINITY
    } else {
        -400.0 * (1.0 / score - 1.0).log10()
    }
}

/// ペンタノミアル度数からEloと95%信頼区間を推定する。
pub fn estimate_elo(results: &[u64; 5]) -> EloEstimate {
    let count = results.iter().sum::<u64>();
    assert!(count > 0, "Elo estimation requires at least one pair");
    let count_f64 = count as f64;
    let mean = results
        .iter()
        .zip(SCORES)
        .map(|(&frequency, score)| frequency as f64 * score)
        .sum::<f64>()
        / count_f64;
    let variance = results
        .iter()
        .zip(SCORES)
        .map(|(&frequency, score)| frequency as f64 * (score - mean).powi(2))
        .sum::<f64>()
        / count_f64;
    let margin = 1.96 * (variance / count_f64).sqrt();

    EloEstimate {
        elo: score_to_elo(mean),
        lower: score_to_elo((mean - margin).max(0.0)),
        upper: score_to_elo((mean + margin).min(1.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::XorShift64;

    fn assert_reference(results: [u64; 5], expected: f64) {
        let actual = gsprt_llr(&results);
        assert!(
            (actual - expected).abs() <= 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn fishtest_reference_balanced_small_sample() {
        assert_reference([10, 25, 45, 25, 10], -0.042_124_179_9);
    }

    #[test]
    fn fishtest_reference_positive_sample() {
        assert_reference([30, 120, 240, 180, 30], 1.622_809_170_6);
    }

    #[test]
    fn fishtest_reference_regularized_zero_cell() {
        assert_reference([0, 3, 65, 4, 2], 0.675_550_992_5);
    }

    #[test]
    fn fishtest_reference_large_sample() {
        assert_reference([50, 150, 400, 200, 60], 1.799_329_087_7);
    }

    #[test]
    fn fishtest_reference_fishtest_scale_sample() {
        assert_reference([141, 593, 1112, 666, 158], 2.146_607_391_5);
    }

    /// 指定分布から1分類を抽出する。
    fn sample(pdf: &Pdf, rng: &mut XorShift64) -> usize {
        let draw = (rng.next() >> 11) as f64 / (1_u64 << 53) as f64;
        let mut cumulative = 0.0;
        for (index, &(_, probability)) in pdf.iter().enumerate() {
            cumulative += probability;
            if draw < cumulative {
                return index;
            }
        }
        4
    }

    /// 指定分布について1000ペア上限のGSPRTを反復する。
    fn simulate(pdf: &Pdf, repetitions: usize, seed: u64) -> [usize; 3] {
        let mut rng = XorShift64::new(seed);
        let mut decisions = [0; 3];
        for _ in 0..repetitions {
            let mut results = [0; 5];
            let mut decision = GsprtDecision::Continue;
            for _ in 0..1000 {
                results[sample(pdf, &mut rng)] += 1;
                decision = gsprt_decision(gsprt_llr(&results));
                if decision != GsprtDecision::Continue {
                    break;
                }
            }
            let index = match decision {
                GsprtDecision::AcceptH0 => 0,
                GsprtDecision::Continue => 1,
                GsprtDecision::AcceptH1 => 2,
            };
            decisions[index] += 1;
        }
        decisions
    }

    #[test]
    fn gsprt_monte_carlo_error_rates_are_sane() {
        const REPETITIONS: usize = 3000;
        let base = [
            (0.0, 0.001),
            (0.25, 0.004),
            (0.5, 0.99),
            (0.75, 0.004),
            (1.0, 0.001),
        ];
        let null_pdf = mle_expected(&base, logistic_score(0.0));
        let alternative_pdf = mle_expected(&base, logistic_score(5.0));

        let null_decisions = simulate(&null_pdf, REPETITIONS, 0x38D4_0A61_D7A2_7E5B);
        let alternative_decisions = simulate(&alternative_pdf, REPETITIONS, 0xA593_82C4_2F16_D8E1);

        assert!(
            null_decisions[2] * 100 <= REPETITIONS * 8,
            "H0でのH1誤採用が多すぎる: {null_decisions:?}"
        );
        assert!(
            alternative_decisions[2] * 100 >= REPETITIONS * 90,
            "H1の検出率が低すぎる: {alternative_decisions:?}"
        );
    }
}
