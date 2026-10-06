//! 中将棋の静的評価関数。

pub mod handcrafted;
pub mod pst;

pub use pst::{Pst, weights};

use minase_core::Position;

/// 学習PSTとFM補正で局面を手番側の視点からセンチポーン評価する。
pub fn evaluate(pst: &Pst, position: &Position) -> i32 {
    pst::evaluate(pst, position)
}
