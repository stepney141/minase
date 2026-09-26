//! 量子化した学習PSTの復号、埋め込みおよび整数評価。

pub(crate) mod accumulator;
mod features;
mod format;
#[cfg(test)]
mod tests;

pub use format::Error;

use std::sync::{Arc, OnceLock};

use self::features::{FEATURE_COUNT, active_features, piece_state};
use crate::{Color, PieceCode, PieceKind, Position};

/// 駒種と現在の成り可否を区別した駒状態の総数。
pub const PIECE_STATE_COUNT: usize = features::PIECE_STATE_COUNT;

/// 駒コードを駒状態の番号へ変換する。
///
/// # Panics
///
/// `piece`が空升または盤外の番兵を表す場合にパニックする。
#[inline]
pub fn piece_state_of(piece: PieceCode) -> usize {
    piece_state(piece)
}

/// 静的評価値の絶対値上限。
const EVALUATION_LIMIT: i32 = 28_999;

/// 学習PSTの量子化重みと勝率尺度。
pub struct Pst {
    /// 「段階6」（movegen-speedup-2.md）に従い、特徴ごとに序中盤と終盤を隣接させる。
    weights: [[i16; 2]; FEATURE_COUNT],
    /// 駒状態ごとのセンチポーン単位の駒価値。
    piece_values: [i32; PIECE_STATE_COUNT],
    /// SEEの逆引き前の判定に使う全駒種の成り益の非負上限（同「段階6」）。
    max_promotion_gain: i32,
    /// 学習時に使ったセンチポーンから勝率ロジットへの尺度。
    k: f32,
    /// 重み本体のSHA-256。学習データのヘッダに生成元のネットとして記録する。
    checksum: [u8; 32],
}

impl Pst {
    /// 盤上の駒コードに対応する駒価値をセンチポーンで返す。
    ///
    /// 値はMNPT本体に格納した探索用駒価値を使う。
    ///
    /// # Panics
    ///
    /// `piece`が空升または盤外の番兵を表す場合にパニックする。
    #[inline]
    pub fn piece_value(&self, piece: PieceCode) -> i32 {
        self.piece_values[piece_state(piece)]
    }

    /// 駒状態に対応する駒価値をセンチポーンで返す。
    ///
    /// # Panics
    ///
    /// `state`が`PIECE_STATE_COUNT`以上の場合にパニックする。
    #[inline]
    pub fn piece_value_of_state(&self, state: usize) -> i32 {
        self.piece_values[state]
    }

    /// 成っていない歩兵の駒価値をセンチポーンで返す。
    ///
    /// `docs/plans/strength-stage4.md`の「採用した余裕値」節で使う単位`p`。
    #[inline]
    pub fn pawn_value(&self) -> i32 {
        let pawn = PieceCode::new(Color::Black, PieceKind::Pawn)
            .expect("pawn must have an unpromoted code");
        self.piece_value(pawn)
    }

    /// 「段階6」（movegen-speedup-2.md）の取り返しで得られる成り益の上限を返す。
    pub(crate) const fn max_promotion_gain(&self) -> i32 {
        self.max_promotion_gain
    }

    /// 学習時に使った勝率尺度Kを返す。
    pub const fn k(&self) -> f32 {
        self.k
    }

    /// 重み本体のSHA-256を返す。
    pub const fn checksum(&self) -> &[u8; 32] {
        &self.checksum
    }
}

/// 盤上総駒数に応じて生重み和を補間し、最後に1回だけ整数除算する。
fn interpolate(sums: [i32; 2], piece_count: u32) -> i32 {
    let q = i64::from(piece_count.saturating_sub(2).min(90));
    let numerator = q * i64::from(sums[0]) + (90 - q) * i64::from(sums[1]);
    (numerator / 720).clamp(-i64::from(EVALUATION_LIMIT), i64::from(EVALUATION_LIMIT)) as i32
}

/// 学習PSTで局面を手番側の視点からセンチポーン評価する。
pub fn evaluate(pst: &Pst, position: &Position) -> i32 {
    let mut sums = [0_i32; 2];
    active_features(position, |feature| {
        pst.add_feature(&mut sums, feature, 1);
    });
    interpolate(sums, position.occupied().popcount())
}

/// 実行バイナリへ埋め込むMNPTバイト列。
static EMBEDDED: &[u8] = include_bytes!("../../../nets/pst.bin");
/// 埋め込み重みの復号結果を保持する領域。
static WEIGHTS: OnceLock<Result<Arc<Pst>, Error>> = OnceLock::new();

/// 検証済みの埋め込み学習PSTを返す。
pub fn weights() -> Result<Arc<Pst>, Error> {
    match WEIGHTS.get_or_init(|| Pst::decode(EMBEDDED).map(Arc::new)) {
        Ok(pst) => Ok(Arc::clone(pst)),
        Err(error) => Err(error.clone()),
    }
}
