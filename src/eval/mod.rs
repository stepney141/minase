//! 中将棋の静的評価関数。

pub mod features;
pub mod handcrafted;
pub mod nnue;
pub mod pst;
pub mod training_data;

pub use nnue::{Network, evaluate, evaluate_position, network};
