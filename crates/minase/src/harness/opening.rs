//! ペア対局で共有する開始局面の生成。

use minase_core::notation::usi;
use minase_core::rng::{XorShift64, derive_seed};
use minase_core::{Game, GameStatus, Move, MoveGenerator, Rules};
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

/// 同じペアの2局で共有する、終局しない8〜12手の開始手順を生成する。
///
/// 開始シードは`derive_seed(pair_seed, 0)`である。
/// 手数は乱数の5に対する剰余に8を足して決める。
/// 各手は合法手列の長さに対する乱数の剰余を添字として選ぶ。
/// 添字は[`XorShift64::index`]で生成し、剰余による偏りを補正しない。
/// 途中で終局したら`derive_seed(直前の開始シード, 0)`でシードを更新し、初形から作り直す。
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

    // docs/guides/sprt.mdペア対局と再現性節に従い、開始局面は初期局面から8〜12手
    // だけランダムに進めて作り、ペアシードから決定的に再現される。
    #[test]
    fn openings_stay_within_8_to_12_plies_and_vary_between_pairs() {
        let rules = Rules::ENGINE_DEFAULT;
        let mut all_moves = Vec::new();
        // rng::derive_seedのdocにあるsplitmix64(pair_seed + 0)の固定ベクトル。
        for (pair_seed, expected_seed) in [
            (1, 0x910a_2dec_8902_5cc1),
            (2, 0x9758_35de_1c97_56ce),
            (3, 0x1d0b_14e4_db01_8fed),
            (4, 0x6e73_e372_e233_8aca),
        ] {
            let pair_seed = NonZeroU64::new(pair_seed).unwrap();
            let opening = generate_opening(rules, pair_seed);
            let repeated = generate_opening(rules, pair_seed);
            assert_eq!(opening.moves, repeated.moves);
            assert_eq!(opening.usi_moves, repeated.usi_moves);
            assert_eq!(opening.game.position(), repeated.game.position());
            assert_eq!(opening.seed.get(), expected_seed);
            // 派生値を報告するだけでなく、その値で乱数を初期化していることも確認する。
            let mut rng = XorShift64::new(NonZeroU64::new(expected_seed).unwrap());
            assert_eq!(
                opening.moves.len(),
                8 + rng.index(NonZeroUsize::new(5).unwrap())
            );
            let mut replay = Game::new(rules);
            for &mv in &opening.moves {
                let legal = replay.legal_moves();
                assert_eq!(
                    mv,
                    legal[rng.index(NonZeroUsize::new(legal.len()).unwrap())]
                );
                replay.play(mv).unwrap();
            }
            assert_eq!(opening.game.position(), replay.position());
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
