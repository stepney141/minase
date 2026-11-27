//! 盤の升に格納する駒コード。

use super::color::Color;
use super::kind::PieceKind;

/// 盤の1升に格納する駒コード。下位5ビットが駒種番号+1、`0x20`が成りフラグ、
/// `0x40`が後手フラグを表す。`0x00`は空升、`0xff`は盤外の番兵を表す。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PieceCode(u8);

impl PieceCode {
    /// 空升を表すコード。
    pub const EMPTY: Self = Self(0);
    /// 盤外の番兵を表すコード。
    pub const WALL: Self = Self(0xff);

    /// 駒コードを表の添字（0〜255）として返す。
    ///
    /// 添字の値やビットの割り当ては安定した契約ではない。
    #[inline]
    pub const fn table_index(self) -> usize {
        self.0 as usize
    }

    /// 後手の駒を示すビット。
    const WHITE_BIT: u8 = 0x40;
    /// 成駒を示すビット。
    const PROMOTED_BIT: u8 = 0x20;
    /// 駒種番号を取り出すマスク。
    const KIND_MASK: u8 = 0x1f;

    const KIND_BY_CODE: [Option<PieceKind>; 256] = Self::build_kind_table();

    const fn build_kind_table() -> [Option<PieceKind>; 256] {
        let mut table = [None; 256];
        let mut index = 0;
        while index < table.len() {
            let code = Self(index as u8);
            let number = code.0 & Self::KIND_MASK;
            if !code.is_empty() && !code.is_wall() && number != 0 {
                table[index] = PieceKind::from_index(number - 1);
            }
            index += 1;
        }
        table
    }

    /// 所有者と駒種から成っていない駒のコードを作る。
    ///
    /// 成駒としてのみ現れる駒種なら`None`を返す。
    pub const fn new(color: Color, kind: PieceKind) -> Option<Self> {
        if !kind.can_exist_unpromoted() {
            return None;
        }
        Some(Self::from_parts(color, kind, false))
    }

    /// 検証済みの所有者、駒種および成り状態をコード化する。
    const fn from_parts(color: Color, kind: PieceKind, promoted: bool) -> Self {
        let color_bit = match color {
            Color::Black => 0,
            Color::White => Self::WHITE_BIT,
        };
        let promoted_bit = if promoted { Self::PROMOTED_BIT } else { 0 };
        Self(color_bit | promoted_bit | (kind as u8 + 1))
    }

    /// 指定駒種を成駒として持つコードを作る。その駒種が成駒として現れないなら`None`を返す。
    pub const fn new_promoted(color: Color, kind: PieceKind) -> Option<Self> {
        if kind.unpromoted().is_some() {
            Some(Self::from_parts(color, kind, true))
        } else {
            None
        }
    }

    /// 空升かどうかを返す。
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == Self::EMPTY.0
    }

    /// 盤外の番兵かどうかを返す。
    #[inline]
    pub const fn is_wall(self) -> bool {
        self.0 == Self::WALL.0
    }

    /// 成駒かどうかを返す。
    #[inline]
    pub const fn is_promoted(self) -> bool {
        !self.is_empty() && !self.is_wall() && self.0 & Self::PROMOTED_BIT != 0
    }

    /// 駒の所有者を返す。空升・番兵なら`None`を返す。
    pub const fn color(self) -> Option<Color> {
        if self.is_empty() || self.is_wall() {
            None
        } else if self.0 & Self::WHITE_BIT == 0 {
            Some(Color::Black)
        } else {
            Some(Color::White)
        }
    }

    /// 駒種を定数表から引く。空升・番兵、または駒種番号が1〜29の範囲外なら`None`を返す。
    /// 構築子で作れないコードでも、駒種番号が範囲内なら対応する駒種を返す。
    pub const fn kind(self) -> Option<PieceKind> {
        Self::KIND_BY_CODE[self.table_index()]
    }

    /// 成った後の駒コードを返す。成れない駒なら`None`を返す(第17条)。
    pub const fn promote(self) -> Option<Self> {
        if self.is_promoted() {
            return None;
        }
        let Some(color) = self.color() else {
            return None;
        };
        let Some(kind) = self.kind() else {
            return None;
        };
        let Some(promoted_kind) = kind.promoted() else {
            return None;
        };
        Self::new_promoted(color, promoted_kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_decodes_every_byte() {
        for byte in 0..=u8::MAX {
            let expected = match (byte, byte & 0x1f) {
                (0x00 | 0xff, _) => None,
                (_, number @ 1..=29) => Some(PieceKind::ALL[usize::from(number) - 1]),
                _ => None,
            };
            assert_eq!(PieceCode(byte).kind(), expected, "code {byte:#04x}");
        }
    }

    #[test]
    fn kind_round_trips_constructible_codes() {
        assert_eq!(PieceCode::EMPTY.kind(), None);
        assert_eq!(PieceCode::WALL.kind(), None);
        for color in Color::ALL {
            for kind in PieceKind::ALL {
                if let Some(code) = PieceCode::new(color, kind) {
                    assert_eq!(code.kind(), Some(kind), "{color:?} {kind:?}");
                }
                if let Some(code) = PieceCode::new_promoted(color, kind) {
                    assert_eq!(code.kind(), Some(kind), "{color:?} promoted {kind:?}");
                }
            }
        }
    }
}
