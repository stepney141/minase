//! 駒種ごとの動き(固定利き・走り・特殊移動)の定義テーブル。

use crate::core::direction::Direction;
use crate::core::piece::{Color, PieceKind};

/// 動きプロファイルの識別子。王将と太子のように動きが同じ駒種は1つのプロファイルを共有する。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct MovementProfileId(u8);

impl MovementProfileId {
    /// 配列添字用の番号を返す。
    #[inline]
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }

    /// 番号から識別子を作る。
    const fn from_index(index: usize) -> Self {
        debug_assert!(index < MOVEMENT_PROFILE_COUNT);
        Self(index as u8)
    }
}

/// 駒の所有者から見た1歩分の相対移動量。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct RelativeDelta {
    /// 筋の増分(所有者から見て右が正)。
    file: i8,
    /// 段の増分(所有者から見て前が正)。
    rank: i8,
}

impl RelativeDelta {
    /// 所有者に応じた絶対座標の増分へ変換して返す。後手は180度回転する。
    pub(crate) const fn for_color(self, color: Color) -> (i8, i8) {
        match color {
            Color::Black => (self.file, self.rank),
            Color::White => (-self.file, -self.rank),
        }
    }
}

/// 駒の所有者から見た相対8方向。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum RelativeDirection {
    /// 前。
    Forward,
    /// 後。
    Backward,
    /// 右。
    Right,
    /// 左。
    Left,
    /// 右前。
    ForwardRight,
    /// 左前。
    ForwardLeft,
    /// 右後。
    BackwardRight,
    /// 左後。
    BackwardLeft,
}

impl RelativeDirection {
    #[cfg(test)]
    const ALL: [Self; 8] = [
        Self::Forward,
        Self::Backward,
        Self::Right,
        Self::Left,
        Self::ForwardRight,
        Self::ForwardLeft,
        Self::BackwardRight,
        Self::BackwardLeft,
    ];

    /// 所有者に応じた絶対方向へ変換して返す。後手は180度回転する。
    pub(crate) const fn for_color(self, color: Color) -> Direction {
        match (self, color) {
            (Self::Forward, Color::Black) | (Self::Backward, Color::White) => Direction::North,
            (Self::Backward, Color::Black) | (Self::Forward, Color::White) => Direction::South,
            (Self::Right, Color::Black) | (Self::Left, Color::White) => Direction::East,
            (Self::Left, Color::Black) | (Self::Right, Color::White) => Direction::West,
            (Self::ForwardRight, Color::Black) | (Self::BackwardLeft, Color::White) => {
                Direction::NorthEast
            }
            (Self::ForwardLeft, Color::Black) | (Self::BackwardRight, Color::White) => {
                Direction::NorthWest
            }
            (Self::BackwardRight, Color::Black) | (Self::ForwardLeft, Color::White) => {
                Direction::SouthEast
            }
            (Self::BackwardLeft, Color::Black) | (Self::ForwardRight, Color::White) => {
                Direction::SouthWest
            }
        }
    }
}

/// 走り1本分の指定。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct SlideSpec {
    /// 走る方向。
    pub(crate) direction: RelativeDirection,
    /// 進める最大升数。`None`は盤端まで無制限。
    pub(crate) max_steps: Option<u8>,
}

/// 角鷹・飛鷲が2段階移動(第11条)を行える方向の集合。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct LionLikeProfile {
    /// 2段階移動できる相対方向。
    pub(crate) directions: &'static [RelativeDirection],
}

/// 固定利きと走りだけでは表せない特殊な動き。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum SpecialMovement {
    /// 特殊な動きなし。
    None,
    /// 獅子の2段階移動と跳び(第12条)。
    Lion,
    /// 角鷹・飛鷲の限定方向の2段階移動(第11条)。
    LionLike(LionLikeProfile),
}

/// 1駒種分の動きの定義。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct MovementProfile {
    /// 1升移動と跳びの移動量。
    pub(crate) fixed_deltas: &'static [RelativeDelta],
    /// 走りの指定。
    pub(crate) slides: &'static [SlideSpec],
    /// 特殊な動き。
    pub(crate) special: SpecialMovement,
}

/// [`RelativeDelta`]を作る。
const fn delta(file: i8, rank: i8) -> RelativeDelta {
    RelativeDelta { file, rank }
}

/// 盤端まで無制限の走り指定を作る。
const fn slide(direction: RelativeDirection) -> SlideSpec {
    SlideSpec {
        direction,
        max_steps: None,
    }
}

/// 固定利きなし。
const EMPTY_DELTAS: &[RelativeDelta] = &[];
/// 走りなし。
const EMPTY_SLIDES: &[SlideSpec] = &[];
/// 歩兵: 前へ1升。
const PAWN: &[RelativeDelta] = &[delta(0, 1)];
/// 仲人: 前後へ1升。
const GO_BETWEEN: &[RelativeDelta] = &[delta(0, 1), delta(0, -1)];
/// 縦横4方向へ1升。
const ORTHOGONAL_STEPS: &[RelativeDelta] = &[delta(0, 1), delta(1, 0), delta(-1, 0), delta(0, -1)];
/// 斜め4方向へ1升。
const DIAGONAL_STEPS: &[RelativeDelta] = &[delta(1, 1), delta(-1, 1), delta(1, -1), delta(-1, -1)];
/// 王将・玉将: 周囲8方向へ1升。
const KING_STEPS: &[RelativeDelta] = &[
    delta(0, 1),
    delta(1, 1),
    delta(-1, 1),
    delta(1, 0),
    delta(-1, 0),
    delta(0, -1),
    delta(1, -1),
    delta(-1, -1),
];
/// 醉象: 後以外の7方向へ1升。
const DRUNK_ELEPHANT: &[RelativeDelta] = &[
    delta(0, 1),
    delta(1, 1),
    delta(-1, 1),
    delta(1, 0),
    delta(-1, 0),
    delta(1, -1),
    delta(-1, -1),
];
/// 猛豹: 前後および斜め4方向へ1升。
const FEROCIOUS_LEOPARD: &[RelativeDelta] = &[
    delta(0, 1),
    delta(1, 1),
    delta(-1, 1),
    delta(0, -1),
    delta(1, -1),
    delta(-1, -1),
];
/// 盲虎: 前以外の7方向へ1升。
const BLIND_TIGER: &[RelativeDelta] = &[
    delta(1, 1),
    delta(-1, 1),
    delta(1, 0),
    delta(-1, 0),
    delta(0, -1),
    delta(1, -1),
    delta(-1, -1),
];
/// 銅将: 前、前斜めおよび後へ1升。
const COPPER_GENERAL: &[RelativeDelta] = &[delta(0, 1), delta(1, 1), delta(-1, 1), delta(0, -1)];
/// 銀将: 前および斜め4方向へ1升。
const SILVER_GENERAL: &[RelativeDelta] = &[
    delta(0, 1),
    delta(1, 1),
    delta(-1, 1),
    delta(1, -1),
    delta(-1, -1),
];
/// 金将: 前、前斜め、左右および後へ1升。
const GOLD_GENERAL: &[RelativeDelta] = &[
    delta(0, 1),
    delta(1, 1),
    delta(-1, 1),
    delta(1, 0),
    delta(-1, 0),
    delta(0, -1),
];
/// 麒麟: 斜めへ1升、縦横へ2升跳ぶ。
const KIRIN: &[RelativeDelta] = &[
    delta(1, 1),
    delta(-1, 1),
    delta(1, -1),
    delta(-1, -1),
    delta(0, 2),
    delta(2, 0),
    delta(-2, 0),
    delta(0, -2),
];
/// 鳳凰: 縦横へ1升、斜めへ2升跳ぶ。
const PHOENIX: &[RelativeDelta] = &[
    delta(0, 1),
    delta(1, 0),
    delta(-1, 0),
    delta(0, -1),
    delta(2, 2),
    delta(-2, 2),
    delta(2, -2),
    delta(-2, -2),
];
/// 飛鹿の1升移動部分: 左右および斜め4方向。前後の走りは別に定義する。
const FLYING_STAG_STEPS: &[RelativeDelta] = &[
    delta(1, 1),
    delta(-1, 1),
    delta(1, 0),
    delta(-1, 0),
    delta(1, -1),
    delta(-1, -1),
];

/// 前への走り(香車)。
const FORWARD: &[SlideSpec] = &[slide(RelativeDirection::Forward)];
/// 前後への走り(反車ほか)。
const VERTICAL: &[SlideSpec] = &[
    slide(RelativeDirection::Forward),
    slide(RelativeDirection::Backward),
];
/// 左右への走り(横行ほか)。
const HORIZONTAL: &[SlideSpec] = &[
    slide(RelativeDirection::Right),
    slide(RelativeDirection::Left),
];
/// 斜め4方向への走り(角行ほか)。
const DIAGONAL: &[SlideSpec] = &[
    slide(RelativeDirection::ForwardRight),
    slide(RelativeDirection::ForwardLeft),
    slide(RelativeDirection::BackwardRight),
    slide(RelativeDirection::BackwardLeft),
];
/// 縦横4方向への走り(飛車ほか)。
const ORTHOGONAL: &[SlideSpec] = &[
    slide(RelativeDirection::Forward),
    slide(RelativeDirection::Backward),
    slide(RelativeDirection::Right),
    slide(RelativeDirection::Left),
];
/// 8方向への走り(奔王)。
const ALL_DIRECTIONS: &[SlideSpec] = &[
    slide(RelativeDirection::Forward),
    slide(RelativeDirection::Backward),
    slide(RelativeDirection::Right),
    slide(RelativeDirection::Left),
    slide(RelativeDirection::ForwardRight),
    slide(RelativeDirection::ForwardLeft),
    slide(RelativeDirection::BackwardRight),
    slide(RelativeDirection::BackwardLeft),
];
/// 白駒: 前、前斜めおよび後への走り。
const WHITE_HORSE: &[SlideSpec] = &[
    slide(RelativeDirection::Forward),
    slide(RelativeDirection::ForwardRight),
    slide(RelativeDirection::ForwardLeft),
    slide(RelativeDirection::Backward),
];
/// 鯨鯢: 前、後および後斜めへの走り。
const WHALE: &[SlideSpec] = &[
    slide(RelativeDirection::Forward),
    slide(RelativeDirection::Backward),
    slide(RelativeDirection::BackwardRight),
    slide(RelativeDirection::BackwardLeft),
];
/// 飛牛: 前後および斜め4方向への走り。
const FLYING_OX: &[SlideSpec] = &[
    slide(RelativeDirection::Forward),
    slide(RelativeDirection::Backward),
    slide(RelativeDirection::ForwardRight),
    slide(RelativeDirection::ForwardLeft),
    slide(RelativeDirection::BackwardRight),
    slide(RelativeDirection::BackwardLeft),
];
/// 奔猪: 左右および斜め4方向への走り。
const FREE_BOAR: &[SlideSpec] = &[
    slide(RelativeDirection::Right),
    slide(RelativeDirection::Left),
    slide(RelativeDirection::ForwardRight),
    slide(RelativeDirection::ForwardLeft),
    slide(RelativeDirection::BackwardRight),
    slide(RelativeDirection::BackwardLeft),
];
/// 角鷹の走り: 前以外の7方向。前方は2段階移動で扱う。
const HORNED_FALCON_SLIDES: &[SlideSpec] = &[
    slide(RelativeDirection::Backward),
    slide(RelativeDirection::Right),
    slide(RelativeDirection::Left),
    slide(RelativeDirection::ForwardRight),
    slide(RelativeDirection::ForwardLeft),
    slide(RelativeDirection::BackwardRight),
    slide(RelativeDirection::BackwardLeft),
];
/// 飛鷲の走り: 前斜め以外の6方向。前斜めは2段階移動で扱う。
const SOARING_EAGLE_SLIDES: &[SlideSpec] = &[
    slide(RelativeDirection::Forward),
    slide(RelativeDirection::Backward),
    slide(RelativeDirection::Right),
    slide(RelativeDirection::Left),
    slide(RelativeDirection::BackwardRight),
    slide(RelativeDirection::BackwardLeft),
];
/// 角鷹が2段階移動できる方向(前方、第11条第1項)。
const HORNED_FALCON_LION: &[RelativeDirection] = &[RelativeDirection::Forward];
/// 飛鷲が2段階移動できる方向(左右の前斜め、第11条第2項)。
const SOARING_EAGLE_LION: &[RelativeDirection] = &[
    RelativeDirection::ForwardRight,
    RelativeDirection::ForwardLeft,
];

/// [`MovementProfile`]を作る。
const fn profile(
    fixed_deltas: &'static [RelativeDelta],
    slides: &'static [SlideSpec],
    special: SpecialMovement,
) -> MovementProfile {
    MovementProfile {
        fixed_deltas,
        slides,
        special,
    }
}

/// プロファイルの総数。王将と太子が1つを共有するため、駒種数29より1少ない。
pub(crate) const MOVEMENT_PROFILE_COUNT: usize = 28;

/// 全プロファイルの実体。並び順は[`movement_profile`]が返す番号と一致する。
const PROFILES: [MovementProfile; MOVEMENT_PROFILE_COUNT] = [
    profile(PAWN, EMPTY_SLIDES, SpecialMovement::None),
    profile(GO_BETWEEN, EMPTY_SLIDES, SpecialMovement::None),
    profile(EMPTY_DELTAS, FORWARD, SpecialMovement::None),
    profile(EMPTY_DELTAS, VERTICAL, SpecialMovement::None),
    profile(GO_BETWEEN, HORIZONTAL, SpecialMovement::None),
    profile(
        &[delta(1, 0), delta(-1, 0)],
        VERTICAL,
        SpecialMovement::None,
    ),
    profile(EMPTY_DELTAS, DIAGONAL, SpecialMovement::None),
    profile(EMPTY_DELTAS, ORTHOGONAL, SpecialMovement::None),
    profile(ORTHOGONAL_STEPS, DIAGONAL, SpecialMovement::None),
    profile(DIAGONAL_STEPS, ORTHOGONAL, SpecialMovement::None),
    profile(EMPTY_DELTAS, ALL_DIRECTIONS, SpecialMovement::None),
    profile(KING_STEPS, EMPTY_SLIDES, SpecialMovement::None),
    profile(DRUNK_ELEPHANT, EMPTY_SLIDES, SpecialMovement::None),
    profile(FEROCIOUS_LEOPARD, EMPTY_SLIDES, SpecialMovement::None),
    profile(BLIND_TIGER, EMPTY_SLIDES, SpecialMovement::None),
    profile(COPPER_GENERAL, EMPTY_SLIDES, SpecialMovement::None),
    profile(SILVER_GENERAL, EMPTY_SLIDES, SpecialMovement::None),
    profile(GOLD_GENERAL, EMPTY_SLIDES, SpecialMovement::None),
    profile(KIRIN, EMPTY_SLIDES, SpecialMovement::None),
    profile(PHOENIX, EMPTY_SLIDES, SpecialMovement::None),
    profile(EMPTY_DELTAS, EMPTY_SLIDES, SpecialMovement::Lion),
    profile(EMPTY_DELTAS, WHITE_HORSE, SpecialMovement::None),
    profile(EMPTY_DELTAS, WHALE, SpecialMovement::None),
    profile(EMPTY_DELTAS, FLYING_OX, SpecialMovement::None),
    profile(EMPTY_DELTAS, FREE_BOAR, SpecialMovement::None),
    profile(FLYING_STAG_STEPS, VERTICAL, SpecialMovement::None),
    profile(
        EMPTY_DELTAS,
        HORNED_FALCON_SLIDES,
        SpecialMovement::LionLike(LionLikeProfile {
            directions: HORNED_FALCON_LION,
        }),
    ),
    profile(
        EMPTY_DELTAS,
        SOARING_EAGLE_SLIDES,
        SpecialMovement::LionLike(LionLikeProfile {
            directions: SOARING_EAGLE_LION,
        }),
    ),
];

/// 駒種に対応するプロファイル識別子を返す。
pub(crate) const fn movement_profile(kind: PieceKind) -> MovementProfileId {
    let index = match kind {
        PieceKind::Pawn => 0,
        PieceKind::GoBetween => 1,
        PieceKind::Lance => 2,
        PieceKind::ReverseChariot => 3,
        PieceKind::SideMover => 4,
        PieceKind::VerticalMover => 5,
        PieceKind::Bishop => 6,
        PieceKind::Rook => 7,
        PieceKind::DragonHorse => 8,
        PieceKind::DragonKing => 9,
        PieceKind::FreeKing => 10,
        PieceKind::King | PieceKind::CrownPrince => 11,
        PieceKind::DrunkElephant => 12,
        PieceKind::FerociousLeopard => 13,
        PieceKind::BlindTiger => 14,
        PieceKind::CopperGeneral => 15,
        PieceKind::SilverGeneral => 16,
        PieceKind::GoldGeneral => 17,
        PieceKind::Kirin => 18,
        PieceKind::Phoenix => 19,
        PieceKind::Lion => 20,
        PieceKind::WhiteHorse => 21,
        PieceKind::Whale => 22,
        PieceKind::FlyingOx => 23,
        PieceKind::FreeBoar => 24,
        PieceKind::FlyingStag => 25,
        PieceKind::HornedFalcon => 26,
        PieceKind::SoaringEagle => 27,
    };
    MovementProfileId(index)
}

/// 識別子からプロファイル定義を返す。
#[inline]
pub(crate) const fn movement_profile_data(profile: MovementProfileId) -> &'static MovementProfile {
    &PROFILES[profile.index()]
}

/// 全プロファイル識別子を走査するイテレータを返す。
pub(crate) fn all_profiles() -> impl ExactSizeIterator<Item = MovementProfileId> {
    (0..MOVEMENT_PROFILE_COUNT).map(MovementProfileId::from_index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_piece_kind_has_a_valid_profile() {
        for kind in PieceKind::ALL {
            assert!(movement_profile(kind).index() < MOVEMENT_PROFILE_COUNT);
        }
        assert_eq!(
            movement_profile(PieceKind::King),
            movement_profile(PieceKind::CrownPrince)
        );
    }

    #[test]
    fn white_directions_are_rotated_by_180_degrees() {
        for relative in RelativeDirection::ALL {
            assert_eq!(
                relative.for_color(Color::White),
                relative.for_color(Color::Black).opposite()
            );
        }
    }
}
