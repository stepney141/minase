//! 保存記録の再生による先読みの集計。

use crate::harness::player::Protocol;
use crate::harness::records::{GameRecord, TurnResponse};
use crate::harness::referee::{ponder_move, validate_bestmove};
use crate::{Game, GameStatus};
use std::io;

/// 保存済みの応答列から再計算するエンジン別の先読み件数。
#[derive(Default, Debug, PartialEq, Eq)]
pub struct PonderCounts {
    /// 予想手を返した回数。
    pub predictions: u64,
    /// 不合法な予想手の回数。
    pub illegal_predictions: u64,
    /// 先読みを開始した回数。
    pub starts: u64,
    /// 予想手が的中した回数。
    pub hits: u64,
    /// 指した着手の数。
    pub moves: u64,
}

/// 候補・基準に同じ審判規則を適用し、通信を再現する。
pub fn count_ponder_game(
    record: &GameRecord,
    mut game: Game,
    ponder: bool,
    max_ply: u32,
    counts: &mut [PonderCounts; 2],
) -> io::Result<()> {
    let mut pending = [None; 2];
    for turn in &record.turns {
        let side = game.position().side_to_move();
        let index = usize::from(turn.side != record.candidate_color);
        counts[index].predictions += u64::from(turn.ponder.is_some());
        let TurnResponse::Move { usi: text } = &turn.response else {
            continue;
        };
        counts[index].moves += 1;
        let selected = validate_bestmove(&game, text, Protocol::Usi).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "cannot replay saved move for ponder counts",
            )
        })?;
        let status = game.play(selected).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "cannot apply saved move for ponder counts",
            )
        })?;
        if matches!(status, GameStatus::Finished(_)) {
            continue;
        }
        if pending[1 - index].take() == Some(selected) && game.ply_count() < max_ply {
            counts[1 - index].hits += 1;
        }
        if let Some(text) = &turn.ponder {
            match ponder_move(&game, text) {
                Err(_) => counts[index].illegal_predictions += 1,
                Ok((predicted, true)) if ponder => {
                    counts[index].starts += 1;
                    pending[index] = Some(predicted);
                }
                Ok(_) => {}
            }
        }
        debug_assert_eq!(game.position().side_to_move(), side.opposite());
    }
    Ok(())
}
