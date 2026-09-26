//! 生成統計の表示に使う分布と割合の計算。

/// 探索値の度数表から平均と母標準偏差を返す。
pub(super) fn score_mean_and_std(frequencies: &[u64]) -> Option<(f64, f64)> {
    let mut count = 0_u64;
    let mut sum = 0.0;
    let mut squared_sum = 0.0;
    for (index, &frequency) in frequencies.iter().enumerate() {
        if frequency == 0 {
            continue;
        }
        let score = index as f64 + f64::from(i16::MIN);
        count += frequency;
        let frequency_as_f64 = frequency as f64;
        sum += score * frequency_as_f64;
        squared_sum += score * score * frequency_as_f64;
    }
    if count == 0 {
        return None;
    }
    let count = count as f64;
    let mean = sum / count;
    let variance = (squared_sum / count - mean * mean).max(0.0);
    Some((mean, variance.sqrt()))
}

/// 累積度数が指定百分率以上となる最初の探索値を返す。
pub(super) fn score_percentile(frequencies: &[u64], percentile: u8) -> Option<i16> {
    let count = frequencies
        .iter()
        .map(|&frequency| u128::from(frequency))
        .sum::<u128>();
    if count == 0 {
        return None;
    }
    let target = (count * u128::from(percentile)).div_ceil(100);
    let mut cumulative = 0_u128;
    for (index, &frequency) in frequencies.iter().enumerate() {
        cumulative += u128::from(frequency);
        if cumulative >= target {
            let score =
                i32::try_from(index).expect("a score index fits in i32") + i32::from(i16::MIN);
            return Some(i16::try_from(score).expect("a score-table index maps to i16"));
        }
    }
    unreachable!("the cumulative frequency reaches the total count")
}

/// 件数を百分率へ変換し、分母が0なら`n/a`を返す。
pub(super) fn format_rate_percent(count: u64, total: u64) -> String {
    if total == 0 {
        "n/a".to_owned()
    } else {
        format!("{:.6}", count as f64 * 100.0 / total as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase::datagen::statistics::{SCORE_VALUE_COUNT, score_index};

    /// 探索値度数表から平均、母標準偏差、分位点を求める。
    #[test]
    fn score_distribution_uses_signed_order_and_nearest_rank() {
        let mut frequencies = vec![0_u64; SCORE_VALUE_COUNT];
        for score in [-2, 0, 2] {
            frequencies[score_index(score)] += 1;
        }

        let (mean, standard_deviation) =
            score_mean_and_std(&frequencies).expect("the table is non-empty");
        assert_eq!(mean, 0.0);
        assert!((standard_deviation - (8.0_f64 / 3.0).sqrt()).abs() < f64::EPSILON);
        assert_eq!(score_percentile(&frequencies, 1), Some(-2));
        assert_eq!(score_percentile(&frequencies, 50), Some(0));
        assert_eq!(score_percentile(&frequencies, 99), Some(2));
        assert_eq!(score_index(i16::MIN), 0);
        assert_eq!(score_index(i16::MAX), SCORE_VALUE_COUNT - 1);

        frequencies.fill(0);
        assert_eq!(score_mean_and_std(&frequencies), None);
        assert_eq!(score_percentile(&frequencies, 50), None);
    }

    /// 除外率は分母0なら数値ではなく`n/a`になる。
    #[test]
    fn rate_percent_is_not_available_for_zero_denominator() {
        assert_eq!(format_rate_percent(0, 0), "n/a");
        assert_eq!(format_rate_percent(1, 4), "25.000000");
    }
}
