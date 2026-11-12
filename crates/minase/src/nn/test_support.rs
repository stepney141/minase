//! 入力の符号化と方策の試験に使う局面と座標の補助。

use minase_core::rng::XorShift64;
use minase_core::{Color, MoveGenerator, Position, Rules, Square};
use std::num::{NonZeroU64, NonZeroUsize};

pub(super) fn rotate(square: Square) -> Square {
    Square::new(11 - square.file(), 11 - square.rank()).unwrap()
}

/// 3局を最大500手ずつ進め、王駒の消失または合法手なしで打ち切る。
pub(super) fn random_positions() -> Vec<Position> {
    let rules = Rules::JCSA.moves;
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
