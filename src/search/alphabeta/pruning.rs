//! 枝刈りと探索深さの削減の式。

use std::sync::OnceLock;

use crate::core::mv::Move;
use crate::core::position::Position;
use crate::core::rules::MoveRules;
use crate::eval::Pst;
use crate::search::MAX_PLY;

use super::params;
use super::see::see_prunes;

/// LMRの減深量の上限。
const LMR_MAX_REDUCTION: u32 = 3;

/// LMRの減深量表に保持する手番号の列数。
const LMR_MOVE_COUNT: usize = 256;

/// 残り深さと手番号から引く、切り詰め前のLMR減深量表を1回だけ生成する。
pub(super) fn lmr_table() -> &'static [[u8; LMR_MOVE_COUNT]; MAX_PLY as usize + 1] {
    static TABLE: OnceLock<[[u8; LMR_MOVE_COUNT]; MAX_PLY as usize + 1]> = OnceLock::new();
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

/// LMR表の1要素を、百分率の除数から計算する。
pub(super) fn lmr_base(depth: usize, index: usize, divisor: i32) -> u8 {
    ((depth as f64).ln() * (index as f64).ln() / (f64::from(divisor) / 100.0)).floor() as u8
}

/// 深さ1〜3のfutility余裕値を求める。
pub(super) fn futility_margin(pawn: i32, depth: u32) -> i32 {
    let percent = match depth {
        1 => params::futility_margin1(),
        2 => params::futility_margin2(),
        3 => params::futility_margin3(),
        _ => unreachable!("futility applies only at depths 1 to 3"),
    };
    pawn * percent / 100
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

/// 探索深さからnull moveの減深量を求める。
pub(super) fn null_move_reduction(depth: u32) -> u32 {
    (params::null_move_base() as u32 + depth * params::null_move_slope() as u32) / 1200
}

/// history値で補正し、減深後の深さを1以上に保つLMR減深量を求める。
///
/// 深さ2未満では減深せず、手番号が表の列数以上なら最後の列を使う。
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
    let upper = LMR_MAX_REDUCTION.min(depth.saturating_sub(2)) as i32;
    (base + adjustment).clamp(0, upper) as u32
}

/// 静的交換評価で損と判定できる捕獲手かを返す。
pub(super) fn capture_is_pruned_by_see(
    position: &Position,
    rules: MoveRules,
    pst: &Pst,
    mv: Move,
) -> bool {
    see_prunes(position, rules, pst, mv, 0)
}
