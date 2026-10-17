//! 枝刈りと探索深さの削減の式。

use std::sync::OnceLock;

use crate::eval::Nnue;
use crate::eval::nnue::EVALUATION_LIMIT;
use crate::search::{MAX_PLY, REPETITION_BAND_START};
use minase_core::mv::Move;
use minase_core::position::Position;
use minase_core::rules::MoveRules;

use super::params;
use super::see::see_prunes;
use super::tt::Bound;

/// 補正後の静的評価を評価関数の出力範囲に収める（Stockfish src/search.cpp:113-118）。
pub(super) fn corrected_static_eval(raw: i32, correction: i32) -> i32 {
    (raw + correction).clamp(-EVALUATION_LIMIT, EVALUATION_LIMIT)
}

/// 同じ手番の2手前より静的評価が上がったかを返す（Stockfish src/search.cpp:883）。
/// 2手前がなければ偽とし、null moveの区間も通常どおり比較する。
pub(super) fn is_improving(static_eval: i32, previous: Option<i32>) -> bool {
    previous.is_some_and(|value| static_eval > value)
}

/// 静的評価を、方向の合う置換表の値で置き換える（Stockfish src/search.cpp:862-865）。
/// 経路に依存する専用の帯と詰み帯の値は、判定用の評価値に使わない。
pub(super) fn pruning_eval(static_eval: i32, tt_hit: Option<(i32, Bound)>) -> i32 {
    if let Some((value, bound)) = tt_hit
        && value.abs() < REPETITION_BAND_START
        && (bound == Bound::Exact
            || (value > static_eval && bound == Bound::Lower)
            || (value <= static_eval && bound == Bound::Upper))
    {
        return value;
    }
    static_eval
}

/// LMRの減深量表に保持する手番号の列数。
/// 256手を超える局面は稀で、後方の減深量は上限で抑えるため、それ以降は最終列を使う。
const LMR_MOVE_COUNT: usize = 256;

/// 残り深さと手番号から引く、上限適用前のLMR減深量表を1/100手単位で1回だけ生成する。
pub(super) fn lmr_table() -> &'static [[u16; LMR_MOVE_COUNT]; MAX_PLY as usize + 1] {
    static TABLE: OnceLock<[[u16; LMR_MOVE_COUNT]; MAX_PLY as usize + 1]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let divisor = params::lmr_divisor();
        let mut table = [[0; LMR_MOVE_COUNT]; MAX_PLY as usize + 1];
        for (depth, row) in table.iter_mut().enumerate().skip(1) {
            for (index, value) in row.iter_mut().enumerate().skip(1) {
                *value = lmr_base(depth, index, divisor);
            }
        }
        table
    })
}

/// LMR表の1要素を、百分率の除数から1/100手単位で計算し、端数を切り捨てる。
pub(super) fn lmr_base(depth: usize, index: usize, divisor: i32) -> u16 {
    let reduction = (depth as f64).ln() * (index as f64).ln() / (f64::from(divisor) / 100.0);
    (100.0 * reduction).floor() as u16
}

/// 深さ1〜3のfutility余裕値を求める。
pub(super) fn futility_margin(pawn: i32, depth: u32, improving: bool) -> i32 {
    let percent = match depth {
        1 => params::futility_margin1(),
        2 => params::futility_margin2(),
        3 => params::futility_margin3(),
        _ => unreachable!("futility applies only at depths 1 to 3"),
    };
    let margin = pawn * percent / 100;
    if improving {
        return margin;
    }
    let scale = match depth {
        1 => params::non_improving_futility1(),
        2 => params::non_improving_futility2(),
        3 => params::non_improving_futility3(),
        _ => unreachable!("futility applies only at depths 1 to 3"),
    };
    // 百分率の余裕値を求めてから縮め、尺度100では丸めも含めて同じ値にする。
    margin * scale / 100
}

/// 深さ1〜3で数える静かな手の数の上限を整数除算で求める。
///
/// 式は `(LmpBase + LmpSlope × 深さ²) / 100` であり、端数は切り捨てる。
/// 手番号ではなく、現在の手より前に返った非捕獲手のうち、置換表の手、成る手、killer手を除いた数と比べる。
/// 枝刈りで展開しなかった手もこの数に含める。
/// 捕獲手に静かな手の枠を消費させない理由は`docs/adr/count-only-quiet-moves-for-late-move-pruning.md`にある。
pub(super) fn late_move_limit(depth: u32) -> i32 {
    let depth = depth as i32;
    (params::lmp_base() + params::lmp_slope() * depth * depth) / 100
}

/// 深さ1〜3のSEE余裕値を求める。
pub(super) fn see_margin(pawn: i32, depth: u32) -> i32 {
    let percent = match depth {
        1 => params::see_margin1(),
        2 => params::see_margin2(),
        3 => params::see_margin3(),
        _ => unreachable!("SEE pruning applies only at depths 1 to 3"),
    };
    pawn * percent / 100
}

/// 深さ1〜3共通のreverse futility余裕値を求める。
pub(super) fn reverse_futility_margin(pawn: i32) -> i32 {
    pawn * params::reverse_futility_margin() / 100
}

/// 深さ1〜2のrazoring余裕値を求める。
pub(super) fn razoring_margin(pawn: i32, depth: u32) -> i32 {
    let percent = match depth {
        1 => params::razoring_margin1(),
        2 => params::razoring_margin2(),
        _ => unreachable!("razoring applies only at depths 1 to 2"),
    };
    pawn * percent / 100
}

/// 深さと静的評価のβに対する余裕からnull moveの減深量を求める。
///
/// 基本量は `(NullMoveBase + 深さ × NullMoveSlope) / 1200` 手である。
/// `(静的評価 − β) × NullMoveEvalScale` を100、歩兵価値の2倍で順に整数除算し、0〜3手に制限して加える。
/// 縮めた深さが0になり静止探索だけで打ち切りを判定する頻度を抑えるため、加算に上限を設ける。
/// 尺度0では基本量と一致する。
pub(super) fn null_move_reduction(depth: u32, static_eval: i32, beta: i32, pawn: i32) -> u32 {
    let extra = (((i64::from(static_eval) - i64::from(beta))
        * i64::from(params::null_move_eval_scale())
        / 100)
        / (2 * i64::from(pawn)))
    .clamp(0, 3) as u32;
    (params::null_move_base() as u32 + depth * params::null_move_slope() as u32) / 1200 + extra
}

/// butterfly historyの値で対数表を補正し、LMRの減深量を手数で返す。
///
/// 手番号は0始まりで、0の表要素は0とし、列数以上なら最後の列を使う。
/// historyが閾値以上なら表の値から100を引き、その条件に当てはまらず負の閾値以下なら100を加える。
/// それ以外は変えず、100で割って負の値も切り捨て、手数に戻す。
/// 結果を `0..=min(LmrMaxReduction, 深さ − 2)` に収めるため、残り深さ2以下では減深しない。
/// 深さ3以上では着手による1手の消費も含めて、減深後の深さを1以上に保つ。
/// 呼び出し側は置換表の記録手、捕獲手、killer手を対象から除く。
pub(super) fn lmr_reduction(depth: u32, index: usize, history: i32) -> u32 {
    let base = i32::from(lmr_table()[depth as usize][index.min(LMR_MOVE_COUNT - 1)]);
    let threshold = params::lmr_history_threshold();
    let adjustment = if history >= threshold {
        -1
    } else if history <= -threshold {
        1
    } else {
        0
    };
    let upper = params::lmr_max_reduction().min(depth.saturating_sub(2) as i32);
    (base + 100 * adjustment).div_euclid(100).clamp(0, upper) as u32
}

/// 静的交換評価で損と判定できる捕獲手かを返す。
pub(super) fn capture_is_pruned_by_see(
    position: &Position,
    rules: MoveRules,
    nnue: &Nnue,
    mv: Move,
) -> bool {
    see_prunes(position, rules, nnue, mv, 0)
}
