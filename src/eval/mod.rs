//! 中将棋の静的評価関数。

pub mod features;
pub mod handcrafted;
pub mod pst;
pub mod training_data;

pub use pst::{Pst, weights};

use crate::Position;

/// 学習PSTで局面を手番側の視点からセンチポーン評価する。
pub fn evaluate(pst: &Pst, position: &Position) -> i32 {
    pst::evaluate(pst, position)
}
