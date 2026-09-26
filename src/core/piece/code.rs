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

    /// 後手の駒を示すビット。
    const WHITE_BIT: u8 = 0x40;
    /// 成駒を示すビット。
    const PROMOTED_BIT: u8 = 0x20;
    /// 駒種番号を取り出すマスク。
    const KIND_MASK: u8 = 0x1f;

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

    /// 駒種を返す。空升・番兵なら`None`を返す。
    pub const fn kind(self) -> Option<PieceKind> {
        if self.is_empty() || self.is_wall() {
            None
        } else {
            PieceKind::from_index((self.0 & Self::KIND_MASK) - 1)
        }
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
