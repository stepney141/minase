//! ペア対局で共有する開始局面の生成。

use crate::notation::usi;
use crate::rng::{XorShift64, derive_seed};
use crate::{Game, GameStatus, Move, MoveGenerator, Rules};
use std::num::{NonZeroU64, NonZeroUsize};

/// ペア対局で共有する開始状態。
pub struct Opening {
    /// 開始手順を適用済みの対局。
    pub game: Game,
    /// 開始手順の生成に使ったシード。
    pub seed: NonZeroU64,
    /// 開始手順の指し手列。
    pub moves: Vec<Move>,
    /// 開始手順のUSI表記列。エンジンへの`position`送信に使う。
    pub usi_moves: Vec<String>,
}

/// 終局しない8手から12手の開始手順を生成する。
pub fn generate_opening(rules: Rules, pair_seed: NonZeroU64) -> Opening {
    let mut opening_seed = derive_seed(pair_seed.get(), 0);
    loop {
        let mut game = Game::new(rules);
        let mut rng = XorShift64::new(opening_seed);
        let opening_plies = 8 + rng.index(NonZeroUsize::new(5).unwrap());
        let mut moves = Vec::with_capacity(opening_plies);
        let mut usi_moves = Vec::with_capacity(opening_plies);
        let mut finished = false;

        for _ in 0..opening_plies {
            let legal_moves = game.legal_moves();
            assert!(
                !legal_moves.is_empty(),
                "ongoing game must have legal moves"
            );
            let selected = legal_moves
                [rng.index(NonZeroUsize::new(legal_moves.len()).expect("moves are non-empty"))];
            moves.push(selected);
            usi_moves.push(
                usi::text(
                    game.position(),
                    selected,
                    &MoveGenerator::new(game.rules().moves),
                )
                .expect("a move returned by legal_moves must be renderable"),
            );
            let status = game
                .play(selected)
                .expect("a move returned by legal_moves must be accepted");
            if matches!(status, GameStatus::Finished(_)) {
                finished = true;
                break;
            }
        }

        if !finished {
            return Opening {
                game,
                seed: opening_seed,
                moves,
                usi_moves,
            };
        }
        opening_seed = derive_seed(opening_seed.get(), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // D8-HARN-02(sprt.mdペア対局と再現性節): 開始局面は初期局面から8〜12手
    // だけランダムに進めて作り、ペアシードから決定的に再現される。
    #[test]
    fn openings_stay_within_8_to_12_plies_and_vary_between_pairs() {
        let rules = Rules::ENGINE_DEFAULT;
        let base_seed = 20_260_814_u64;
        let mut all_moves = Vec::new();
        for pair_number in 1..=4 {
            let pair_seed = derive_seed(base_seed, pair_number);
            let opening = generate_opening(rules, pair_seed);
            assert!(
                (8..=12).contains(&opening.moves.len()),
                "opening length {} is outside the documented 8..=12 range",
                opening.moves.len()
            );
            // エンジンへ送るUSI表記列は開始手順と同数
            assert_eq!(opening.usi_moves.len(), opening.moves.len());
            all_moves.push(opening.moves);
        }
        // ペア間独立: 異なるペア番号がすべて同じ開始手順なら派生が退化している
        assert!(all_moves.windows(2).any(|pair| pair[0] != pair[1]));
    }
}
