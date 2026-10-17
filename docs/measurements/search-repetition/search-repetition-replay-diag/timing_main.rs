//! フェーズ3の一時診断。保存棋譜の敗着直前で反復手の分類を総当たりと照合する。

use minase::harness::{OpeningRecord, StoredColor, TerminationRecord, TurnResponse, stored_color};
use minase_core::notation::usi;
use minase_core::rules::parse_rule_set;
use minase_core::{Color, Game, GameResult, GameStatus, Move, Rules, WinReason};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    search_repetition_replay_diag()
}

fn search_repetition_replay_diag() -> Result<(), Box<dyn std::error::Error>> {
    // この保存記録には、現行のTurnRecordが必須とするponder欄がない。
    // 値を補完せず、再生に必要な欄だけを読み、公開の記録型で解釈する。
    #[derive(Deserialize)]
    struct ReplayTurn {
        side: StoredColor,
        response: TurnResponse,
    }

    #[derive(Deserialize)]
    struct ReplayGame {
        turns: Vec<ReplayTurn>,
        termination: TerminationRecord,
    }

    #[derive(Deserialize)]
    struct ReplayPair {
        pair_number: u64,
        opening: OpeningRecord,
        games: [ReplayGame; 2],
    }

    #[derive(Deserialize)]
    struct ReplayManifest {
        canonical_rules: Vec<String>,
    }

    fn play_usi(game: &mut Game, text: &str) -> Result<GameStatus, String> {
        let mv = usi::parse(game.position(), text)
            .map_err(|error| format!("cannot parse {text}: {error}"))?;
        game.play(mv)
            .map_err(|error| format!("cannot play {text}: {error}"))
    }

    // 指示書と設計書の定義だけを用いる参照実装。局面キーによる絞り込みは行わない。
    fn classify_reference(game: &Game) -> (Vec<Move>, Vec<Move>) {
        let mover = game.position().side_to_move();
        let mut winning = Vec::new();
        let mut losing = Vec::new();
        for mv in game.legal_moves() {
            let mut child = game.clone();
            match child.play(mv).expect("root legal move must be accepted") {
                GameStatus::Finished(GameResult::Win {
                    winner,
                    reason: WinReason::Repetition,
                }) => {
                    if winner == mover {
                        winning.push(mv);
                    } else {
                        losing.push(mv);
                    }
                }
                GameStatus::Finished(_) => {}
                GameStatus::Ongoing => {
                    for reply in child.legal_moves() {
                        let mut grandchild = child.clone();
                        let status = grandchild
                            .play(reply)
                            .expect("reply legal move must be accepted");
                        if status
                            == (GameStatus::Finished(GameResult::Win {
                                winner: mover.opposite(),
                                reason: WinReason::Repetition,
                            }))
                        {
                            losing.push(mv);
                            break;
                        }
                    }
                }
            }
        }
        (winning, losing)
    }

    fn summary(values: &mut [f64]) -> Value {
        assert!(!values.is_empty(), "cannot summarize an empty sample");
        values.sort_by(f64::total_cmp);
        let middle = values.len() / 2;
        let median = if values.len().is_multiple_of(2) {
            (values[middle - 1] + values[middle]) / 2.0
        } else {
            values[middle]
        };
        json!({"min": values[0], "median": median, "max": values[values.len() - 1]})
    }

    fn micros(elapsed: Duration) -> f64 {
        elapsed.as_secs_f64() * 1_000_000.0
    }

    let run_dir = Path::new(
        "/home/stepney141/board-games/minase/data/matches/movegen-speedup-2-stage7-9-ltc",
    );
    let output = Path::new("../phase3-results-release.json");
    let manifest: ReplayManifest =
        serde_json::from_slice(&fs::read(run_dir.join("manifest.json"))?)?;
    let canonical_rules = manifest.canonical_rules.join(",");
    assert_eq!(canonical_rules, "L0,P0,R1,E0");
    let rules = Rules::from_codes(&parse_rule_set(&canonical_rules)?)?;
    let mut paths = fs::read_dir(run_dir.join("pairs"))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "json")
    });
    paths.sort();

    let mut games = 0;
    let mut replay_mismatches = Vec::new();
    let mut classification_mismatches = Vec::new();
    let mut final_move_in_losing = 0;
    let mut final_move_not_in_losing = Vec::new();
    let mut all_moves_losing_games = Vec::new();
    let mut legal_counts = Vec::new();
    let mut losing_counts = Vec::new();
    let mut winning_nonempty = 0;
    let mut first_call_times = Vec::new();
    let mut min_of_10_times = Vec::new();
    let mut reference_times = Vec::new();
    let mut measurements = Vec::new();

    for path in paths {
        let pair: ReplayPair = serde_json::from_slice(&fs::read(&path)?)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        for (game_index, record) in pair.games.iter().enumerate() {
            let winner = match &record.termination {
                TerminationRecord::AdjudicatedWin { winner, reason } if reason == "Repetition" => {
                    match winner {
                        StoredColor::Black => Color::Black,
                        StoredColor::White => Color::White,
                    }
                }
                _ => continue,
            };
            games += 1;
            let loser = winner.opposite();
            // 勝者の応手で終わる局では、敗者の最後の着手は末尾から2手目である。
            let root_index = record
                .turns
                .iter()
                .rposition(|turn| turn.side == stored_color(loser))
                .expect("repetition loss must contain a move by the loser");
            let mut game = Game::new(rules);
            let mut root = None;
            let replay = (|| -> Result<GameStatus, String> {
                for text in &pair.opening.moves {
                    let status = play_usi(&mut game, text)?;
                    if status != GameStatus::Ongoing {
                        return Err(format!("opening ended early: {status:?}"));
                    }
                }
                for (index, turn) in record.turns.iter().enumerate() {
                    if turn.side != stored_color(game.position().side_to_move()) {
                        return Err(format!("turn {index}: recorded side does not match replay"));
                    }
                    if index == root_index {
                        root = Some(game.clone());
                    }
                    let TurnResponse::Move { usi: text } = &turn.response else {
                        return Err(format!(
                            "turn {index}: expected a move, got {:?}",
                            turn.response
                        ));
                    };
                    let status = play_usi(&mut game, text)?;
                    if status != GameStatus::Ongoing && index + 1 != record.turns.len() {
                        return Err(format!("turn {index}: game ended early: {status:?}"));
                    }
                }
                Ok(game.status())
            })();
            let expected = GameStatus::Finished(GameResult::Win {
                winner,
                reason: WinReason::Repetition,
            });
            if replay != Ok(expected) {
                let actual = match replay {
                    Ok(status) => format!("{status:?}"),
                    Err(error) => error,
                };
                replay_mismatches.push(json!({
                    "pair": pair.pair_number, "game": game_index,
                    "expected": format!("{expected:?}"), "actual": actual,
                }));
            }
            let Some(root) = root else {
                // 再生できない根は測定に混ぜず、JSON保存後に検査を失敗させる。
                continue;
            };

            let started = Instant::now();
            let classified = black_box(&root).repetition_root_moves();
            let first_call = started.elapsed();
            let mut min_of_10 = Duration::MAX;
            for _ in 0..10 {
                let started = Instant::now();
                let repeated = black_box(&root).repetition_root_moves();
                min_of_10 = min_of_10.min(started.elapsed());
                assert_eq!(repeated.winning(), classified.winning());
                assert_eq!(repeated.losing(), classified.losing());
            }
            let started = Instant::now();
            let (winning_ref, losing_ref) = classify_reference(&root);
            let reference_time = started.elapsed();

            let to_usi = |moves: &[Move]| -> Vec<String> {
                moves
                    .iter()
                    .map(|&mv| usi::text_generated(root.position(), mv))
                    .collect()
            };
            if classified.winning() != winning_ref || classified.losing() != losing_ref {
                classification_mismatches.push(json!({
                    "pair": pair.pair_number, "game": game_index,
                    "winning_fn": to_usi(classified.winning()), "winning_ref": to_usi(&winning_ref),
                    "losing_fn": to_usi(classified.losing()), "losing_ref": to_usi(&losing_ref),
                }));
            }
            let TurnResponse::Move { usi: final_text } = &record.turns[root_index].response else {
                panic!("repetition loss must end with a move by the loser");
            };
            let final_move = usi::parse(root.position(), final_text)?;
            let legal = root.legal_moves().len();
            if losing_ref.contains(&final_move) {
                final_move_in_losing += 1;
            } else {
                final_move_not_in_losing.push(json!({
                    "pair": pair.pair_number, "game": game_index, "move": final_text,
                }));
            }
            if losing_ref.len() == legal {
                all_moves_losing_games.push(json!({
                    "pair": pair.pair_number, "game": game_index, "legal": legal,
                }));
            }
            if !winning_ref.is_empty() {
                winning_nonempty += 1;
            }
            legal_counts.push(legal as f64);
            losing_counts.push(losing_ref.len() as f64);
            first_call_times.push(micros(first_call));
            min_of_10_times.push(micros(min_of_10));
            reference_times.push(micros(reference_time));
            // 集計値を局ごとの実測値から検算できるよう、元の標本も保存する。
            measurements.push(json!({
                "pair": pair.pair_number, "game": game_index,
                "root_turn_index": root_index, "turns": record.turns.len(), "move": final_text,
                "legal": legal, "losing": losing_ref.len(), "winning": winning_ref.len(),
                "classification_time_us": {
                    "first_call": micros(first_call), "min_of_10": micros(min_of_10),
                },
                "reference_time_us": micros(reference_time),
            }));
        }
    }

    let report = json!({
        "run_dir": run_dir,
        "games": games,
        "replay_mismatches": replay_mismatches,
        "classification_mismatches": classification_mismatches,
        "final_move_in_losing": final_move_in_losing,
        "final_move_not_in_losing": final_move_not_in_losing,
        "all_moves_losing": all_moves_losing_games.len(),
        "all_moves_losing_games": all_moves_losing_games,
        "legal_moves": summary(&mut legal_counts),
        "losing_counts": summary(&mut losing_counts),
        "winning_nonempty": winning_nonempty,
        "classification_time_us": {
            "first_call": summary(&mut first_call_times),
            "min_of_10": summary(&mut min_of_10_times),
        },
        "reference_time_us": summary(&mut reference_times),
        "measurements": measurements,
    });
    fs::create_dir_all(output.parent().expect("output path must have a parent"))?;
    fs::write(
        output,
        format!("{}\n", serde_json::to_string_pretty(&report)?),
    )?;

    // 不一致は全件をJSONへ保存したうえで、診断の失敗として通知する。
    assert_eq!(games, 184, "unexpected number of repetition losses");
    assert_eq!(
        measurements.len(),
        games,
        "some roots could not be replayed"
    );
    assert!(replay_mismatches.is_empty(), "see {}", output.display());
    assert!(
        classification_mismatches.is_empty(),
        "see {}",
        output.display()
    );
    assert!(
        final_move_not_in_losing.is_empty(),
        "see {}",
        output.display()
    );
    assert_eq!(final_move_in_losing, games);
    Ok(())
}
