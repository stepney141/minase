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

    // D8-STAT-01(sprt.md統計的手続き節): fishtest本家LLRcalc.pyで事前計算した
    // 参照値5件との1e-6以内の一致が外部契約である。この5組は旧テストから
    // 引き継ぐ再取得困難な資産であり、絶対に失ってはならない。
    #[test]
    fn fishtest_reference_values_match_within_1e_6() {
        let references: [([u64; 5], f64); 5] = [
            ([10, 25, 45, 25, 10], -0.042_124_179_9),
            ([30, 120, 240, 180, 30], 1.622_809_170_6),
            // 度数0のセルを含み、fishtestと同一の正則化経路との一致を検証する
            ([0, 3, 65, 4, 2], 0.675_550_992_5),
            ([50, 150, 400, 200, 60], 1.799_329_087_7),
            ([141, 593, 1112, 666, 158], 2.146_607_391_5),
        ];
        for (results, expected) in references {
            let actual = gsprt_llr(&results);
            assert!(
                (actual - expected).abs() <= 1e-6,
                "pentanomial {results:?}: expected {expected}, got {actual}"
            );
        }
    }

    // D8-STAT-01性質: 度数の左右反転(候補と基準の入替に相当)でLLRの符号傾向が
    // 反転する。仮説がelo 0対5で非対称なため厳密な反対称性は要求しない。
    #[test]
    fn mirrored_frequencies_flip_llr_sign_tendency() {
        let positive = [141, 593, 1112, 666, 158];
        let mirrored = [158, 666, 1112, 593, 141];
        assert!(gsprt_llr(&positive) > 0.0);
        assert!(gsprt_llr(&mirrored) < 0.0);
    }

    // D8-STAT-02(sprt.md統計的手続き節): 判定境界はα=β=0.05から
    // log((1-β)/α)と log(β/(1-α))で定まる。式からの導出値と照合し、
    // 実装定数を写さない。
    #[test]
    fn gsprt_bounds_equal_log_error_rate_ratios() {
        let upper = (0.95_f64 / 0.05_f64).ln();
        let lower = (0.05_f64 / 0.95_f64).ln();
        assert!((GSPRT_UPPER_BOUND - upper).abs() < 1e-12);
        assert!((GSPRT_LOWER_BOUND - lower).abs() < 1e-12);
    }

    // D8-STAT-02: 判定は3値(H1/H0/継続)で網羅的・排他的。上側境界超過でH1、
    // 下側境界超過でH0、境界内は継続。境界同値の比較演算はSPEC_UNCLEAR-01に
    // つき検証しない。
    #[test]
    fn gsprt_decision_is_three_way_and_exhaustive() {
        assert_eq!(
            gsprt_decision(GSPRT_UPPER_BOUND + 1e-9),
            GsprtDecision::AcceptH1
        );
        assert_eq!(
            gsprt_decision(GSPRT_LOWER_BOUND - 1e-9),
            GsprtDecision::AcceptH0
        );
        assert_eq!(gsprt_decision(0.0), GsprtDecision::Continue);
        assert_eq!(gsprt_decision(2.9), GsprtDecision::Continue);
        assert_eq!(gsprt_decision(-2.9), GsprtDecision::Continue);
    }

    // D8-STAT-03(sprt.md統計的手続き節): ロジスティック写像
    // s = 1/(1+10^(-elo/400))。期待値はすべて式から独立に導出した値である。
    #[test]
    fn logistic_elo_mapping_matches_documented_formula() {
        assert!((logistic_score(0.0) - 0.5).abs() < 1e-15);
        // s(5) = 1/(1+10^(-1/80)) = 0.5071950817…
        assert!((logistic_score(5.0) - 0.507_195_081_709_051_4).abs() < 1e-12);
        // 境界事例: elo=±400でs=1/(1+10^∓1)
        assert!((logistic_score(400.0) - 10.0 / 11.0).abs() < 1e-12);
        assert!((logistic_score(-400.0) - 1.0 / 11.0).abs() < 1e-12);
        // 性質: 狭義単調増加と点対称 s(-x) = 1 - s(x)
        assert!(logistic_score(0.0) < logistic_score(5.0));
        assert!(logistic_score(5.0) < logistic_score(400.0));
        for elo in [1.0, 5.0, 123.0, 400.0] {
            assert!((logistic_score(-elo) - (1.0 - logistic_score(elo))).abs() < 1e-12);
        }
    }

    // D8-STAT-07(sprt.md統計的手続き節): 完全対称な度数は平均正規化ペア得点0.5
    // に対応し、Elo点推定は0、95%信頼区間は0と点推定を含む。
    #[test]
    fn symmetric_frequencies_estimate_zero_elo_with_interval_containing_zero() {
        let estimate = estimate_elo(&[10, 25, 45, 25, 10]);
        assert!(estimate.elo.abs() < 1e-12);
        assert!(estimate.lower <= 0.0 && 0.0 <= estimate.upper);
        assert!(estimate.lower <= estimate.elo && estimate.elo <= estimate.upper);
    }

    // sprt.md固定局数Elo節: 95%信頼区間は、ペンタノミアル度数の平均と分散による
    // 正規近似 mean ± 1.96·sqrt(var/n) をロジスティック写像でEloへ写した区間である。
    // 期待値は度数[10,25,45,25,10](n=115、平均0.5)から式で独立に導出した。
    // 変異検証(フェーズ4)で検出したz値無検証の補強。
    #[test]
    fn elo_confidence_interval_uses_the_95_percent_normal_margin() {
        let estimate = estimate_elo(&[10, 25, 45, 25, 10]);
        let count: f64 = 115.0;
        let variance = (10.0 * 0.25 + 25.0 * 0.0625 + 25.0 * 0.0625 + 10.0 * 0.25) / count;
        let margin = 1.96 * (variance / count).sqrt();
        assert!((estimate.lower - score_to_elo(0.5 - margin)).abs() < 1e-9);
        assert!((estimate.upper - score_to_elo(0.5 + margin)).abs() < 1e-9);
    }

    // D8-STAT-07/D8-STAT-03: 点推定はロジスティック写像の逆写像に一致する。
    // 平均正規化ペア得点0.75の度数に対し、elo = -400*log10(1/0.75-1)
    // = 400*log10(3) = 190.8485018879…(式から独立に導出)。
    #[test]
    fn elo_point_estimate_inverts_the_logistic_mapping() {
        let estimate = estimate_elo(&[0, 0, 1, 0, 1]);
        assert!((estimate.elo - 190.848_501_887_864_98).abs() < 1e-9);
    }

    // D8-STAT-07性質: 度数の左右反転(候補と基準の入替)でElo点推定の符号が
    // 反転する(ロジスティック写像の点対称から従う)。
    #[test]
    fn mirrored_frequencies_flip_elo_sign() {
        let forward = estimate_elo(&[1, 2, 4, 3, 1]);
        let mirrored = estimate_elo(&[1, 3, 4, 2, 1]);
        assert!((forward.elo + mirrored.elo).abs() < 1e-9);
    }

    // D8-STAT-07性質: 同一分布のスケーリング(度数の定数倍)で点推定は不変、
    // 信頼区間は狭くなる(観測単位はペアであり、ペア数増で分散推定が締まる)。
    #[test]
    fn scaling_frequencies_preserves_estimate_and_narrows_interval() {
        let small = estimate_elo(&[1, 2, 4, 3, 1]);
        let large = estimate_elo(&[10, 20, 40, 30, 10]);
        assert!((small.elo - large.elo).abs() < 1e-9);
        assert!(large.upper - large.lower < small.upper - small.lower);
        assert!(small.lower <= small.elo && small.elo <= small.upper);
    }

    // D8-STAT-07境界(sprt.md進捗記録の固定局数Elo節): 全勝・全敗の標本では
    // ロジスティックEloの点推定が無限大になる。出力表現の細部は
    // SPEC_UNCLEAR-03につき、無限大の符号だけを契約とする。
    #[test]
    fn perfect_and_zero_scores_yield_infinite_estimates() {
        assert_eq!(estimate_elo(&[0, 0, 0, 0, 50]).elo, f64::INFINITY);
        assert_eq!(estimate_elo(&[50, 0, 0, 0, 0]).elo, f64::NEG_INFINITY);
    }

    /// 合成した5分類分布から1分類を抽出する(モンテカルロ用の検定入力生成器)。
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

    /// 指定分布について上限つきGSPRTを反復し、[H0, 継続, H1]の件数を返す。
    fn simulate(pdf: &Pdf, repetitions: usize, max_pairs: usize, seed: u64) -> [usize; 3] {
        let mut rng = XorShift64::new(seed);
        let mut decisions = [0; 3];
        for _ in 0..repetitions {
            let mut results = [0; 5];
            let mut decision = GsprtDecision::Continue;
            // sprt.md: 1ペア取り込むごとにLLRを再計算し、境界交差で停止する
            for _ in 0..max_pairs {
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

    // D8-STAT-05(search.md測定基盤・実施状況): 既知の分布から合成した結果で
    // 検定全体を3,000反復し、真のelo=0でH1誤採用率8%以下(名目α=0.05と整合)、
    // 真のelo=5で検出率90%以上(名目1-β=0.95と整合)を確認する。
    // 固定シードにより決定的である。
    #[test]
    fn gsprt_monte_carlo_error_rates_match_documented_thresholds() {
        const REPETITIONS: usize = 3000;
        // 得点0.5のセルから隣接セルへ移す確率質量。真のelo=5の分布は平均
        // 正規化ペア得点がs(5)になるよう、0.75のセルへ4*(s(5)-0.5) =
        // 0.0287803268…を移して構成する(sprt.mdの写像式から独立に導出)。
        const SHIFT: f64 = 0.028_780_326_836_205_46;
        // 真のelo=0: 平均0.5の対称分布
        let null_pdf: Pdf = [
            (0.0, 0.0),
            (0.25, SHIFT / 2.0),
            (0.5, 1.0 - SHIFT),
            (0.75, SHIFT / 2.0),
            (1.0, 0.0),
        ];
        // 真のelo=5: 平均 0.5 + 0.25*SHIFT = s(5)
        let alternative_pdf: Pdf = [
            (0.0, 0.0),
            (0.25, 0.0),
            (0.5, 1.0 - SHIFT),
            (0.75, SHIFT),
            (1.0, 0.0),
        ];

        let null_decisions = simulate(&null_pdf, REPETITIONS, 5000, 0x38D4_0A61_D7A2_7E5B);
        let alternative_decisions =
            simulate(&alternative_pdf, REPETITIONS, 5000, 0xA593_82C4_2F16_D8E1);

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
