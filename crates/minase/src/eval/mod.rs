//! 中将棋の静的評価関数。

pub mod features;
pub mod handcrafted;
pub mod nnue;
pub mod piece_values;
// 比較用PSTの探索向け内部APIもソースとテストを保持する。
#[allow(dead_code)]
pub mod pst;

use minase_core::{PieceCode, Position};

/// 駒種と現在の成り可否を区別した駒状態の総数。
pub const PIECE_STATE_COUNT: usize = features::PIECE_STATE_COUNT;

/// 駒コードを駒状態の番号へ変換する。
///
/// # Panics
/// 空升または盤外の番兵ではパニックする。
pub fn piece_state_of(piece: PieceCode) -> usize {
    features::piece_state(piece)
}

/// NNUEで局面を手番側の視点からセンチポーン評価する。
pub fn evaluate(
    network: &nnue::Network,
    position: &Position,
) -> Result<i32, nnue::input::AnchorError> {
    network.evaluate(position)
}
