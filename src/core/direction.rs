//! 盤座標系での絶対8方向。

use crate::core::square::Square;

/// 方向の総数(8)。
pub const DIRECTION_COUNT: usize = 8;

/// 盤座標系での絶対方向。`North`は段が増える向き、すなわち先手の前方を指す。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Direction {
    /// 東(筋が減る向き)。
    East = 0,
    /// 西(筋が増える向き)。
    West = 1,
    /// 北(段が増える向き、先手の前方)。
    North = 2,
    /// 南(段が減る向き、後手の前方)。
    South = 3,
    /// 北東。
    NorthEast = 4,
    /// 北西。
    NorthWest = 5,
    /// 南東。
    SouthEast = 6,
    /// 南西。
    SouthWest = 7,
}

impl Direction {
    /// 全8方向。
    pub const ALL: [Self; DIRECTION_COUNT] = [
        Self::East,
        Self::West,
        Self::North,
        Self::South,
        Self::NorthEast,
        Self::NorthWest,
        Self::SouthEast,
        Self::SouthWest,
    ];

    /// 配列添字用の番号を返す。
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// この方向へ1升進むときの筋の増分を返す。
    #[inline]
    pub const fn file_delta(self) -> i8 {
        match self {
            Self::East | Self::NorthEast | Self::SouthEast => 1,
            Self::West | Self::NorthWest | Self::SouthWest => -1,
            Self::North | Self::South => 0,
        }
    }

    /// この方向へ1升進むときの段の増分を返す。
    #[inline]
    pub const fn rank_delta(self) -> i8 {
        match self {
            Self::North | Self::NorthEast | Self::NorthWest => 1,
            Self::South | Self::SouthEast | Self::SouthWest => -1,
            Self::East | Self::West => 0,
        }
    }

    /// [`Square`]の生値(16升幅)での増分を返す。
    #[inline]
    pub const fn raw_delta(self) -> i16 {
        self.rank_delta() as i16 * 16 + self.file_delta() as i16
    }

    /// 生値が増える方向かどうかを返す。走査の先頭ビットをlsbとmsbのどちらに取るかの判定に使う。
    #[inline]
    pub const fn increases_raw_index(self) -> bool {
        self.raw_delta() > 0
    }

    /// 逆方向を返す。
    #[inline]
    pub const fn opposite(self) -> Self {
        match self {
            Self::East => Self::West,
            Self::West => Self::East,
            Self::North => Self::South,
            Self::South => Self::North,
            Self::NorthEast => Self::SouthWest,
            Self::NorthWest => Self::SouthEast,
            Self::SouthEast => Self::NorthWest,
            Self::SouthWest => Self::NorthEast,
        }
    }
}

/// 升を指定方向へ1升進めた升を返す。盤外なら`None`を返す。
#[inline]
pub const fn step_square(square: Square, direction: Direction) -> Option<Square> {
    square.offset(direction.file_delta(), direction.rank_delta())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_deltas_match_the_padded_layout() {
        assert_eq!(Direction::East.raw_delta(), 1);
        assert_eq!(Direction::West.raw_delta(), -1);
        assert_eq!(Direction::North.raw_delta(), 16);
        assert_eq!(Direction::South.raw_delta(), -16);
        assert_eq!(Direction::NorthEast.raw_delta(), 17);
        assert_eq!(Direction::NorthWest.raw_delta(), 15);
        assert_eq!(Direction::SouthEast.raw_delta(), -15);
        assert_eq!(Direction::SouthWest.raw_delta(), -17);
    }
}
