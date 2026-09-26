//! 1局の進行と終局結果。

use crate::harness::clock::GameClocks;
use crate::harness::engine::{EngineProcess, EngineResponse, ThinkResult};
use crate::harness::failure::EngineFailure;
use crate::harness::player::PlayerConfig;
use crate::harness::records::convert::{
    RecordedGame, duration_ns, evaluation_record, recorded_game, stored_color, stored_failure,
};
use crate::harness::records::{TurnRecord, TurnResponse};
use crate::harness::referee::validate_bestmove;
use crate::notation::usi;
use crate::{Color, Game, GameResult, GameStatus, Move, MoveGenerator};
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// 審判裁定またはエンジン反則による終局結果。
#[derive(Clone, Copy)]
pub enum GameOutcome {
    /// 審判層の裁定による終局。
    Adjudicated(GameResult),
    /// エンジンの反則による負け。
    Forfeit {
        /// 反則をしなかった側。
        winner: Color,
        /// 反則の分類。
        reason: EngineFailure,
    },
    /// エンジンの投了による勝敗。
    Resigned {
        /// 投了しなかった側。
        winner: Color,
    },
}

/// 1局の完走または手数上限による打ち切り。
#[derive(Clone, Copy)]
pub enum PlayedGame {
    /// 終局した1局。
    Finished {
        /// 終了時点の手数。
        plies: u32,
        /// 終局結果。
        outcome: GameOutcome,
    },
    /// 手数上限による打ち切り。
    Cutoff {
        /// 打ち切り時点の手数。
        plies: u32,
    },
}

/// 1局を既存の対局管理層で進行する。
#[allow(clippy::too_many_arguments)]
pub fn play_game(
    mut game: Game,
    mut usi_history: Vec<String>,
    mut move_history: Vec<Move>,
    max_ply: u32,
    player_a_color: Color,
    player_a: &PlayerConfig,
    player_a_seed: NonZeroU64,
    player_b: &PlayerConfig,
    player_b_seed: NonZeroU64,
    timeout: Duration,
    ponder: bool,
    stop: &AtomicBool,
) -> Option<RecordedGame> {
    let started = Instant::now();
    let forfeit = |plies, loser: Color, reason| PlayedGame::Finished {
        plies,
        outcome: GameOutcome::Forfeit {
            winner: loser.opposite(),
            reason,
        },
    };
    let mut turns = Vec::new();
    if game.ply_count() >= max_ply {
        return Some(recorded_game(
            PlayedGame::Cutoff {
                plies: game.ply_count(),
            },
            player_a_color,
            player_a_seed,
            player_b_seed,
            turns,
            started,
            None,
            None,
        ));
    }
    let mut player_a_process = match EngineProcess::start(player_a, player_a_seed.get(), timeout) {
        Ok(process) => process,
        Err(reason) => {
            return Some(recorded_game(
                forfeit(game.ply_count(), player_a_color, reason),
                player_a_color,
                player_a_seed,
                player_b_seed,
                turns,
                started,
                None,
                None,
            ));
        }
    };
    let mut player_b_process = match EngineProcess::start(player_b, player_b_seed.get(), timeout) {
        Ok(process) => process,
        Err(reason) => {
            return Some(recorded_game(
                forfeit(game.ply_count(), player_a_color.opposite(), reason),
                player_a_color,
                player_a_seed,
                player_b_seed,
                turns,
                started,
                Some(&mut player_a_process),
                None,
            ));
        }
    };
    let mut clocks = GameClocks::new(player_a_color, player_a.limit, player_b.limit);

    loop {
        if stop.load(Ordering::Acquire) {
            return None;
        }
        if game.ply_count() >= max_ply {
            return Some(recorded_game(
                PlayedGame::Cutoff {
                    plies: game.ply_count(),
                },
                player_a_color,
                player_a_seed,
                player_b_seed,
                turns,
                started,
                Some(&mut player_a_process),
                Some(&mut player_b_process),
            ));
        }

        let side_to_move = game.position().side_to_move();
        let (process, limit) = if side_to_move == player_a_color {
            (&mut player_a_process, player_a.limit)
        } else {
            (&mut player_b_process, player_b.limit)
        };
        let ThinkResult {
            response,
            elapsed,
            evaluation,
            stop_reason,
            completed_time_ms,
            ponder: prediction,
        } = match process.bestmove(&usi_history, &move_history, &clocks, side_to_move, limit) {
            Ok(response) => response,
            Err(reason) => {
                return Some(recorded_game(
                    forfeit(game.ply_count(), side_to_move, reason),
                    player_a_color,
                    player_a_seed,
                    player_b_seed,
                    turns,
                    started,
                    Some(&mut player_a_process),
                    Some(&mut player_b_process),
                ));
            }
        };
        if stop.load(Ordering::Acquire) {
            return None;
        }
        if let Some(clock) = clocks.get_mut(side_to_move)
            && let Err(reason) = clock.update(elapsed)
        {
            turns.push(TurnRecord {
                side: stored_color(side_to_move),
                think_time_ns: duration_ns(elapsed),
                evaluation: evaluation_record(evaluation, side_to_move),
                stop_reason,
                completed_time_ms,
                ponder: prediction.clone(),
                response: TurnResponse::Failure {
                    reason: stored_failure(reason),
                },
            });
            return Some(recorded_game(
                forfeit(game.ply_count(), side_to_move, reason),
                player_a_color,
                player_a_seed,
                player_b_seed,
                turns,
                started,
                Some(&mut player_a_process),
                Some(&mut player_b_process),
            ));
        }
        let EngineResponse::Move(response) = response else {
            turns.push(TurnRecord {
                side: stored_color(side_to_move),
                think_time_ns: duration_ns(elapsed),
                evaluation: evaluation_record(evaluation, side_to_move),
                stop_reason,
                completed_time_ms,
                ponder: prediction.clone(),
                response: TurnResponse::Resigned,
            });
            return Some(recorded_game(
                PlayedGame::Finished {
                    plies: game.ply_count(),
                    outcome: GameOutcome::Resigned {
                        winner: side_to_move.opposite(),
                    },
                },
                player_a_color,
                player_a_seed,
                player_b_seed,
                turns,
                started,
                Some(&mut player_a_process),
                Some(&mut player_b_process),
            ));
        };
        let selected = match validate_bestmove(&game, &response, process.protocol) {
            Ok(selected) => selected,
            Err(reason) => {
                turns.push(TurnRecord {
                    side: stored_color(side_to_move),
                    think_time_ns: duration_ns(elapsed),
                    evaluation: evaluation_record(evaluation, side_to_move),
                    stop_reason,
                    completed_time_ms,
                    ponder: prediction.clone(),
                    response: TurnResponse::Failure {
                        reason: stored_failure(reason),
                    },
                });
                return Some(recorded_game(
                    forfeit(game.ply_count(), side_to_move, reason),
                    player_a_color,
                    player_a_seed,
                    player_b_seed,
                    turns,
                    started,
                    Some(&mut player_a_process),
                    Some(&mut player_b_process),
                ));
            }
        };
        let canonical = usi::text(
            game.position(),
            selected,
            &MoveGenerator::new(game.rules().moves),
        )
        .expect("a move validated against legal_moves must be renderable");
        turns.push(TurnRecord {
            side: stored_color(side_to_move),
            think_time_ns: duration_ns(elapsed),
            evaluation: evaluation_record(evaluation, side_to_move),
            stop_reason,
            completed_time_ms,
            ponder: prediction.clone(),
            response: TurnResponse::Move {
                usi: canonical.clone(),
            },
        });
        usi_history.push(canonical);
        move_history.push(selected);
        let status = game
            .play(selected)
            .expect("a move validated against legal_moves must be accepted");
        if let GameStatus::Finished(result) = status {
            return Some(recorded_game(
                PlayedGame::Finished {
                    plies: game.ply_count(),
                    outcome: GameOutcome::Adjudicated(result),
                },
                player_a_color,
                player_a_seed,
                player_b_seed,
                turns,
                started,
                Some(&mut player_a_process),
                Some(&mut player_b_process),
            ));
        }
        if ponder {
            process.start_ponder(
                &game,
                &usi_history,
                prediction.as_deref(),
                &clocks.think_request(side_to_move, limit),
            );
        }
    }
}
