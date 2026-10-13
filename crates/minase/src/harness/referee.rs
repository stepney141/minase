//! エンジンの着手と予想手の審判層による検査。

use crate::harness::engine::canonical_jitto;
use crate::harness::failure::EngineFailure;
use crate::harness::player::Protocol;
use minase_core::notation::{cecp, usi};
use minase_core::{Game, GameStatus, Move};

/// エンジンの着手表記を解析し、審判層の合法手集合と照合する。
///
/// 対局進行の正は[`Game`]であり、解析できない着手や合法手集合にない着手は
/// [`EngineFailure::IllegalMove`]として起こした側の負けにする。
/// 理由は`docs/adr/score-engine-faults-as-losses-and-discard-max-ply-pairs.md`にある。
pub fn validate_bestmove(
    game: &Game,
    text: &str,
    protocol: Protocol,
) -> Result<Move, EngineFailure> {
    let legal_moves = game.legal_moves();
    let selected = match protocol {
        Protocol::Usi => {
            usi::parse(game.position(), text).map_err(|_| EngineFailure::IllegalMove)?
        }
        Protocol::Cecp if text == "@@@@" => {
            canonical_jitto(&legal_moves).ok_or(EngineFailure::IllegalMove)?
        }
        Protocol::Cecp => {
            cecp::parse(game.position(), text).map_err(|_| EngineFailure::IllegalMove)?
        }
    };
    if legal_moves.contains(&selected) {
        Ok(selected)
    } else {
        Err(EngineFailure::IllegalMove)
    }
}

/// 予想手の合法性と、その手で対局が続くかを審判層で判定する。
pub(super) fn ponder_move(game: &Game, text: &str) -> Result<(Move, bool), EngineFailure> {
    let selected = validate_bestmove(game, text, Protocol::Usi)?;
    let mut predicted = game.clone();
    let status = predicted
        .play(selected)
        .expect("a legal prediction must be accepted");
    Ok((selected, !matches!(status, GameStatus::Finished(_))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase_core::{MoveGenerator, Rules};

    // [`validate_bestmove`]の審判層との照合を検査する。
    #[test]
    fn referee_rejects_bestmove_outside_the_legal_move_list() {
        let game = Game::new(Rules::ENGINE_DEFAULT);
        let legal = game.legal_moves()[0];
        let legal_text = usi::text(
            game.position(),
            legal,
            &MoveGenerator::new(game.rules().moves),
        )
        .unwrap();
        assert_eq!(
            validate_bestmove(&game, &legal_text, Protocol::Usi),
            Ok(legal)
        );
        // 表記として解釈できない応答
        assert_eq!(
            validate_bestmove(&game, "not-a-move", Protocol::Usi),
            Err(EngineFailure::IllegalMove)
        );
        // 表記としては読めるが初期局面では指せない着手(空升からの移動)
        assert_eq!(
            validate_bestmove(&game, "6f6g", Protocol::Usi),
            Err(EngineFailure::IllegalMove)
        );
    }
}
