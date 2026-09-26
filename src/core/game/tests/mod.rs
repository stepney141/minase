//! 審判の条文別試験と共通の補助関数。

mod exhaustion;
mod illegal_move;
mod local_rules;
mod no_legal_move;
mod repetition;
mod royals;
mod self_play;
mod win;

use super::{DrawReason, Game, GameError, GameResult, GameStatus, IllegalMoveCause, WinReason};
use crate::core::board::Square;
use crate::core::movegen::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::Color;
use crate::core::piece::{PieceCode, PieceKind};
use crate::core::position::Position;
use crate::core::rules::RuleCode;
use crate::core::rules::Rules;
use crate::test_util::{position_from_codes as position, sq};

fn piece(color: Color, kind: PieceKind) -> PieceCode {
    PieceCode::new(color, kind).expect("fixture uses an unpromoted-capable kind")
}

fn prince(color: Color) -> PieceCode {
    PieceCode::new_promoted(color, PieceKind::CrownPrince).unwrap()
}

fn game(position: Position) -> Game {
    Game::from_position(Rules::ENGINE_DEFAULT, position)
}

fn game_with_codes(position: Position, codes: &[RuleCode]) -> Game {
    let lion = if codes.contains(&RuleCode::L1) {
        RuleCode::L1
    } else {
        RuleCode::L0
    };
    let promotion = if codes.contains(&RuleCode::P1) {
        RuleCode::P1
    } else if codes.contains(&RuleCode::P2) {
        RuleCode::P2
    } else {
        RuleCode::P0
    };
    let repetition = [RuleCode::R1, RuleCode::R2, RuleCode::R3]
        .into_iter()
        .find(|code| codes.contains(code))
        .unwrap_or(RuleCode::R1);
    let exhaustion = if codes.contains(&RuleCode::E3) {
        RuleCode::E3
    } else if codes.contains(&RuleCode::E2) {
        RuleCode::E2
    } else {
        RuleCode::E0
    };
    let mut complete = vec![lion];
    complete.extend(codes.iter().copied().filter(|code| {
        !matches!(
            code,
            RuleCode::L0
                | RuleCode::L1
                | RuleCode::P0
                | RuleCode::P1
                | RuleCode::P2
                | RuleCode::R1
                | RuleCode::R2
                | RuleCode::R3
                | RuleCode::E0
                | RuleCode::E2
                | RuleCode::E3
        )
    }));
    complete.extend([promotion, repetition, exhaustion]);
    Game::from_position(Rules::from_codes(&complete).unwrap(), position)
}

fn step(from: Square, to: Square) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote: false,
    }
}

fn promoting(from: Square, to: Square) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote: true,
    }
}

fn win(winner: Color, reason: WinReason) -> Result<GameStatus, GameError> {
    Ok(GameStatus::Finished(GameResult::Win { winner, reason }))
}

fn draw(reason: DrawReason) -> Result<GameStatus, GameError> {
    Ok(GameStatus::Finished(GameResult::Draw { reason }))
}

// 詰み直前局面(第21条2項)。白の(10,8)→(10,9)の完了時に黒王将の受けが尽きる。
fn mate_predecessor() -> (Position, Move) {
    let mv = step(sq(10, 8), sq(10, 9));
    (
        position(
            Color::White,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(0, 11), piece(Color::White, PieceKind::Rook)),
                (sq(11, 0), piece(Color::White, PieceKind::Rook)),
                (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
                (sq(10, 8), piece(Color::White, PieceKind::King)),
            ],
        ),
        mv,
    )
}

// F3(第23条): 手番先手に着手が1つも存在しない局面。王手はかかっていない。
fn f3_no_move_position() -> Position {
    position(
        Color::White,
        &[
            (sq(0, 11), piece(Color::Black, PieceKind::King)),
            (sq(1, 11), piece(Color::Black, PieceKind::Pawn)),
            (sq(0, 10), piece(Color::Black, PieceKind::Lance)),
            (sq(1, 10), piece(Color::Black, PieceKind::Lance)),
            (sq(10, 0), piece(Color::White, PieceKind::King)),
        ],
    )
}

// 第22条5項の猶予直前局面。黒金が白歩を取ると条件が成立し、白王将が
// (4,7)から金を取り返せる。
fn grace_predecessor() -> (Position, Move) {
    (
        position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(5, 5), piece(Color::Black, PieceKind::GoldGeneral)),
                (sq(4, 7), piece(Color::White, PieceKind::King)),
                (sq(5, 6), piece(Color::White, PieceKind::Pawn)),
            ],
        ),
        step(sq(5, 5), sq(5, 6)),
    )
}
