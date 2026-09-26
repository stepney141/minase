//! 先後入替ペアの実行、得点、および表示。

use crate::harness::failure::FailureCounts;
use crate::harness::game::{GameOutcome, PlayedGame, play_game};
use crate::harness::opening::generate_opening;
use crate::harness::player::PlayerConfig;
use crate::harness::records::{OpeningRecord, PairRecord};
use crate::rng::derive_seed;
use crate::{Color, GameResult, Move, Rules, Square};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// 1ペアの集計結果。
pub struct PairResult {
    /// 候補側ペア得点のペンタノミアル分類(0〜4)。打ち切りを含むペアは`None`。
    pub category: Option<usize>,
    /// このペアで発生した異常の件数。
    pub failures: FailureCounts,
}

/// 1ペアの番号、表示内容、集計結果。
pub struct CompletedPair {
    /// 1起算のペア番号。
    pub number: u64,
    /// ペア番号順に出力する表示内容。
    pub output: String,
    /// 集計へ取り込む結果。
    pub result: PairResult,
    /// 原子的に保存する監査記録。
    pub record: PairRecord,
}

/// 升をperftと同じ0起算座標で表記する。
fn square_text(square: Square) -> String {
    format!("({},{})", square.file(), square.rank())
}

/// 着手をperftの`move_text`と同じ形式で表記する。
fn move_text(mv: Move) -> String {
    if let Some(mid) = mv.mid {
        format!(
            "double {}->{}->{}{}",
            square_text(mv.from),
            square_text(mid),
            square_text(mv.to),
            if mv.promote { "+" } else { "" }
        )
    } else {
        format!(
            "move {}->{}{}",
            square_text(mv.from),
            square_text(mv.to),
            if mv.promote { "+" } else { "" }
        )
    }
}

/// 1局の結果を候補Aから見た半点単位の得点へ変換する。
pub fn half_points(outcome: GameOutcome, player_a_color: Color) -> u8 {
    let winner = match outcome {
        GameOutcome::Adjudicated(GameResult::Win { winner, .. })
        | GameOutcome::Forfeit { winner, .. }
        | GameOutcome::Resigned { winner } => Some(winner),
        GameOutcome::Adjudicated(GameResult::Draw { .. }) => None,
    };
    match winner {
        Some(winner) if winner == player_a_color => 2,
        Some(_) => 0,
        None => 1,
    }
}

/// 1局の結果を表示文字列へ変換する。
fn played_game_text(game: PlayedGame) -> String {
    match game {
        PlayedGame::Finished {
            plies,
            outcome: GameOutcome::Adjudicated(GameResult::Win { winner, reason }),
        } => format!("plies={plies} result=win winner={winner:?} reason={reason:?}"),
        PlayedGame::Finished {
            plies,
            outcome: GameOutcome::Adjudicated(GameResult::Draw { reason }),
        } => format!("plies={plies} result=draw reason={reason:?}"),
        PlayedGame::Finished {
            plies,
            outcome: GameOutcome::Forfeit { winner, reason },
        } => format!("plies={plies} result=win winner={winner:?} reason={reason:?}"),
        PlayedGame::Finished {
            plies,
            outcome: GameOutcome::Resigned { winner },
        } => format!("plies={plies} result=win winner={winner:?} reason=Resigned"),
        PlayedGame::Cutoff { plies } => format!("plies={plies} result=cutoff"),
    }
}

/// 1局の異常理由を集計する。
pub fn record_game_failure(game: PlayedGame, counts: &mut FailureCounts) {
    if let PlayedGame::Finished {
        outcome: GameOutcome::Forfeit { reason, .. },
        ..
    } = game
    {
        counts.record(reason);
    }
}

/// 1ペアを実行し、表示内容とペンタノミアル分類を返す。
#[allow(clippy::too_many_arguments)]
pub fn run_pair(
    rules: Rules,
    rules_text: &str,
    base_seed: u64,
    pair_number: u64,
    max_ply: u32,
    candidate: &PlayerConfig,
    baseline: &PlayerConfig,
    timeout: Duration,
    ponder: bool,
    stop: &AtomicBool,
) -> Option<CompletedPair> {
    if stop.load(Ordering::Acquire) {
        return None;
    }
    let pair_seed = derive_seed(base_seed, pair_number);
    let opening = generate_opening(rules, pair_seed);
    let opening_record = OpeningRecord {
        seed: opening.seed.get(),
        moves: opening.usi_moves.clone(),
    };
    let game1_a_seed = derive_seed(pair_seed.get(), 1);
    let game1_b_seed = derive_seed(pair_seed.get(), 2);
    let game2_a_seed = derive_seed(pair_seed.get(), 3);
    let game2_b_seed = derive_seed(pair_seed.get(), 4);

    let mut output = String::new();
    writeln!(
        output,
        "pair {pair_number}: pair_seed={pair_seed} opening_seed={} player_a={} player_b={} rules={rules_text} max_ply={max_ply}",
        opening.seed,
        candidate.name(),
        baseline.name()
    )
    .expect("writing to String cannot fail");
    writeln!(
        output,
        "pair {pair_number} opening: plies={}",
        opening.moves.len()
    )
    .expect("writing to String cannot fail");
    for (index, &mv) in opening.moves.iter().enumerate() {
        writeln!(output, "  {}: {}", index + 1, move_text(mv))
            .expect("writing to String cannot fail");
    }

    writeln!(
        output,
        "pair {pair_number} game 1 settings: A=Black seed_a={game1_a_seed} B=White seed_b={game1_b_seed}"
    )
    .expect("writing to String cannot fail");
    let game1 = play_game(
        opening.game.clone(),
        opening.usi_moves.clone(),
        opening.moves.clone(),
        max_ply,
        Color::Black,
        candidate,
        game1_a_seed,
        baseline,
        game1_b_seed,
        timeout,
        ponder,
        stop,
    )?;
    writeln!(
        output,
        "pair {pair_number} game 1: {}",
        played_game_text(game1.played)
    )
    .expect("writing to String cannot fail");

    writeln!(
        output,
        "pair {pair_number} game 2 settings: B=Black seed_b={game2_b_seed} A=White seed_a={game2_a_seed}"
    )
    .expect("writing to String cannot fail");
    let game2 = play_game(
        opening.game,
        opening.usi_moves,
        opening.moves,
        max_ply,
        Color::White,
        candidate,
        game2_a_seed,
        baseline,
        game2_b_seed,
        timeout,
        ponder,
        stop,
    )?;
    writeln!(
        output,
        "pair {pair_number} game 2: {}",
        played_game_text(game2.played)
    )
    .expect("writing to String cannot fail");

    let mut failures = FailureCounts::default();
    record_game_failure(game1.played, &mut failures);
    record_game_failure(game2.played, &mut failures);
    let pair_record = |category| PairRecord {
        pair_number,
        pair_seed: pair_seed.get(),
        opening: opening_record.clone(),
        games: [game1.record.clone(), game2.record.clone()],
        category,
    };
    let (
        PlayedGame::Finished {
            outcome: game1_outcome,
            ..
        },
        PlayedGame::Finished {
            outcome: game2_outcome,
            ..
        },
    ) = (game1.played, game2.played)
    else {
        writeln!(output, "pair {pair_number} result: discarded")
            .expect("writing to String cannot fail");
        return Some(CompletedPair {
            number: pair_number,
            output,
            result: PairResult {
                category: None,
                failures,
            },
            record: pair_record(None),
        });
    };
    let category = usize::from(
        half_points(game1_outcome, Color::Black) + half_points(game2_outcome, Color::White),
    );
    writeln!(
        output,
        "pair {pair_number} result: score_a={:.1} category={category}",
        category as f64 / 2.0
    )
    .expect("writing to String cannot fail");
    Some(CompletedPair {
        number: pair_number,
        output,
        result: PairResult {
            category: Some(category),
            failures,
        },
        record: pair_record(Some(u8::try_from(category).expect("category is in 0..=4"))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::failure::EngineFailure;
    use crate::{DrawReason, WinReason};

    // sprt.md: 各局の勝敗を候補側の得点へ変換し、反則負けも敗北として扱う。
    // match-harness.md: 投了を異常件数へ算入しない。
    #[test]
    fn game_outcomes_map_to_candidate_scores_and_resignation_is_not_failure() {
        let win_black = GameOutcome::Adjudicated(GameResult::Win {
            winner: Color::Black,
            reason: WinReason::RoyalCapture,
        });
        let draw = GameOutcome::Adjudicated(GameResult::Draw {
            reason: DrawReason::Repetition,
        });
        let forfeit_win_black = GameOutcome::Forfeit {
            winner: Color::Black,
            reason: EngineFailure::Crash,
        };
        let resignation_win_black = GameOutcome::Resigned {
            winner: Color::Black,
        };

        // 1局の得点は半点単位: 勝ち2、引き分け1、負け0(候補の色に依存)
        assert_eq!(half_points(win_black, Color::Black), 2);
        assert_eq!(half_points(win_black, Color::White), 0);
        assert_eq!(half_points(draw, Color::Black), 1);
        assert_eq!(half_points(draw, Color::White), 1);
        // 反則負けも通常の勝敗として得点化される
        assert_eq!(half_points(forfeit_win_black, Color::Black), 2);
        assert_eq!(half_points(forfeit_win_black, Color::White), 0);
        assert_eq!(half_points(resignation_win_black, Color::Black), 2);
        assert_eq!(half_points(resignation_win_black, Color::White), 0);

        // D8-HARN-06（match-harness.md「異常時裁定」）: 投了は通常の敗北であり、
        // engine_failuresには算入しない。
        let resigned = PlayedGame::Finished {
            plies: 12,
            outcome: resignation_win_black,
        };
        assert_eq!(
            played_game_text(resigned),
            "plies=12 result=win winner=Black reason=Resigned"
        );
        let mut resignation_failures = FailureCounts::default();
        record_game_failure(resigned, &mut resignation_failures);
        assert_eq!(resignation_failures, FailureCounts::default());
    }
}
