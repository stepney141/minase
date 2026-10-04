//! 駒種と成りの対応。

/// 成駒を含む駒種の総数(29)(第4条)。
pub const PIECE_KIND_COUNT: usize = 29;

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

    /// 成っていない状態で盤上に存在できる駒種かどうかを返す。
    pub(super) const fn can_exist_unpromoted(self) -> bool {
        !matches!(
            self,
            Self::CrownPrince
                | Self::WhiteHorse
                | Self::Whale
                | Self::FlyingOx
                | Self::FreeBoar
                | Self::FlyingStag
                | Self::HornedFalcon
                | Self::SoaringEagle
        )
    }
}
