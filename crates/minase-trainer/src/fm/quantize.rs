use super::Result;
use minase::eval::{
    features::FEATURE_COUNT,
    pst::{FM_RANK, FmWeights},
};
use serde::Serialize;

/// Kと出力係数を吸収した、ランク32の量子化FM。
#[derive(Debug, PartialEq, Eq)]
pub struct Quantized {
    /// 特徴番号順の埋め込み。
    pub embeddings: Vec<[i16; FM_RANK]>,
    /// 出力係数が負なら−1、それ以外は+1。
    pub signs: [i8; FM_RANK],
    /// 埋め込みに掛けた2の指数。
    pub exponent: u32,
}

impl Quantized {
    /// MNPT版3の構築へ渡す配列を借用する。
    pub fn as_weights(&self) -> FmWeights<'_> {
        FmWeights {
            embeddings: &self.embeddings,
            signs: &self.signs,
            exponent: self.exponent,
        }
    }
}

/// sqrt(K)·V·sqrt(|a|)の順にf64で計算し、偶数丸め後の絶対値が16,383以下となる
/// 最大の指数を30から0まで探す。形状、有限性、正のKと吸収後の非零性を検査する。
pub fn quantize(v: &[[f64; FM_RANK]], a: &[f64; FM_RANK], k: f64) -> Result<Quantized> {
    if v.len() != FEATURE_COUNT
        || !k.is_finite()
        || k <= 0.0
        || v.iter().flatten().chain(a).any(|x| !x.is_finite())
    {
        return Err("invalid floating FM weights or K".into());
    }
    let sqrt_k = k.sqrt();
    let sqrt_a = a.map(|x| x.abs().sqrt());
    let absorbed: Vec<[f64; FM_RANK]> = v
        .iter()
        .map(|row| std::array::from_fn(|f| sqrt_k * row[f] * sqrt_a[f]))
        .collect();
    if absorbed.iter().flatten().any(|x| !x.is_finite())
        || absorbed.iter().flatten().all(|&x| x == 0.0)
    {
        return Err("absorbed FM embeddings must be finite and nonzero".into());
    }
    for exponent in (0..=30).rev() {
        let scale = f64::from(1_u32 << exponent);
        if absorbed
            .iter()
            .flatten()
            .all(|x| (x * scale).round_ties_even().abs() <= 16383.0)
        {
            return Ok(Quantized {
                embeddings: absorbed
                    .iter()
                    .map(|row| row.map(|x| (x * scale).round_ties_even() as i16))
                    .collect(),
                signs: a.map(|x| if x < 0.0 { -1 } else { 1 }),
                exponent,
            });
        }
    }
    Err("FM embeddings cannot be quantized within 16383 at exponent 0".into())
}

/// 全特徴対と、最大145特徴の絶対値和から求める補正0の十分条件。
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ZeroCorrectionReport {
    /// すべての特徴対の補正を個別に切り捨てると0なら真。
    pub all_pair_corrections_zero: bool,
    /// 最大145特徴の対の絶対値和の上界が1cp未満なら真。
    pub identically_zero: bool,
    /// 異なる特徴対の符号付き内積の最大絶対値。
    pub max_absolute_pair_numerator: i128,
    /// 全特徴対の絶対値和と、最大内積×最大対数の小さい方。
    pub absolute_sum_bound: i128,
    /// 1対の補正の分母、2の2e乗。
    pub pair_denominator: i128,
}

/// 零行を除いて全特徴対を調べる。総和と上界はi128で求める。
pub fn zero_correction_report(
    u: &[[i16; FM_RANK]],
    signs: &[i8; FM_RANK],
    exponent: u32,
) -> Result<ZeroCorrectionReport> {
    if u.len() > FEATURE_COUNT || exponent > 30 || signs.iter().any(|&d| !matches!(d, -1 | 1)) {
        return Err("invalid integer FM weights or exponent".into());
    }
    let rows: Vec<_> = u.iter().filter(|row| row.iter().any(|&x| x != 0)).collect();
    let mut maximum = 0_i128;
    let mut absolute_sum = 0_i128;
    for (i, left) in rows.iter().enumerate() {
        for right in &rows[i + 1..] {
            // i16の全範囲でも32項の内積はi64に収まる。
            let pair: i64 = (0..FM_RANK)
                .map(|f| i64::from(left[f]) * i64::from(right[f]) * i64::from(signs[f]))
                .sum();
            let absolute = i128::from(pair).abs();
            maximum = maximum.max(absolute);
            absolute_sum += absolute;
        }
    }
    let active = rows.len().min(145) as i128;
    let upper = absolute_sum.min(maximum * active * (active - 1) / 2);
    let denominator = 1_i128 << (2 * exponent);
    Ok(ZeroCorrectionReport {
        all_pair_corrections_zero: maximum < denominator,
        identically_zero: upper < denominator,
        max_absolute_pair_numerator: maximum,
        absolute_sum_bound: upper,
        pair_denominator: denominator,
    })
}
