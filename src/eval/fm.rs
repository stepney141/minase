//! 固定次元のFactorization Machineの整数累算と2駒関係補正。

use super::features::{FEATURE_COUNT, active_features_for};
use super::pst::Error;
use crate::{Color, Position};

/// 埋め込むFMの潜在次元。MNPTの次元もこの値と一致する必要がある。
pub const FM_RANK: usize = 32;
/// MNPT本体のFM節のバイト数。
pub(super) const ENCODED_LENGTH: usize = 8 + FM_RANK + FEATURE_COUNT * FM_RANK * 2;

/// 量子化埋め込みと、その整数値から導出した自己相互作用項。
pub(super) struct Fm {
    embeddings: Box<[[i16; FM_RANK]]>,
    self_terms: Box<[i64]>,
    signs: [i8; FM_RANK],
    exponent: u32,
}

/// 1視点の埋め込み和Aと自己相互作用項の和C。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) struct FmAccumulator {
    sums: [i32; FM_RANK],
    self_sum: i64,
}

impl Fm {
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

    /// 2駒関係の補正をセンチポーンへ換算し、負値も0方向へ切り捨てる。
    pub(super) fn correction(&self, accumulator: &FmAccumulator) -> i64 {
        let square_sum: i64 = accumulator
            .sums
            .iter()
            .zip(self.signs)
            .map(|(&a, d)| i64::from(d) * i64::from(a).pow(2))
            .sum();
        (square_sum - accumulator.self_sum) / (1_i64 << (2 * self.exponent + 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{position_from_codes, sq};
    use crate::{PieceCode, PieceKind};

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
}
