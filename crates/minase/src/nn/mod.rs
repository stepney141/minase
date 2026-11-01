//! モンテカルロ木探索（MCTS）が使うネットの入力平面、方策のラベル、および評価器。
//!
//! AlphaZero型探索のMCTS、自己対局生成器、およびPythonの学習器が共有するネットの入出力の契約を定める。
//! 入力は159面、方策のラベルは249面×144升で、価値は手番側の勝率1つである。
//! 推論には評価器の抽象[`Evaluator`]、試験用の[`UniformEvaluator`]、およびONNX Runtimeによる[`OnnxEvaluator`]を用意する。
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
//! - `start-the-zero-network-as-a-10-block-128-channel-resnet-with-global-pooling-and-a-32-channel-value-head.md`
//!
//! 出力の照合の手順は`docs/guides/alphazero-network-check.md`にある。

use minase_core::{BOARD_SQUARE_COUNT, Color, Square};

pub mod control;
pub mod encoding;
mod error;
pub mod evaluator;
pub mod onnx;
pub mod policy;
pub mod tensorrt_cache;

pub use encoding::{INPUT_PLANES, INPUT_SIZE, encode};
pub use error::NnError;
pub use evaluator::{Evaluator, Query, UniformEvaluator};
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
mod test_util {
    use minase_core::rng::XorShift64;
    use minase_core::{Color, MoveGenerator, Position, Rules, Square};
    use std::num::{NonZeroU64, NonZeroUsize};

    pub(super) fn rotate(square: Square) -> Square {
        Square::new(11 - square.file(), 11 - square.rank()).unwrap()
    }

    /// 3局を最大500手ずつ進め、王駒の消失または合法手なしで打ち切る。
    pub(super) fn random_positions() -> Vec<Position> {
        let rules = Rules::ENGINE_DEFAULT.moves;
        let generator = MoveGenerator::new(rules);
        let mut positions = Vec::new();
        let mut moves = Vec::new();
        for seed in [1, 71, 2026] {
            let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
            let mut position = Position::initial();
            for _ in 0..500 {
                positions.push(position.clone());
                if Color::ALL
                    .iter()
                    .any(|&side| position.royal_pieces(side).is_empty())
                {
                    break;
                }
                moves.clear();
                generator.generate_moves(&position, &mut moves);
                let Some(count) = NonZeroUsize::new(moves.len()) else {
                    break;
                };
                position.make_move_unchecked(moves[rng.index(count)], rules);
            }
        }
        positions
    }
}
