//! 診断と書き出し時の検査が使う参照差分。
//! 行番号0以上47×72未満は手番側、47×72以上94×72未満は相手側の駒で、それ以上は先獅子の行である。

use super::check_batch_arrays;
use crate::nnue::{
    data::{BatchSlot, CANONICAL_ROWS, FEATURE_COLUMNS, SENTINEL},
    model::{Error, Result, canonical},
};
use minase::eval::{
    features::{FEATURE_COUNT, PIECE_STATE_COUNT, mirror_feature, piece_state},
    handcrafted,
};
use minase_core::{Color, PieceCode, PieceKind, Position, PositionBuilder, Square};

const PHASE_DIVISOR: i64 = 90;
const EVALUATION_LIMIT: i64 = 28_999;

/// v0の駒価値を[`handcrafted::piece_value`]から取り、駒状態ごとに並べる。
pub fn v0_piece_values() -> [i32; PIECE_STATE_COUNT] {
    let mut values = [0; PIECE_STATE_COUNT];
    for kind in PieceKind::ALL {
        let value = handcrafted::piece_value(kind);
        values[kind.index()] = value;
        if let Some(piece) = PieceCode::new(Color::Black, kind) {
            values[piece_state(piece)] = value;
        }
    }
    values
}

/// 鏡映共有の整数PST。1/8センチポーンの2端点を正準行で保持する。
#[derive(Clone, Debug)]
pub struct RemovalReference {
    rows: Vec<[i64; 2]>,
}
impl RemovalReference {
    /// 手番側の行に+8×v、相手側に−8×vを両端点へ同じ値で置く。
    /// vは[`v0_piece_values`]のセンチポーン値であり、表の単位は1/8センチポーン、先獅子の行は0である。
    pub fn v0() -> Self {
        let values = v0_piece_values();
        let mut rows = vec![[0; 2]; CANONICAL_ROWS];
        for (row, value) in rows[..94 * 72].iter_mut().enumerate() {
            let sign = if row / (47 * 72) == 0 { 1 } else { -1 };
            *value = [sign * i64::from(values[row / 72 % 47]) * 8; 2];
        }
        Self { rows }
    }
    /// 独立な整数PSTを参照として使う。境界と上限制約の試験にも使える。
    pub fn new(endpoints: &[[i16; 2]]) -> Result<Self> {
        if endpoints.len() != FEATURE_COUNT {
            return Err(Error::Invalid("removal reference length"));
        }
        let mut rows = vec![[0; 2]; CANONICAL_ROWS];
        for base in 0..FEATURE_COUNT {
            if endpoints[base] != endpoints[mirror_feature(base)] {
                return Err(Error::Invalid("removal reference mirror sharing"));
            }
            rows[canonical(base)] = endpoints[base].map(i64::from);
        }
        Ok(Self { rows })
    }
    /// 参照表の端点和から`before = q×Σmg + (90−q)×Σeg`と除去後のafterを求める。
    /// 整数化は720で割って0方向へ切り捨て、±28,999へ制限する。
    /// 符号用の差は`integer(after)−integer(before)`、大きさは切り捨て前の`|after−before|/720`である。
    /// 評価上限によって整数差分が0になる除去も区別する。
    pub fn deltas(&self, batch: &BatchSlot, b: usize) -> Result<(Vec<i32>, Vec<f64>)> {
        check_batch_arrays(batch)?;
        if b >= batch.len() || self.rows.len() != CANONICAL_ROWS {
            return Err(Error::Invalid("removal reference shape or position"));
        }
        let columns = &batch.linear_features()[b * FEATURE_COLUMNS..(b + 1) * FEATURE_COLUMNS];
        let row = |col: usize| {
            if columns[col] == SENTINEL {
                [0; 2]
            } else {
                self.rows[columns[col] as usize]
            }
        };
        let mut sums = [0_i64; 2];
        for col in 0..FEATURE_COLUMNS {
            for (s, value) in sums.iter_mut().zip(row(col)) {
                *s += value;
            }
        }
        let q = i64::from(batch.piece_counts()[b].saturating_sub(2)).min(PHASE_DIVISOR);
        let after_q = i64::from(batch.piece_counts()[b].saturating_sub(3)).min(PHASE_DIVISOR);
        let before = q * sums[0] + (PHASE_DIVISOR - q) * sums[1];
        let integer = |x: i64| (x / (PHASE_DIVISOR * 8)).clamp(-EVALUATION_LIMIT, EVALUATION_LIMIT);
        let mut deltas = vec![0; 144];
        let mut magnitude = vec![0.0; 144];
        for col in 0..144 {
            let w = row(col);
            let after = after_q * (sums[0] - w[0]) + (PHASE_DIVISOR - after_q) * (sums[1] - w[1]);
            deltas[col] = (integer(after) - integer(before)) as i32;
            magnitude[col] = ((after - before) as f64 / (PHASE_DIVISOR * 8) as f64).abs();
        }
        Ok((deltas, magnitude))
    }
}

/// 除いた駒が先獅子の升にいても、升そのものは変えない。
pub(crate) fn remove_piece(position: &Position, removed: Square) -> crate::data::Result<Position> {
    let mut builder = PositionBuilder::new(position.side_to_move());
    for square in Square::all() {
        if square != removed
            && let Some(piece) = position.piece_at(square)
        {
            builder
                .put(square, piece)
                .map_err(crate::data::Error::PositionBuild)?;
        }
    }
    let mut after = builder
        .finish()
        .map_err(crate::data::Error::PositionBuild)?;
    after
        .set_lion_capture(position.lion_capture_square())
        .map_err(crate::data::Error::Position)?;
    Ok(after)
}
