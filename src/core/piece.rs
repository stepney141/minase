pub const COLOR_COUNT: usize = 2;
pub const PIECE_KIND_COUNT: usize = 29;

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Color {
    Black = 0,
    White = 1,
}

impl Color {
    pub const ALL: [Self; COLOR_COUNT] = [Self::Black, Self::White];

    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[inline]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Black => Self::White,
            Self::White => Self::Black,
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PieceKind {
    Pawn = 0,
    GoBetween,
    Lance,
    ReverseChariot,
    SideMover,
    VerticalMover,
    Bishop,
    Rook,
    DragonHorse,
    DragonKing,
    FreeKing,
    King,
    DrunkElephant,
    FerociousLeopard,
    BlindTiger,
    CopperGeneral,
    SilverGeneral,
    GoldGeneral,
    Kirin,
    Phoenix,
    Lion,
    CrownPrince,
    WhiteHorse,
    Whale,
    FlyingOx,
    FreeBoar,
    FlyingStag,
    HornedFalcon,
    SoaringEagle,
}

impl PieceKind {
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

    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn from_index(index: u8) -> Option<Self> {
        if index < PIECE_KIND_COUNT as u8 {
            Some(Self::ALL[index as usize])
        } else {
            None
        }
    }

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

    #[inline]
    pub const fn can_promote(self) -> bool {
        self.promoted().is_some()
    }

    #[inline]
    pub const fn is_royal(self) -> bool {
        matches!(self, Self::King | Self::CrownPrince)
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PieceCode(u8);

impl PieceCode {
    pub const EMPTY: Self = Self(0);
    pub const WALL: Self = Self(0xff);

    const WHITE_BIT: u8 = 0x40;
    const PROMOTED_BIT: u8 = 0x20;
    const KIND_MASK: u8 = 0x1f;

    pub const fn new(color: Color, kind: PieceKind) -> Self {
        let color_bit = match color {
            Color::Black => 0,
            Color::White => Self::WHITE_BIT,
        };
        Self(color_bit | (kind as u8 + 1))
    }

    pub const fn new_promoted(color: Color, kind: PieceKind) -> Option<Self> {
        if kind.unpromoted().is_some() {
            Some(Self(Self::new(color, kind).0 | Self::PROMOTED_BIT))
        } else {
            None
        }
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == Self::EMPTY.0
    }

    #[inline]
    pub const fn is_wall(self) -> bool {
        self.0 == Self::WALL.0
    }

    #[inline]
    pub const fn is_promoted(self) -> bool {
        !self.is_empty() && !self.is_wall() && self.0 & Self::PROMOTED_BIT != 0
    }

    pub const fn color(self) -> Option<Color> {
        if self.is_empty() || self.is_wall() {
            None
        } else if self.0 & Self::WHITE_BIT == 0 {
            Some(Color::Black)
        } else {
            Some(Color::White)
        }
    }

    pub const fn kind(self) -> Option<PieceKind> {
        if self.is_empty() || self.is_wall() {
            None
        } else {
            PieceKind::from_index((self.0 & Self::KIND_MASK) - 1)
        }
    }

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
