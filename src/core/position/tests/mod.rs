//! 局面の試験と共通の補助関数。

use crate::MoveGenerator;
use crate::core::board::Square;
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::{Position, PositionBuilder};
use crate::test_util::{position as position_with_pieces, sq};

mod builder;
mod lion_trigger;
mod make_move;
mod placement;
mod promotion_rights;
mod setup;
mod validate;
mod zobrist;

/// 同じ盤面に、手番・先獅子状態・成り権保留の全組み合わせを設定する。
fn positions_with_distinct_temporary_states() -> Vec<Position> {
    let mut positions = Vec::new();
    for side in Color::ALL {
        for lion_capture in [None, Some(sq(3, 3))] {
            for deferred in [false, true] {
                let mut builder = PositionBuilder::new(side);
                builder
                    .put(
                        sq(4, 9),
                        PieceCode::new(Color::Black, PieceKind::SilverGeneral).unwrap(),
                    )
                    .unwrap();
                if deferred {
                    builder.mark_promotion_deferred(sq(4, 9)).unwrap();
                }
                let mut position = builder.finish().unwrap();
                position.set_lion_capture(lion_capture).unwrap();
                positions.push(position);
            }
        }
    }
    positions
}

/// 既存局面と同じ盤面を、指定手番・先獅子状態なしで直接構築し直す。
fn rebuild_board_with_side(source: &Position, side_to_move: Color) -> Position {
    let mut builder = PositionBuilder::new(side_to_move);
    for square in Square::all() {
        if let Some(piece) = source.piece_at(square) {
            builder.put(square, piece).unwrap();
        }
    }
    builder.finish().unwrap()
}

/// 非獅子(角行)が相手獅子を取った直後の局面と、獅子が取られた升を返す(第15条1項の前提)。
fn position_after_non_lion_captures_lion() -> (Position, Square) {
    let captured_lion = sq(1, 1);
    let mut position = position_with_pieces(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::Bishop),
            (captured_lion, Color::White, PieceKind::Lion),
            (sq(10, 10), Color::White, PieceKind::Pawn),
        ],
    );
    position
        .try_make_move(
            Move {
                from: sq(0, 0),
                mid: None,
                to: captured_lion,
                promote: false,
            },
            &MoveGenerator::standard(),
        )
        .unwrap();
    (position, captured_lion)
}
