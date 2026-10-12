use std::collections::BTreeMap;

use serde::Serialize;

use super::Result;

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn std(values: &[f64]) -> f64 {
    let center = mean(values);
    (values
        .iter()
        .map(|value| (value - center).powi(2))
        .sum::<f64>()
        / values.len() as f64)
        .sqrt()
}

// NumPyのmethod="linear": (n-1)*p/100に隣接する2点を線形補間する。
fn percentile(sorted: &[f64], percent: u8) -> f64 {
    let index = (sorted.len() - 1) as f64 * f64::from(percent) / 100.0;
    let lower = index.floor() as usize;
    let upper = index.ceil() as usize;
    sorted[lower] + (index - lower as f64) * (sorted[upper] - sorted[lower])
}

fn absolute_percentiles(values: &[f64], percents: &[u8]) -> BTreeMap<u8, f64> {
    let mut absolute: Vec<_> = values.iter().map(|value| value.abs()).collect();
    absolute.sort_by(f64::total_cmp);
    percents
        .iter()
        .map(|&p| (p, percentile(&absolute, p)))
        .collect()
}

fn average_ranks(values: &[f64]) -> Vec<f64> {
    let mut indices: Vec<_> = (0..values.len()).collect();
    indices.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut ranks = vec![0.0; values.len()];
    let mut start = 0;
    while start < indices.len() {
        let mut end = start + 1;
        while end < indices.len() && values[indices[end]] == values[indices[start]] {
            end += 1;
        }
        let rank = (start + 1 + end) as f64 / 2.0;
        for &index in &indices[start..end] {
            ranks[index] = rank;
        }
        start = end;
    }
    ranks
}

fn correlation(x: &[f64], y: &[f64]) -> f64 {
    let (mx, my) = (mean(x), mean(y));
    let covariance = x
        .iter()
        .zip(y)
        .map(|(a, b)| (a - mx) * (b - my))
        .sum::<f64>();
    (covariance / x.len() as f64 / std(x) / std(y)).clamp(-1.0, 1.0)
}

#[derive(Debug, Serialize)]
pub(super) struct Summary {
    mean_cp: f64,
    std_cp: f64,
    mean_absolute_cp: f64,
    absolute_percentiles_cp: BTreeMap<u8, f64>,
}

pub(super) fn summarize(values: &[f64]) -> Result<Summary> {
    if values.is_empty() {
        return Err("cannot summarize an empty evaluation distribution".into());
    }
    if values.iter().any(|v| !v.is_finite()) {
        return Err("non-finite evaluation".into());
    }
    Ok(Summary {
        mean_cp: mean(values),
        std_cp: std(values),
        mean_absolute_cp: values.iter().map(|v| v.abs()).sum::<f64>() / values.len() as f64,
        absolute_percentiles_cp: absolute_percentiles(values, &[5, 25, 50, 75, 95]),
    })
}

#[derive(Debug, Serialize)]
pub(super) struct Comparison {
    pub(super) std_ratio_candidate_over_baseline: f64,
    regression_slope: f64,
    regression_intercept_cp: f64,
    pearson_correlation: f64,
    spearman_correlation: f64,
    regression_residual_std_cp: f64,
    difference_std_cp: f64,
    absolute_difference_percentiles_cp: BTreeMap<u8, f64>,
    absolute_difference_over_100_cp_fraction: f64,
}

pub(super) fn compare_scores(baseline: &[f64], candidate: &[f64]) -> Result<Comparison> {
    if baseline.len() < 2 || baseline.len() != candidate.len() {
        return Err("comparison requires at least two paired evaluations".into());
    }
    if baseline.iter().chain(candidate).any(|v| !v.is_finite()) {
        return Err("non-finite evaluation".into());
    }
    let (base_std, candidate_std) = (std(baseline), std(candidate));
    if base_std == 0.0 || candidate_std == 0.0 {
        return Err("both evaluation distributions must have nonzero variance".into());
    }
    let (base_mean, candidate_mean) = (mean(baseline), mean(candidate));
    let centered: Vec<_> = baseline
        .iter()
        .zip(candidate)
        .map(|(b, c)| (b - base_mean, c - candidate_mean))
        .collect();
    let slope = centered.iter().map(|(b, c)| b * c).sum::<f64>()
        / centered.iter().map(|(b, _)| b * b).sum::<f64>();
    let residual: Vec<_> = centered.iter().map(|(b, c)| c - slope * b).collect();
    let difference: Vec<_> = baseline.iter().zip(candidate).map(|(b, c)| c - b).collect();
    Ok(Comparison {
        std_ratio_candidate_over_baseline: candidate_std / base_std,
        regression_slope: slope,
        regression_intercept_cp: candidate_mean - slope * base_mean,
        pearson_correlation: correlation(baseline, candidate),
        spearman_correlation: correlation(&average_ranks(baseline), &average_ranks(candidate)),
        regression_residual_std_cp: std(&residual),
        difference_std_cp: std(&difference),
        absolute_difference_percentiles_cp: absolute_percentiles(&difference, &[50, 90, 99]),
        absolute_difference_over_100_cp_fraction: difference
            .iter()
            .filter(|v| v.abs() > 100.0)
            .count() as f64
            / difference.len() as f64,
    })
}

pub(super) fn scale_verdict(ratio: f64) -> &'static str {
    if (0.9..=1.1).contains(&ratio) {
        "全体の尺度はほとんど変えていない"
    } else {
        "散らばりの尺度を変えた"
    }
}

#[derive(Serialize)]
pub(super) struct Interval {
    lower_inclusive: i64,
    upper_inclusive: Option<i64>,
    sample_count: usize,
    baseline_statistics: Summary,
    candidate_statistics: Summary,
    #[serde(flatten)]
    comparison: Comparison,
}

pub(super) fn compare_intervals(
    baseline: &[f64],
    candidate: &[f64],
    values: &[i64],
    intervals: &[(i64, Option<i64>)],
) -> Result<Vec<Interval>> {
    intervals
        .iter()
        .map(|&(lower, upper)| {
            let (base, cand): (Vec<_>, Vec<_>) = baseline
                .iter()
                .zip(candidate)
                .zip(values)
                .filter(|&(_, &value)| value >= lower && upper.is_none_or(|upper| value <= upper))
                .map(|((&b, &c), _)| (b, c))
                .unzip();
            let compare = || -> Result<Interval> {
                Ok(Interval {
                    lower_inclusive: lower,
                    upper_inclusive: upper,
                    sample_count: base.len(),
                    baseline_statistics: summarize(&base)?,
                    candidate_statistics: summarize(&cand)?,
                    comparison: compare_scores(&base, &cand)?,
                })
            };
            compare().map_err(|error| {
                format!(
                    "interval [{lower}, {}]: {error}",
                    upper.map_or_else(|| "None".into(), |v| v.to_string())
                )
                .into()
            })
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub(super) struct DeltaStatistics {
    pub(super) count: usize,
    mean_cp: Option<f64>,
    std_cp: Option<f64>,
    mean_absolute_cp: Option<f64>,
    max_absolute_cp: Option<f64>,
}

pub(super) fn delta_statistics(values: &[f64]) -> DeltaStatistics {
    DeltaStatistics {
        count: values.len(),
        mean_cp: (!values.is_empty()).then(|| mean(values)),
        std_cp: (!values.is_empty()).then(|| std(values)),
        mean_absolute_cp: (!values.is_empty())
            .then(|| values.iter().map(|v| v.abs()).sum::<f64>() / values.len() as f64),
        max_absolute_cp: values.iter().map(|v| v.abs()).reduce(f64::max),
    }
}

#[derive(Debug, Serialize)]
pub(super) struct Reversals {
    count: usize,
    total: usize,
    fraction: Option<f64>,
    mean_absolute_cp: Option<f64>,
    max_absolute_cp: Option<f64>,
}

pub(super) fn reversal_statistics(pairs: impl Iterator<Item = (i32, i32)>) -> Reversals {
    let mut total = 0;
    let mut wrong = Vec::new();
    for (delta, reference) in pairs {
        if reference != 0 {
            total += 1;
            if delta * reference.signum() < 0 {
                wrong.push(f64::from(delta));
            }
        }
    }
    let stats = delta_statistics(&wrong);
    Reversals {
        count: wrong.len(),
        total,
        fraction: (total != 0).then(|| wrong.len() as f64 / total as f64),
        mean_absolute_cp: stats.mean_absolute_cp,
        max_absolute_cp: stats.max_absolute_cp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
    }

    #[test]
    fn linear_percentiles_and_population_standard_deviation() {
        // Pythonのlinear定義では[0,10,20,30]の5%位置は0.15、95%位置は2.85。
        let s = summarize(&[0.0, -10.0, 20.0, -30.0]).unwrap();
        for (p, value) in [(5, 1.5), (25, 7.5), (50, 15.0), (75, 22.5), (95, 28.5)] {
            close(s.absolute_percentiles_cp[&p], value);
        }
        close(s.mean_cp, -5.0);
        close(s.std_cp, 325.0_f64.sqrt());
        close(s.mean_absolute_cp, 15.0);
        close(percentile(&[7.0], 95), 7.0);
        close(percentile(&[2.0, 8.0], 0), 2.0);
        close(percentile(&[2.0, 8.0], 100), 8.0);
    }

    #[test]
    fn spearman_assigns_average_ranks_to_ties() {
        assert_eq!(average_ranks(&[4.0, 0.0, 2.0, 0.0]), [4.0, 1.5, 3.0, 1.5]);
        // 中心化順位の内積3.75、各平方和4.5よりρ=5/6。
        let c = compare_scores(&[0.0, 0.0, 2.0, 4.0], &[0.0, 2.0, 2.0, 4.0]).unwrap();
        close(c.spearman_correlation, 5.0 / 6.0);
    }

    #[test]
    fn regression_includes_an_intercept_and_centered_residuals() {
        // x中心化=(-1,0,1)、y中心化=(-2,-1,3)。傾き5/2、切片-2。
        let c = compare_scores(&[1.0, 2.0, 3.0], &[1.0, 2.0, 6.0]).unwrap();
        close(c.regression_slope, 2.5);
        close(c.regression_intercept_cp, -2.0);
        close(c.regression_residual_std_cp, 0.5_f64.sqrt());
        close(c.pearson_correlation, 5.0 / 28.0_f64.sqrt());
        close(c.std_ratio_candidate_over_baseline, 7.0_f64.sqrt());
        close(c.difference_std_cp, 2.0_f64.sqrt());
        close(c.absolute_difference_percentiles_cp[&90], 2.4);
        let shifted = compare_scores(&[1.0, 2.0, 3.0], &[101.0, 103.0, 105.0]).unwrap();
        close(shifted.regression_intercept_cp, 99.0);
        close(shifted.absolute_difference_over_100_cp_fraction, 2.0 / 3.0);
    }

    #[test]
    fn invalid_distributions_empty_moves_and_zero_references() {
        assert!(summarize(&[]).is_err());
        assert!(compare_scores(&[1.0], &[1.0]).is_err());
        assert!(compare_scores(&[1.0, 1.0], &[1.0, 2.0]).is_err());
        assert!(compare_scores(&[1.0, 2.0], &[1.0, 1.0]).is_err());
        assert!(compare_intervals(&[0.0, 1.0], &[0.0, 1.0], &[0, 0], &[(1, Some(2))]).is_err());
        let empty = delta_statistics(&[]);
        assert!(empty.mean_cp.is_none() && empty.std_cp.is_none());
        let r = reversal_statistics([(10, -1), (-4, 2), (0, 1), (9, 0)].into_iter());
        assert_eq!((r.count, r.total), (2, 3));
        close(r.mean_absolute_cp.unwrap(), 7.0);
        assert!(reversal_statistics([(9, 0)].into_iter()).fraction.is_none());
        for ratio in [0.9, 1.1] {
            assert_eq!(scale_verdict(ratio), "全体の尺度はほとんど変えていない");
        }
        assert_eq!(scale_verdict(1.1001), "散らばりの尺度を変えた");
    }

    #[test]
    fn intervals_include_both_bounds() {
        let scores = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        let intervals = compare_intervals(
            &scores,
            &scores,
            &[0, 29, 30, 59, 60, 90],
            &[(0, Some(29)), (30, Some(59)), (60, Some(90))],
        )
        .unwrap();
        assert_eq!(
            intervals.iter().map(|r| r.sample_count).collect::<Vec<_>>(),
            [2, 2, 2]
        );
        let scores = [0.0, 99.0, 100.0, 299.0, 300.0, 999.0, 1000.0, 1001.0];
        let intervals = compare_intervals(
            &scores,
            &scores,
            &[0, 99, 100, 299, 300, 999, 1000, 1001],
            &[
                (0, Some(99)),
                (100, Some(299)),
                (300, Some(999)),
                (1000, None),
            ],
        )
        .unwrap();
        assert_eq!(
            intervals.iter().map(|r| r.sample_count).collect::<Vec<_>>(),
            [2, 2, 2, 2]
        );
    }
}
