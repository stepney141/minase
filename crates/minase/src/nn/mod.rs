//! モンテカルロ木探索（MCTS）が使うネットの入力平面、方策のラベル、および評価器。
//!
//! AlphaZero型探索のMCTS、自己対局生成器、およびPythonの学習器が共有するネットの入出力の契約を定める。
//! 入力は159面、方策のラベルは249面×144升で、価値は手番側の勝率1つである。
//! ONNXの入出力は入力がu8、方策がf16、価値がf32であり、評価器は合法手のlogitだけをf32へ変換して返す。
//! ONNXの方策はメタデータ`policy_labels`の昇順の表に従って詰め、評価器が元のラベルから逆引きする。
//! 推論には評価器の抽象[`Evaluator`]とONNX Runtimeによる[`OnnxEvaluator`]を用意する。
//! `test-util`フィーチャでは、各着手に同じ方策logitを返す試験用の評価器も公開する。
//! プロジェクト内では`minase_core`と駒の状態番号を得るための`crate::eval::features`にだけ依存し、`search`を参照しない。
//!
//! 設計判断は`docs/adr/`の次の7本に記録している。
//!
//! - `encode-zero-network-inputs-as-159-planes-with-pseudo-control-from-the-side-to-move.md`
//! - `encode-the-zero-policy-as-249-planes-per-from-square.md`
//! - `predict-a-single-win-probability-in-the-zero-network.md`
//! - `run-zero-network-inference-through-ort-load-dynamic.md`
//! - `select-zero-networks-by-startup-argument-and-embed-only-the-judged-one.md`
//! - `key-tensorrt-engine-caches-by-network-checksum-gpu-and-max-batch.md`
//! - `start-the-zero-network-as-a-20-block-256-channel-resnet-with-global-pooling-and-a-32-channel-value-head.md`
//!
//! 出力の照合の手順は`docs/guides/alphazero-network-check.md`にある。

use minase_core::{BOARD_SQUARE_COUNT, Color, Square};

pub mod control;
pub mod encoding;
mod error;
pub mod evaluator;
pub mod onnx;
pub mod policy;
mod policy_map;
pub mod tensorrt_cache;
#[cfg(any(test, feature = "test-util"))]
pub mod test_util;

pub use encoding::{INPUT_PLANES, INPUT_SIZE, encode};
pub use error::NnError;
pub use evaluator::{Evaluator, Query};
pub use onnx::{ExecutionProvider, Network, OnnxEvaluator};
pub use policy::{POLICY_PLANES, POLICY_SIZE, policy_label, softmax};

/// 後手視点では盤を180度回転し、手番側から見た升番号を返す。
fn relative_square(square: Square, side: Color) -> usize {
    match side {
        Color::Black => square.dense_index(),
        Color::White => BOARD_SQUARE_COUNT - 1 - square.dense_index(),
    }
}

#[cfg(test)]
mod test_support;
