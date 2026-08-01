//! 144升の集合を3個の`u64`で表すビットボード。

use core::iter::FusedIterator;
use core::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not};

use crate::core::square::Square;

/// 盤上の升の集合。ビット位置は[`Square`]の生値に対応し、各`u64`が4段分(16ビット×4)を受け持つ。
/// 筋12〜15にあたる番兵ビットは常に0とする。
#[repr(transparent)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
#[must_use]
pub struct Bitboard([u64; 3]);

const _: [(); 24] = [(); core::mem::size_of::<Bitboard>()];

impl Bitboard {
    /// 1ワード中の有効升(各段の筋0〜11)を示すマスク。
    pub const VALID_WORD: u64 = 0x0fff_0fff_0fff_0fff;
    /// 空集合。
    pub const EMPTY: Self = Self([0; 3]);
    /// 全144升を含む集合。
    pub const FULL: Self = Self([Self::VALID_WORD; 3]);

    /// 生ワードから集合を作る。番兵ビットは取り除く。
    #[inline]
    pub const fn from_words(words: [u64; 3]) -> Self {
        Self([
            words[0] & Self::VALID_WORD,
            words[1] & Self::VALID_WORD,
            words[2] & Self::VALID_WORD,
        ])
    }

    /// 内部の生ワードへの参照を返す。
    #[inline]
    pub const fn words(&self) -> &[u64; 3] {
        &self.0
    }

    /// 指定した1升だけを含む集合を作る。
    #[inline]
    pub fn from_square(square: Square) -> Self {
        let raw = square.raw_index();
        let mut words = [0; 3];
        words[raw / 64] = 1_u64 << (raw % 64);
        Self(words)
    }

    /// 複数の升から集合を作る。
    pub fn from_squares(squares: impl IntoIterator<Item = Square>) -> Self {
        let mut result = Self::EMPTY;
        for square in squares {
            result.set(square);
        }
        result
    }

    /// 空集合かどうかを返す。
    #[inline]
    pub const fn is_empty(self) -> bool {
        (self.0[0] | self.0[1] | self.0[2]) == 0
    }

    /// `other`と共通の升を持つかどうかを返す。
    #[inline]
    pub const fn intersects(self, other: Self) -> bool {
        ((self.0[0] & other.0[0]) | (self.0[1] & other.0[1]) | (self.0[2] & other.0[2])) != 0
    }

    /// 指定升を含むかどうかを返す。
    #[inline]
    pub fn contains(self, square: Square) -> bool {
        let raw = square.raw_index();
        self.0[raw / 64] & (1_u64 << (raw % 64)) != 0
    }

    /// 指定升を集合へ加える。
    #[inline]
    pub fn set(&mut self, square: Square) {
        let raw = square.raw_index();
        self.0[raw / 64] |= 1_u64 << (raw % 64);
    }

    /// 指定升を集合から除く。
    #[inline]
    pub fn clear(&mut self, square: Square) {
        let raw = square.raw_index();
        self.0[raw / 64] &= !(1_u64 << (raw % 64));
    }

    /// 含まれる升の数を返す。
    #[inline]
    pub const fn popcount(self) -> u32 {
        self.0[0].count_ones() + self.0[1].count_ones() + self.0[2].count_ones()
    }

    /// 生値が最小の升を返す。空集合なら`None`を返す。
    #[inline]
    pub fn lsb(self) -> Option<Square> {
        for word_index in 0..3 {
            let word = self.0[word_index];
            if word != 0 {
                let raw = word_index * 64 + word.trailing_zeros() as usize;
                return Square::from_raw(raw as u8);
            }
        }
        None
    }

    /// 生値が最大の升を返す。空集合なら`None`を返す。
    #[inline]
    pub fn msb(self) -> Option<Square> {
        for word_index in (0..3).rev() {
            let word = self.0[word_index];
            if word != 0 {
                let raw = word_index * 64 + (63 - word.leading_zeros() as usize);
                return Square::from_raw(raw as u8);
            }
        }
        None
    }

    /// 生値が最小の升を集合から取り除いて返す。空集合なら`None`を返す。
    #[inline]
    pub fn pop_lsb(&mut self) -> Option<Square> {
        for word_index in 0..3 {
            let word = self.0[word_index];
            if word != 0 {
                let raw = word_index * 64 + word.trailing_zeros() as usize;
                self.0[word_index] &= word - 1;
                return Square::from_raw(raw as u8);
            }
        }
        None
    }

    /// 生値が最大の升を集合から取り除いて返す。空集合なら`None`を返す。
    #[inline]
    pub fn pop_msb(&mut self) -> Option<Square> {
        for word_index in (0..3).rev() {
            let word = self.0[word_index];
            if word != 0 {
                let bit = 63 - word.leading_zeros() as usize;
                self.0[word_index] &= !(1_u64 << bit);
                return Square::from_raw((word_index * 64 + bit) as u8);
            }
        }
        None
    }

    /// 含まれる升を生値の昇順に走査するイテレータを返す。
    #[inline]
    pub fn iter(self) -> SquareIter {
        SquareIter { remaining: self }
    }
}

impl BitAnd for Bitboard {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self([
            self.0[0] & rhs.0[0],
            self.0[1] & rhs.0[1],
            self.0[2] & rhs.0[2],
        ])
    }
}

impl BitAndAssign for Bitboard {
    fn bitand_assign(&mut self, rhs: Self) {
        *self = *self & rhs;
    }
}

impl BitOr for Bitboard {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self([
            self.0[0] | rhs.0[0],
            self.0[1] | rhs.0[1],
            self.0[2] | rhs.0[2],
        ])
    }
}

impl BitOrAssign for Bitboard {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}

impl BitXor for Bitboard {
    type Output = Self;
    fn bitxor(self, rhs: Self) -> Self {
        Self([
            self.0[0] ^ rhs.0[0],
            self.0[1] ^ rhs.0[1],
            self.0[2] ^ rhs.0[2],
        ])
    }
}

impl BitXorAssign for Bitboard {
    fn bitxor_assign(&mut self, rhs: Self) {
        *self = *self ^ rhs;
    }
}

impl Not for Bitboard {
    type Output = Self;
    fn not(self) -> Self {
        Self::from_words([!self.0[0], !self.0[1], !self.0[2]])
    }
}

/// [`Bitboard`]内の升を生値の昇順に返すイテレータ。
#[derive(Clone, Debug)]
pub struct SquareIter {
    /// まだ走査していない升の集合。
    remaining: Bitboard,
}

impl Iterator for SquareIter {
    type Item = Square;

    fn next(&mut self) -> Option<Self::Item> {
        self.remaining.pop_lsb()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.remaining.popcount() as usize;
        (len, Some(len))
    }
}

impl DoubleEndedIterator for SquareIter {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.remaining.pop_msb()
    }
}

impl ExactSizeIterator for SquareIter {}
impl FusedIterator for SquareIter {}

impl IntoIterator for Bitboard {
    type Item = Square;
    type IntoIter = SquareIter;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl IntoIterator for &Bitboard {
    type Item = Square;
    type IntoIter = SquareIter;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_public_construction_masks_padding() {
        assert_eq!(core::mem::size_of::<Bitboard>(), 24);
        assert_eq!(Bitboard::FULL.popcount(), 144);
        assert_eq!(Bitboard::from_words([u64::MAX; 3]), Bitboard::FULL);
        assert_eq!(!Bitboard::EMPTY, Bitboard::FULL);
        assert_eq!(!Bitboard::FULL, Bitboard::EMPTY);
    }

    #[test]
    fn every_square_can_be_set_toggled_and_cleared() {
        for square in Square::all() {
            let mut board = Bitboard::EMPTY;
            board.set(square);
            assert!(board.contains(square));
            board ^= Bitboard::from_square(square);
            assert!(board.is_empty());
            board.set(square);
            board.clear(square);
            assert!(board.is_empty());
        }
    }

    #[test]
    fn iteration_is_complete_and_ordered() {
        let squares: Vec<_> = Bitboard::FULL.iter().collect();
        assert_eq!(squares.len(), 144);
        assert!(squares.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
