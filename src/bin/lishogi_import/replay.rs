//! 棋譜の再生と規則および勝者の検査。

use super::{
    filter::{Exclusion, winner},
    input::InputGame,
};
use minase::{Game, GameResult, Position, Rules, datagen, notation::usi};

pub(super) struct ReplayPosition {
    pub(super) position: Position,
    pub(super) ply: u16,
    pub(super) repeated: bool,
}

pub(super) struct Replay {
    pub(super) positions: Vec<ReplayPosition>,
    pub(super) truncated: Option<u32>,
}

/// 対局全体の規則と勝者を照合してから、探索対象を返す。
pub(super) fn replay(input: &InputGame, openings: bool) -> Result<Replay, Exclusion> {
    let mut game = Game::new(Rules::ENGINE_DEFAULT);
    let mut positions = Vec::new();
    let mut truncated = None;
    let mut moves = input.moves.split_whitespace();
    loop {
        if !openings && !game.position().promotion_deferred().is_empty() {
            return Err(Exclusion::DeferredPromotion);
        }
        if game.result().is_some() {
            break;
        }
        let ply = game.ply_count();
        if !openings
            || (ply >= 40
                && ply.is_multiple_of(20)
                && game.position().occupied().popcount() >= 47
                && game.position().promotion_deferred().is_empty())
        {
            positions.push(ReplayPosition {
                position: game.position().clone(),
                ply: u16::try_from(ply).map_err(|_| Exclusion::IllegalDefaultRules)?,
                repeated: datagen::current_position_is_repeated(&game),
            });
        }
        let Some(text) = moves.next() else {
            break;
        };
        let legal = usi::parse(game.position(), text)
            .ok()
            .and_then(|mv| game.play(mv).ok());
        if legal.is_none() {
            if !openings {
                return Err(Exclusion::IllegalDefaultRules);
            }
            truncated = Some(ply + 1);
            break;
        }
    }
    if !openings {
        match game.result() {
            Some(GameResult::Win { winner: actual, .. }) if Some(actual) == winner(input) => {}
            Some(_) => return Err(Exclusion::WinnerMismatch),
            None if input.status.as_deref() == Some("royalsLost") => {
                return Err(Exclusion::WinnerMismatch);
            }
            None if Some(game.position().side_to_move()) == winner(input) => {
                return Err(Exclusion::ResignerMismatch);
            }
            None => {}
        }
    }
    Ok(Replay {
        positions,
        truncated,
    })
}
