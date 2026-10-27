//! モンテカルロ木探索（MCTS）が使うネットの入力平面、方策のラベル、および評価器。

use core::fmt;
use minase_core::{BOARD_SQUARE_COUNT, Color, Square};

pub mod encoding;
pub mod evaluator;
pub mod policy;

pub use encoding::{INPUT_PLANES, INPUT_SIZE, encode};
pub use evaluator::{Evaluator, Query, UniformEvaluator};
pub use policy::{POLICY_PLANES, POLICY_SIZE, policy_label, softmax};

/// ネットの入力の符号化または評価に失敗した理由。
#[derive(Debug)]
pub enum NnError {
    /// 95面の入力では表せない成り権の保留がある。
    PromotionDeferred,
}

impl fmt::Display for NnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PromotionDeferred => {
                formatter.write_str("deferred promotion rights cannot be encoded in 95 planes")
            }
        }
    }
}

impl std::error::Error for NnError {}

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
