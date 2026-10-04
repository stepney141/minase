//! 駒の所有者と対局者の数。

/// 対局者の数(2)。
pub const COLOR_COUNT: usize = 2;

/// 駒の所有者。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Color {
    /// 先手。
    Black = 0,
    /// 後手。
    White = 1,
}

impl Color {
    /// 両対局者。
    pub const ALL: [Self; COLOR_COUNT] = [Self::Black, Self::White];

    /// 配列添字用の番号を返す。
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// 相手側を返す。
    #[inline]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Black => Self::White,
            Self::White => Self::Black,
        }
    }
}
