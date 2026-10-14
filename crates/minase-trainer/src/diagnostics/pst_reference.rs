//! MNSDの駒コードから直接計算する、エンジンとは独立したPST参照評価。
//!
//! 実数PSTの入力配列は書き手の[`crate::pst::encode_pst_npz`]に定める。
//! この診断の読み手は端点のdtypeとして`<f4`と`<f8`を受け付け、f32へ変換する。

use crate::data::Sample;
use std::path::Path;

use minase::eval::pst::Pst;
use serde::Serialize;

use super::Result;
use crate::{
    nn::npz,
    pst::{export::numpy_sum, linear},
};

pub(super) const FEATURE_COUNT: usize = 13_680;
/// [`derived`]で静的な駒価値を求める35状態の順序。
/// 29〜46の18状態に、4、5、6、7、8、9、10、12、17、20、22、23、24、25、26、27、28を続ける。
pub(super) const STATES: [usize; 35] = [
    29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 4, 5, 6, 7, 8, 9, 10,
    12, 17, 20, 22, 23, 24, 25, 26, 27, 28,
];

#[derive(Default, Serialize)]
pub(super) struct Models<T> {
    pub base: T,
    pub candidate: T,
}
impl<T> Models<T> {
    pub fn each(&self) -> [(&'static str, &T); 2] {
        [("base", &self.base), ("candidate", &self.candidate)]
    }
}

pub(super) struct Weights {
    pub engine: Pst,
}
impl Weights {
    pub fn read(path: &Path) -> Result<Self> {
        Ok(Self {
            engine: Pst::decode(&std::fs::read(path)?)?,
        })
    }

    pub fn k(&self) -> f64 {
        f64::from(self.engine.k())
    }

    /// 独立した整数PST参照による、手番側視点のセンチポーン評価。
    /// 各特徴の1/8センチポーン単位の整数重みをi64で足して`S_mg`と`S_eg`を作る。
    /// `(q×S_mg + (90−q)×S_eg) / 720`を0方向へ切り捨て、[−30000, 30000]へ切り詰める。
    /// エンジンの補間関数は使わない。
    pub fn evaluate(&self, features: &Features) -> i32 {
        let mut sums = [0_i64; 2];
        for &feature in &features.columns {
            if feature != FEATURE_COUNT {
                for (sum, &weight) in sums.iter_mut().zip(&self.engine.weights()[feature]) {
                    *sum += i64::from(weight);
                }
            }
        }
        let q = i64::from(features.q);
        // Rustの符号付き除算は0方向への切り捨て。エンジンの補間関数は使わない。
        ((q * sums[0] + (90 - q) * sums[1]) / 720).clamp(-30_000, 30_000) as i32
    }
}

pub(super) fn piece(byte: u8) -> (usize, u8) {
    // Sampleは局面から構築した駒コードだけを保持する。
    let color = u8::from(byte >= 65);
    let payload = byte - (1 + color * 64);
    let kind = usize::from(payload % 29);
    const PROMOTABLE: [usize; 18] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 13, 14, 15, 16, 17, 18, 19];
    let state = if payload < 29 {
        PROMOTABLE
            .iter()
            .position(|&k| k == kind)
            .map_or(kind, |rank| 29 + rank)
    } else {
        kind
    };
    (state, color)
}

/// MNSDの記録から直接作る、エンジンとは独立したPST特徴。
/// `count`は盤上の駒数で、先獅子特徴を駒数へ加えず、`q = min(max(count−2, 0), 90)`を位相分子とする。
/// `columns`は145個の特徴番号で、各升は`(相対色×47 + 駒状態)×144 + 手番側から見た升番号`である。
/// 145番目は先獅子の対象升の`94×144 + 手番側から見た升番号`である。
/// 空升と先獅子なしは、重み0の埋め草番号13,680を置く。
/// 相対色は手番側の駒が0、相手の駒が1で、後手番では段を反転して`(11 − 升/12)×12 + 升%12`とする。
/// 駒状態は[`piece`]が返す番号で、成っていない成れる駒は`29 + 成れる駒の中の順位`、それ以外は駒種の番号である。
pub(super) struct Features {
    pub columns: [usize; 145],
    pub count: usize,
    pub q: u8,
}
impl Features {
    pub fn new(record: &Sample) -> Self {
        let bytes = record.board_codes();
        let mut columns = [FEATURE_COUNT; 145];
        let mut count = 0_usize;
        let square_from_side = |square: usize| {
            if record.side_to_move() as u8 == 0 {
                square
            } else {
                (11 - square / 12) * 12 + square % 12
            }
        };
        for (square, &byte) in bytes[..144].iter().enumerate() {
            if byte != 0 {
                let (state, color) = piece(byte & 127);
                columns[square] = (usize::from(color != record.side_to_move() as u8) * 47 + state)
                    * 144
                    + square_from_side(square);
                count += 1;
            }
        }
        if let Some(lion) = record.lion_square() {
            columns[144] = 94 * 144 + square_from_side(lion.dense_index());
        }
        Self {
            columns,
            count,
            q: count.saturating_sub(2).min(90) as u8,
        }
    }
}

pub(super) struct FloatWeights(Vec<[f32; 2]>);
impl FloatWeights {
    pub fn read(path: &Path) -> Result<Self> {
        let mut arrays = npz::decode_selected(&std::fs::read(path)?, &["middlegame", "endgame"])?;
        let mut weights = vec![[0.0; 2]; FEATURE_COUNT + 1];
        for (endpoint, key) in ["middlegame", "endgame"].into_iter().enumerate() {
            let array = arrays.remove(key).ok_or("missing float PST endpoint")?;
            if array.shape != [FEATURE_COUNT] {
                return Err("float weights do not match candidate feature count".into());
            }
            let values: Vec<f32> = match array.dtype.as_str() {
                "<f4" => array
                    .data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|&bytes| f32::from_le_bytes(bytes))
                    .collect(),
                "<f8" => array
                    .data
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|&bytes| f64::from_le_bytes(bytes) as f32)
                    .collect(),
                _ => {
                    return Err(
                        "float PST endpoints must have little-endian f32 or f64 dtype".into(),
                    );
                }
            };
            if values.iter().any(|v| !v.is_finite()) {
                return Err("non-finite float PST weight".into());
            }
            for (row, value) in weights.iter_mut().zip(values) {
                row[endpoint] = value;
            }
        }
        Ok(Self(weights))
    }

    pub fn evaluate(&self, features: &Features) -> f32 {
        let sums = std::array::from_fn(|endpoint| {
            numpy_sum(&features.columns.map(|feature| self.0[feature][endpoint]))
        });
        linear::blend(sums, features.q)
    }
}

/// 指定端点の自駒と敵駒の全升平均差から、[`STATES`]順の静的な駒価値をセンチポーンで求める。
/// 状態sの自駒の行`s×144 … s×144+143`の重みの和から、敵駒の行`(47+s)×144 … (47+s)×144+143`の和を引く。
/// 重みの単位は1/8センチポーンであり、この差を2,304（144升×8×2）で割る。
/// 半整数を0から遠ざける向きに丸めて整数にする。
/// `endpoint = 0`は序中盤、1は終盤を表す。
pub(super) fn derived(weights: &Weights, endpoint: usize) -> Vec<i32> {
    STATES
        .iter()
        .map(|&state| {
            let own: i64 = weights.engine.weights()[state * 144..(state + 1) * 144]
                .iter()
                .map(|w| i64::from(w[endpoint]))
                .sum();
            let enemy: i64 = weights.engine.weights()[(47 + state) * 144..(48 + state) * 144]
                .iter()
                .map(|w| i64::from(w[endpoint]))
                .sum();
            ((own - enemy) as f64 / 2304.0).round() as i32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn piece_values() -> [i32; 47] {
        let mut values = [100; 47];
        values[11] = 200;
        values[21] = 200;
        values
    }

    fn record(board: &[(usize, u8)], side: u8, lion: u8) -> Sample {
        use minase_core::{Color, PieceCode, PieceKind, PositionBuilder, Square};
        let mut builder = PositionBuilder::new(if side == 0 {
            Color::Black
        } else {
            Color::White
        });
        for &(square, code) in board {
            let color = if code >= 65 {
                Color::White
            } else {
                Color::Black
            };
            let payload = code - 1 - 64 * color as u8;
            let kind = PieceKind::from_index(payload % 29).unwrap();
            let piece = if payload >= 29 {
                PieceCode::new_promoted(color, kind)
            } else {
                PieceCode::new(color, kind)
            }
            .unwrap();
            builder
                .put(Square::from_dense(square).unwrap(), piece)
                .unwrap();
        }
        let mut position = builder.finish().unwrap();
        position
            .set_lion_capture(Square::from_dense(lion as usize))
            .unwrap();
        Sample::from_position(
            &position,
            0,
            crate::data::Outcome::Draw,
            0,
            0,
            minase::training::records::Origin::Synthetic,
        )
        .unwrap()
    }

    #[test]
    fn integer_reference_truncates_after_blending_and_clamps() {
        let mut table = vec![[0; 2]; FEATURE_COUNT];
        // 後手の自歩はrankを反転。盤上3駒なのでq=1、先獅子は駒数に含めない。
        table[29 * 144 + 132] = [-8, -16];
        table[94 * 144 + 133] = [1, 1];
        let weights = Weights {
            engine: Pst::from_weights(&table, piece_values(), 1000.0, "L0,P0,R1,E0").unwrap(),
        };
        let f = Features::new(&record(&[(0, 65), (1, 12), (2, 76)], 1, 1));
        assert_eq!(f.q, 1);
        assert_eq!(weights.evaluate(&f), -1); // (-7 - 15*89)/720は0方向へ切り捨てる。
        let weights = Weights {
            engine: Pst::from_weights(
                &vec![[i16::MAX; 2]; FEATURE_COUNT],
                piece_values(),
                1000.0,
                "L0,P0,R1,E0",
            )
            .unwrap(),
        };
        let f = Features::new(&record(
            &(0..20).map(|s| (s, 1)).collect::<Vec<_>>(),
            0,
            255,
        ));
        assert_eq!(weights.evaluate(&f), 30_000);
    }

    #[test]
    fn derived_piece_values_round_both_half_signs_away_from_zero() {
        let mut table = vec![[0; 2]; FEATURE_COUNT];
        table[29 * 144] = [1152, -1152];
        table[30 * 144] = [3456, -3456];
        table[31 * 144] = [1151, -1151];
        let model = Weights {
            engine: Pst::from_weights(&table, piece_values(), 1000.0, "L0,P0,R1,E0").unwrap(),
        };
        assert_eq!(&derived(&model, 0)[..3], &[1, 2, 0]);
        assert_eq!(&derived(&model, 1)[..3], &[-1, -2, 0]);
    }
}
