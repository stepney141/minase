//! v0または初期PSTを用いた整数の駒除去参照。
use super::{
    Batch, Error, Result,
    batch::{FEATURE_COLUMNS, SENTINEL},
    model::{CANONICAL_ROWS, canonical},
};
use minase::eval::{
    features::{FEATURE_COUNT, PIECE_STATE_COUNT, mirror_feature, piece_state},
    handcrafted,
};
use minase_core::{Color, PieceCode, PieceKind};

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
    /// 自駒の行に+8×value、相手駒に−8×valueを両端点へ同じ値で置き、先獅子の行は0とする。
    /// valueは[`v0_piece_values`]であり、表の単位は1/8センチポーンである。
    /// 評価の±28,999への切り詰めがなければ、自駒の除去は評価を下げ、相手駒の除去は上げる。
    /// 同時学習の参照にはこの表だけを使う。
    pub fn v0() -> Self {
        let values = v0_piece_values();
        let mut rows = vec![[0; 2]; CANONICAL_ROWS];
        for (row, value) in rows[..94 * 72].iter_mut().enumerate() {
            let sign = if row / (47 * 72) == 0 { 1 } else { -1 };
            *value = [sign * i64::from(values[row / 72 % 47]) * 8; 2];
        }
        Self { rows }
    }
    /// `--removal-reference init`に使う初期MNPTの整数PSTを受け取る。
    /// 鏡映対の値が異なれば`removal reference mirror sharing`エラーにし、平均して補わない。
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
    /// 参照表の1/8センチポーンの端点和Tから`X = q×T_mg + (90−q)×T_eg`と除去後のX′を求める。
    /// 差分は`clamp(trunc(X′/720), ±28,999) − clamp(trunc(X/720), ±28,999)`とする。
    /// 符号にはこの整数差分、大きさには切り捨て前の`|X′−X|/720`を使う。
    /// 評価上限で差分0になる除去は対象外である。
    pub fn deltas(&self, batch: &Batch, b: usize) -> Result<(Vec<i32>, Vec<f64>)> {
        if b >= batch.len() || self.rows.len() != CANONICAL_ROWS {
            return Err(Error::Invalid("removal reference shape or position"));
        }
        let columns = batch.columns_at(b);
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
        let [q, after_q] = batch.phase_numerators()[b].map(i64::from);
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
