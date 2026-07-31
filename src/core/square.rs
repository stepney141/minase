use core::iter::FusedIterator;

pub const BOARD_FILES: u8 = 12;
pub const BOARD_RANKS: u8 = 12;
pub const BOARD_SQUARE_COUNT: usize = 144;
pub const RAW_SQUARE_COUNT: usize = 192;

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Square(u8);

impl Square {
    #[inline]
    pub const fn new(file: u8, rank: u8) -> Option<Self> {
        if file < BOARD_FILES && rank < BOARD_RANKS {
            Some(Self::new_unchecked(file, rank))
        } else {
            None
        }
    }

    #[inline]
    pub const fn from_raw(raw: u8) -> Option<Self> {
        Self::new(raw & 0x0f, raw >> 4)
    }

    #[inline]
    pub const fn from_dense(index: usize) -> Option<Self> {
        if index < BOARD_SQUARE_COUNT {
            Some(Self::new_unchecked(
                (index % BOARD_FILES as usize) as u8,
                (index / BOARD_FILES as usize) as u8,
            ))
        } else {
            None
        }
    }

    #[inline]
    pub(crate) const fn new_unchecked(file: u8, rank: u8) -> Self {
        Self((rank << 4) | file)
    }

    #[inline]
    pub const fn file(self) -> u8 {
        self.0 & 0x0f
    }

    #[inline]
    pub const fn rank(self) -> u8 {
        self.0 >> 4
    }

    #[inline]
    pub const fn raw(self) -> u8 {
        self.0
    }

    #[inline]
    pub const fn raw_index(self) -> usize {
        self.0 as usize
    }

    #[inline]
    pub const fn dense_index(self) -> usize {
        self.rank() as usize * BOARD_FILES as usize + self.file() as usize
    }

    #[inline]
    pub const fn offset(self, file_delta: i8, rank_delta: i8) -> Option<Self> {
        let file = self.file() as i16 + file_delta as i16;
        let rank = self.rank() as i16 + rank_delta as i16;

        if file >= 0 && file < BOARD_FILES as i16 && rank >= 0 && rank < BOARD_RANKS as i16 {
            Some(Self::new_unchecked(file as u8, rank as u8))
        } else {
            None
        }
    }

    #[inline]
    pub const fn all() -> SquareRange {
        SquareRange { next_dense: 0 }
    }
}

#[derive(Clone, Debug)]
pub struct SquareRange {
    next_dense: usize,
}

impl Iterator for SquareRange {
    type Item = Square;

    fn next(&mut self) -> Option<Self::Item> {
        let square = Square::from_dense(self.next_dense)?;
        self.next_dense += 1;
        Some(square)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = BOARD_SQUARE_COUNT - self.next_dense;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for SquareRange {}
impl FusedIterator for SquareRange {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_and_raw_indices_round_trip() {
        for dense in 0..BOARD_SQUARE_COUNT {
            let square = Square::from_dense(dense).unwrap();
            assert_eq!(square.dense_index(), dense);
            assert_eq!(Square::from_raw(square.raw()), Some(square));
        }

        assert_eq!(Square::all().count(), BOARD_SQUARE_COUNT);
        assert!(Square::from_raw(0x0c).is_none());
        assert!(Square::from_raw(0xc0).is_none());
    }

    #[test]
    fn offset_rejects_every_board_edge() {
        let south_west = Square::new(0, 0).unwrap();
        let north_east = Square::new(11, 11).unwrap();

        assert!(south_west.offset(-1, 0).is_none());
        assert!(south_west.offset(0, -1).is_none());
        assert!(north_east.offset(1, 0).is_none());
        assert!(north_east.offset(0, 1).is_none());
    }
}
