use std::io::{BufRead, BufReader};

use flate2::read::GzDecoder;
use minase::notation::{sfen::parse_extended_sfen, usi};
use minase::{
    Color, DrawReason, Game, GameResult, GameStatus, Position, RuleCode, Rules, WinReason,
};
use serde_json::Value;

const REPLAYS: &[u8] = include_bytes!("fixtures/lishogi_replays.ndjson.gz");

#[test]
fn lishogi_replays_match_legal_moves_and_adjudication() {
    let decoder = GzDecoder::new(REPLAYS);
    let mut failures = Vec::new();
    let mut replay_count = 0;

    for (line_index, line) in BufReader::new(decoder).lines().enumerate() {
        let line = line.unwrap_or_else(|error| {
            panic!(
                "failed to read replay fixture line {}: {error}",
                line_index + 1
            )
        });
        let replay: Value = serde_json::from_str(&line).unwrap_or_else(|error| {
            panic!(
                "failed to parse replay fixture line {}: {error}",
                line_index + 1
            )
        });
        replay_count += 1;
        if let Err(error) = replay_game(&replay) {
            failures.push(error);
        }
    }

    assert_eq!(replay_count, 10, "fixture must contain exactly 10 replays");
    assert!(
        failures.is_empty(),
        "lishogi replay mismatches:\n{}",
        failures.join("\n")
    );
}

fn replay_game(replay: &Value) -> Result<(), String> {
    let id = string_field(replay, "id")?;
    let initial_sfen = string_field(replay, "initial_sfen")?;
    let moves = string_field(replay, "moves")?;
    let status = string_field(replay, "status")?;
    let winner = winner_field(replay, id)?;
    let rules = Rules::from_codes(&[
        RuleCode::L1,
        RuleCode::L2,
        RuleCode::P3,
        RuleCode::R1,
        RuleCode::E1,
        RuleCode::E3,
    ])
    .expect("the lishogi replay rule set must be valid");

    let setup = parse_extended_sfen(initial_sfen, rules)
        .map_err(|error| format!("game {id}: failed to parse initial_sfen: {error}"))?;
    if setup.position != Position::initial()
        || setup.lion_capture.is_some()
        || setup.next_move_number != 1
    {
        return Err(format!(
            "game {id}: initial_sfen is not the standard initial position: {setup:?}"
        ));
    }

    let mut position = setup.position;
    position
        .set_lion_capture(setup.lion_capture)
        .map_err(|error| format!("game {id}: failed to restore lion-capture state: {error}"))?;
    let mut game = Game::from_position(rules, position)
        .map_err(|error| format!("game {id}: failed to construct game: {error}"))?;
    let moves: Vec<_> = moves.split_whitespace().collect();
    if moves.is_empty() {
        return Err(format!("game {id}: moves must not be empty"));
    }

    let mut final_status = GameStatus::Ongoing;
    for (index, move_text) in moves.iter().enumerate() {
        let ply = index + 1;
        let mv = usi::parse(game.position(), move_text).map_err(|error| {
            format!("game {id}, ply {ply}: failed to parse move {move_text}: {error}")
        })?;
        final_status = game.play(mv).map_err(|error| {
            format!("game {id}, ply {ply}: failed to play move {move_text}: {error}")
        })?;
        if ply < moves.len() && final_status != GameStatus::Ongoing {
            return Err(format!(
                "game {id}, ply {ply}: game finished before the final move; actual {final_status:?}"
            ));
        }
    }

    match status {
        "royalsLost" => expect_final_status(
            id,
            moves.len(),
            final_status,
            GameStatus::Finished(GameResult::Win {
                winner: required_winner(id, winner)?,
                reason: WinReason::RoyalCapture,
            }),
        ),
        // 第32条E3を採用し、lishogiの裸玉裁定と照合する。
        "bareKing" => expect_final_status(
            id,
            moves.len(),
            final_status,
            GameStatus::Finished(GameResult::Win {
                winner: required_winner(id, winner)?,
                reason: WinReason::PieceExhaustion,
            }),
        ),
        "repetition" => {
            if winner.is_some() {
                return Err(format!(
                    "game {id}: draw by repetition must not have a winner"
                ));
            }
            expect_final_status(
                id,
                moves.len(),
                final_status,
                GameStatus::Finished(GameResult::Draw {
                    reason: DrawReason::Repetition,
                }),
            )
        }
        "draw" => {
            if winner.is_some() {
                return Err(format!("game {id}: agreed draw must not have a winner"));
            }
            expect_final_status(id, moves.len(), final_status, GameStatus::Ongoing)?;
            let agreed = game
                .agree_draw()
                .map_err(|error| format!("game {id}: failed to agree draw: {error}"))?;
            expect_final_status(
                id,
                moves.len(),
                agreed,
                GameStatus::Finished(GameResult::Draw {
                    reason: DrawReason::Agreement,
                }),
            )
        }
        "resign" => {
            let winner = required_winner(id, winner)?;
            expect_final_status(id, moves.len(), final_status, GameStatus::Ongoing)?;
            let resigned = game
                .resign(winner.opposite())
                .map_err(|error| format!("game {id}: failed to resign: {error}"))?;
            expect_final_status(
                id,
                moves.len(),
                resigned,
                GameStatus::Finished(GameResult::Win {
                    winner,
                    reason: WinReason::Resignation,
                }),
            )
        }
        other => unreachable!("game {id}: unsupported lishogi status {other:?}"),
    }
}

fn string_field<'a>(replay: &'a Value, field: &str) -> Result<&'a str, String> {
    replay
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("replay field {field:?} must be a string"))
}

fn winner_field(replay: &Value, id: &str) -> Result<Option<Color>, String> {
    match replay.get("winner") {
        Some(Value::String(winner)) if winner == "sente" => Ok(Some(Color::Black)),
        Some(Value::String(winner)) if winner == "gote" => Ok(Some(Color::White)),
        Some(Value::Null) => Ok(None),
        Some(other) => Err(format!("game {id}: invalid winner {other}")),
        None => Err(format!("game {id}: missing winner")),
    }
}

fn required_winner(id: &str, winner: Option<Color>) -> Result<Color, String> {
    winner.ok_or_else(|| format!("game {id}: status requires a winner"))
}

fn expect_final_status(
    id: &str,
    ply: usize,
    actual: GameStatus,
    expected: GameStatus,
) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "game {id}, ply {ply}: expected {expected:?}, actual {actual:?}"
        ))
    }
}
