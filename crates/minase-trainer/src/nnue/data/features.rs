use minase::{
    eval::features::{FEATURE_COUNT, feature_index, lion_feature_index},
    eval::nnue::feature_set::{FeatureSet, HalfKpHmB8},
};
use minase_core::{Color, Position, Square};
use serde::Serialize;

use super::{Error, Result};

/// 元の盤面の144升と先獅子の1枠を合わせた、1視点の列数。
pub const FEATURE_COLUMNS: usize = 145;
/// 鏡映対の片側6,840行と埋め草1行。
pub const CANONICAL_ROWS: usize = FEATURE_COUNT / 2 + 1;
/// 空升または先獅子のない列。逆索引には含めない。
pub const SENTINEL: u32 = u32::MAX;

/// 学習器で使用する入力特徴集合。
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Serialize)]
pub enum InputFeatureSet {
    /// 王の位置で条件付けず、正準行と半幅回転で鏡映を共有する。
    #[value(name = "P")]
    P,
    /// 自玉の位置を8区分に分け、文脈に応じて鏡映する。
    #[value(name = "HalfKP-hm-b8")]
    #[serde(rename = "HalfKP-hm-b8")]
    HalfKpHmB8,
}

impl InputFeatureSet {
    /// 入力特徴集合の特徴数。P型では圧縮前の13,680行。
    pub fn dimensions(self) -> usize {
        match self {
            Self::P => FEATURE_COUNT,
            Self::HalfKpHmB8 => HalfKpHmB8::DIMENSIONS,
        }
    }

    /// 第1層用逆索引の行数。P型だけ埋め草行を含む。
    pub fn first_layer_rows(self) -> usize {
        match self {
            Self::P => CANONICAL_ROWS,
            Self::HalfKpHmB8 => HalfKpHmB8::DIMENSIONS,
        }
    }
}

pub(super) fn canonical_row(base: u32) -> u32 {
    let square = base % 144;
    let file = square % 12;
    base / 144 * 72 + square / 12 * 6 + file.min(11 - file)
}

pub(super) fn encode_p_feature(base: u32) -> u32 {
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

pub(super) fn base_features(
    position: &Position,
    perspective: Color,
) -> [[u32; FEATURE_COLUMNS]; 2] {
    let mut result = [[SENTINEL; FEATURE_COLUMNS]; 2];
    for (view, side) in [perspective, perspective.opposite()]
        .into_iter()
        .enumerate()
    {
        for square in Square::all() {
            if let Some(piece) = position.piece_at(square) {
                result[view][square.dense_index()] = feature_index(side, piece, square) as u32;
            }
        }
        if let Some(trigger) = position.lion_taken_by_non_lion() {
            result[view][144] = lion_feature_index(side, trigger.square) as u32;
        }
    }
    result
}

pub(super) fn input_features(
    position: &Position,
    bases: &[[u32; FEATURE_COLUMNS]; 2],
    feature_set: InputFeatureSet,
    perspective: Color,
) -> [[u32; FEATURE_COLUMNS]; 2] {
    let mut result = [[SENTINEL; FEATURE_COLUMNS]; 2];
    for (view, side) in [perspective, perspective.opposite()]
        .into_iter()
        .enumerate()
    {
        let context = HalfKpHmB8::context(position, side);
        for (column, &base) in bases[view].iter().enumerate() {
            if base != SENTINEL {
                result[view][column] = match feature_set {
                    InputFeatureSet::P => encode_p_feature(base),
                    InputFeatureSet::HalfKpHmB8 => HalfKpHmB8::index(context, base as usize) as u32,
                };
            }
        }
    }
    result
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
