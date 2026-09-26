//! 再生した局面の探索と教師ラベルの付与。

use super::{cli::Common, filter::winner, input::InputGame, replay::replay};
use minase::datagen::{
    self, data_error,
    game::{CompletedGame, CompletedRecord},
    statistics::Statistics,
};
use minase::notation::sfen::{SetupPosition, to_extended_sfen};
use minase::search::{DEFAULT_THREADS, SearchLimits, SearchSnapshot, TranspositionTable, search};
use minase::training::records::{Outcome, Record, best_move_is_tactical};
use minase::{Game, MoveGenerator, Rules};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeMap, io};

#[derive(Serialize)]
pub(super) struct GameDetails {
    pub(super) game: u32,
    pub(super) sente_rating: Option<i32>,
    pub(super) gote_rating: Option<i32>,
    pub(super) speed: Option<String>,
    pub(super) clock: Option<Value>,
    pub(super) days_per_turn: Option<Value>,
    pub(super) status: Option<String>,
    pub(super) winner: Option<String>,
    pub(super) plies: usize,
    pub(super) recorded_positions: usize,
    pub(super) excluded_positions: BTreeMap<&'static str, u64>,
}

pub(super) struct Job {
    pub(super) input: InputGame,
    pub(super) number: u32,
}

pub(super) struct Processed {
    pub(super) completed: CompletedGame,
    pub(super) openings: Vec<(u32, String)>,
    pub(super) score_exclusions: usize,
    pub(super) no_legal_moves: usize,
    pub(super) candidates: usize,
}

pub(super) fn process_job(
    job: &Job,
    common: &Common,
    openings: bool,
    table: &mut TranspositionTable,
) -> io::Result<Processed> {
    let pst = minase::eval::weights().map_err(data_error)?;
    let limits = SearchLimits::new(None, Some(u64::from(common.nodes.get())), None, None)
        .map_err(data_error)?;
    let mut records = Vec::new();
    let mut stats = Statistics::default();
    let mut retained = Vec::new();
    let mut score_exclusions = 0;
    let mut no_legal_moves = 0;
    let replay = replay(&job.input, openings).map_err(data_error)?;
    for candidate in &replay.positions {
        let position = &candidate.position;
        let game = Game::from_position(Rules::ENGINE_DEFAULT, position.clone());
        if openings && game.legal_moves().is_empty() {
            no_legal_moves += 1;
            continue;
        }
        let snapshot = SearchSnapshot::from_game(&game).map_err(data_error)?;
        table.clear();
        let result =
            search(&pst, &snapshot, &limits, DEFAULT_THREADS, table).map_err(data_error)?;
        if openings {
            if result.score.unsigned_abs() > 500 {
                score_exclusions += 1;
                continue;
            }
            let setup = SetupPosition::new(
                position.clone(),
                position.lion_capture_square(),
                u32::from(candidate.ply) + 1,
            )
            .map_err(data_error)?;
            retained.push((u32::from(candidate.ply), to_extended_sfen(&setup)));
        } else if result.score.unsigned_abs() >= datagen::MATE_BAND_START {
            stats.excluded_mate_band += 1;
        } else if best_move_is_tactical(
            position,
            &MoveGenerator::new(Rules::ENGINE_DEFAULT.moves),
            result.best_move,
        )
        .map_err(data_error)?
        {
            stats.excluded_tactical += 1;
        } else if candidate.repeated {
            stats.excluded_repetition += 1;
        } else {
            let outcome = if winner(&job.input) == Some(position.side_to_move()) {
                Outcome::Win
            } else {
                Outcome::Loss
            };
            records.push(CompletedRecord {
                record: Record::from_position(
                    position,
                    i16::try_from(result.score).map_err(data_error)?,
                    outcome,
                    job.number,
                    candidate.ply,
                ),
                search_key: position.zobrist() ^ position.rights_zobrist(),
            });
        }
    }
    stats.recorded_positions = records.len() as u64;
    Ok(Processed {
        completed: CompletedGame {
            game_number: job.number,
            records,
            stats,
        },
        openings: retained,
        score_exclusions,
        no_legal_moves,
        candidates: replay.positions.len(),
    })
}
