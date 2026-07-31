use crate::core::bitboard::Bitboard;
use crate::core::direction::{DIRECTION_COUNT, Direction, step_square};
use crate::core::square::{RAW_SQUARE_COUNT, Square};

pub(crate) type RayTable = [[Bitboard; RAW_SQUARE_COUNT]; DIRECTION_COUNT];

pub(crate) fn build_ray_table() -> RayTable {
    let mut rays = [[Bitboard::EMPTY; RAW_SQUARE_COUNT]; DIRECTION_COUNT];

    for from in Square::all() {
        for direction in Direction::ALL {
            let mut current = from;
            while let Some(next) = step_square(current, direction) {
                rays[direction.index()][from.raw_index()].set(next);
                current = next;
            }
        }
    }
    rays
}
