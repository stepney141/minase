//! P型の正準行と半幅回転による鏡映共有。
use crate::nnue::data::{Error, Result, SENTINEL};
use minase::eval::nnue::input::FeatureSet;
const FEATURE_COUNT: usize = FeatureSet::P.spec().dimensions();
/// 鏡映対の片側と埋め草1行。
pub const CANONICAL_ROWS: usize = FEATURE_COUNT / 2 + 1;
pub(crate) fn canonical_row(base: u32) -> u32 {
    let square = base % 144;
    let file = square % 12;
    base / 144 * 72 + square / 12 * 6 + file.min(11 - file)
}

pub(crate) fn encode_p_feature(base: u32) -> u32 {
    canonical_row(base) * 2 + u32::from(base % 12 >= 6)
}

/// P型の`正準行番号×2＋回転ビット`を基底特徴番号へ戻す。番兵は保持する。
pub fn decode_p_feature(value: u32) -> Result<u32> {
    if value == SENTINEL {
        return Ok(SENTINEL);
    }
    if value >= FEATURE_COUNT as u32 {
        return Err(Error::PFeature { value });
    }
    let row = value / 2;
    let file = row % 6;
    let file = if value & 1 == 0 { file } else { 11 - file };
    Ok(row / 72 * 144 + row % 72 / 6 * 12 + file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p_features_round_trip_every_base_and_preserve_the_sentinel() {
        for base in 0..FEATURE_COUNT as u32 {
            let packed = encode_p_feature(base);
            assert_eq!(decode_p_feature(packed).unwrap(), base);
        }
        assert_eq!(decode_p_feature(SENTINEL).unwrap(), SENTINEL);
        assert!(decode_p_feature(FEATURE_COUNT as u32).is_err());
    }

    /// 筋0〜11を左右対称に0〜5へ折り、右半分の筋に回転ビットを立てる。
    /// 期待値は「駒×72＋段×6＋min(筋, 11−筋)」の2倍に回転ビットを足して手で求めた。
    #[test]
    fn p_features_fold_files_symmetrically_into_canonical_rows() {
        for (base, packed) in [
            (0, 0),           // 駒0、段0、筋0
            (5, 10),          // 筋5は折り返さない最後の筋
            (6, 11),          // 筋6は筋5の行に回転ビット1
            (11, 1),          // 筋11は筋0の行に回転ビット1
            (157, 158),       // 駒1、段1、筋1
            (13_679, 13_669), // 駒94、段11、筋11
        ] {
            assert_eq!(encode_p_feature(base), packed, "base={base}");
        }
    }
}
