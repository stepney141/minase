//! 12×12盤の升の座標表現。
//!
//! 段0は先手側の最下段であり、段が増える向きが先手の前方にあたる。

use core::iter::FusedIterator;

/// 盤の筋数(12)。
pub const BOARD_FILES: u8 = 12;
/// 盤の段数(12)。
pub const BOARD_RANKS: u8 = 12;
/// 有効な升の総数(144)(第4条)。
pub const BOARD_SQUARE_COUNT: usize = 144;
/// 番兵を含む生インデックス空間の大きさ(16升幅×12段=192)。
pub const RAW_SQUARE_COUNT: usize = 192;

/// 盤上の升。内部表現は `rank << 4 | file` の生値で、16升幅の番兵付き盤に対応する。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Square(u8);

impl Square {
    /// 筋と段から升を作る。盤外なら`None`を返す。
    #[inline]
    pub const fn new(file: u8, rank: u8) -> Option<Self> {
        if file < BOARD_FILES && rank < BOARD_RANKS {
            Some(Self::new_unchecked(file, rank))
        } else {
            None
        }
    }

    /// 生値(`rank << 4 | file`)から升を作る。番兵位置なら`None`を返す。
    #[inline]
    pub const fn from_raw(raw: u8) -> Option<Self> {
        Self::new(raw & 0x0f, raw >> 4)
    }

    /// 密インデックス(0..144)から升を作る。範囲外なら`None`を返す。
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

    /// 範囲検査なしで筋と段から升を作る。
    #[inline]
    pub(crate) const fn new_unchecked(file: u8, rank: u8) -> Self {
        Self((rank << 4) | file)
    }

    /// 筋(0始まり)を返す。
    #[inline]
    pub const fn file(self) -> u8 {
        self.0 & 0x0f
    }

    /// 段(0始まり)を返す。
    #[inline]
    pub const fn rank(self) -> u8 {
        self.0 >> 4
    }

    /// 生値を返す。
    #[inline]
    pub const fn raw(self) -> u8 {
        self.0
    }

    /// 生値を`usize`で返す。番兵込み配列の添字に使う。
    #[inline]
    pub const fn raw_index(self) -> usize {
        self.0 as usize
    }

    /// 密インデックス(0..144)を返す。番兵を除いた配列の添字に使う。
    #[inline]
    pub const fn dense_index(self) -> usize {
        self.rank() as usize * BOARD_FILES as usize + self.file() as usize
    }

    /// 筋と段の差分だけ移動した升を返す。盤外なら`None`を返す。
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

    /// 全144升を密インデックス順に走査するイテレータを返す。
    #[inline]
    pub const fn all() -> SquareRange {
        SquareRange { next_dense: 0 }
    }
}

/// 全升を密インデックス順に走査するイテレータ。
#[derive(Clone, Debug)]
pub struct SquareRange {
    /// 次に返す升の密インデックス。
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
