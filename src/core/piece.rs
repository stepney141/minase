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

    /// 成っていない状態で盤上に存在できる駒種かどうかを返す。
    const fn can_exist_unpromoted(self) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 初期配置で使用する21種の駒種(第4条・第5条)。王将・玉将は性能同一の
    /// 1種として数える(第5条、SPEC_UNCLEAR SU-D4-3)。
    const INITIAL_KINDS: [PieceKind; 21] = [
        PieceKind::Pawn,
        PieceKind::GoBetween,
        PieceKind::Lance,
        PieceKind::ReverseChariot,
        PieceKind::SideMover,
        PieceKind::VerticalMover,
        PieceKind::Bishop,
        PieceKind::Rook,
        PieceKind::DragonHorse,
        PieceKind::DragonKing,
        PieceKind::FreeKing,
        PieceKind::King,
        PieceKind::DrunkElephant,
        PieceKind::FerociousLeopard,
        PieceKind::BlindTiger,
        PieceKind::CopperGeneral,
        PieceKind::SilverGeneral,
        PieceKind::GoldGeneral,
        PieceKind::Kirin,
        PieceKind::Phoenix,
        PieceKind::Lion,
    ];

    /// 成駒としてのみ現れる8種の駒種(第10条)。
    const PROMOTED_ONLY_KINDS: [PieceKind; 8] = [
        PieceKind::WhiteHorse,
        PieceKind::Whale,
        PieceKind::FlyingStag,
        PieceKind::FreeBoar,
        PieceKind::FlyingOx,
        PieceKind::CrownPrince,
        PieceKind::HornedFalcon,
        PieceKind::SoaringEagle,
    ];

    // 駒種は初期21種＋成駒のみ8種の29種である(第4条3項・第10条、D4-004-03)。
    #[test]
    fn article_4_3_piece_kinds_are_21_initial_plus_8_promoted_only() {
        assert_eq!(PieceKind::ALL.len(), PIECE_KIND_COUNT);
        assert_eq!(INITIAL_KINDS.len() + PROMOTED_ONLY_KINDS.len(), 29);

        // 21種と8種は互いに素で、全29種をちょうど覆う。
        for kind in PieceKind::ALL {
            let initial = INITIAL_KINDS.contains(&kind);
            let promoted_only = PROMOTED_ONLY_KINDS.contains(&kind);
            assert!(
                initial != promoted_only,
                "初期駒と成駒専用のどちらか一方に属する: {kind:?}"
            );
        }
        // ALLに重複はない。
        for (index, kind) in PieceKind::ALL.iter().enumerate() {
            assert!(
                !PieceKind::ALL[..index].contains(kind),
                "駒種の重複: {kind:?}"
            );
        }

        // 成りによって新たに現れる駒種は、ちょうど第10条の8種である(D4-004-03性質)。
        let mut newly_appearing: Vec<PieceKind> = PieceKind::ALL
            .iter()
            .filter_map(|kind| kind.promoted())
            .filter(|target| !INITIAL_KINDS.contains(target))
            .collect();
        newly_appearing.sort_by_key(|kind| kind.index());
        newly_appearing.dedup();
        let mut expected = PROMOTED_ONLY_KINDS.to_vec();
        expected.sort_by_key(|kind| kind.index());
        assert_eq!(newly_appearing, expected);
    }

    // 実装契約(D4-IMP-06): 成り対応は第9条の表とちょうど一致し、成る→戻すが恒等である。
    // 成れない駒種(第17条1項)と成駒の再成り禁止(第17条4項)も表から確定する。
    #[test]
    fn promotion_pairs_match_article_9_table_and_demotion_is_inverse() {
        // 第9条の成駒対応表の写し(成れる18種)。
        let pairs = [
            (PieceKind::GoldGeneral, PieceKind::Rook),
            (PieceKind::SilverGeneral, PieceKind::VerticalMover),
            (PieceKind::CopperGeneral, PieceKind::SideMover),
            (PieceKind::FerociousLeopard, PieceKind::Bishop),
            (PieceKind::BlindTiger, PieceKind::FlyingStag),
            (PieceKind::DrunkElephant, PieceKind::CrownPrince),
            (PieceKind::Pawn, PieceKind::GoldGeneral),
            (PieceKind::GoBetween, PieceKind::DrunkElephant),
            (PieceKind::Lance, PieceKind::WhiteHorse),
            (PieceKind::ReverseChariot, PieceKind::Whale),
            (PieceKind::SideMover, PieceKind::FreeBoar),
            (PieceKind::VerticalMover, PieceKind::FlyingOx),
            (PieceKind::Bishop, PieceKind::DragonHorse),
            (PieceKind::Rook, PieceKind::DragonKing),
            (PieceKind::DragonHorse, PieceKind::HornedFalcon),
            (PieceKind::DragonKing, PieceKind::SoaringEagle),
            (PieceKind::Kirin, PieceKind::Lion),
            (PieceKind::Phoenix, PieceKind::FreeKing),
        ];

        for (base, target) in pairs {
            assert_eq!(base.promoted(), Some(target), "{base:?}の成り先");
            // demote∘promoteは恒等である。
            assert_eq!(target.unpromoted(), Some(base), "{target:?}の成り元");
            assert!(base.can_promote());
        }

        // 表にない駒種は成り先を持たない: 王将・玉将・獅子・奔王(第17条1項)と、
        // 成駒としてのみ現れる8種(第17条4項・第10条)。
        for kind in PieceKind::ALL {
            let in_table = pairs.iter().any(|&(base, _)| base == kind);
            assert_eq!(kind.promoted().is_some(), in_table, "{kind:?}");
            assert_eq!(kind.can_promote(), in_table);
            // 成駒として現れる駒種は、ちょうど表の右列に現れる駒種である。
            let is_target = pairs.iter().any(|&(_, target)| target == kind);
            assert_eq!(kind.unpromoted().is_some(), is_target, "{kind:?}");
        }

        for color in Color::ALL {
            for (base, target) in pairs {
                // 駒コードでの成りも表と一致し、所有者を保存する。
                let promoted = PieceCode::new(color, base).unwrap().promote().unwrap();
                assert_eq!(promoted.kind(), Some(target));
                assert_eq!(promoted.color(), Some(color));
                assert!(promoted.is_promoted());
                // 成った駒はさらに成れない(第17条4項)。仲人の成駒(醉象の動き)が
                // 太子へ成れないことも、この一般則に含まれる。
                assert!(promoted.promote().is_none());
            }
            for kind in [PieceKind::King, PieceKind::Lion, PieceKind::FreeKing] {
                assert!(PieceCode::new(color, kind).unwrap().promote().is_none());
            }
            // 成駒コードを構築できるのは、成駒として現れる駒種にちょうど限られる。
            for kind in PieceKind::ALL {
                assert_eq!(
                    PieceCode::new_promoted(color, kind).is_some(),
                    kind.unpromoted().is_some()
                );
            }
        }
    }

    // 実装契約(D4-IMP-05): 駒コードの符号化は(所有者, 駒種, 成否)の上で単射で、
    // 復号との往復は恒等である。空升・番兵はどの駒コードとも区別される。
    // 生の金将と歩兵の成駒のような動き同一の対も、成否によって区別される(第24条1項a)。
    #[test]
    fn piece_codes_are_injective_over_owner_kind_and_promotion() {
        let mut codes = Vec::new();
        for color in Color::ALL {
            for kind in PieceKind::ALL {
                let Some(raw) = PieceCode::new(color, kind) else {
                    assert!(!kind.can_exist_unpromoted());
                    continue;
                };
                assert_eq!(raw.color(), Some(color));
                assert_eq!(raw.kind(), Some(kind));
                assert!(!raw.is_promoted());
                codes.push(raw);
                if let Some(promoted) = PieceCode::new_promoted(color, kind) {
                    assert_eq!(promoted.color(), Some(color));
                    assert_eq!(promoted.kind(), Some(kind));
                    assert!(promoted.is_promoted());
                    codes.push(promoted);
                }
            }
        }

        // 相異なる(所有者, 駒種, 成否)は相異なる符号を持つ。
        for (index, code) in codes.iter().enumerate() {
            assert!(!codes[..index].contains(code), "駒コードの衝突: {code:?}");
            assert!(!code.is_empty());
            assert!(!code.is_wall());
            assert_ne!(*code, PieceCode::EMPTY);
            assert_ne!(*code, PieceCode::WALL);
        }

        // 空升・番兵は駒として復号されない。
        for sentinel in [PieceCode::EMPTY, PieceCode::WALL] {
            assert_eq!(sentinel.color(), None);
            assert_eq!(sentinel.kind(), None);
            assert!(!sentinel.is_promoted());
        }
        assert!(PieceCode::EMPTY.is_empty());
        assert!(PieceCode::WALL.is_wall());
        assert_ne!(PieceCode::EMPTY, PieceCode::WALL);
    }

    // 第10条。成駒専用8種は未成の盤上駒コードとして構築できない。
    #[test]
    fn promoted_only_kinds_have_no_unpromoted_piece_code() {
        for color in Color::ALL {
            for kind in PROMOTED_ONLY_KINDS {
                assert_eq!(PieceCode::new(color, kind), None, "{color:?} {kind:?}");
                assert!(!kind.can_exist_unpromoted());
            }
            for kind in INITIAL_KINDS {
                assert!(PieceCode::new(color, kind).is_some(), "{color:?} {kind:?}");
                assert!(kind.can_exist_unpromoted());
            }
        }
    }
}
