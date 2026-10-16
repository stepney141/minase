use serde::Serialize;

use super::{Error, Result};

/// 有限な0以上1以下の教師の混合比を許す。
pub fn validate_lambda(value: f64) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(Error::TeacherLambda);
    }
    Ok(())
}

/// 探索値、分類番号、結果の配列から教師値を構築する。すべて局面順で同じ長さを要求する。
/// 分類ごとの教師の勝率尺度（1ロジットあたりのcp）を`teacher_cp_per_logit`で指定する。
/// `λ / (1 + exp(-score / teacher_cp_per_logit)) + (1 - λ) * result / 2`をf64で
/// 計算してf32へ丸める。λが0の分類では教師の勝率尺度と探索値を参照しない。
/// 尺度補正係数を`f = cp_per_logit / teacher_cp_per_logit`とすると、実数では
/// `σ(score / teacher_cp_per_logit) = σ((score × f) / cp_per_logit)`である。
/// 丸めを保つため補正係数は記録だけに使い、教師値の計算式には掛けない。
pub fn build_targets_from_arrays(
    scores: &[f32],
    classes: &[u32],
    outcomes: &[u8],
    teacher_cp_per_logit: &[f64],
    teacher_lambdas: &[f64],
) -> Result<Vec<f32>> {
    if teacher_cp_per_logit.len() != teacher_lambdas.len()
        || scores.len() != classes.len()
        || scores.len() != outcomes.len()
        || outcomes.iter().any(|&outcome| outcome > 2)
    {
        return Err(Error::TeacherTargets);
    }
    for &mix in teacher_lambdas {
        validate_lambda(mix)?;
    }
    classes
        .iter()
        .enumerate()
        .map(|(position, &class)| {
            let class = class as usize;
            let mix = *teacher_lambdas.get(class).ok_or(Error::TeacherTargets)?;
            let result = f64::from(outcomes[position]) / 2.0;
            if mix == 0.0 {
                return Ok(result as f32);
            }
            let cp_per_logit = teacher_cp_per_logit[class];
            let score = f64::from(scores[position]);
            if !cp_per_logit.is_finite() || cp_per_logit <= 0.0 || !score.is_finite() {
                return Err(Error::TeacherTargets);
            }
            Ok((mix / (1.0 + (-score / cp_per_logit).exp()) + (1.0 - mix) * result) as f32)
        })
        .collect()
}

/// 検査済みの先読み条件。同じ対局の将来の探索値だけを使う。
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Lookahead {
    gamma: f64,
    plies: u16,
}

impl Lookahead {
    /// 有限な`0 < gamma < 1`と`1 <= plies <= 4096`を検証する。
    pub fn new(gamma: f64, plies: u16) -> Result<Self> {
        if !gamma.is_finite() || !(0.0 < gamma && gamma < 1.0) || !(1..=4096).contains(&plies) {
            return Err(Error::LookaheadParameters);
        }
        Ok(Self { gamma, plies })
    }

    /// 手数差1あたりの減衰率を返す。
    pub fn gamma(self) -> f64 {
        self.gamma
    }

    /// 先読み窓の手数を返す。
    pub fn plies(self) -> u16 {
        self.plies
    }
}

/// 同じ対局の将来の探索値から先読み値をf64で求める。
/// 局面tからの手数差δが1〜nの記録だけを使い、`Σ γ^(δ−δ₁)×(−1)^δ×v / Σ γ^(δ−δ₁)`を手数順に計算する。
/// δ₁は窓内の最小の手数差であり、通常の重み`γ^(δ−1)`から共通因子を除いても正規化後の値は同じである。
/// 奇数差の符号反転で局面tの手番側へ視点をそろえ、局面t自身と記録されていない手数は和に含めない。
/// 窓が空なら元の探索値を保つ。
/// 入力は対局番号の非減少順と同一対局内の手数の厳密増加順を要求し、違反は`RecordOrder`エラーにする。
/// 採用する教師の条件はγ=0.9、n=40である。
/// 理由は`docs/adr/use-lookahead-teacher-values.md`にある。
pub fn compute_lookahead(
    games: &[u32],
    plies: &[u16],
    scores: &[f64],
    options: Lookahead,
) -> Result<Vec<f64>> {
    if games.len() != plies.len()
        || games.len() != scores.len()
        || scores.iter().any(|value| !value.is_finite())
    {
        return Err(Error::LookaheadSamples);
    }
    for index in 1..games.len() {
        if games[index] < games[index - 1]
            || (games[index] == games[index - 1] && plies[index] <= plies[index - 1])
        {
            return Err(Error::RecordOrder { index });
        }
    }
    let mut result = scores.to_vec();
    for index in 0..games.len().saturating_sub(1) {
        if games[index + 1] != games[index] {
            continue;
        }
        let first_delta = plies[index + 1] - plies[index];
        let mut numerator = 0.0;
        let mut denominator = 0.0;
        for future in index + 1..games.len() {
            if games[future] != games[index] {
                break;
            }
            let delta = plies[future] - plies[index];
            if delta > options.plies {
                break;
            }
            let weight = options.gamma.powf(f64::from(delta - first_delta));
            let sign = if delta.is_multiple_of(2) { 1.0 } else { -1.0 };
            numerator += weight * sign * scores[future];
            denominator += weight;
        }
        if denominator > 0.0 {
            result[index] = numerator / denominator;
        }
    }
    Ok(result)
}

fn binary_cross_entropy_sum(scores: &[f64], results: &[u8], cp_per_logit: f64) -> f64 {
    scores
        .iter()
        .zip(results)
        .map(|(&score, &result)| {
            let logit = score / cp_per_logit;
            // np.logaddexp(0, logit)の安定な評価式。
            logit.max(0.0) + (-logit.abs()).exp().ln_1p() - f64::from(result) / 2.0 * logit
        })
        .sum()
}

/// 教師の勝率尺度の推定に黄金分割探索を64回使う。
///
/// `[50, 4000]`で、探索値から終局結果を予測する二値交差エントロピーを最小化する。
/// `results`は手番側の負け0、引き分け1、勝ち2。計算はすべてf64で行う。
pub fn estimate_cp_per_logit(scores: &[f64], results: &[u8]) -> Result<CpPerLogitEstimate> {
    if scores.is_empty()
        || scores.len() != results.len()
        || scores.iter().any(|value| !value.is_finite())
        || results.iter().any(|&value| value > 2)
    {
        return Err(Error::TeacherSamples);
    }
    if scores.iter().all(|&value| value == 0.0) {
        return Err(Error::UndeterminedScale);
    }
    let (mut lower, mut upper) = (50.0, 4000.0);
    let ratio = (5.0_f64.sqrt() - 1.0) / 2.0;
    let mut left = upper - ratio * (upper - lower);
    let mut right = lower + ratio * (upper - lower);
    let mut left_loss = binary_cross_entropy_sum(scores, results, left);
    let mut right_loss = binary_cross_entropy_sum(scores, results, right);
    for _ in 0..64 {
        if left_loss <= right_loss {
            upper = right;
            right = left;
            right_loss = left_loss;
            left = upper - ratio * (upper - lower);
            left_loss = binary_cross_entropy_sum(scores, results, left);
        } else {
            lower = left;
            left = right;
            left_loss = right_loss;
            right = lower + ratio * (upper - lower);
            right_loss = binary_cross_entropy_sum(scores, results, right);
        }
    }
    Ok(CpPerLogitEstimate {
        value: (lower + upper) / 2.0,
        at_search_bound: lower == 50.0 || upper == 4000.0,
    })
}

/// 黄金分割探索で推定した教師の勝率尺度と探索範囲の端への到達状況。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CpPerLogitEstimate {
    /// 教師の勝率尺度（1ロジットあたりのcp）。
    pub value: f64,
    /// 64回の反復で下限50または上限4000が一度も動かなかった場合に真となる。
    pub at_search_bound: bool,
}

/// コマンドラインのλを有限な0以上1以下に制限する。
pub fn parse_lambda(text: &str) -> std::result::Result<f64, String> {
    let value: f64 = text
        .parse()
        .map_err(|error| format!("invalid lambda: {error}"))?;
    validate_lambda(value).map_err(|error| error.to_string())?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teacher_arrays_preserve_mixing_and_ignore_inactive_scores_and_cp_per_logit() {
        // score=0でσ=1/2。λ=3/4なら結果0,1,2に対し3/8,1/2,5/8。
        // 0.75×σ(1)+0.125 = 0.6732939339725037（Python mathのf64計算）をf32へ丸める。
        let mixed = 0.6732939339725037_f64 as f32;
        for (name, scores, classes, outcomes, teacher_cp_per_logit, lambdas, expected) in [
            (
                "mixed classes",
                vec![f32::NAN, 0.0, 1000.0],
                vec![0, 1, 1],
                vec![2, 0, 1],
                vec![f64::NAN, 1000.0],
                vec![0.0, 0.75],
                vec![1.0, 0.375, mixed],
            ),
            (
                "three outcomes",
                vec![0.0; 3],
                vec![0; 3],
                vec![0, 1, 2],
                vec![1000.0],
                vec![0.75],
                vec![0.375, 0.5, 0.625],
            ),
            (
                "inactive cp_per_logit",
                vec![0.0; 3],
                vec![0; 3],
                vec![0, 1, 2],
                vec![f64::NAN],
                vec![0.0],
                vec![0.0, 0.5, 1.0],
            ),
            (
                "inactive extreme scores",
                vec![f32::MAX, -f32::MAX, f32::MAX],
                vec![0; 3],
                vec![0, 1, 2],
                vec![f64::NAN],
                vec![0.0],
                vec![0.0, 0.5, 1.0],
            ),
        ] {
            let actual = build_targets_from_arrays(
                &scores,
                &classes,
                &outcomes,
                &teacher_cp_per_logit,
                &lambdas,
            )
            .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(
                actual.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                expected.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                "{name}"
            );
        }
        for (name, scores, classes, outcomes) in [
            ("missing class", vec![0.0], vec![], vec![1]),
            ("missing outcome", vec![0.0], vec![0], vec![]),
            ("unknown class", vec![0.0], vec![2], vec![1]),
            ("invalid outcome", vec![0.0], vec![0], vec![3]),
            ("nonfinite active score", vec![f32::NAN], vec![0], vec![1]),
        ] {
            assert!(
                build_targets_from_arrays(&scores, &classes, &outcomes, &[1.0], &[1.0]).is_err(),
                "{name}"
            );
        }
        for cp_per_logit in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(
                build_targets_from_arrays(&[0.0], &[0], &[1], &[cp_per_logit], &[0.75]).is_err(),
                "cp_per_logit={cp_per_logit}"
            );
        }
        for mix in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
            assert!(
                build_targets_from_arrays(&[0.0], &[0], &[1], &[1000.0], &[mix]).is_err(),
                "lambda={mix}"
            );
        }
        for (name, teacher_cp_per_logit, lambdas) in [
            ("missing teacher classes", vec![], vec![]),
            ("missing lambda", vec![1000.0], vec![]),
        ] {
            assert!(
                build_targets_from_arrays(&[0.0], &[0], &[1], &teacher_cp_per_logit, &lambdas)
                    .is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn parse_lambda_accepts_only_finite_values_in_the_unit_interval() {
        for (text, expected) in [("0", 0.0), ("0.75", 0.75), ("1", 1.0)] {
            assert_eq!(parse_lambda(text).unwrap(), expected, "{text}");
        }
        for text in ["-0.1", "1.1", "NaN", "inf", "-inf", "", "invalid"] {
            assert!(parse_lambda(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn lookahead_matches_python_example() {
        // compute_lookaheadの式にγ=0.5、窓幅3を代入した期待値である。
        let scores = [7.0, 2.0, 1.0, 99.0, 10.0, -5.0];
        let actual = compute_lookahead(
            &[1, 1, 1, 1, 2, 2],
            &[0, 1, 3, 8, 0, 2],
            &scores,
            Lookahead::new(0.5, 3).unwrap(),
        )
        .unwrap();
        assert_eq!(actual.len(), 6);
        for (actual, expected) in actual.iter().zip([-1.8, 1.0, 1.0, 99.0, -5.0, -5.0]) {
            assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
        }
    }

    #[test]
    fn lookahead_handles_empty_extreme_and_sparse_windows() {
        assert!(
            compute_lookahead(&[], &[], &[], Lookahead::new(0.9, 40).unwrap())
                .unwrap()
                .is_empty()
        );
        for (plies, scores, gamma, window, expected) in [
            ([65534, 65535], [1.0, -32768.0], 0.9, 1, [32768.0, -32768.0]),
            ([0, 4096], [7.0, 123.0], 1e-200, 4096, [123.0, 123.0]),
            ([0, 4097], [7.0, 123.0], 0.9, 4096, [7.0, 123.0]),
        ] {
            assert_eq!(
                compute_lookahead(
                    &[1, 1],
                    &plies,
                    &scores,
                    Lookahead::new(gamma, window).unwrap()
                )
                .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn lookahead_rejects_invalid_parameters_and_order() {
        for (gamma, plies) in [
            (0.0, 40),
            (1.0, 40),
            (-0.1, 40),
            (f64::NAN, 40),
            (f64::INFINITY, 40),
            (0.9, 0),
            (0.9, 4097),
        ] {
            assert!(Lookahead::new(gamma, plies).is_err());
        }
        let options = Lookahead::new(0.9, 40).unwrap();
        for (games, plies, index) in [
            (vec![2, 1], vec![0, 1], 1),
            (vec![1, 1], vec![1, 1], 1),
            (vec![1, 1], vec![2, 1], 1),
            (vec![1, 2, 1], vec![0, 0, 1], 2),
        ] {
            assert!(
                matches!(
                    compute_lookahead(&games, &plies, &vec![0.0; games.len()], options),
                    Err(Error::RecordOrder { index: actual }) if actual == index
                ),
                "games={games:?}, plies={plies:?}"
            );
        }
        assert!(compute_lookahead(&[1], &[], &[0.0], options).is_err());
        assert!(compute_lookahead(&[1], &[0], &[f64::NAN], options).is_err());
    }

    #[test]
    fn teacher_cp_per_logit_matches_python_reference() {
        // 同じ標本の二値交差エントロピーを独立に最小化した参照値である。
        // 最初の例は解析解1000/ln(3)も持ち、平坦な最小点での丸めだけを許す。
        let estimate = estimate_cp_per_logit(&[1000.0; 4], &[2, 2, 2, 0]).unwrap();
        assert!(!estimate.at_search_bound);
        let actual = estimate.value;
        assert!((actual - 910.2392016071356).abs() < 1e-4, "{actual}");
        assert!((actual - 1000.0 / 3.0_f64.ln()).abs() < 1e-4);
        assert!(
            (estimate_cp_per_logit(&[-1000.0, 1000.0], &[0, 2])
                .unwrap()
                .value
                - 50.00000000008325)
                .abs()
                < 1e-10
        );
        assert!(
            (estimate_cp_per_logit(&[-1000.0, 1000.0], &[2, 0])
                .unwrap()
                .value
                - 3999.999999999917)
                .abs()
                < 1e-10
        );
    }

    #[test]
    fn teacher_cp_per_logit_reports_search_bounds_and_undetermined_scale() {
        for results in [[0, 2], [2, 0]] {
            assert!(
                estimate_cp_per_logit(&[-1000.0, 1000.0], &results)
                    .unwrap()
                    .at_search_bound
            );
        }
        assert!(matches!(
            estimate_cp_per_logit(&[0.0, -0.0], &[0, 2]),
            Err(Error::UndeterminedScale)
        ));
    }

    #[test]
    fn teacher_cp_per_logit_rejects_invalid_samples() {
        for (scores, results) in [
            (vec![], vec![]),
            (vec![1.0], vec![]),
            (vec![f64::INFINITY], vec![1]),
            (vec![f64::NAN], vec![1]),
            (vec![0.0], vec![3]),
        ] {
            assert!(matches!(
                estimate_cp_per_logit(&scores, &results),
                Err(Error::TeacherSamples)
            ));
        }
    }
}
