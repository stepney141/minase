use crate::bitboard::Bitboard;
use crate::direction::{DIRECTION_COUNT, Direction, step_square};
use crate::square::{RAW_SQUARE_COUNT, Square};

pub type RayTable = [[Bitboard; RAW_SQUARE_COUNT]; DIRECTION_COUNT];

pub fn build_ray_table() -> RayTable {
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

#[inline]
pub fn ray_control(
    rays: &RayTable,
    from: Square,
    direction: Direction,
    occupied: Bitboard,
) -> Bitboard {
    let candidates = rays[direction.index()][from.raw_index()];
    let blockers = candidates & occupied;
    let first = if direction.increases_raw_index() {
        blockers.lsb()
    } else {
        blockers.msb()
    };

    match first {
        None => candidates,
        Some(blocker) => {
            let beyond = rays[direction.index()][blocker.raw_index()];
            candidates & !beyond
        }
    }
}

pub fn sliding_control(
    rays: &RayTable,
    from: Square,
    directions: &[Direction],
    occupied: Bitboard,
) -> Bitboard {
    directions
        .iter()
        .copied()
        .fold(Bitboard::EMPTY, |control, direction| {
            control | ray_control(rays, from, direction, occupied)
        })
}

#[inline]
pub fn sliding_destinations(
    rays: &RayTable,
    from: Square,
    directions: &[Direction],
    occupied: Bitboard,
    own_pieces: Bitboard,
) -> Bitboard {
    sliding_control(rays, from, directions, occupied) & !own_pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_ray(from: Square, direction: Direction, occupied: Bitboard) -> Bitboard {
        let mut result = Bitboard::EMPTY;
        let mut current = from;
        while let Some(next) = step_square(current, direction) {
            result.set(next);
            if occupied.contains(next) {
                break;
            }
            current = next;
        }
        result
    }

    #[test]
    fn every_ray_matches_coordinate_reference() {
        let rays = build_ray_table();
        let mut occupancies = vec![
            Bitboard::EMPTY,
            Bitboard::FULL,
            Bitboard::from_squares(Square::all().filter(|square| square.dense_index() % 7 == 0)),
        ];
        let mut state = 0x6a09_e667_f3bc_c909_u64;
        for _ in 0..64 {
            let mut occupied = Bitboard::EMPTY;
            for square in Square::all() {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                if state >> 63 != 0 {
                    occupied.set(square);
                }
            }
            occupancies.push(occupied);
        }

        for from in Square::all() {
            for direction in Direction::ALL {
                for &occupied in &occupancies {
                    assert_eq!(
                        ray_control(&rays, from, direction, occupied),
                        reference_ray(from, direction, occupied),
                        "from={from:?}, direction={direction:?}"
                    );
                }
            }
        }
    }
}
