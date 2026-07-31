use crate::piece::{Color, PieceCode, PieceKind};
use crate::position::{Position, PositionBuilder};
use crate::square::Square;

pub(crate) fn sq(file: u8, rank: u8) -> Square {
    Square::new(file, rank).unwrap()
}

pub(crate) fn position(
    side_to_move: Color,
    pieces: &[(Square, Color, PieceKind)],
) -> Position {
    let mut builder = PositionBuilder::new(side_to_move);
    for &(square, color, kind) in pieces {
        builder.put(square, PieceCode::new(color, kind)).unwrap();
    }
    builder.finish().unwrap()
}

pub(crate) fn position_from_codes(
    side_to_move: Color,
    pieces: &[(Square, PieceCode)],
) -> Position {
    let mut builder = PositionBuilder::new(side_to_move);
    for &(square, piece) in pieces {
        builder.put(square, piece).unwrap();
    }
    builder.finish().unwrap()
}
