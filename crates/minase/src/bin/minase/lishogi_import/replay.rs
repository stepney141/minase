//! 棋譜の再生と規則および勝者の検査。

use super::{
    filter::{Exclusion, winner},
    input::InputGame,
};
use minase::datagen;
use minase_core::{Game, GameResult, Move, Position, Rules, notation::usi};

pub(super) struct ReplayPosition {
    pub(super) position: Position,
    pub(super) ply: u16,
    pub(super) repeated: bool,
}

pub(super) struct Replay {
    pub(super) positions: Vec<ReplayPosition>,
    pub(super) truncated: Option<u32>,
    pub(super) moves: Vec<Move>,
    pub(super) result: Option<GameResult>,
}

/// 対局全体の規則と勝者を照合してから、探索対象を返す。
pub(super) fn replay(input: &InputGame, openings: bool) -> Result<Replay, Exclusion> {
    let mut game = Game::new(Rules::ENGINE_DEFAULT);
    let mut positions = Vec::new();
    let mut truncated = None;
    let mut played_moves = Vec::new();
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
            .and_then(|mv| game.play(mv).ok().map(|_| mv));
        if let Some(mv) = legal {
            played_moves.push(mv);
        } else {
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
        moves: played_moves,
        result: game.result(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase_core::{Color, WinReason};

    #[test]
    fn terminal_winner_is_checked_and_later_moves_are_ignored() {
        let mut value: serde_json::Value =
            include_str!("../../../../tests/fixtures/lishogi_import_cases.ndjson")
                .lines()
                .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                .find(|game| game["id"] == "SNjoPiHz")
                .unwrap();
        let input: InputGame = serde_json::from_value(value.clone()).unwrap();
        let original = replay(&input, false).unwrap();
        assert_eq!(
            original.result,
            Some(GameResult::Win {
                winner: Color::White,
                reason: WinReason::RoyalCapture
            })
        );
        value["moves"] = serde_json::json!(format!("{} 1あ", input.moves));
        let input = serde_json::from_value(value.clone()).unwrap();
        let trailing = replay(&input, false).unwrap();
        assert_eq!(trailing.result, original.result);
        assert_eq!(trailing.moves, original.moves);
        assert_eq!(trailing.truncated, None);
        value["winner"] = serde_json::json!("sente");
        let input = serde_json::from_value(value).unwrap();
        assert_eq!(replay(&input, false).err(), Some(Exclusion::WinnerMismatch));
    }
}
