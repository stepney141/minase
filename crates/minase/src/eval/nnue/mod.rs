//! MNUE版5の符号化・復号、差分更新および整数推論。

pub(crate) mod accumulator;
pub mod feature_set;
mod format;
pub mod head;
mod stats;
#[cfg(test)]
mod tests;

use super::features::{FEATURE_COUNT, PIECE_STATE_COUNT, feature_index, piece_state};
use feature_set::{FeatureSet, P};
pub use format::{EncodeInput, Error, architecture, encode};
use head::{DirectScrelu, Head};
use minase_core::{Color, Move, PieceCode, PieceKind, Position};
pub use stats::AccumulatorStats;
use std::marker::PhantomData;
use std::sync::{Arc, OnceLock};

const EVALUATION_LIMIT: i32 = 28_999;

/// 検証済みのMNUE整数重みと探索用駒価値。
pub struct Network<F: FeatureSet, H: Head> {
    accumulator_stats: stats::SharedStats,
    linear: Box<[[i16; 2]]>,
    weights: Box<[i16]>,
    biases: Box<[i32]>,
    head: H,
    feature_set: PhantomData<F>,
    width: usize,
    scale: i64,
    rule_set: [u8; 32],
    piece_values: [i32; PIECE_STATE_COUNT],
    max_promotion_gain: i32,
    k: f32,
    checksum: [u8; 32],
}

/// 探索が使う入力特徴集合。
pub type ActiveFeatureSet = P;
/// 探索が使う出力部。
pub type ActiveHead = DirectScrelu;
/// 探索が使う具体化済みのNNUE。
pub type Nnue = Network<ActiveFeatureSet, ActiveHead>;

impl<F: FeatureSet, H: Head> Network<F, H> {
    /// 盤上の駒コードに対応する駒価値をセンチポーンで返す。
    ///
    /// 値はMNUE本体に格納した探索用駒価値を使う。
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

    /// 第1層の幅Hを返す。
    pub const fn width(&self) -> usize {
        self.width
    }

    /// `docs/plans/move-ordering.md`「候補5」の同点破りに使う、手番側視点の線形経路（PST）の差分。
    /// 非線形経路は含めず、補間後の除算前の分子を返す。
    pub(crate) fn quiet_move_pst_delta(&self, position: &Position, mv: Move) -> i64 {
        let side = position.side_to_move();
        let piece = position.piece_at(mv.from).expect("quiet mover exists");
        let piece_after = if mv.promote {
            piece.promote().expect("legal promotion")
        } else {
            piece
        };
        let from = self.linear[feature_index(side, piece, mv.from)];
        let to = self.linear[feature_index(side, piece_after, mv.to)];
        let q = i64::from(position.occupied().popcount().saturating_sub(2)).min(90);
        let [mg, eg] = [
            i64::from(to[0]) - i64::from(from[0]),
            i64::from(to[1]) - i64::from(from[1]),
        ];
        q * mg + (90 - q) * eg
    }

    /// 生成時の規則セット名を返す。
    pub fn rule_set(&self) -> &str {
        let end = self
            .rule_set
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(32);
        std::str::from_utf8(&self.rule_set[..end]).expect("validated UTF-8 rule set")
    }
}

/// 局面を完全再計算し、手番側の視点で評価する。
pub fn evaluate<F: FeatureSet, H: Head>(nnue: &Network<F, H>, position: &Position) -> i32 {
    evaluate_for_diagnostics(nnue, position, position.side_to_move())
}

/// 先獅子を含む配置を保ち、指定した視点から完全再計算する。
pub fn evaluate_for_diagnostics<F: FeatureSet, H: Head>(
    nnue: &Network<F, H>,
    position: &Position,
    perspective: Color,
) -> i32 {
    nnue.evaluate_accumulator(&nnue.refresh_accumulator(position), perspective)
}

static EMBEDDED: &[u8] = include_bytes!("../../../nets/nnue.bin");
static WEIGHTS: OnceLock<Result<Arc<Nnue>, Error>> = OnceLock::new();

/// 検証済みの埋め込みNNUE重みを返す。
pub fn weights() -> Result<Arc<Nnue>, Error> {
    match WEIGHTS.get_or_init(|| Nnue::decode(EMBEDDED).map(Arc::new)) {
        Ok(nnue) => Ok(Arc::clone(nnue)),
        Err(error) => Err(error.clone()),
    }
}

/// USI診断に使う線形の特徴別寄与と非線形出力。
pub(crate) struct Breakdown {
    pub(crate) score: i32,
    pub(crate) q: i64,
    pub(crate) board: [i64; 144],
    pub(crate) lion: i64,
    pub(crate) nonlinear: i64,
}

/// 特徴の線形寄与は除算前の値として返す。
pub(crate) fn breakdown(nnue: &Nnue, position: &Position) -> Breakdown {
    use super::features::{feature_index, lion_feature_index};
    let side = position.side_to_move();
    let accumulator = nnue.refresh_accumulator(position);
    let q = i64::from(accumulator.piece_count.saturating_sub(2)).min(90);
    let numerator = |feature: usize| {
        let [mg, eg] = nnue.linear[feature].map(i64::from);
        q * mg + (90 - q) * eg
    };
    let mut result = Breakdown {
        score: nnue.evaluate_accumulator(&accumulator, side),
        q,
        board: [0; 144],
        lion: 0,
        nonlinear: nnue.nonlinear_cp(&accumulator, side),
    };
    for square in position.occupied() {
        let piece = position
            .piece_at(square)
            .expect("occupied square contains piece");
        result.board[square.dense_index()] = numerator(feature_index(side, piece, square));
    }
    if let Some(trigger) = position.lion_taken_by_non_lion() {
        result.lion = numerator(lion_feature_index(side, trigger.square));
    }
    result
}
