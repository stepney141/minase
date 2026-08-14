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
    use crate::core::square::BOARD_SQUARE_COUNT;

    /// 決定的に構成した検査用の升集合標本を返す。空集合・全集合・単升・
    /// 構造的な部分集合・擬似乱数集合を含む。
    fn sample_sets() -> Vec<Bitboard> {
        let corner_squares = [
            Square::new(0, 0).unwrap(),
            Square::new(11, 0).unwrap(),
            Square::new(0, 11).unwrap(),
            Square::new(11, 11).unwrap(),
            Square::new(5, 6).unwrap(),
        ];
        let mut sets = vec![Bitboard::EMPTY, Bitboard::FULL];
        sets.extend(corner_squares.map(Bitboard::from_square));
        sets.push(Bitboard::from_squares(
            Square::all().filter(|square| square.rank() == 0),
        ));
        sets.push(Bitboard::from_squares(
            Square::all().filter(|square| square.file() == 11),
        ));
        sets.push(Bitboard::from_squares(
            Square::all().filter(|square| square.dense_index() % 2 == 0),
        ));
        // 旧テスト資産のシード系列から派生した決定的な擬似乱数集合。
        let mut state = 0x5a4f_4252_4953_5401_u64;
        for _ in 0..3 {
            let members = Square::all()
                .filter(|_| {
                    state = state
                        .wrapping_mul(2_862_933_555_777_941_757)
                        .wrapping_add(3_037_000_493);
                    state & 1 == 0
                })
                .collect::<Vec<_>>();
            sets.push(Bitboard::from_squares(members));
        }
        sets
    }

    // 実装契約(D4-IMP-05・D4-IMP-01の基盤): ビットボードは升の集合として振る舞う。
    // 追加・除去・所属・要素数・構築の各操作が集合の公理と一致し、内部語構成には依存しない。
    #[test]
    fn bitboard_acts_as_the_set_of_its_squares() {
        assert!(Bitboard::EMPTY.is_empty());
        assert_eq!(Bitboard::EMPTY.popcount(), 0);
        assert_eq!(Bitboard::FULL.popcount() as usize, BOARD_SQUARE_COUNT);
        for square in Square::all() {
            assert!(!Bitboard::EMPTY.contains(square));
            assert!(Bitboard::FULL.contains(square));
        }

        for square in Square::all() {
            let mut board = Bitboard::EMPTY;
            board.set(square);
            assert!(board.contains(square));
            assert_eq!(board.popcount(), 1);
            assert_eq!(board, Bitboard::from_square(square));
            // 追加は冪等、除去は逆操作、不在升の除去は無作用。
            board.set(square);
            assert_eq!(board.popcount(), 1);
            board.clear(square);
            assert!(board.is_empty());
            board.clear(square);
            assert!(board.is_empty());
        }

        for set in sample_sets() {
            // from_squaresは列挙した升をちょうど含む集合を作る。
            assert_eq!(Bitboard::from_squares(set.iter()), set);
            // 生ワードの往復は恒等である(語の構成自体は仮定しない)。
            assert_eq!(Bitboard::from_words(*set.words()), set);
            // 要素数は所属する升の個数と一致する。
            let member_count = Square::all().filter(|&square| set.contains(square)).count();
            assert_eq!(set.popcount() as usize, member_count);
        }
    }

    // 実装契約: ビット演算は升ごとの集合演算(和・積・対称差・補)と一致し、
    // 補集合は144升の宇宙に閉じる。intersectsは共通要素の存在と同値である。
    #[test]
    fn bitwise_operators_match_elementwise_set_operations() {
        let sets = sample_sets();
        for &a in &sets {
            // 補集合: 全升で所属が反転し、宇宙の外に要素を作らない。
            let complement = !a;
            for square in Square::all() {
                assert_eq!(complement.contains(square), !a.contains(square));
            }
            assert_eq!(
                complement.popcount() + a.popcount(),
                Bitboard::FULL.popcount()
            );
            assert_eq!(!complement, a, "二重補集合は恒等");

            for &b in &sets {
                for square in Square::all() {
                    assert_eq!(
                        (a | b).contains(square),
                        a.contains(square) || b.contains(square)
                    );
                    assert_eq!(
                        (a & b).contains(square),
                        a.contains(square) && b.contains(square)
                    );
                    assert_eq!(
                        (a ^ b).contains(square),
                        a.contains(square) != b.contains(square)
                    );
                }
                assert_eq!(a.intersects(b), !(a & b).is_empty());
            }
            assert_eq!(a ^ a, Bitboard::EMPTY);
        }
    }

    // 実装契約: 走査は含まれる升をちょうど1回ずつ、Squareの全順序の昇順に返す。
    // lsb/msbは走査の両端、pop_lsb/pop_msbによる取り尽くしも同じ列を返す。
    #[test]
    fn iteration_yields_each_contained_square_exactly_once_in_order() {
        for set in sample_sets() {
            let squares: Vec<_> = set.iter().collect();
            assert_eq!(squares.len(), set.popcount() as usize);
            assert!(squares.iter().all(|&square| set.contains(square)));
            assert!(
                squares.windows(2).all(|pair| pair[0] < pair[1]),
                "昇順かつ重複なし"
            );
            assert_eq!(set.lsb(), squares.first().copied());
            assert_eq!(set.msb(), squares.last().copied());

            let mut ascending = set;
            let mut popped = Vec::new();
            while let Some(square) = ascending.pop_lsb() {
                popped.push(square);
            }
            assert!(ascending.is_empty());
            assert_eq!(popped, squares);

            let mut descending = set;
            let mut popped_back = Vec::new();
            while let Some(square) = descending.pop_msb() {
                popped_back.push(square);
            }
            popped_back.reverse();
            assert_eq!(popped_back, squares);

            let mut reversed: Vec<_> = set.iter().rev().collect();
            reversed.reverse();
            assert_eq!(reversed, squares);
        }
    }
}
