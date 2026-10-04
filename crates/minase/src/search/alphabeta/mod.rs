//! αβ探索の実装。

mod capture_history;
mod correction;
mod deepening;
pub(crate) mod history;
mod negamax;
mod ordering;
pub(crate) mod params;
mod pruning;
mod quiesce;
mod root;
mod royal;
mod searcher;
mod see;
pub(in crate::search) mod team;
#[cfg(test)]
mod tests;
mod time;
pub(in crate::search) mod tt;

use crate::search::MATE;

/// 探索窓の初期値。全評価値より大きい。
const INFINITY: i32 = MATE + 1;

/// USIの履歴持ち越しテストで使う減衰率。
#[cfg(test)]
pub(crate) fn history_decay_for_test() -> i32 {
    params::history_decay()
}
