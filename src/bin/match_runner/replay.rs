//! 保存済み対局の再生と再開時の記録検証。

use crate::storage::RunStore;
use minase::harness::*;
use minase::notation::usi;
use minase::rng::derive_seed;
use minase::{Color, Game, GameResult, GameStatus, MoveGenerator, Rules};
use std::{io, time::Duration};

/// 再開以前の確定ペアも含め、保存記録の全件から件数を得る。
pub(super) fn ponder_summary(
    store: &RunStore,
    rules: Rules,
    ponder: bool,
    max_ply: u32,
    target_pairs: u64,
) -> io::Result<[PonderCounts; 2]> {
    let mut counts = [PonderCounts::default(), PonderCounts::default()];
    for record in store.records(target_pairs)?.values() {
        let mut opening = Game::new(rules);
        for text in &record.opening.moves {
            let selected = validate_bestmove(&opening, text, Protocol::Usi)
                .map_err(|_| invalid_pair_record("cannot replay saved opening"))?;
            opening
                .play(selected)
                .map_err(|_| invalid_pair_record("cannot apply saved opening"))?;
        }
        for game in &record.games {
            count_ponder_game(game, opening.clone(), ponder, max_ply, &mut counts)?;
        }
    }
    Ok(counts)
}

/// 保存形式の手番を対局管理層の手番へ戻す。
const fn color_from_stored(color: StoredColor) -> Color {
    match color {
        StoredColor::Black => Color::Black,
        StoredColor::White => Color::White,
    }
}

/// 審判層の終局結果が保存済みの終局理由と一致するかを返す。
fn adjudication_matches(result: GameResult, termination: &TerminationRecord) -> bool {
    match (result, termination) {
        (
            GameResult::Win { winner, reason },
            TerminationRecord::AdjudicatedWin {
                winner: saved_winner,
                reason: saved_reason,
            },
        ) => stored_color(winner) == *saved_winner && format!("{reason:?}") == *saved_reason,
        (
            GameResult::Draw { reason },
            TerminationRecord::AdjudicatedDraw {
                reason: saved_reason,
            },
        ) => format!("{reason:?}") == *saved_reason,
        _ => false,
    }
}

/// 保存済み棋譜のプロトコルと時計を検証するための実効条件。
#[derive(Clone, Copy)]
struct SavedGameConditions {
    candidate_protocol: Protocol,
    candidate_limit: SearchLimit,
    baseline_protocol: Protocol,
    baseline_limit: SearchLimit,
    response_timeout: Duration,
}

/// 保存済み1局を開始局面から再生し、集計可能な終局結果へ戻す。
fn validate_saved_game(
    record: &GameRecord,
    opening: &Opening,
    max_ply: u32,
    conditions: SavedGameConditions,
) -> io::Result<PlayedGame> {
    if record.wall_time_ns == 0 {
        return Err(invalid_pair_record("saved game wall time is zero"));
    }
    let total_think_time_ns = record.turns.iter().try_fold(0_u128, |total, turn| {
        total.checked_add(u128::from(turn.think_time_ns))
    });
    if total_think_time_ns.is_none_or(|total| total > u128::from(record.wall_time_ns)) {
        return Err(invalid_pair_record(
            "saved think times exceed the game wall time",
        ));
    }
    let mut game = opening.game.clone();
    let candidate_color = color_from_stored(record.candidate_color);
    let mut clocks = GameClocks::new(
        candidate_color,
        conditions.candidate_limit,
        conditions.baseline_limit,
    );
    for (index, turn) in record.turns.iter().enumerate() {
        if game.ply_count() >= max_ply {
            return Err(invalid_pair_record("saved turn exceeds the ply limit"));
        }
        if Duration::from_nanos(turn.think_time_ns) > conditions.response_timeout {
            return Err(invalid_pair_record(
                "saved think time exceeds the response timeout",
            ));
        }
        let side = game.position().side_to_move();
        if turn.side != stored_color(side)
            || turn
                .evaluation
                .as_ref()
                .is_some_and(|evaluation| evaluation.perspective != turn.side)
        {
            return Err(invalid_pair_record(
                "turn side or evaluation perspective is inconsistent",
            ));
        }
        let protocol = if side == candidate_color {
            conditions.candidate_protocol
        } else {
            conditions.baseline_protocol
        };
        if protocol == Protocol::Cecp
            && (turn.evaluation.is_some()
                || turn.stop_reason.is_some()
                || turn.completed_time_ms.is_some()
                || turn.ponder.is_some())
        {
            return Err(invalid_pair_record(
                "CECP turn must not contain USI search information",
            ));
        }
        if turn.ponder.as_ref().is_some_and(|text| {
            text.is_empty() || text.split_whitespace().count() != 1 || text.trim() != text
        }) {
            return Err(invalid_pair_record(
                "saved prediction must be one USI token",
            ));
        }
        let expects_time_forfeit = matches!(
            turn.response,
            TurnResponse::Failure {
                reason: FailureKind::TimeForfeit
            }
        );
        match clocks.get_mut(side) {
            Some(clock) => {
                let timed_out = clock
                    .update(Duration::from_nanos(turn.think_time_ns))
                    .is_err();
                if timed_out != expects_time_forfeit {
                    return Err(invalid_pair_record(
                        "saved think time does not match the clock result",
                    ));
                }
            }
            None if expects_time_forfeit => {
                return Err(invalid_pair_record(
                    "fixed-limit engine cannot lose on time",
                ));
            }
            _ => {}
        }
        let is_last = index + 1 == record.turns.len();
        match &turn.response {
            TurnResponse::Move { usi: text } => {
                let selected = usi::parse(game.position(), text)
                    .map_err(|_| invalid_pair_record("saved move is not valid USI"))?;
                if !game.legal_moves().contains(&selected) {
                    return Err(invalid_pair_record("saved move is illegal"));
                }
                let canonical = usi::text(
                    game.position(),
                    selected,
                    &MoveGenerator::new(game.rules().moves),
                )
                .map_err(|_| invalid_pair_record("saved move cannot be rendered"))?;
                if canonical != *text {
                    return Err(invalid_pair_record("saved move is not canonical USI"));
                }
                if let GameStatus::Finished(result) = game
                    .play(selected)
                    .map_err(|_| invalid_pair_record("saved move was rejected"))?
                {
                    if !is_last || !adjudication_matches(result, &record.termination) {
                        return Err(invalid_pair_record(
                            "adjudicated result does not match moves",
                        ));
                    }
                    return Ok(PlayedGame::Finished {
                        plies: game.ply_count(),
                        outcome: GameOutcome::Adjudicated(result),
                    });
                }
            }
            TurnResponse::Resigned => {
                if !is_last
                    || record.termination
                        != (TerminationRecord::Resigned {
                            loser: stored_color(side),
                        })
                {
                    return Err(invalid_pair_record(
                        "resignation does not match termination",
                    ));
                }
                return Ok(PlayedGame::Finished {
                    plies: game.ply_count(),
                    outcome: GameOutcome::Resigned {
                        winner: side.opposite(),
                    },
                });
            }
            TurnResponse::Failure { reason } => {
                if !matches!(reason, FailureKind::IllegalMove | FailureKind::TimeForfeit) {
                    return Err(invalid_pair_record(
                        "failure without an engine response must not be a turn",
                    ));
                }
                if !is_last
                    || record.termination
                        != (TerminationRecord::Forfeit {
                            loser: stored_color(side),
                            reason: *reason,
                        })
                {
                    return Err(invalid_pair_record(
                        "engine failure does not match termination",
                    ));
                }
                return Ok(PlayedGame::Finished {
                    plies: game.ply_count(),
                    outcome: GameOutcome::Forfeit {
                        winner: side.opposite(),
                        reason: failure_from_stored(*reason),
                    },
                });
            }
        }
    }

    match record.termination {
        TerminationRecord::Cutoff if game.ply_count() >= max_ply => Ok(PlayedGame::Cutoff {
            plies: game.ply_count(),
        }),
        TerminationRecord::Forfeit { loser, reason }
            if game.ply_count() < max_ply
                && ((record.turns.is_empty()
                    && matches!(reason, FailureKind::Crash | FailureKind::Timeout))
                    || (color_from_stored(loser) == game.position().side_to_move()
                        && matches!(
                            reason,
                            FailureKind::Crash | FailureKind::Timeout | FailureKind::RejectedMove
                        )
                        && (record.turns.is_empty()
                            || matches!(
                                record.turns.last().map(|turn| &turn.response),
                                Some(TurnResponse::Move { .. })
                            )))) =>
        {
            Ok(PlayedGame::Finished {
                plies: game.ply_count(),
                outcome: GameOutcome::Forfeit {
                    winner: color_from_stored(loser).opposite(),
                    reason: failure_from_stored(reason),
                },
            })
        }
        _ => Err(invalid_pair_record(
            "termination is not explained by the saved turns",
        )),
    }
}

/// 破損した対局記録を表すエラーを作る。
fn invalid_pair_record(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// 保存済みペアを決定的に検証し、番号順集計へ戻す。
pub(super) fn completed_pair_from_record(
    record: PairRecord,
    rules: Rules,
    base_seed: u64,
    max_ply: u32,
    response_timeout: Duration,
    candidate: &PlayerConfig,
    baseline: &PlayerConfig,
) -> io::Result<CompletedPair> {
    let pair_seed = derive_seed(base_seed, record.pair_number);
    if record.pair_seed != pair_seed.get() {
        return Err(invalid_pair_record("pair seed does not match pair number"));
    }
    let opening = generate_opening(rules, pair_seed);
    if record.opening.seed != opening.seed.get() || record.opening.moves != opening.usi_moves {
        return Err(invalid_pair_record("opening does not match the pair seed"));
    }
    if record.games[0].candidate_color != StoredColor::Black
        || record.games[1].candidate_color != StoredColor::White
    {
        return Err(invalid_pair_record(
            "candidate colors are not a swapped pair",
        ));
    }
    let expected_seeds = [
        (
            derive_seed(pair_seed.get(), 1),
            derive_seed(pair_seed.get(), 2),
        ),
        (
            derive_seed(pair_seed.get(), 3),
            derive_seed(pair_seed.get(), 4),
        ),
    ];
    for (index, game) in record.games.iter().enumerate() {
        let (candidate, baseline) = expected_seeds[index];
        if game.candidate_seed != candidate.get() || game.baseline_seed != baseline.get() {
            return Err(invalid_pair_record(
                "engine seed does not match the pair seed",
            ));
        }
    }
    let conditions = SavedGameConditions {
        candidate_protocol: candidate.protocol,
        candidate_limit: candidate.limit,
        baseline_protocol: baseline.protocol,
        baseline_limit: baseline.limit,
        response_timeout,
    };
    let game1 = validate_saved_game(&record.games[0], &opening, max_ply, conditions)?;
    let game2 = validate_saved_game(&record.games[1], &opening, max_ply, conditions)?;
    let mut failures = FailureCounts::default();
    record_game_failure(game1, &mut failures);
    record_game_failure(game2, &mut failures);
    let category = match (game1, game2) {
        (
            PlayedGame::Finished { outcome: first, .. },
            PlayedGame::Finished {
                outcome: second, ..
            },
        ) => Some(usize::from(
            half_points(first, Color::Black) + half_points(second, Color::White),
        )),
        _ => None,
    };
    if record.category.map(usize::from) != category {
        return Err(invalid_pair_record(
            "pair category does not match the game results",
        ));
    }
    Ok(CompletedPair {
        number: record.pair_number,
        output: format!("pair {}: loaded from saved record\n", record.pair_number),
        result: PairResult { category, failures },
        record,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // match-harness-efficiency.md「実行記録と再開」: 1手以上進んだ後に応答が
    // 途絶えた局は最終着手の次の手番側の反則負けとして再検証できる。
    #[test]
    fn saved_timeout_after_a_move_is_valid_without_a_failure_turn() {
        let opening = generate_opening(Rules::ENGINE_DEFAULT, derive_seed(7, 1));
        let mut game = opening.game.clone();
        let selected = game.legal_moves()[0];
        let text = usi::text(
            game.position(),
            selected,
            &MoveGenerator::new(game.rules().moves),
        )
        .unwrap();
        let mover = stored_color(game.position().side_to_move());
        assert!(matches!(game.play(selected).unwrap(), GameStatus::Ongoing));
        let loser = stored_color(game.position().side_to_move());
        let record = GameRecord {
            candidate_color: StoredColor::Black,
            candidate_seed: 1,
            baseline_seed: 2,
            wall_time_ns: 1,
            candidate_cpu_time_ns: Some(1),
            baseline_cpu_time_ns: Some(1),
            candidate_peak_rss_bytes: Some(1),
            baseline_peak_rss_bytes: Some(1),
            turns: vec![TurnRecord {
                side: mover,
                think_time_ns: 1,
                evaluation: None,
                stop_reason: None,
                completed_time_ms: None,
                ponder: None,
                response: TurnResponse::Move { usi: text },
            }],
            termination: TerminationRecord::Forfeit {
                loser,
                reason: FailureKind::Timeout,
            },
        };
        assert!(matches!(
            validate_saved_game(
                &record,
                &opening,
                u32::MAX,
                SavedGameConditions {
                    candidate_protocol: Protocol::Usi,
                    candidate_limit: SearchLimit::Fixed {
                        depth: None,
                        nodes: Some(1),
                    },
                    baseline_protocol: Protocol::Usi,
                    baseline_limit: SearchLimit::Fixed {
                        depth: None,
                        nodes: Some(1),
                    },
                    response_timeout: Duration::from_secs(1),
                },
            )
            .unwrap(),
            PlayedGame::Finished {
                outcome: GameOutcome::Forfeit {
                    reason: EngineFailure::Timeout,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn saved_game_validation_rejects_protocol_clock_and_ply_contradictions() {
        let opening = generate_opening(Rules::ENGINE_DEFAULT, derive_seed(8, 1));
        let side = stored_color(opening.game.position().side_to_move());
        let fixed_conditions = SavedGameConditions {
            candidate_protocol: Protocol::Cecp,
            candidate_limit: SearchLimit::Fixed {
                depth: Some(1),
                nodes: None,
            },
            baseline_protocol: Protocol::Usi,
            baseline_limit: SearchLimit::Fixed {
                depth: Some(1),
                nodes: None,
            },
            response_timeout: Duration::from_secs(1),
        };
        let mut record = GameRecord {
            candidate_color: side,
            candidate_seed: 1,
            baseline_seed: 2,
            wall_time_ns: 1,
            candidate_cpu_time_ns: Some(1),
            baseline_cpu_time_ns: Some(1),
            candidate_peak_rss_bytes: Some(1),
            baseline_peak_rss_bytes: Some(1),
            turns: vec![TurnRecord {
                side,
                think_time_ns: 1,
                evaluation: Some(EvaluationRecord {
                    perspective: side,
                    depth: Some(1),
                    score: ScoreRecord::Cp { value: 0 },
                    bound: ScoreBound::Exact,
                }),
                stop_reason: None,
                completed_time_ms: None,
                ponder: None,
                response: TurnResponse::Resigned,
            }],
            termination: TerminationRecord::Resigned { loser: side },
        };
        assert!(validate_saved_game(&record, &opening, u32::MAX, fixed_conditions).is_err());

        record.turns[0].evaluation = None;
        record.turns[0].think_time_ns = u64::MAX;
        assert!(validate_saved_game(&record, &opening, u32::MAX, fixed_conditions).is_err());

        record.turns[0].think_time_ns = 1;
        record.turns[0].response = TurnResponse::Failure {
            reason: FailureKind::TimeForfeit,
        };
        record.termination = TerminationRecord::Forfeit {
            loser: side,
            reason: FailureKind::TimeForfeit,
        };
        assert!(validate_saved_game(&record, &opening, u32::MAX, fixed_conditions).is_err());

        record.turns.clear();
        record.termination = TerminationRecord::Forfeit {
            loser: side,
            reason: FailureKind::Timeout,
        };
        assert!(
            validate_saved_game(
                &record,
                &opening,
                opening.game.ply_count(),
                fixed_conditions
            )
            .is_err()
        );
    }

    // D8-HARN-26（ponder.md保存形式）: 不合法な予想手も記録としては有効である。
    #[test]
    fn saved_predictions_validate_tokens_and_protocol_without_requiring_legality() {
        let opening =
            generate_opening(Rules::ENGINE_DEFAULT, std::num::NonZeroU64::new(1).unwrap());
        let side = stored_color(opening.game.position().side_to_move());
        let mut record = GameRecord {
            candidate_color: side,
            candidate_seed: 1,
            baseline_seed: 1,
            wall_time_ns: 100,
            candidate_cpu_time_ns: None,
            baseline_cpu_time_ns: None,
            candidate_peak_rss_bytes: None,
            baseline_peak_rss_bytes: None,
            turns: vec![TurnRecord {
                side,
                think_time_ns: 1,
                evaluation: None,
                stop_reason: None,
                completed_time_ms: None,
                ponder: Some("not-a-move".to_owned()),
                response: TurnResponse::Resigned,
            }],
            termination: TerminationRecord::Resigned { loser: side },
        };
        record.wall_time_ns = 100;
        let mut conditions = SavedGameConditions {
            candidate_protocol: Protocol::Usi,
            baseline_protocol: Protocol::Usi,
            candidate_limit: parse_search_limit("time=1000+0").unwrap(),
            baseline_limit: parse_search_limit("time=1000+0").unwrap(),
            response_timeout: Duration::from_secs(1),
        };
        assert!(validate_saved_game(&record, &opening, 100, conditions).is_ok());
        for invalid in ["", "two tokens", " trailing", "trailing "] {
            record.turns[0].ponder = Some(invalid.to_owned());
            assert!(validate_saved_game(&record, &opening, 100, conditions).is_err());
        }
        record.turns[0].ponder = Some("not-a-move".to_owned());
        conditions.candidate_protocol = Protocol::Cecp;
        assert!(validate_saved_game(&record, &opening, 100, conditions).is_err());
    }
}
