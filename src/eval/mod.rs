//! 中将棋の静的評価関数。

pub(crate) mod features;
pub mod handcrafted;
pub(crate) mod king_features;
pub mod provenance;
pub mod pst;
pub mod rescore;
pub mod training_data;

pub use king_features::{COLUMN_COUNT, DEFINITION_ID, extract as king_feature_values};
pub use pst::{Pst, weights};

use crate::Position;

/// 学習PSTと王の安全度で局面を手番側の視点からセンチポーン評価する。
pub fn evaluate(pst: &Pst, position: &Position) -> i32 {
    pst::evaluate(pst, position)
}
