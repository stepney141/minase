//! ランダム対局を実行し、合法手適用と終局の不変条件を検査する。

use std::{num::NonZeroUsize, process};

use minase::{Game, GameError, GameResult, GameStatus, Move, Rules, rng::XorShift64, to_sfen};

use super::{seed::game_seed, text::move_text};

/// 正常に終了した1局の結果。
pub(super) struct CompletedGame {
    /// 終了時点の手数。
    pub(super) plies: u32,
    /// 終局結果または打ち切り。
    pub(super) outcome: Outcome,
    /// 実際に適用した全手順。
    pub(super) moves: Vec<Move>,
}

/// 終局または手数上限による打ち切りを表す。
#[derive(Clone, Copy)]
pub(super) enum Outcome {
    /// 裁定による終局。
    Finished(GameResult),
    /// 手数上限による打ち切り。
    Cutoff,
}

/// 異常時に再現に必要な情報を保持する。
struct Failure<'a> {
    /// 検出した不変条件違反の説明。
    issue: String,
    /// 採用ルールの表示文字列。
    rules: &'a str,
    /// 全局共通の基本シード。
    base_seed: u64,
    /// 1起算の局番号。
    game_number: u64,
    /// 違反を検出した手数。
    ply: u32,
    /// 違反に関与した着手。
    problem_move: Option<Move>,
    /// 違反直前の局面のSFEN。
    previous_sfen: String,
    /// 違反までに適用した全手順。
    moves: &'a [Move],
}

/// 異常情報と全手順を標準エラー出力へ表示して終了する。
fn fail(failure: Failure<'_>) -> ! {
    eprintln!("random-play invariant failure: {}", failure.issue);
    eprintln!("rules: {}", failure.rules);
    eprintln!("seed: {}", failure.base_seed);
    eprintln!("game: {}", failure.game_number);
    eprintln!("ply: {}", failure.ply);
    match failure.problem_move {
        Some(mv) => eprintln!("problem move: {}", move_text(mv)),
        None => eprintln!("problem move: none"),
    }
    eprintln!("previous SFEN: {}", failure.previous_sfen);
    eprintln!("moves:");
    if failure.moves.is_empty() {
        eprintln!("  (none)");
    } else {
        for (index, &mv) in failure.moves.iter().enumerate() {
            eprintln!("  {}: {}", index + 1, move_text(mv));
        }
    }
    process::exit(1);
}

/// 指定局番号のランダム対局を実行して不変条件を検査する。
pub(super) fn run_game(
    rules: Rules,
    rules_text: &str,
    base_seed: u64,
    game_number: u64,
    max_ply: u32,
    verify_all: bool,
) -> CompletedGame {
    let mut game = Game::new(rules);
    let mut rng = XorShift64::new(game_seed(base_seed, game_number));
    let mut history = Vec::new();

    loop {
        if game.ply_count() >= max_ply {
            return CompletedGame {
                plies: game.ply_count(),
                outcome: Outcome::Cutoff,
                moves: history,
            };
        }

        let legal_moves = game.legal_moves();
        let previous_sfen = to_sfen(game.position());
        if legal_moves.is_empty() {
            fail(Failure {
                issue: "ongoing game has no legal moves".to_owned(),
                rules: rules_text,
                base_seed,
                game_number,
                ply: game.ply_count(),
                problem_move: None,
                previous_sfen,
                moves: &history,
            });
        }

        if verify_all {
            for &candidate in &legal_moves {
                let mut probe = game.clone();
                if let Err(error) = probe.play(candidate) {
                    fail(Failure {
                        issue: format!("legal move rejected during full verification: {error}"),
                        rules: rules_text,
                        base_seed,
                        game_number,
                        ply: game.ply_count() + 1,
                        problem_move: Some(candidate),
                        previous_sfen,
                        moves: &history,
                    });
                }
            }
        }

        let selected = legal_moves[rng.index(
            NonZeroUsize::new(legal_moves.len()).expect("legal moves were checked as non-empty"),
        )];
        let status = match game.play(selected) {
            Ok(status) => status,
            Err(error) => fail(Failure {
                issue: format!("legal move rejected: {error}"),
                rules: rules_text,
                base_seed,
                game_number,
                ply: game.ply_count() + 1,
                problem_move: Some(selected),
                previous_sfen,
                moves: &history,
            }),
        };
        history.push(selected);

        if let Err(error) = game.position().validate() {
            fail(Failure {
                issue: format!("position validation failed: {error}"),
                rules: rules_text,
                base_seed,
                game_number,
                ply: game.ply_count(),
                problem_move: Some(selected),
                previous_sfen,
                moves: &history,
            });
        }

        let GameStatus::Finished(result) = status else {
            continue;
        };

        let terminal_sfen = to_sfen(game.position());
        let terminal_moves = game.legal_moves();
        if let Some(&candidate) = terminal_moves.first() {
            fail(Failure {
                issue: format!(
                    "finished game returned {} legal moves",
                    terminal_moves.len()
                ),
                rules: rules_text,
                base_seed,
                game_number,
                ply: game.ply_count() + 1,
                problem_move: Some(candidate),
                previous_sfen: terminal_sfen,
                moves: &history,
            });
        }

        match game.play(selected) {
            Err(GameError::GameAlreadyOver) => {}
            unexpected => fail(Failure {
                issue: format!("additional move after game end returned {unexpected:?}"),
                rules: rules_text,
                base_seed,
                game_number,
                ply: game.ply_count() + 1,
                problem_move: Some(selected),
                previous_sfen: terminal_sfen,
                moves: &history,
            }),
        }

        return CompletedGame {
            plies: game.ply_count(),
            outcome: Outcome::Finished(result),
            moves: history,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase::RuleCode;

    // D8-HARN-17性質/D8-HARN-18(random-play.md): 同一の(--rules, --seed,
    // --game, --max-ply)で着手列が完全に再現される。手数上限への到達は
    // 異常ではなく打ち切りである。
    #[test]
    fn same_seed_and_game_number_reproduce_the_same_game() {
        let rules = Rules::from_codes(&[RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E0])
            .expect("the complete engine-default rule set must be valid");
        let first = run_game(rules, "L0,P0,R1,E0", 20_260_814, 1, 16, false);
        // random-play.md: 全合法手の適用検査を有効にしても同じ対局になる。
        let second = run_game(rules, "L0,P0,R1,E0", 20_260_814, 1, 16, true);
        assert_eq!(first.moves, second.moves);
        assert_eq!(first.plies, second.plies);
        assert_eq!(first.plies, 16);
        assert!(matches!(first.outcome, Outcome::Cutoff));
        assert!(matches!(second.outcome, Outcome::Cutoff));
    }
}
