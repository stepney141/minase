use serde::Serialize;

use super::{BatchSlot, Error, Result};

/// 省略、または有限な0以上1以下の教師の混合比を許す。
pub fn validate_lambda_override(value: Option<f64>) -> Result<()> {
    if value.is_some_and(|mix| !mix.is_finite() || !(0.0..=1.0).contains(&mix)) {
        return Err(Error::TeacherLambda);
    }
    Ok(())
}

/// Pythonの`pst.teacher.build_targets`と同じ教師値を構築する。
///
/// 分類ごとに`λ / (1 + exp(-score / K)) + (1 - λ) * result / 2`をf64で
/// 計算してf32へ丸める。λが0の分類ではKと探索値を参照しない。
pub fn build_targets(
    batch: &BatchSlot,
    teacher_ks: &[f64],
    teacher_lambdas: &[f64],
) -> Result<Vec<f32>> {
    build_targets_from_arrays(
        batch.teacher_scores(),
        batch.teacher_classes(),
        batch.outcomes(),
        teacher_ks,
        teacher_lambdas,
    )
}

/// 探索値、分類番号、結果の配列から教師値を構築する。すべて局面順で同じ長さを要求する。
/// 混合と丸めの演算順はバッチを受け取る関数と同じ。
pub fn build_targets_from_arrays(
    scores: &[f32],
    classes: &[u32],
    outcomes: &[u8],
    teacher_ks: &[f64],
    teacher_lambdas: &[f64],
) -> Result<Vec<f32>> {
    if teacher_ks.len() != teacher_lambdas.len()
        || scores.len() != classes.len()
        || scores.len() != outcomes.len()
        || outcomes.iter().any(|&outcome| outcome > 2)
    {
        return Err(Error::TeacherTargets);
    }
    for &mix in teacher_lambdas {
        validate_lambda_override(Some(mix))?;
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
            let k = teacher_ks[class];
            let score = f64::from(scores[position]);
            if !k.is_finite() || k <= 0.0 || !score.is_finite() {
                return Err(Error::TeacherTargets);
            }
            Ok((mix / (1.0 + (-score / k).exp()) + (1.0 - mix) * result) as f32)
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

/// Pythonの`data.lookahead.compute_lookahead`と同じ値をf64で求める。
///
/// 窓内の最小の手数差を`d1`として、`gamma^(d-d1)`を昇順に足す。
/// 奇数の手数差では符号を反転し、窓が空なら元の探索値を保持する。
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

fn binary_cross_entropy_sum(scores: &[f64], results: &[u8], k: f64) -> f64 {
    scores
        .iter()
        .zip(results)
        .map(|(&score, &result)| {
            let logit = score / k;
            // np.logaddexp(0, logit)の安定な評価式。
            logit.max(0.0) + (-logit.abs()).exp().ln_1p() - f64::from(result) / 2.0 * logit
        })
        .sum()
}

/// Pythonの`pst.teacher.estimate_k`の黄金分割探索を64回行う。
///
/// `[50, 4000]`で、探索値から終局結果を予測する二値交差エントロピーを最小化する。
/// `results`は手番側の負け0、引き分け1、勝ち2。計算はすべてf64で行う。
pub fn estimate_k(scores: &[f64], results: &[u8]) -> Result<f64> {
    if scores.is_empty()
        || scores.len() != results.len()
        || scores.iter().any(|value| !value.is_finite())
        || results.iter().any(|&value| value > 2)
    {
        return Err(Error::TeacherSamples);
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
    Ok((lower + upper) / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teacher_arrays_preserve_mixing_and_ignore_inactive_scores_and_k() {
        let targets = build_targets_from_arrays(
            &[f32::NAN, 0.0, 1000.0],
            &[0, 1, 1],
            &[2, 0, 1],
            &[f64::NAN, 1000.0],
            &[0.0, 0.75],
        )
        .unwrap();
        assert_eq!(targets[0], 1.0);
        assert_eq!(targets[1], 0.375);
        let expected = 0.75 / (1.0 + (-1.0_f64).exp()) + 0.125;
        assert_eq!(targets[2].to_bits(), (expected as f32).to_bits());
        for (scores, classes, outcomes) in [
            (vec![0.0], vec![], vec![1]),
            (vec![0.0], vec![0], vec![]),
            (vec![0.0], vec![2], vec![1]),
            (vec![0.0], vec![0], vec![3]),
            (vec![f32::NAN], vec![0], vec![1]),
        ] {
            assert!(
                build_targets_from_arrays(&scores, &classes, &outcomes, &[1.0], &[1.0]).is_err()
            );
        }
    }

    #[test]
    fn lookahead_matches_python_example() {
        // tools/train/tests/test_lookahead.pyの手計算例。
        let scores = [7.0, 2.0, 1.0, 99.0, 10.0, -5.0];
        let actual = compute_lookahead(
            &[1, 1, 1, 1, 2, 2],
            &[0, 1, 3, 8, 0, 2],
            &scores,
            Lookahead::new(0.5, 3).unwrap(),
        )
        .unwrap();
        assert_eq!(actual, [-1.8, 1.0, 1.0, 99.0, -5.0, -5.0]);
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
        for (games, plies) in [
            (vec![2, 1], vec![0, 1]),
            (vec![1, 1], vec![1, 1]),
            (vec![1, 1], vec![2, 1]),
            (vec![1, 2, 1], vec![0, 0, 1]),
        ] {
            assert!(matches!(
                compute_lookahead(&games, &plies, &vec![0.0; games.len()], options),
                Err(Error::RecordOrder { .. })
            ));
        }
        assert!(compute_lookahead(&[1], &[], &[0.0], options).is_err());
        assert!(compute_lookahead(&[1], &[0], &[f64::NAN], options).is_err());
    }

    #[test]
    fn teacher_k_matches_python_reference() {
        // Pythonのestimate_kを同じ標本に実行した参照値。
        // 最初の例は解析解1000/ln(3)も持つ。平坦な最小点での丸めだけを許す。
        let actual = estimate_k(&[1000.0; 4], &[2, 2, 2, 0]).unwrap();
        assert!((actual - 910.2392016071356).abs() < 1e-4, "{actual}");
        assert!((actual - 1000.0 / 3.0_f64.ln()).abs() < 1e-4);
        assert!(
            (estimate_k(&[-1000.0, 1000.0], &[0, 2]).unwrap() - 50.00000000008325).abs() < 1e-10
        );
        assert!(
            (estimate_k(&[-1000.0, 1000.0], &[2, 0]).unwrap() - 3999.999999999917).abs() < 1e-10
        );
    }

    #[test]
    fn teacher_k_rejects_invalid_samples() {
        for (scores, results) in [
            (vec![], vec![]),
            (vec![1.0], vec![]),
            (vec![f64::INFINITY], vec![1]),
            (vec![0.0], vec![3]),
        ] {
            assert!(matches!(
                estimate_k(&scores, &results),
                Err(Error::TeacherSamples)
            ));
        }
    }
}
