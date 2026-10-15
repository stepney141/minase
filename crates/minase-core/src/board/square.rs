//! 12×12盤の升の座標表現。
//!
//! 段0は先手側の最下段であり、段が増える向きが先手の前方にあたる。
//! 内部の筋0はUSIの筋12に対応し、内部の筋が増えるとUSIの筋は減る。
//! 番兵を除く内部密番号は[`Square::dense_index`]で定義する。

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

    /// 内部密番号`rank * 12 + file`（`0..144`）を返す。
    /// 番兵を除いた配列の添字に使う。
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

    // 盤は縦12升・横12升の144升である(第4条)。
    // 升の列挙は重複なく網羅的で、有効な筋・段の組からだけ升を構築できる。
    #[test]
    fn article_4_1_board_enumerates_all_144_squares_exactly_once() {
        let mut seen = [[false; BOARD_RANKS as usize]; BOARD_FILES as usize];
        let mut count = 0;
        for square in Square::all() {
            let (file, rank) = (square.file(), square.rank());
            assert!(file < BOARD_FILES && rank < BOARD_RANKS);
            assert!(
                !seen[file as usize][rank as usize],
                "升の重複列挙: {square:?}"
            );
            seen[file as usize][rank as usize] = true;
            count += 1;
        }
        assert_eq!(count, BOARD_SQUARE_COUNT);

        // 有効な筋・段の全組から升を構築でき、元の筋・段を復元できる。
        for file in 0..BOARD_FILES {
            for rank in 0..BOARD_RANKS {
                let square = Square::new(file, rank).unwrap();
                assert_eq!((square.file(), square.rank()), (file, rank));
            }
        }

        // 盤外座標(内部0始まりでは筋12・段12以上に相当)からの構築は拒否される。
        for file in 0..=BOARD_FILES {
            assert!(Square::new(file, BOARD_RANKS).is_none());
        }
        for rank in 0..=BOARD_RANKS {
            assert!(Square::new(BOARD_FILES, rank).is_none());
        }
        assert!(Square::new(u8::MAX, 0).is_none());
        assert!(Square::new(0, u8::MAX).is_none());
    }

    // 実装上の契約として、升の符号往復は恒等で、相異なる升は相異なる符号を持ち、
    // 復号は符号化の像に一致する値の上でだけ成功する。具体的なビット割当は仮定しない。
    #[test]
    fn square_encodings_round_trip_and_reject_out_of_domain_values() {
        let mut dense_seen = [false; BOARD_SQUARE_COUNT];
        let mut raw_image = [false; 256];
        for square in Square::all() {
            assert_eq!(Square::from_dense(square.dense_index()), Some(square));
            assert_eq!(Square::from_raw(square.raw()), Some(square));
            assert_eq!(square.raw_index(), usize::from(square.raw()));
            assert!(!dense_seen[square.dense_index()], "密インデックスの衝突");
            dense_seen[square.dense_index()] = true;
            assert!(!raw_image[square.raw_index()], "生値の衝突");
            raw_image[square.raw_index()] = true;
        }

        // 密インデックスは0..144を過不足なく覆い、範囲外の復号は拒否される。
        assert!(dense_seen.iter().all(|&seen| seen));
        for dense in BOARD_SQUARE_COUNT..2 * BOARD_SQUARE_COUNT {
            assert!(Square::from_dense(dense).is_none());
        }

        // 生値の復号は、ちょうど符号化の像の上でだけ成功する(全単射性)。
        for raw in 0..=u8::MAX {
            assert_eq!(
                Square::from_raw(raw).is_some(),
                raw_image[usize::from(raw)],
                "raw={raw:#04x}"
            );
            if let Some(square) = Square::from_raw(raw) {
                assert_eq!(square.raw(), raw);
            }
        }
    }

    // 実装上の契約として、変位の適用は、移動後の筋・段がともに盤内に収まるとき、
    // そのときに限り成功し、反対側の筋・段への回り込みは決して起こらない。
    // 変位は駒の動きに現れる全種(8方向の1升・2升、桂馬型の跳び。第9条・第12条)を覆う。
    #[test]
    fn offset_succeeds_exactly_when_the_target_stays_on_the_board() {
        use crate::test_util::sq;

        // 四隅と各辺について、盤内へ進む例と盤外へ出る例を対にする。
        for (from, delta, expected) in [
            (sq(0, 0), (1, 1), Some(sq(1, 1))),
            (sq(0, 0), (-1, -1), None),
            (sq(11, 0), (-1, 1), Some(sq(10, 1))),
            (sq(11, 0), (1, -1), None),
            (sq(0, 11), (1, -1), Some(sq(1, 10))),
            (sq(0, 11), (-1, 1), None),
            (sq(11, 11), (-1, -1), Some(sq(10, 10))),
            (sq(11, 11), (1, 1), None),
            (sq(0, 5), (1, 0), Some(sq(1, 5))),
            (sq(0, 5), (-1, 0), None),
            (sq(11, 5), (-1, 0), Some(sq(10, 5))),
            (sq(11, 5), (1, 0), None),
            (sq(5, 0), (0, 1), Some(sq(5, 1))),
            (sq(5, 0), (0, -1), None),
            (sq(5, 11), (0, -1), Some(sq(5, 10))),
            (sq(5, 11), (0, 1), None),
            (sq(1, 5), (-2, 0), None),
            (sq(10, 5), (2, 0), None),
            (sq(5, 1), (0, -2), None),
            (sq(5, 10), (0, 2), None),
            (sq(5, 5), (2, -1), Some(sq(7, 4))),
            (sq(5, 5), (-1, 2), Some(sq(4, 7))),
        ] {
            assert_eq!(
                from.offset(delta.0, delta.1),
                expected,
                "{from:?}, {delta:?}"
            );
        }

        for square in Square::all() {
            for file_delta in -2_i8..=2 {
                for rank_delta in -2_i8..=2 {
                    if let Some(target) = square.offset(file_delta, rank_delta) {
                        assert_eq!(
                            i16::from(target.file()) - i16::from(square.file()),
                            i16::from(file_delta)
                        );
                        assert_eq!(
                            i16::from(target.rank()) - i16::from(square.rank()),
                            i16::from(rank_delta)
                        );
                        assert_eq!(target.offset(-file_delta, -rank_delta), Some(square));
                    }
                }
            }
        }
    }
}
