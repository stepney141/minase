//! 固定次元のFactorization Machineの整数累算と2駒関係補正。

use super::Error;
use super::features::{FEATURE_COUNT, active_features_for};
use minase_core::{Color, Position};

/// 埋め込むFMの潜在次元。MNPTの次元もこの値と一致する必要がある。
pub const FM_RANK: usize = 32;
/// MNPT本体のFM節のバイト数。
pub(super) const ENCODED_LENGTH: usize = 8 + FM_RANK + FEATURE_COUNT * FM_RANK * 2;

/// 量子化埋め込みと、その整数値から導出した自己相互作用項。
pub(super) struct Fm {
    /// 特徴番号順の量子化埋め込み。
    embeddings: Box<[[i16; FM_RANK]]>,
    /// 特徴ごとの符号付き二乗和。
    self_terms: Box<[i64]>,
    /// 各潜在次元の符号。
    signs: [i8; FM_RANK],
    /// 量子化尺度の2進指数。
    exponent: u32,
}

/// 1視点の埋め込み和Aと自己相互作用項の和C。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) struct FmAccumulator {
    /// 有効特徴の埋め込み和。
    sums: [i32; FM_RANK],
    /// 有効特徴の自己相互作用項の和。
    self_sum: i64,
}

impl Fm {
    /// FM節を仕様順のリトルエンディアンで追記する。
    pub(super) fn encode_into(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&(FM_RANK as u32).to_le_bytes());
        bytes.extend_from_slice(&self.exponent.to_le_bytes());
        bytes.extend(self.signs.map(|sign| sign as u8));
        for row in &self.embeddings {
            for value in row {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
    }

    /// 長さと検査和をPst::decodeで検証済みのFM節を復号する。
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let rank = u32::from_le_bytes(bytes[..4].try_into().expect("validated FM length"));
        if rank != FM_RANK as u32 {
            return Err(Error::UnexpectedFmRank { actual: rank });
        }
        let exponent = u32::from_le_bytes(bytes[4..8].try_into().expect("validated FM length"));
        if exponent > 30 {
            return Err(Error::InvalidFmExponent { actual: exponent });
        }
        let mut signs = [0; FM_RANK];
        for (dimension, sign) in signs.iter_mut().enumerate() {
            *sign = bytes[8 + dimension] as i8;
            if !matches!(*sign, -1 | 1) {
                return Err(Error::InvalidFmSign {
                    dimension,
                    actual: *sign,
                });
            }
        }
        let embeddings = bytes[8 + FM_RANK..]
            .as_chunks::<{ FM_RANK * 2 }>()
            .0
            .iter()
            .map(|row| std::array::from_fn(|f| i16::from_le_bytes([row[2 * f], row[2 * f + 1]])))
            .collect::<Box<[[i16; FM_RANK]]>>();
        let self_terms = embeddings
            .iter()
            .map(|row| {
                row.iter()
                    .zip(signs)
                    .map(|(&u, d)| i64::from(d) * i64::from(u).pow(2))
                    .sum()
            })
            .collect();
        Ok(Self {
            embeddings,
            self_terms,
            signs,
            exponent,
        })
    }

    /// 指定視点の有効特徴から累算値を全再計算する。
    pub(super) fn refresh(&self, perspective: Color, position: &Position) -> FmAccumulator {
        let mut accumulator = FmAccumulator::default();
        active_features_for(perspective, position, |feature| {
            self.add(&mut accumulator, feature)
        });
        accumulator
    }

    /// 有効になった特徴を加える。
    pub(super) fn add(&self, accumulator: &mut FmAccumulator, feature: usize) {
        for (sum, &u) in accumulator.sums.iter_mut().zip(&self.embeddings[feature]) {
            *sum += i32::from(u);
        }
        accumulator.self_sum += self.self_terms[feature];
    }

    /// 無効になった特徴を除く。
    pub(super) fn remove(&self, accumulator: &mut FmAccumulator, feature: usize) {
        for (sum, &u) in accumulator.sums.iter_mut().zip(&self.embeddings[feature]) {
            *sum -= i32::from(u);
        }
        accumulator.self_sum -= self.self_terms[feature];
    }

    /// 親からの複写と移動元の減算、移動先の加算を1回の走査で行う。
    pub(super) fn replace(
        &self,
        before: &FmAccumulator,
        after: &mut FmAccumulator,
        removed: usize,
        added: usize,
    ) {
        let removed_row = &self.embeddings[removed];
        let added_row = &self.embeddings[added];
        for f in 0..FM_RANK {
            after.sums[f] = before.sums[f] - i32::from(removed_row[f]) + i32::from(added_row[f]);
        }
        after.self_sum = before.self_sum - self.self_terms[removed] + self.self_terms[added];
    }

    /// 2駒関係の補正をセンチポーンへ換算し、負値も0方向へ切り捨てる。
    pub(super) fn correction(&self, accumulator: &FmAccumulator) -> i64 {
        // 最大145特徴でも|a|は4,751,360以下なので、符号は32ビットのまま掛けられる。
        let square_sum: i64 = accumulator
            .sums
            .iter()
            .zip(self.signs)
            .map(|(&a, d)| i64::from(a * i32::from(d)) * i64::from(a))
            .sum();
        let numerator = square_sum - accumulator.self_sum;
        #[cfg(not(feature = "tuning"))]
        let correction = numerator / (1_i64 << (2 * self.exponent + 1));
        #[cfg(feature = "tuning")]
        let correction = scaled_correction(
            numerator,
            self.exponent,
            crate::search::alphabeta::params::fm_scale(),
        );
        correction
    }
}

#[cfg(any(feature = "tuning", test))]
fn scaled_correction(numerator: i64, exponent: u32, scale: i32) -> i64 {
    // 指数の上限30では分母が2^71となり、i64に収まらないのでi128で計算する。
    // scaleは0..=1024なので結果の絶対値は|numerator|以下となり、i64に収まる。
    (i128::from(numerator) * i128::from(scale) / (1024_i128 << (2 * exponent + 1))) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase_core::test_util::{position_from_codes, sq};
    use minase_core::{PieceCode, PieceKind};

    /// 固定した整数値から仕様順のFM節を作る。
    fn encoded(
        exponent: u32,
        signs: [i8; FM_RANK],
        value: impl Fn(usize, usize) -> i16,
    ) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(ENCODED_LENGTH);
        bytes.extend_from_slice(&(FM_RANK as u32).to_le_bytes());
        bytes.extend_from_slice(&exponent.to_le_bytes());
        bytes.extend(signs.map(|d| d as u8));
        for i in 0..FEATURE_COUNT {
            for f in 0..FM_RANK {
                bytes.extend_from_slice(&value(i, f).to_le_bytes());
            }
        }
        bytes
    }

    /// 各特徴・次元で異なる、小さな決定的擬似乱数。
    fn embedding(i: usize, f: usize) -> i16 {
        let state = ((i * FM_RANK + f) as u32)
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        ((state >> 16) % 201) as i16 - 100
    }

    /// 倍率1は、全指数で通常ビルドの0方向への切り捨てを保存する。
    #[test]
    fn full_scale_matches_unscaled_correction_for_every_exponent() {
        for exponent in 0..=30 {
            let denominator = 1_i64 << (2 * exponent + 1);
            for numerator in [
                i64::MIN,
                i64::MIN + 1,
                -(1_i64 << 62),
                -3,
                -2,
                -1,
                0,
                1,
                2,
                3,
                1_i64 << 62,
                i64::MAX,
                -3 * (denominator / 2),
                -denominator / 2,
                denominator / 2,
                3 * (denominator / 2),
            ] {
                assert_eq!(
                    scaled_correction(numerator, exponent, 1024),
                    numerator / denominator,
                    "numerator={numerator}, exponent={exponent}"
                );
            }
        }
    }

    /// 除算を再実装せず、商の絶対値の上下限と符号から切り捨てを検査する。
    fn assert_scaled_truncation(numerator: i64, exponent: u32, scale: i32) {
        let quotient = i128::from(scaled_correction(numerator, exponent, scale));
        let product = i128::from(numerator) * i128::from(scale);
        let denominator = 1024 * 2_i128.pow(2 * exponent + 1);
        assert!(
            quotient.abs() * denominator <= product.abs()
                && product.abs() < (quotient.abs() + 1) * denominator,
            "numerator={numerator}, exponent={exponent}, scale={scale}, quotient={quotient}"
        );
        if quotient != 0 {
            assert_eq!(quotient.signum(), product.signum());
        }
    }

    /// 倍率0、1/2、3/4は、正負の分子と全指数で定義の不等式を満たす。
    #[test]
    fn fractional_scales_truncate_toward_zero() {
        for exponent in 0..=30 {
            let denominator = 1_i64 << (2 * exponent + 1);
            for numerator in [
                -3 * denominator,
                -3 * (denominator / 2),
                -denominator,
                -denominator / 2,
                -3,
                -1,
                0,
                1,
                3,
                denominator / 2,
                denominator,
                3 * (denominator / 2),
                3 * denominator,
            ] {
                for scale in [0, 512, 768] {
                    assert_scaled_truncation(numerator, exponent, scale);
                }
            }
        }
    }

    /// 倍率3/4は整数化より先に掛ける。倍率1/2の例も正負で固定する。
    #[test]
    fn scaling_precedes_integer_truncation() {
        for sign in [-1, 1] {
            let numerator = sign * 12;
            assert_eq!(scaled_correction(numerator, 1, 768), sign);
            assert_eq!((numerator / 8) * 768 / 1024, 0);
            for (numerator, expected) in [(12, 0), (24, 1)] {
                assert_eq!(scaled_correction(sign * numerator, 1, 512), sign * expected);
                assert_eq!((sign * numerator / 8) * 512 / 1024, sign * expected);
            }
        }
    }

    /// 指数26と30では分母がi64を超えるが、分子の両端でも定義を満たす。
    #[test]
    fn scaled_extreme_numerators_do_not_overflow() {
        for exponent in [26, 30] {
            for numerator in [i64::MIN, i64::MAX] {
                for scale in [0, 512, 768, 1024] {
                    assert_scaled_truncation(numerator, exponent, scale);
                }
            }
        }
    }

    /// 調整用ビルドの既定倍率が、実際のFM補正の計算にも適用される。
    #[cfg(feature = "tuning")]
    #[test]
    fn tuning_default_correction_matches_full_scale() {
        for sign in [-1, 1] {
            let fm = Fm::decode(&encoded(1, [sign; FM_RANK], |i, f| match (i, f) {
                (0, 0) => 2,
                (1, 0) => 3,
                _ => 0,
            }))
            .unwrap();
            let mut accumulator = FmAccumulator::default();
            fm.add(&mut accumulator, 0);
            fm.add(&mut accumulator, 1);
            // 2特徴の組による分子は、符号 × 2 × 2 × 3となる。
            assert_eq!(
                fm.correction(&accumulator),
                scaled_correction(i64::from(sign) * 12, 1, 1024)
            );
        }
    }

    /// v3の特徴番号優先配置を復号し、整数表からの再符号化で全バイトを保存する。
    #[test]
    fn section_round_trip_preserves_every_embedding_and_signed_self_term() {
        let signs = std::array::from_fn(|f| if f % 3 == 0 { -1 } else { 1 });
        let bytes = encoded(30, signs, embedding);
        let fm = Fm::decode(&bytes).unwrap();
        assert_eq!(fm.exponent, 30);
        assert_eq!(fm.signs, signs);
        assert_eq!(fm.embeddings.len(), FEATURE_COUNT);
        for i in 0..FEATURE_COUNT {
            let expected: i64 = (0..FM_RANK)
                .map(|f| i64::from(signs[f]) * i64::from(embedding(i, f)).pow(2))
                .sum();
            assert_eq!(fm.self_terms[i], expected);
        }
        assert_eq!(
            encoded(fm.exponent, fm.signs, |i, f| fm.embeddings[i][f]),
            bytes
        );
    }

    /// 2特徴の組を直接列挙した値を、両視点の全再計算と特徴の加除で再現する。
    #[test]
    fn pairs_full_refresh_and_incremental_updates_agree() {
        let signs = std::array::from_fn(|f| if f % 3 == 0 { -1 } else { 1 });
        let pieces: Vec<_> = (0..8)
            .map(|i| {
                (
                    sq(i, i),
                    PieceCode::new(Color::ALL[i as usize % 2], PieceKind::Pawn).unwrap(),
                )
            })
            .collect();
        let mut position = position_from_codes(Color::Black, &pieces);
        position.set_lion_capture(Some(sq(4, 5))).unwrap();
        for exponent in [0, 1, 7, 30] {
            let fm = Fm::decode(&encoded(exponent, signs, embedding)).unwrap();
            for perspective in Color::ALL {
                let mut features = Vec::new();
                active_features_for(perspective, &position, |i| features.push(i));
                let mut pairs = 0_i64;
                for (offset, &i) in features.iter().enumerate() {
                    for &j in &features[..offset] {
                        for (f, &d) in signs.iter().enumerate() {
                            pairs += i64::from(d)
                                * i64::from(embedding(i, f))
                                * i64::from(embedding(j, f));
                        }
                    }
                }
                let full = fm.refresh(perspective, &position);
                assert_eq!(fm.correction(&full), pairs / (1_i64 << (2 * exponent)));
                let mut incremental = FmAccumulator::default();
                for &i in &features {
                    fm.add(&mut incremental, i);
                }
                assert_eq!(incremental, full);
                for &i in features.iter().rev() {
                    fm.remove(&mut incremental, i);
                }
                assert_eq!(incremental, FmAccumulator::default());
            }
        }
    }

    /// 分子−2、分母8の結果は0であり、算術シフトの−1ではない。
    #[test]
    fn negative_numerator_truncates_toward_zero() {
        let fm = Fm::decode(&encoded(1, [1; FM_RANK], |i, f| match (i, f) {
            (0, 0) => 1,
            (1, 0) => -1,
            _ => 0,
        }))
        .unwrap();
        let mut accumulator = FmAccumulator::default();
        fm.add(&mut accumulator, 0);
        fm.add(&mut accumulator, 1);
        assert_eq!(accumulator.sums, [0; FM_RANK]);
        assert_eq!(accumulator.self_sum, 2);
        assert_eq!(fm.correction(&accumulator), 0);
    }

    /// 2特徴の積を直接割った仕様値と、全指数で一致する。
    #[test]
    fn pair_correction_truncates_toward_zero_for_every_exponent() {
        let values = [
            i16::MIN,
            -32767,
            -17,
            -16,
            -15,
            -1,
            0,
            1,
            15,
            16,
            17,
            i16::MAX,
        ];
        for exponent in 0..=30 {
            let fm = Fm::decode(&encoded(exponent, [1; FM_RANK], |i, f| {
                if f == 0 { values[i % values.len()] } else { 0 }
            }))
            .unwrap();
            for (i, &left) in values.iter().enumerate() {
                for (j, &right) in values.iter().enumerate() {
                    let mut accumulator = FmAccumulator::default();
                    fm.add(&mut accumulator, i);
                    fm.add(&mut accumulator, values.len() + j);
                    let expected = i64::from(left) * i64::from(right) / (1_i64 << (2 * exponent));
                    assert_eq!(
                        fm.correction(&accumulator),
                        expected,
                        "exponent={exponent}, left={left}, right={right}"
                    );
                }
            }
        }
    }

    /// 最大145特徴とi16の両端でも、累算と補正は指定した整数型に収まる。
    #[test]
    fn extreme_inputs_do_not_overflow() {
        for u in [i16::MIN, -i16::MAX, i16::MAX] {
            for sign in [-1, 1] {
                let fm = Fm::decode(&encoded(0, [sign; FM_RANK], |_, _| u)).unwrap();
                let mut accumulator = FmAccumulator::default();
                for i in 0..145 {
                    fm.add(&mut accumulator, i);
                }
                assert_eq!(accumulator.sums, [145 * i32::from(u); FM_RANK]);
                let expected =
                    i64::from(sign) * FM_RANK as i64 * (145 * 144 / 2) * i64::from(u).pow(2);
                assert_eq!(fm.correction(&accumulator), expected);
                assert!(expected.abs() > i64::from(i32::MAX));
                for i in 0..145 {
                    fm.remove(&mut accumulator, i);
                }
                assert_eq!(accumulator, FmAccumulator::default());
            }
        }
    }
    /// 指数を1増やした補正は、元の補正を0方向へ切り捨てて4で割った値になる。
    #[test]
    fn increasing_exponent_quarters_signed_correction() {
        for sign in [-1, 1] {
            for exponent in 0..30 {
                let original = Fm::decode(&encoded(exponent, [sign; FM_RANK], embedding)).unwrap();
                let quarter =
                    Fm::decode(&encoded(exponent + 1, [sign; FM_RANK], embedding)).unwrap();
                let mut accumulator = FmAccumulator::default();
                for feature in 0..145 {
                    original.add(&mut accumulator, feature);
                    assert_eq!(
                        quarter.correction(&accumulator),
                        original.correction(&accumulator) / 4
                    );
                }
            }
        }
    }
}
