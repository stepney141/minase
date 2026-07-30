use crate::square::Square;

pub const DIRECTION_COUNT: usize = 8;

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Direction {
    East = 0,
    West = 1,
    North = 2,
    South = 3,
    NorthEast = 4,
    NorthWest = 5,
    SouthEast = 6,
    SouthWest = 7,
}

impl Direction {
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

    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[inline]
    pub const fn file_delta(self) -> i8 {
        match self {
            Self::East | Self::NorthEast | Self::SouthEast => 1,
            Self::West | Self::NorthWest | Self::SouthWest => -1,
            Self::North | Self::South => 0,
        }
    }

    #[inline]
    pub const fn rank_delta(self) -> i8 {
        match self {
            Self::North | Self::NorthEast | Self::NorthWest => 1,
            Self::South | Self::SouthEast | Self::SouthWest => -1,
            Self::East | Self::West => 0,
        }
    }

    #[inline]
    pub const fn raw_delta(self) -> i16 {
        self.rank_delta() as i16 * 16 + self.file_delta() as i16
    }

    #[inline]
    pub const fn increases_raw_index(self) -> bool {
        self.raw_delta() > 0
    }

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
