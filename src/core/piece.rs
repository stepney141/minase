//! 駒種・所有者・駒コードの表現。

/// 対局者の数(2)。
pub const COLOR_COUNT: usize = 2;
/// 成駒を含む駒種の総数(29)(第4条)。
pub const PIECE_KIND_COUNT: usize = 29;

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

/// 成駒を含む29種の駒種(第9条・第10条)。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PieceKind {
    /// 歩兵。
    Pawn = 0,
    /// 仲人。
    GoBetween,
    /// 香車。
    Lance,
    /// 反車。
    ReverseChariot,
    /// 横行。
    SideMover,
    /// 竪行。
    VerticalMover,
    /// 角行。
    Bishop,
    /// 飛車。
    Rook,
    /// 龍馬。
    DragonHorse,
    /// 龍王。
    DragonKing,
    /// 奔王。
    FreeKing,
    /// 王将・玉将。
    King,
    /// 醉象。
    DrunkElephant,
    /// 猛豹。
    FerociousLeopard,
    /// 盲虎。
    BlindTiger,
    /// 銅将。
    CopperGeneral,
    /// 銀将。
    SilverGeneral,
    /// 金将。
    GoldGeneral,
    /// 麒麟。
    Kirin,
    /// 鳳凰。
    Phoenix,
    /// 獅子。
    Lion,
    /// 太子(醉象の成駒)。
    CrownPrince,
    /// 白駒(香車の成駒)。
    WhiteHorse,
    /// 鯨鯢(反車の成駒)。
    Whale,
    /// 飛牛(竪行の成駒)。
    FlyingOx,
    /// 奔猪(横行の成駒)。
    FreeBoar,
    /// 飛鹿(盲虎の成駒)。
    FlyingStag,
    /// 角鷹(龍馬の成駒)。
    HornedFalcon,
    /// 飛鷲(龍王の成駒)。
    SoaringEagle,
}

impl PieceKind {
    /// 全駒種。
    pub const ALL: [Self; PIECE_KIND_COUNT] = [
        Self::Pawn,
        Self::GoBetween,
        Self::Lance,
        Self::ReverseChariot,
        Self::SideMover,
        Self::VerticalMover,
        Self::Bishop,
        Self::Rook,
        Self::DragonHorse,
        Self::DragonKing,
        Self::FreeKing,
        Self::King,
        Self::DrunkElephant,
        Self::FerociousLeopard,
        Self::BlindTiger,
        Self::CopperGeneral,
        Self::SilverGeneral,
        Self::GoldGeneral,
        Self::Kirin,
        Self::Phoenix,
        Self::Lion,
        Self::CrownPrince,
        Self::WhiteHorse,
        Self::Whale,
        Self::FlyingOx,
        Self::FreeBoar,
        Self::FlyingStag,
        Self::HornedFalcon,
        Self::SoaringEagle,
    ];

    /// 配列添字用の番号を返す。
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// 番号から駒種を返す。範囲外なら`None`を返す。
    pub const fn from_index(index: u8) -> Option<Self> {
        if index < PIECE_KIND_COUNT as u8 {
            Some(Self::ALL[index as usize])
        } else {
            None
        }
    }

    /// 成った後の駒種を返す(第9条)。成れない駒種なら`None`を返す。
    pub const fn promoted(self) -> Option<Self> {
        match self {
            Self::Pawn => Some(Self::GoldGeneral),
            Self::GoBetween => Some(Self::DrunkElephant),
            Self::Lance => Some(Self::WhiteHorse),
            Self::ReverseChariot => Some(Self::Whale),
            Self::SideMover => Some(Self::FreeBoar),
            Self::VerticalMover => Some(Self::FlyingOx),
            Self::Bishop => Some(Self::DragonHorse),
            Self::Rook => Some(Self::DragonKing),
            Self::DragonHorse => Some(Self::HornedFalcon),
            Self::DragonKing => Some(Self::SoaringEagle),
            Self::DrunkElephant => Some(Self::CrownPrince),
            Self::FerociousLeopard => Some(Self::Bishop),
            Self::BlindTiger => Some(Self::FlyingStag),
            Self::CopperGeneral => Some(Self::SideMover),
            Self::SilverGeneral => Some(Self::VerticalMover),
            Self::GoldGeneral => Some(Self::Rook),
            Self::Kirin => Some(Self::Lion),
            Self::Phoenix => Some(Self::FreeKing),
            Self::FreeKing
            | Self::King
            | Self::Lion
            | Self::CrownPrince
            | Self::WhiteHorse
            | Self::Whale
            | Self::FlyingOx
            | Self::FreeBoar
            | Self::FlyingStag
            | Self::HornedFalcon
            | Self::SoaringEagle => None,
        }
    }

    /// 成る前の駒種を返す。成駒として現れない駒種なら`None`を返す。
    pub const fn unpromoted(self) -> Option<Self> {
        match self {
            Self::GoldGeneral => Some(Self::Pawn),
            Self::DrunkElephant => Some(Self::GoBetween),
            Self::WhiteHorse => Some(Self::Lance),
            Self::Whale => Some(Self::ReverseChariot),
            Self::FreeBoar => Some(Self::SideMover),
            Self::FlyingOx => Some(Self::VerticalMover),
            Self::DragonHorse => Some(Self::Bishop),
            Self::DragonKing => Some(Self::Rook),
            Self::HornedFalcon => Some(Self::DragonHorse),
            Self::SoaringEagle => Some(Self::DragonKing),
            Self::CrownPrince => Some(Self::DrunkElephant),
            Self::Bishop => Some(Self::FerociousLeopard),
            Self::FlyingStag => Some(Self::BlindTiger),
            Self::SideMover => Some(Self::CopperGeneral),
            Self::VerticalMover => Some(Self::SilverGeneral),
            Self::Rook => Some(Self::GoldGeneral),
            Self::Lion => Some(Self::Kirin),
            Self::FreeKing => Some(Self::Phoenix),
            Self::Pawn
            | Self::GoBetween
            | Self::Lance
            | Self::ReverseChariot
            | Self::King
            | Self::FerociousLeopard
            | Self::BlindTiger
            | Self::CopperGeneral
            | Self::SilverGeneral
            | Self::Kirin
            | Self::Phoenix => None,
        }
    }

    /// 成れる駒種かどうかを返す。
    #[inline]
    pub const fn can_promote(self) -> bool {
        self.promoted().is_some()
    }

    /// 王駒(王将・玉将・太子)かどうかを返す(第3条)。
    #[inline]
    pub const fn is_royal(self) -> bool {
        matches!(self, Self::King | Self::CrownPrince)
    }
}

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
    pub const fn new(color: Color, kind: PieceKind) -> Self {
        let color_bit = match color {
            Color::Black => 0,
            Color::White => Self::WHITE_BIT,
        };
        Self(color_bit | (kind as u8 + 1))
    }

    /// 指定駒種を成駒として持つコードを作る。その駒種が成駒として現れないなら`None`を返す。
    pub const fn new_promoted(color: Color, kind: PieceKind) -> Option<Self> {
        if kind.unpromoted().is_some() {
            Some(Self(Self::new(color, kind).0 | Self::PROMOTED_BIT))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_piece_code_round_trips() {
        for color in Color::ALL {
            for kind in PieceKind::ALL {
                let piece = PieceCode::new(color, kind);
                assert_eq!(piece.color(), Some(color));
                assert_eq!(piece.kind(), Some(kind));
                assert!(!piece.is_empty());
                assert!(!piece.is_wall());
                assert!(!piece.is_promoted());
            }
        }
    }

    #[test]
    fn promotion_pairs_are_mutual() {
        for kind in PieceKind::ALL {
            if let Some(promoted) = kind.promoted() {
                assert_eq!(promoted.unpromoted(), Some(kind));
            }
        }
    }

    #[test]
    fn promoted_piece_cannot_promote_again() {
        let pawn = PieceCode::new(Color::Black, PieceKind::Pawn);
        let tokin = pawn.promote().unwrap();
        assert_eq!(tokin.kind(), Some(PieceKind::GoldGeneral));
        assert!(tokin.is_promoted());
        assert!(tokin.promote().is_none());

        let gold = PieceCode::new(Color::Black, PieceKind::GoldGeneral);
        assert_eq!(gold.promote().unwrap().kind(), Some(PieceKind::Rook));
    }
}
