//! 自己対局生成器で共有する、対局シードから決まる乱択序盤。

use std::{
    io,
    num::{NonZeroU64, NonZeroUsize},
};

use minase_core::rng::{XorShift64, derive_seed};
use minase_core::{Game, GameStatus, Move, Rules};

use super::{data_error, invalid_data};

/// 同じ対局シードから決定的に、8〜16手を一様に選んでランダム序盤を作る。
/// 同じ評価関数どうしが毎回同じ手順をたどり、同じ局面だけを記録することを避ける。
/// 対局ハーネスの8〜12手より長い範囲を使い、開始局面の多様性を優先する。
pub fn generate_opening(rules: Rules, game_seed: NonZeroU64) -> io::Result<(Game, Vec<Move>)> {
    let mut opening_seed = derive_seed(game_seed.get(), 0);
    loop {
        if let Some(candidate) = generate_opening_candidate(rules, opening_seed)? {
            return Ok(candidate);
        }
        opening_seed = derive_seed(opening_seed.get(), 0);
    }
}

/// 指定した序盤シードで候補を1つ作り、途中で終局した場合は`None`を返す。
/// 次の候補のシードは呼び手が`derive_seed(opening_seed.get(), 0)`で導く。
pub fn generate_opening_candidate(
    rules: Rules,
    opening_seed: NonZeroU64,
) -> io::Result<Option<(Game, Vec<Move>)>> {
    let mut game = Game::new(rules);
    let mut moves = Vec::new();
    let mut rng = XorShift64::new(opening_seed);
    let opening_plies = 8 + rng.index(NonZeroUsize::new(9).unwrap());
    let mut finished = false;
    for _ in 0..opening_plies {
        let legal_moves = game.legal_moves();
        if legal_moves.is_empty() {
            return Err(invalid_data(
                "an opening game is ongoing but has no legal moves",
            ));
        }
        let move_count =
            NonZeroUsize::new(legal_moves.len()).expect("the empty move list was rejected above");
        let selected = legal_moves[rng.index(move_count)];
        let status = game.play(selected).map_err(data_error)?;
        moves.push(selected);
        if matches!(status, GameStatus::Finished(_)) {
            finished = true;
            break;
        }
    }
    Ok((!finished).then_some((game, moves)))
}
