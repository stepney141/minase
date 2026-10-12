//! MNSDの駒コードから直接計算する、エンジンとは独立したPST参照評価。

use std::path::Path;

use minase::{eval::pst::Pst, training::records::Record};
use serde::Serialize;

use super::Result;
use crate::{
    nn::npz,
    pst::{export::numpy_sum, linear},
};

pub(super) const FEATURE_COUNT: usize = 13_680;
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
    // 呼び手はRecord::decodeでMNSDコードを検査済み。
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

pub(super) struct Features {
    pub columns: [usize; 145],
    pub count: usize,
    pub q: u8,
}
impl Features {
    pub fn new(record: &Record) -> Self {
        let bytes = record.encode();
        let mut columns = [FEATURE_COUNT; 145];
        let mut count = 0_usize;
        let square_from_side = |square: usize| {
            if bytes[144] == 0 {
                square
            } else {
                (11 - square / 12) * 12 + square % 12
            }
        };
        for (square, &byte) in bytes[..144].iter().enumerate() {
            if byte != 0 {
                let (state, color) = piece(byte);
                columns[square] = (usize::from(color != bytes[144]) * 47 + state) * 144
                    + square_from_side(square);
                count += 1;
            }
        }
        if bytes[145] != 255 {
            columns[144] = 94 * 144 + square_from_side(usize::from(bytes[145]));
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

// 各端点の自駒・敵駒144升の平均差。半整数は最近接偶数ではなく0から遠ざける。
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

    use minase::training::records::RECORD_LEN;

    fn record(board: &[(usize, u8)], side: u8, lion: u8) -> Record {
        let mut bytes = [0; RECORD_LEN];
        for &(square, piece) in board {
            bytes[square] = piece;
        }
        bytes[144] = side;
        bytes[145] = lion;
        Record::decode(&bytes).unwrap()
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
