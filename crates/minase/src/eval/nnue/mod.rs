//! MNUE版5の復号、差分更新、および線形と非線形の和による整数評価。
//! 線形の経路は[`crate::eval::features`]の13,680個の基底特徴に載る序中盤用と終盤用のi16重みを、手番側視点で足す。
//! 重みの単位は1/8センチポーンであり、王駒を含む盤上総駒数Nから`q = min(90, max(0, N − 2))`を求め、`(q×mg + (90−q)×eg)/720`で補間する。
//! 非線形の経路は両視点の幅Hの第1層を、値を制限してから2乗する活性化関数SCReLUに通す。
//! 出力部は`direct-screlu`（直接内積）、`direct-screlu-phase2`（2端点補間）、
//! `screlu-hidden16`（幅16の隠れ層を加算）、`screlu-hidden16-phase2`（隠れ層を共有する2端点補間）を扱う。
//! 各端点の区画は出力重み`i16[2H]`とバイアス`i32`で、序盤、終盤の順に置く。
//! 隠れ層つきでは、続いて行優先の`W1: i8[16][2H]`、`b1: i32[16]`、各端点の`w2: i16[16]`を置く。
//! 出力区画はファイル先頭から`192 + 13,680×4 + 47×4 + D×2H + 4H`バイト目に始まり、
//! 全長はその位置に`Head::byte_len(H)`を足した値となる。Dは入力特徴数、Hは第1層の幅である。
//! 補間は線形経路と同じqを使い、負値も床方向へ丸める。
//! PST＋FMを置き換えた理由は`docs/adr/use-nnue-as-the-only-search-evaluation.md`にある。
//! 構造の理由は`docs/adr/compose-the-nnue-of-a-pst-path-and-a-direct-screlu-head.md`、型引数を使う理由は`docs/adr/parameterize-the-nnue-by-feature-set-and-head-types.md`にある。

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

/// 静的評価の絶対値の上限。単位はセンチポーンである。
/// [`crate::search::MATE_THRESHOLD`]未満に保ち、詰み帯と区別するとともに置換表の`i16`へ収まる範囲に制限する。
pub(crate) const EVALUATION_LIMIT: i32 = 28_999;

/// 検証済みのMNUE整数重みと探索用駒価値。
pub struct Network<F: FeatureSet, H: Head> {
    accumulator_stats: stats::SharedStats,
    /// 基底特徴ごとの[序中盤, 終盤]。単位は1/8センチポーンである。
    linear: Box<[[i16; 2]]>,
    /// 特徴ごとにH個が連続する第1層のi16重み。実数に対する尺度は127である。
    weights: Box<[i16]>,
    /// 第1層のi32バイアス。実数に対する尺度は127である。
    biases: Box<[i32]>,
    head: H,
    feature_set: PhantomData<F>,
    width: usize,
    /// 勝率尺度（1ロジットあたりのcp）を`cp_per_logit`とする。
    /// `round_ties_even(f64(cp_per_logit)×65,536/(127×4,096))`で求めた出力換算係数。
    scale: i64,
    /// 生成時の規則セット名。実行時の規則との一致は要求しない。
    rule_set: [u8; 32],
    /// 学習で更新しない固定の探索用駒価値。
    /// 盤上に現れる駒の値は[`Self::piece_value`]の不変条件を満たす。
    piece_values: [i32; PIECE_STATE_COUNT],
    /// 全駒状態の固定駒価値の成り前後の差と0の最大値。
    max_promotion_gain: i32,
    /// 出力をセンチポーンへ戻す勝率尺度。
    cp_per_logit: f32,
    /// 重み本体のSHA-256。
    checksum: [u8; 32],
}

/// 探索が使う、王の位置で条件付けないP型の入力特徴集合。
/// 理由は`docs/adr/use-p-type-input-features-for-the-nnue.md`にある。
pub type ActiveFeatureSet = P;
/// 探索が使う出力部。
pub type ActiveHead = DirectScrelu;
/// 探索が使う具体化済みのNNUE。
pub type Nnue = Network<ActiveFeatureSet, ActiveHead>;

impl<F: FeatureSet, H: Head> Network<F, H> {
    /// 盤上の駒コードに対応する駒価値をセンチポーンで返す。
    ///
    /// MNUE重みファイルに格納した固定の探索用駒価値を使い、NNUEの学習では更新しない。
    /// 盤上に現れ得る非王駒はすべて正値であり、王駒は非王駒の最大値に成っていない歩兵価値を加えた値とする。
    /// この正値性は、静的交換評価（SEE）の最も安い駒の選択、歩兵価値からの余裕値の計算、捕獲価値の合計による捕獲手の判別の前提である。
    /// 固定値を使う理由は`docs/adr/derive-search-piece-values-from-the-learned-pst.md`にある。
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
    /// 探索の余裕値とaspiration windowの幅を百分率で換算する基準値である。
    #[inline]
    pub fn pawn_value(&self) -> i32 {
        let pawn = PieceCode::new(Color::Black, PieceKind::Pawn)
            .expect("pawn must have an unpromoted code");
        self.piece_value(pawn)
    }

    /// SEEの取り返しで得られる成り益の上限を、固定の探索用駒価値の差として返す。
    pub(crate) const fn max_promotion_gain(&self) -> i32 {
        self.max_promotion_gain
    }

    /// 学習時に使った勝率尺度を返す。
    pub const fn cp_per_logit(&self) -> f32 {
        self.cp_per_logit
    }

    /// 重み本体のSHA-256を返す。
    pub const fn checksum(&self) -> &[u8; 32] {
        &self.checksum
    }

    /// 第1層の幅Hを返す。
    pub const fn width(&self) -> usize {
        self.width
    }

    /// 静かな手の履歴値が同じ場合の同点破りに使う、手番側視点の線形の移動差分。
    /// 移動元の特徴を引き、移動先の特徴を足すときは成りも反映する。
    /// 非線形の経路を含めず、`q×Δmg + (90−q)×Δeg`を720で割る前の分子として返す。
    /// 線形の経路は王の位置で条件付けないため、入力特徴集合によらず成り立つ。
    /// 理由は`docs/adr/order-quiets-by-the-stockfish-history-sum-and-update-histories-in-stockfish-form.md`と`docs/adr/compose-the-nnue-of-a-pst-path-and-a-direct-screlu-head.md`にある。
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

/// 埋め込んだ`nets/nnue.bin`を初回呼び出しで1回だけ[`Nnue::decode`]で検証する。
/// 以後は同じ結果を返し、成功時は同じネットを共有する`Arc`、失敗時は同じ`Error`の複製を返す。
/// 呼び手は起動時の失敗を検出して停止する。
/// 埋め込む理由は`docs/adr/use-nnue-as-the-only-search-evaluation.md`にある。
pub fn weights() -> Result<Arc<Nnue>, Error> {
    match WEIGHTS.get_or_init(|| Nnue::decode(EMBEDDED).map(Arc::new)) {
        Ok(nnue) => Ok(Arc::clone(nnue)),
        Err(error) => Err(error.clone()),
    }
}

/// USIの評価の内訳表示に使う、線形の特徴別寄与と非線形の出力。
pub(crate) struct Breakdown {
    /// 線形と非線形の和を±28,999へ制限したセンチポーン評価。
    pub(crate) score: i32,
    /// 盤上総駒数から求めた序中盤の補間係数。
    pub(crate) q: i64,
    /// 升ごとの線形寄与`q×mg + (90−q)×eg`。720で割る前の分子である。
    pub(crate) board: [i64; 144],
    /// 先獅子の線形寄与。単位は[`Self::board`]と同じである。
    pub(crate) lion: i64,
    /// センチポーンへ換算した非線形の出力。
    pub(crate) nonlinear: i64,
}

/// 局面を全再計算し、手番側視点の[`Breakdown`]を返す。
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
        nonlinear: nnue.nonlinear_cp(&accumulator, side, q as i32),
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
