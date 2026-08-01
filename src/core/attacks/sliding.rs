//! 走り駒の利き線(レイ)の前計算。

use crate::core::bitboard::Bitboard;
use crate::core::direction::{DIRECTION_COUNT, Direction, step_square};
use crate::core::square::{RAW_SQUARE_COUNT, Square};

/// 方向×升ごとに、その升から盤端まで伸びる利き線を持つテーブル。
pub(crate) type RayTable = [[Bitboard; RAW_SQUARE_COUNT]; DIRECTION_COUNT];

/// 各升から各方向へ盤端まで伸びる利き線を構築して返す。
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
