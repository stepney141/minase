use crate::core::direction::Direction;
use crate::core::piece::{Color, PieceKind};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct MovementProfileId(u8);

impl MovementProfileId {
    #[inline]
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }

    const fn from_index(index: usize) -> Self {
        debug_assert!(index < MOVEMENT_PROFILE_COUNT);
        Self(index as u8)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct RelativeDelta {
    file: i8,
    rank: i8,
}

impl RelativeDelta {
    pub(crate) const fn for_color(self, color: Color) -> (i8, i8) {
        match color {
            Color::Black => (self.file, self.rank),
            Color::White => (-self.file, -self.rank),
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum RelativeDirection {
    Forward,
    Backward,
    Right,
    Left,
    ForwardRight,
    ForwardLeft,
    BackwardRight,
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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct SlideSpec {
    pub(crate) direction: RelativeDirection,
    pub(crate) max_steps: Option<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct LionLikeProfile {
    pub(crate) directions: &'static [RelativeDirection],
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum SpecialMovement {
    None,
    Lion,
    LionLike(LionLikeProfile),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct MovementProfile {
    pub(crate) fixed_deltas: &'static [RelativeDelta],
    pub(crate) slides: &'static [SlideSpec],
    pub(crate) special: SpecialMovement,
}

const fn delta(file: i8, rank: i8) -> RelativeDelta {
    RelativeDelta { file, rank }
}

const fn slide(direction: RelativeDirection) -> SlideSpec {
    SlideSpec {
        direction,
        max_steps: None,
    }
}

const EMPTY_DELTAS: &[RelativeDelta] = &[];
const EMPTY_SLIDES: &[SlideSpec] = &[];
const PAWN: &[RelativeDelta] = &[delta(0, 1)];
const GO_BETWEEN: &[RelativeDelta] = &[delta(0, 1), delta(0, -1)];
const ORTHOGONAL_STEPS: &[RelativeDelta] = &[delta(0, 1), delta(1, 0), delta(-1, 0), delta(0, -1)];
const DIAGONAL_STEPS: &[RelativeDelta] = &[delta(1, 1), delta(-1, 1), delta(1, -1), delta(-1, -1)];
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
const DRUNK_ELEPHANT: &[RelativeDelta] = &[
    delta(0, 1),
    delta(1, 1),
    delta(-1, 1),
    delta(1, 0),
    delta(-1, 0),
    delta(1, -1),
    delta(-1, -1),
];
const FEROCIOUS_LEOPARD: &[RelativeDelta] = &[
    delta(0, 1),
    delta(1, 1),
    delta(-1, 1),
    delta(0, -1),
    delta(1, -1),
    delta(-1, -1),
];
const BLIND_TIGER: &[RelativeDelta] = &[
    delta(1, 1),
    delta(-1, 1),
    delta(1, 0),
    delta(-1, 0),
    delta(0, -1),
    delta(1, -1),
    delta(-1, -1),
];
const COPPER_GENERAL: &[RelativeDelta] = &[delta(0, 1), delta(1, 1), delta(-1, 1), delta(0, -1)];
const SILVER_GENERAL: &[RelativeDelta] = &[
    delta(0, 1),
    delta(1, 1),
    delta(-1, 1),
    delta(1, -1),
    delta(-1, -1),
];
const GOLD_GENERAL: &[RelativeDelta] = &[
    delta(0, 1),
    delta(1, 1),
    delta(-1, 1),
    delta(1, 0),
    delta(-1, 0),
    delta(0, -1),
];
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
const FLYING_STAG_STEPS: &[RelativeDelta] = &[
    delta(1, 1),
    delta(-1, 1),
    delta(1, 0),
    delta(-1, 0),
    delta(1, -1),
    delta(-1, -1),
];

const FORWARD: &[SlideSpec] = &[slide(RelativeDirection::Forward)];
const VERTICAL: &[SlideSpec] = &[
    slide(RelativeDirection::Forward),
    slide(RelativeDirection::Backward),
];
const HORIZONTAL: &[SlideSpec] = &[
    slide(RelativeDirection::Right),
    slide(RelativeDirection::Left),
];
const DIAGONAL: &[SlideSpec] = &[
    slide(RelativeDirection::ForwardRight),
    slide(RelativeDirection::ForwardLeft),
    slide(RelativeDirection::BackwardRight),
    slide(RelativeDirection::BackwardLeft),
];
const ORTHOGONAL: &[SlideSpec] = &[
    slide(RelativeDirection::Forward),
    slide(RelativeDirection::Backward),
    slide(RelativeDirection::Right),
    slide(RelativeDirection::Left),
];
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
const WHITE_HORSE: &[SlideSpec] = &[
    slide(RelativeDirection::Forward),
    slide(RelativeDirection::ForwardRight),
    slide(RelativeDirection::ForwardLeft),
    slide(RelativeDirection::Backward),
];
const WHALE: &[SlideSpec] = &[
    slide(RelativeDirection::Forward),
    slide(RelativeDirection::Backward),
    slide(RelativeDirection::BackwardRight),
    slide(RelativeDirection::BackwardLeft),
];
const FLYING_OX: &[SlideSpec] = &[
    slide(RelativeDirection::Forward),
    slide(RelativeDirection::Backward),
    slide(RelativeDirection::ForwardRight),
    slide(RelativeDirection::ForwardLeft),
    slide(RelativeDirection::BackwardRight),
    slide(RelativeDirection::BackwardLeft),
];
const FREE_BOAR: &[SlideSpec] = &[
    slide(RelativeDirection::Right),
    slide(RelativeDirection::Left),
    slide(RelativeDirection::ForwardRight),
    slide(RelativeDirection::ForwardLeft),
    slide(RelativeDirection::BackwardRight),
    slide(RelativeDirection::BackwardLeft),
];
const HORNED_FALCON_SLIDES: &[SlideSpec] = &[
    slide(RelativeDirection::Backward),
    slide(RelativeDirection::Right),
    slide(RelativeDirection::Left),
    slide(RelativeDirection::ForwardRight),
    slide(RelativeDirection::ForwardLeft),
    slide(RelativeDirection::BackwardRight),
    slide(RelativeDirection::BackwardLeft),
];
const SOARING_EAGLE_SLIDES: &[SlideSpec] = &[
    slide(RelativeDirection::Forward),
    slide(RelativeDirection::Backward),
    slide(RelativeDirection::Right),
    slide(RelativeDirection::Left),
    slide(RelativeDirection::BackwardRight),
    slide(RelativeDirection::BackwardLeft),
];
const HORNED_FALCON_LION: &[RelativeDirection] = &[RelativeDirection::Forward];
const SOARING_EAGLE_LION: &[RelativeDirection] = &[
    RelativeDirection::ForwardRight,
    RelativeDirection::ForwardLeft,
];

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

pub(crate) const MOVEMENT_PROFILE_COUNT: usize = 28;

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

#[inline]
pub(crate) const fn movement_profile_data(profile: MovementProfileId) -> &'static MovementProfile {
    &PROFILES[profile.index()]
}

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
