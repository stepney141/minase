//! 量子化した学習PSTの復号と、PST単独またはPST＋FMの整数評価。
//! 探索からは呼ばれず、学習した評価関数の比較に使う。
//! 理由は`docs/adr/use-nnue-as-the-only-search-evaluation.md`にある。

pub(crate) mod accumulator;
mod features;
mod fm;
mod format;
pub use fm::{FM_RANK, FmWeights};
#[cfg(test)]
mod tests;

pub use format::Error;

use std::sync::{Arc, OnceLock};

use self::features::{FEATURE_COUNT, active_features, piece_state};
use minase_core::{Color, Move, PieceCode, PieceKind, Position, Square};

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

/// 学習PSTの量子化重みと勝率尺度（1ロジットあたりのcp）。
pub struct Pst {
    /// 特徴ごとの序中盤用と終盤用の重みを隣接させ、1回の読み込みで両端点を得る。
    /// 整数和なので、この配置によって評価値は変わらない。
    weights: [[i16; 2]; FEATURE_COUNT],
    /// MNPT版3のFM量子化表。次元0では補正を持たない。
    fm: Option<fm::Fm>,
    /// MNPTヘッダの規則セット欄。
    rule_set: [u8; 32],
    /// 駒状態ごとのセンチポーン単位の駒価値。
    piece_values: [i32; PIECE_STATE_COUNT],
    /// [`Self::max_promotion_gain`]が返す、使用中の駒価値表の成り益の非負上限。
    max_promotion_gain: i32,
    /// 学習時に使った勝率尺度。
    cp_per_logit: f32,
    /// 重み本体のSHA-256。学習データのヘッダに生成元のネットとして記録する。
    checksum: [u8; 32],
}

impl Pst {
    /// 特徴番号順の序中盤・終盤重みを、1/8センチポーン単位で返す。
    pub fn weights(&self) -> &[[i16; 2]; FEATURE_COUNT] {
        &self.weights
    }

    /// MNPTに格納された規則セット名を、末尾のNUL埋めを除いて返す。
    pub fn rule_set(&self) -> &str {
        std::str::from_utf8(&self.rule_set)
            .expect("validated rule set")
            .trim_end_matches('\0')
    }

    /// 静かな手の履歴値が同じ場合の同点破りに使う、手番側視点のPST差分。
    /// FMは含めず、補間後の除算前の分子を返す。
    /// 理由は`docs/adr/order-quiets-by-the-stockfish-history-sum-and-update-histories-in-stockfish-form.md`にある。
    pub(crate) fn quiet_move_pst_delta(&self, position: &Position, mv: Move) -> i64 {
        let side = position.side_to_move();
        let piece = position.piece_at(mv.from).expect("quiet mover exists");
        let piece_after = if mv.promote {
            piece.promote().expect("legal promotion")
        } else {
            piece
        };
        let from = self.weights[features::feature_index(side, piece, mv.from)];
        let to = self.weights[features::feature_index(side, piece_after, mv.to)];
        interpolation_numerator(
            [
                i32::from(to[0]) - i32::from(from[0]),
                i32::from(to[1]) - i32::from(from[1]),
            ],
            phase(position.occupied().popcount()),
        )
    }

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
    /// 探索の余裕値とaspiration windowの幅を百分率で換算する基準値である。
    #[inline]
    pub fn pawn_value(&self) -> i32 {
        let pawn = PieceCode::new(Color::Black, PieceKind::Pawn)
            .expect("pawn must have an unpromoted code");
        self.piece_value(pawn)
    }

    /// 使用中の駒価値表で、全駒状態の「成った後の価値 − 成る前の価値」と0の最大値を返す。
    /// 静的交換評価の早期終了に使う成り益の上限である。
    /// 探索ではNNUEの同名の値を用い、取り返す駒の成り益を最大に見積もっても損が余裕値以下なら、取り返しの逆引きを省く。
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
}

/// 初期配置の92枚から双方に残す王駒1枚ずつを除いた、補間係数の上限90。
const PHASE_MAX: i64 = 90;
/// 補間係数の上限90と、1/8センチポーンをセンチポーンへ戻す8の積720。
pub(crate) const INTERPOLATION_DIVISOR: i64 = 720;

/// 王駒を含む盤上総駒数Nから`q = min(90, max(0, N − 2))`を求める。
/// 2枚は双方の王駒を1枚ずつ残す基準であり、実際の王駒数を数えて引くのではない。
/// 先獅子の特徴と盤外の番兵は数えず、92枚を超える入力でもqを90に制限する。
/// 中将棋には駒打ちがないため、通常の対局でqは捕獲によってだけ減り、成りでは変わらない。
/// 理由は`docs/adr/interpolate-the-linear-path-by-total-piece-count.md`にある。
fn phase(piece_count: u32) -> i64 {
    i64::from(piece_count.saturating_sub(2)).min(PHASE_MAX)
}

/// [`phase`]のqで、`q×序中盤の和 + (90 − q)×終盤の和`をi64で計算する。
fn interpolation_numerator(sums: [i32; 2], q: i64) -> i64 {
    q * i64::from(sums[0]) + (PHASE_MAX - q) * i64::from(sums[1])
}

/// 分子を[`INTERPOLATION_DIVISOR`]で1回だけ割り、0方向へ切り捨てた後に±28,999センチポーンへ制限する。
fn evaluation_from_numerator(numerator: i64) -> i32 {
    (numerator / INTERPOLATION_DIVISOR)
        .clamp(-i64::from(EVALUATION_LIMIT), i64::from(EVALUATION_LIMIT)) as i32
}

/// 盤上総駒数に応じて生重み和を補間し、最後に1回だけ整数除算する。
fn interpolate(sums: [i32; 2], piece_count: u32) -> i32 {
    evaluation_from_numerator(interpolation_numerator(sums, phase(piece_count)))
}

/// 静的評価を表示するための、除算前の特徴別寄与。
pub(crate) struct Breakdown {
    /// 手番側視点の全計算評価値。
    pub(crate) score: i32,
    /// 序中盤の補間係数。
    pub(crate) q: i64,
    /// 升の密番号順に並べた駒の寄与。
    pub(crate) board: [i64; 144],
    /// 先獅子特徴の寄与。
    pub(crate) lion: i64,
    /// PSTと別に加えるFM補正のセンチポーン値。
    pub(crate) fm: Option<i64>,
}

/// 全計算と同じ特徴列挙と補間式で、表示用の内訳を得る。
/// 升ごとの寄与と先獅子の寄与は手番側視点の補間分子であり、その和を720で割って上下限を適用するとPST評価になる。
pub(crate) fn breakdown(pst: &Pst, position: &Position) -> Breakdown {
    let q = phase(position.occupied().popcount());
    let mut result = Breakdown {
        score: 0,
        q,
        board: [0; 144],
        lion: 0,
        fm: pst
            .fm
            .as_ref()
            .map(|fm| fm.correction(&fm.refresh(position.side_to_move(), position))),
    };
    let mut sums = [0_i32; 2];
    active_features(position, |feature| {
        pst.add_feature(&mut sums, feature, 1);
        let contribution = interpolation_numerator(pst.weights[feature].map(i32::from), q);
        if feature < features::BOARD_FEATURE_COUNT {
            let relative = feature % 144;
            let rank = relative / 12;
            let rank = match position.side_to_move() {
                Color::Black => rank,
                Color::White => 11 - rank,
            };
            let square = Square::new((relative % 12) as u8, rank as u8)
                .expect("feature coordinates must be on the board");
            result.board[square.dense_index()] = contribution;
        } else {
            result.lion += contribution;
        }
    });
    let baseline = interpolate(sums, position.occupied().popcount());
    result.score = match result.fm {
        Some(correction) => add_correction(baseline, correction),
        None => baseline,
    };
    result
}

/// FM補正を含まないPSTだけの評価を手番側の視点で返す。
pub fn evaluate_pst(pst: &Pst, position: &Position) -> i32 {
    let mut sums = [0_i32; 2];
    active_features(position, |feature| {
        pst.add_feature(&mut sums, feature, 1);
    });
    interpolate(sums, position.occupied().popcount())
}

/// 検査済みの手番側特徴と盤上総駒数から、FMを含まない整数PSTを評価する。
/// 特徴は鏡映後の升番号でもよく、埋め草は渡さない。
///
/// # Panics
///
/// 特徴番号が13,680以上の場合にパニックする。
pub fn evaluate_pst_features(
    pst: &Pst,
    features: impl IntoIterator<Item = usize>,
    piece_count: u32,
) -> i32 {
    let mut sums = [0_i32; 2];
    for feature in features {
        pst.add_feature(&mut sums, feature, 1);
    }
    interpolate(sums, piece_count)
}

/// PSTとFMを64ビットで合算し、静的評価の範囲へ切り詰める。
fn add_correction(baseline: i32, correction: i64) -> i32 {
    (i64::from(baseline) + correction)
        .clamp(-i64::from(EVALUATION_LIMIT), i64::from(EVALUATION_LIMIT)) as i32
}

/// PSTとFMで局面を手番側の視点からセンチポーン評価する。
pub fn evaluate(pst: &Pst, position: &Position) -> i32 {
    let baseline = evaluate_pst(pst, position);
    match &pst.fm {
        Some(fm) => add_correction(
            baseline,
            fm.correction(&fm.refresh(position.side_to_move(), position)),
        ),
        None => baseline,
    }
}

/// 診断用に同一配置を指定側の視点で評価し、(FM込み, PSTのみ)を返す。
///
/// 先獅子状態を含む配置は保持し、評価に使う視点だけを変える。
/// 合法な手番交代やパスを適用する操作ではない。
pub fn evaluate_for_diagnostics(pst: &Pst, position: &Position, perspective: Color) -> (i32, i32) {
    let mut sums = [0; 2];
    features::active_features_for(perspective, position, |feature| {
        pst.add_feature(&mut sums, feature, 1)
    });
    let baseline = interpolate(sums, position.occupied().popcount());
    let score = match &pst.fm {
        Some(fm) => add_correction(baseline, fm.correction(&fm.refresh(perspective, position))),
        None => baseline,
    };
    (score, baseline)
}

/// 比較用のMNPTを実行バイナリへ埋め込んだバイト列。
/// 検証と共有は[`weights()`]が行う。
static EMBEDDED: &[u8] = include_bytes!("../../../nets/pst.bin");
/// 埋め込み重みの復号結果を保持する領域。
static WEIGHTS: OnceLock<Result<Arc<Pst>, Error>> = OnceLock::new();

/// 埋め込んだMNPTを初回呼び出しで1回だけ復号して検証する。
/// 成功時は同じ重みを共有し、失敗時は同じエラーを返す。
/// 復号失敗から旧評価へ切り替えることはない。
pub fn weights() -> Result<Arc<Pst>, Error> {
    match WEIGHTS.get_or_init(|| Pst::decode(EMBEDDED).map(Arc::new)) {
        Ok(pst) => Ok(Arc::clone(pst)),
        Err(error) => Err(error.clone()),
    }
}
