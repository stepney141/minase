use core::fmt;

use crate::bitboard::Bitboard;
use crate::movegen::piece_control_with_occupancy;
use crate::mv::Move;
use crate::piece::{Color, PieceKind};
use crate::position::Position;
use crate::square::{BOARD_RANKS, Square};

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RuleCode {
    L0,
    L1,
    L2,
    L3,
    P1,
    P2,
    P3,
    P4,
    R1,
    R2,
    E1,
    E2,
}

impl RuleCode {
    pub const ALL: [Self; 12] = [
        Self::L0,
        Self::L1,
        Self::L2,
        Self::L3,
        Self::P1,
        Self::P2,
        Self::P3,
        Self::P4,
        Self::R1,
        Self::R2,
        Self::E1,
        Self::E2,
    ];

    const fn bit(self) -> u16 {
        1 << self as u8
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RulesError {
    Conflicting { first: RuleCode, second: RuleCode },
    Duplicate(RuleCode),
    Unsupported(RuleCode),
}

impl fmt::Display for RulesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflicting { first, second } => {
                write!(
                    formatter,
                    "conflicting rule codes: {first:?} and {second:?}"
                )
            }
            Self::Duplicate(code) => write!(formatter, "duplicate rule code: {code:?}"),
            Self::Unsupported(code) => write!(formatter, "unsupported rule code: {code:?}"),
        }
    }
}

impl std::error::Error for RulesError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rules {
    codes: u16,
}

impl Rules {
    pub const fn standard() -> Self {
        Self { codes: 0 }
    }

    pub fn from_codes(codes: &[RuleCode]) -> Result<Self, RulesError> {
        let mut adopted = 0_u16;
        for &code in codes {
            if adopted & code.bit() != 0 {
                return Err(RulesError::Duplicate(code));
            }
            adopted |= code.bit();
        }

        for (first, second) in [
            (RuleCode::L0, RuleCode::L1),
            (RuleCode::P1, RuleCode::P2),
            (RuleCode::R1, RuleCode::R2),
        ] {
            if adopted & first.bit() != 0 && adopted & second.bit() != 0 {
                return Err(RulesError::Conflicting { first, second });
            }
        }

        for &code in codes {
            if code != RuleCode::L0 {
                return Err(RulesError::Unsupported(code));
            }
        }

        Ok(Self { codes: adopted })
    }

    pub const fn contains(self, code: RuleCode) -> bool {
        self.codes & code.bit() != 0
    }

    pub(crate) fn promotion_choice(
        self,
        position: &Position,
        mv: &Move,
        moving_kind: PieceKind,
    ) -> PromotionChoice {
        let Some(piece) = position.piece_at(mv.origin()) else {
            return PromotionChoice::NoPromotion;
        };
        let Some(color) = piece.color() else {
            return PromotionChoice::NoPromotion;
        };
        if piece.is_promoted() || !moving_kind.can_promote() {
            return PromotionChoice::NoPromotion;
        }

        let from_in_zone = in_promotion_zone(color, mv.origin());
        let to_in_zone = in_promotion_zone(color, mv.destination());
        let has_capture = position
            .captured_squares(*mv)
            .into_iter()
            .any(|capture| capture.is_some());
        let enters_zone = !from_in_zone && to_in_zone;
        let capture_in_or_from_zone = has_capture && (from_in_zone || to_in_zone);
        let pawn_reaches_last_rank_without_capture = moving_kind == PieceKind::Pawn
            && !has_capture
            && match color {
                Color::Black => mv.destination().rank() == BOARD_RANKS - 1,
                Color::White => mv.destination().rank() == 0,
            };

        if enters_zone || capture_in_or_from_zone || pawn_reaches_last_rank_without_capture {
            PromotionChoice::PromotionOptional
        } else {
            PromotionChoice::NoPromotion
        }
    }

    pub(crate) fn special_move_is_legal(self, position: &Position, mv: Move) -> bool {
        let captured_lions = captured_lions(position, mv);
        if captured_lions.into_iter().all(|lion| lion.is_none()) {
            return true;
        }

        let moving_kind = position
            .piece_at(mv.origin())
            .and_then(|piece| piece.kind());
        if moving_kind == Some(PieceKind::Lion)
            && captured_lions
                .into_iter()
                .flatten()
                .any(|lion| !lion_capture_is_legal(position, mv, lion))
        {
            return false;
        }

        let move_is_tsukegui = captured_lions
            .into_iter()
            .flatten()
            .any(|lion| is_tsukegui(position, mv, lion));
        if position.lion_taken_by_non_lion().is_some()
            && !move_is_tsukegui
            && captured_lions
                .into_iter()
                .flatten()
                .any(|lion| lion_has_foot_after_capture(position, mv, lion))
        {
            return false;
        }

        true
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PromotionChoice {
    NoPromotion,
    PromotionRequired,
    PromotionOptional,
}

#[inline]
pub const fn in_promotion_zone(color: Color, square: Square) -> bool {
    match color {
        Color::Black => square.rank() >= BOARD_RANKS - 4,
        Color::White => square.rank() < 4,
    }
}

fn captured_lions(position: &Position, mv: Move) -> [Option<Square>; 2] {
    position.captured_squares(mv).map(|capture| {
        capture.filter(|&square| {
            position
                .piece_at(square)
                .is_some_and(|piece| piece.kind() == Some(PieceKind::Lion))
        })
    })
}

fn is_tsukegui(position: &Position, mv: Move, lion_square: Square) -> bool {
    if position
        .piece_at(mv.origin())
        .and_then(|piece| piece.kind())
        != Some(PieceKind::Lion)
    {
        return false;
    }
    let Move::Double { from, mid, to, .. } = mv else {
        return false;
    };
    let distance = from
        .file()
        .abs_diff(lion_square.file())
        .max(from.rank().abs_diff(lion_square.rank()));
    if to != lion_square || distance != 2 {
        return false;
    }

    let [first, second] = position.captured_squares(mv);
    first == Some(mid)
        && second == Some(lion_square)
        && position.piece_at(mid).is_some_and(|piece| {
            !matches!(piece.kind(), Some(PieceKind::Pawn | PieceKind::GoBetween))
        })
}

fn lion_capture_is_legal(position: &Position, mv: Move, lion_square: Square) -> bool {
    let distance = mv
        .origin()
        .file()
        .abs_diff(lion_square.file())
        .max(mv.origin().rank().abs_diff(lion_square.rank()));

    match distance {
        1 => true,
        2 if is_tsukegui(position, mv, lion_square) => true,
        2 => !lion_has_foot_after_capture(position, mv, lion_square),
        _ => false,
    }
}

#[derive(Clone, Copy)]
struct VirtualBoard {
    occupied: Bitboard,
    own: Bitboard,
    enemy: Bitboard,
    current: Square,
}

impl VirtualBoard {
    fn after_move(position: &Position, mv: Move) -> Self {
        let color = position
            .piece_at(mv.origin())
            .and_then(|piece| piece.color())
            .expect("move origin must contain a piece");
        let board = Self {
            occupied: position.occupied(),
            own: position.pieces_of(color),
            enemy: position.pieces_of(color.opposite()),
            current: mv.origin(),
        };

        match mv {
            Move::Step { to, .. } | Move::Jump { to, .. } => board.move_to(to),
            Move::Double { mid, to, .. } => board.move_to(mid).move_to(to),
        }
    }

    fn move_to(mut self, to: Square) -> Self {
        self.occupied.clear(self.current);
        self.own.clear(self.current);
        if self.enemy.contains(to) {
            self.occupied.clear(to);
            self.enemy.clear(to);
        }
        self.occupied.set(to);
        self.own.set(to);
        self.current = to;
        self
    }
}

fn lion_has_foot_after_capture(position: &Position, mv: Move, lion_square: Square) -> bool {
    let defending_color = position
        .piece_at(lion_square)
        .and_then(|piece| piece.color())
        .expect("capture square must contain a lion");
    let board = VirtualBoard::after_move(position, mv);

    let [first, second] = position.captured_squares(mv);
    let captured_pawn_or_go_between_had_foot = second == Some(lion_square)
        && first.is_some_and(|first| {
            position.piece_at(first).is_some_and(|piece| {
                let Some(kind @ (PieceKind::Pawn | PieceKind::GoBetween)) = piece.kind() else {
                    return false;
                };
                piece_control_with_occupancy(board.occupied, defending_color, kind, first)
                    .contains(mv.destination())
            })
        });

    debug_assert!(board.own.contains(mv.destination()));
    debug_assert!(!board.enemy.contains(mv.destination()));
    captured_pawn_or_go_between_had_foot
        || square_is_controlled(position, board, defending_color, mv.destination())
}

fn square_is_controlled(
    position: &Position,
    board: VirtualBoard,
    color: Color,
    target: Square,
) -> bool {
    for kind in PieceKind::ALL {
        let remaining = position.pieces_of_kind(color, kind) & board.enemy;
        for from in remaining {
            if piece_control_with_occupancy(board.occupied, color, kind, from).contains(target) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movegen::MoveGenerator;
    use crate::piece::PieceCode;
    use crate::position::PositionBuilder;

    fn sq(file: u8, rank: u8) -> Square {
        Square::new(file, rank).unwrap()
    }

    fn position(side_to_move: Color, pieces: &[(Square, Color, PieceKind)]) -> Position {
        let mut builder = PositionBuilder::new(side_to_move);
        for &(square, color, kind) in pieces {
            builder.put(square, PieceCode::new(color, kind)).unwrap();
        }
        builder.finish().unwrap()
    }

    fn is_generated(position: &Position, expected: Move) -> bool {
        let mut moves = Vec::new();
        MoveGenerator::standard().generate_moves(position, &mut moves);
        moves.contains(&expected)
    }

    fn after_non_lion_capture(pieces: &[(Square, Color, PieceKind)], capture: Move) -> Position {
        let mut position = position(Color::Black, pieces);
        position.make_move_unchecked(capture);
        assert!(position.lion_taken_by_non_lion().is_some());
        position
    }

    #[test]
    fn article_33_7_conflicting_code_sets_are_invalid_and_other_inputs_are_validated() {
        assert_eq!(
            Rules::from_codes(&[RuleCode::L0, RuleCode::L1]),
            Err(RulesError::Conflicting {
                first: RuleCode::L0,
                second: RuleCode::L1,
            }),
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::P1, RuleCode::P2]),
            Err(RulesError::Conflicting {
                first: RuleCode::P1,
                second: RuleCode::P2,
            }),
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::R1, RuleCode::R2]),
            Err(RulesError::Conflicting {
                first: RuleCode::R1,
                second: RuleCode::R2,
            }),
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::L0, RuleCode::L0]),
            Err(RulesError::Duplicate(RuleCode::L0)),
        );
        for code in RuleCode::ALL
            .into_iter()
            .filter(|&code| code != RuleCode::L0)
        {
            assert_eq!(
                Rules::from_codes(&[code]),
                Err(RulesError::Unsupported(code)),
            );
        }
    }

    #[test]
    fn articles_28_and_33_rules_retain_empty_and_explicit_l0_codes() {
        let standard = Rules::from_codes(&[]).unwrap();
        let explicit_l0 = Rules::from_codes(&[RuleCode::L0]).unwrap();

        assert!(!standard.contains(RuleCode::L0));
        assert!(explicit_l0.contains(RuleCode::L0));
    }

    #[test]
    fn article_9_all_eighteen_promotion_targets_are_explicit() {
        let promotion_pairs = [
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

        assert_eq!(promotion_pairs.len(), 18);
        for (unpromoted, promoted) in promotion_pairs {
            assert_eq!(unpromoted.promoted(), Some(promoted), "{unpromoted:?}");
        }
    }

    #[test]
    fn articles_15_8_and_16_1_tsukegui_exemption_requires_a_lion_mover() {
        let position = after_non_lion_capture(
            &[
                (sq(0, 0), Color::Black, PieceKind::Bishop),
                (sq(1, 1), Color::White, PieceKind::Lion),
                (sq(6, 6), Color::White, PieceKind::SoaringEagle),
                (sq(5, 5), Color::Black, PieceKind::SilverGeneral),
                (sq(4, 4), Color::Black, PieceKind::Lion),
                (sq(4, 1), Color::Black, PieceKind::Rook),
            ],
            Move::Step {
                from: sq(0, 0),
                to: sq(1, 1),
                promote: false,
            },
        );
        let double_capture = Move::Double {
            from: sq(6, 6),
            mid: sq(5, 5),
            to: sq(4, 4),
            promote: false,
        };

        assert!(!Rules::standard().special_move_is_legal(&position, double_capture));
    }

    #[test]
    fn article_14_1_adjacent_lion_capture_is_unconditional() {
        let position = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(3, 2), Color::White, PieceKind::Lion),
                (sq(3, 5), Color::White, PieceKind::Rook),
            ],
        );
        let capture = Move::Step {
            from: sq(2, 2),
            to: sq(3, 2),
            promote: false,
        };

        assert!(is_generated(&position, capture));
    }

    #[test]
    fn article_14_2_lion_cannot_capture_a_defended_lion_at_distance_two() {
        let position = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(4, 2), Color::White, PieceKind::Lion),
                (sq(4, 5), Color::White, PieceKind::Rook),
            ],
        );
        let capture = Move::Jump {
            from: sq(2, 2),
            to: sq(4, 2),
            promote: false,
        };

        assert!(!is_generated(&position, capture));
    }

    #[test]
    fn articles_13_5_and_14_2_attackable_defender_still_blocks_distance_two_lion_capture() {
        let position = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(0, 5), Color::Black, PieceKind::Rook),
                (sq(4, 2), Color::White, PieceKind::Lion),
                (sq(4, 5), Color::White, PieceKind::Rook),
            ],
        );
        let capture_defender = Move::Step {
            from: sq(0, 5),
            to: sq(4, 5),
            promote: false,
        };
        let capture_lion = Move::Jump {
            from: sq(2, 2),
            to: sq(4, 2),
            promote: false,
        };

        assert!(is_generated(&position, capture_defender));
        assert!(!is_generated(&position, capture_lion));
    }

    #[test]
    fn articles_13_2_and_14_4_hidden_sliding_defender_counts_as_a_foot() {
        let position = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(4, 2), Color::White, PieceKind::Lion),
                (sq(6, 2), Color::White, PieceKind::Rook),
            ],
        );
        let capture = Move::Jump {
            from: sq(2, 2),
            to: sq(4, 2),
            promote: false,
        };

        assert!(lion_has_foot_after_capture(&position, capture, sq(4, 2)));
        assert!(!is_generated(&position, capture));
    }

    #[test]
    fn article_14_5_non_lion_can_capture_a_defended_lion() {
        let position = position(
            Color::Black,
            &[
                (sq(4, 0), Color::Black, PieceKind::Rook),
                (sq(4, 2), Color::White, PieceKind::Lion),
                (sq(4, 5), Color::White, PieceKind::Rook),
            ],
        );
        let capture = Move::Step {
            from: sq(4, 0),
            to: sq(4, 2),
            promote: false,
        };

        assert!(is_generated(&position, capture));
    }

    #[test]
    fn article_14_3_undefended_lion_at_distance_two_can_be_captured() {
        let position = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(4, 2), Color::White, PieceKind::Lion),
            ],
        );
        let capture = Move::Jump {
            from: sq(2, 2),
            to: sq(4, 2),
            promote: false,
        };

        assert!(is_generated(&position, capture));
    }

    #[test]
    fn articles_16_1_and_16_4_tsukegui_captures_a_defended_lion() {
        let position = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(3, 2), Color::White, PieceKind::SilverGeneral),
                (sq(4, 2), Color::White, PieceKind::Lion),
                (sq(4, 5), Color::White, PieceKind::Rook),
            ],
        );
        let tsukegui = Move::Double {
            from: sq(2, 2),
            mid: sq(3, 2),
            to: sq(4, 2),
            promote: false,
        };

        assert!(is_generated(&position, tsukegui));
    }

    #[test]
    fn articles_15_1_and_16_1_adjacent_double_capture_is_not_tsukegui() {
        let position = after_non_lion_capture(
            &[
                (sq(0, 0), Color::Black, PieceKind::Bishop),
                (sq(1, 1), Color::White, PieceKind::Lion),
                (sq(2, 2), Color::White, PieceKind::Lion),
                (sq(2, 3), Color::Black, PieceKind::SilverGeneral),
                (sq(3, 2), Color::Black, PieceKind::Lion),
                (sq(3, 5), Color::Black, PieceKind::Rook),
            ],
            Move::Step {
                from: sq(0, 0),
                to: sq(1, 1),
                promote: false,
            },
        );
        let adjacent_double_capture = Move::Double {
            from: sq(2, 2),
            mid: sq(2, 3),
            to: sq(3, 2),
            promote: false,
        };

        assert!(!is_tsukegui(&position, adjacent_double_capture, sq(3, 2)));
        assert!(!is_generated(&position, adjacent_double_capture));
    }

    #[test]
    fn articles_12_13_and_16_8_9_10_restore_a_pawn_that_is_the_only_foot() {
        let position = position(
            Color::Black,
            &[
                (sq(4, 6), Color::Black, PieceKind::Lion),
                (sq(4, 5), Color::White, PieceKind::Pawn),
                (sq(4, 4), Color::White, PieceKind::Lion),
            ],
        );
        let capture = Move::Double {
            from: sq(4, 6),
            mid: sq(4, 5),
            to: sq(4, 4),
            promote: false,
        };

        assert!(lion_has_foot_after_capture(&position, capture, sq(4, 4)));
        assert!(!is_generated(&position, capture));
    }

    #[test]
    fn articles_13_4_14_2_and_16_8_pawn_removal_opens_sliding_foot() {
        let position = position(
            Color::Black,
            &[
                (sq(3, 5), Color::Black, PieceKind::Lion),
                (sq(4, 5), Color::White, PieceKind::Pawn),
                (sq(5, 5), Color::White, PieceKind::Lion),
                (sq(0, 5), Color::White, PieceKind::Rook),
            ],
        );
        let jump = Move::Jump {
            from: sq(3, 5),
            to: sq(5, 5),
            promote: false,
        };
        let double = Move::Double {
            from: sq(3, 5),
            mid: sq(4, 5),
            to: sq(5, 5),
            promote: false,
        };

        assert!(is_generated(&position, jump));
        assert!(!is_generated(&position, double));
    }

    #[test]
    fn article_13_4_vacating_origin_opens_sliding_foot() {
        let position = position(
            Color::Black,
            &[
                (sq(6, 2), Color::Black, PieceKind::Lion),
                (sq(4, 2), Color::White, PieceKind::Lion),
                (sq(9, 2), Color::White, PieceKind::Rook),
            ],
        );
        let before = position.clone();
        let capture = Move::Jump {
            from: sq(6, 2),
            to: sq(4, 2),
            promote: false,
        };

        assert!(!is_generated(&position, capture));
        assert_eq!(position, before);
    }

    #[test]
    fn article_16_3_pawn_between_lions_is_not_tsukegui() {
        let position = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(3, 2), Color::White, PieceKind::Pawn),
                (sq(4, 2), Color::White, PieceKind::Lion),
                (sq(4, 5), Color::White, PieceKind::Rook),
            ],
        );
        let capture = Move::Double {
            from: sq(2, 2),
            mid: sq(3, 2),
            to: sq(4, 2),
            promote: false,
        };

        assert!(!is_tsukegui(&position, capture, sq(4, 2)));
        assert!(!is_generated(&position, capture));
    }

    #[test]
    fn article_15_1_senjishi_prevents_immediate_capture_of_a_defended_lion() {
        let position = after_non_lion_capture(
            &[
                (sq(0, 0), Color::Black, PieceKind::Bishop),
                (sq(4, 4), Color::Black, PieceKind::Lion),
                (sq(4, 3), Color::Black, PieceKind::GoldGeneral),
                (sq(1, 1), Color::White, PieceKind::Lion),
                (sq(4, 6), Color::White, PieceKind::Rook),
            ],
            Move::Step {
                from: sq(0, 0),
                to: sq(1, 1),
                promote: false,
            },
        );
        let recapture = Move::Step {
            from: sq(4, 6),
            to: sq(4, 4),
            promote: false,
        };

        assert!(!is_generated(&position, recapture));
    }

    #[test]
    fn article_15_2_senjishi_allows_immediate_capture_of_an_undefended_lion() {
        let position = after_non_lion_capture(
            &[
                (sq(1, 0), Color::Black, PieceKind::Pawn),
                (sq(4, 4), Color::Black, PieceKind::Lion),
                (sq(1, 1), Color::White, PieceKind::Lion),
                (sq(4, 6), Color::White, PieceKind::Rook),
            ],
            Move::Step {
                from: sq(1, 0),
                to: sq(1, 1),
                promote: false,
            },
        );
        let recapture = Move::Step {
            from: sq(4, 6),
            to: sq(4, 4),
            promote: false,
        };

        assert!(is_generated(&position, recapture));
    }

    #[test]
    fn articles_15_8_and_16_7_tsukegui_takes_priority_over_senjishi() {
        let position = after_non_lion_capture(
            &[
                (sq(0, 0), Color::Black, PieceKind::Bishop),
                (sq(4, 4), Color::Black, PieceKind::Lion),
                (sq(4, 3), Color::Black, PieceKind::SilverGeneral),
                (sq(4, 7), Color::Black, PieceKind::Rook),
                (sq(1, 1), Color::White, PieceKind::Lion),
                (sq(4, 2), Color::White, PieceKind::Lion),
            ],
            Move::Step {
                from: sq(0, 0),
                to: sq(1, 1),
                promote: false,
            },
        );
        let tsukegui = Move::Double {
            from: sq(4, 2),
            mid: sq(4, 3),
            to: sq(4, 4),
            promote: false,
        };

        assert!(is_generated(&position, tsukegui));
    }

    #[test]
    fn articles_15_8_and_16_7_tsukegui_exempts_the_whole_move_from_senjishi() {
        let mut builder = PositionBuilder::new(Color::Black);
        for (square, piece) in [
            (sq(0, 0), PieceCode::new(Color::Black, PieceKind::Bishop)),
            (sq(1, 1), PieceCode::new(Color::White, PieceKind::Lion)),
            (sq(2, 2), PieceCode::new(Color::White, PieceKind::Lion)),
            (sq(3, 2), PieceCode::new(Color::Black, PieceKind::Lion)),
            (
                sq(4, 2),
                PieceCode::new_promoted(Color::Black, PieceKind::Lion).unwrap(),
            ),
            (sq(4, 5), PieceCode::new(Color::Black, PieceKind::Rook)),
        ] {
            builder.put(square, piece).unwrap();
        }
        let mut position = builder.finish().unwrap();
        position.make_move_unchecked(Move::Step {
            from: sq(0, 0),
            to: sq(1, 1),
            promote: false,
        });
        assert!(position.lion_taken_by_non_lion().is_some());

        let tsukegui = Move::Double {
            from: sq(2, 2),
            mid: sq(3, 2),
            to: sq(4, 2),
            promote: false,
        };
        assert!(is_tsukegui(&position, tsukegui, sq(4, 2)));
        assert!(is_generated(&position, tsukegui));
    }

    #[test]
    fn articles_15_6_and_16_6_lion_capture_does_not_trigger_senjishi() {
        let mut position = position(
            Color::Black,
            &[
                (sq(1, 1), Color::Black, PieceKind::Lion),
                (sq(2, 0), Color::Black, PieceKind::GoldGeneral),
                (sq(2, 1), Color::White, PieceKind::Lion),
                (sq(2, 4), Color::White, PieceKind::Rook),
            ],
        );
        position.make_move_unchecked(Move::Step {
            from: sq(1, 1),
            to: sq(2, 1),
            promote: false,
        });
        assert_eq!(position.lion_taken_by_non_lion(), None);

        let capture = Move::Step {
            from: sq(2, 4),
            to: sq(2, 1),
            promote: false,
        };
        assert!(is_generated(&position, capture));
    }

    #[test]
    fn articles_18_1_and_18_6_double_with_only_mid_in_enemy_camp_cannot_promote() {
        let position = position(
            Color::Black,
            &[(sq(4, 7), Color::Black, PieceKind::DragonHorse)],
        );
        // This is a synthetic/hypothetical non-yet-promoted DragonHorse Double that cannot be
        // produced by the real move generator; it tests promotion_choice's isolated contract.
        let mv = Move::Double {
            from: sq(4, 7),
            mid: sq(4, 8),
            to: sq(5, 7),
            promote: false,
        };

        assert_eq!(
            Rules::standard().promotion_choice(&position, &mv, PieceKind::DragonHorse),
            PromotionChoice::NoPromotion,
        );
    }

    #[test]
    fn article_18_3_non_capture_inside_enemy_camp_has_no_promotion_option() {
        let position = position(Color::Black, &[(sq(4, 8), Color::Black, PieceKind::Pawn)]);
        let non_promoting = Move::Step {
            from: sq(4, 8),
            to: sq(4, 9),
            promote: false,
        };
        let promoting = Move::Step {
            from: sq(4, 8),
            to: sq(4, 9),
            promote: true,
        };

        assert!(is_generated(&position, non_promoting));
        assert!(!is_generated(&position, promoting));
    }

    #[test]
    fn article_18_2_a_capture_inside_enemy_camp_can_promote() {
        let position = position(
            Color::Black,
            &[
                (sq(4, 8), Color::Black, PieceKind::Pawn),
                (sq(4, 9), Color::White, PieceKind::GoBetween),
            ],
        );
        let promotion = Move::Step {
            from: sq(4, 8),
            to: sq(4, 9),
            promote: true,
        };

        assert!(is_generated(&position, promotion));
    }

    #[test]
    fn article_18_2_b_capture_leaving_enemy_camp_can_promote() {
        let position = position(
            Color::Black,
            &[
                (sq(4, 8), Color::Black, PieceKind::Rook),
                (sq(4, 7), Color::White, PieceKind::Pawn),
            ],
        );
        let promotion = Move::Step {
            from: sq(4, 8),
            to: sq(4, 7),
            promote: true,
        };

        assert!(is_generated(&position, promotion));
    }

    #[test]
    fn article_19_1_pawn_can_promote_on_a_non_capture_to_the_last_rank() {
        let position = position(Color::Black, &[(sq(4, 10), Color::Black, PieceKind::Pawn)]);
        let promotion = Move::Step {
            from: sq(4, 10),
            to: sq(4, 11),
            promote: true,
        };

        assert!(is_generated(&position, promotion));
    }

    #[test]
    fn article_26_8_try_make_move_rejects_unavailable_promotion() {
        let mut position = position(Color::Black, &[(sq(4, 4), Color::Black, PieceKind::Pawn)]);
        let illegal = Move::Step {
            from: sq(4, 4),
            to: sq(4, 5),
            promote: true,
        };

        assert_eq!(
            position.try_make_move(illegal, &MoveGenerator::standard()),
            Err(crate::IllegalMove(illegal)),
        );
    }

    #[test]
    fn article_27_1_try_make_move_error_preserves_entire_position() {
        let mut position = after_non_lion_capture(
            &[
                (sq(0, 0), Color::Black, PieceKind::Bishop),
                (sq(1, 1), Color::White, PieceKind::Lion),
                (sq(4, 4), Color::Black, PieceKind::Lion),
                (sq(10, 10), Color::White, PieceKind::Pawn),
            ],
            Move::Step {
                from: sq(0, 0),
                to: sq(1, 1),
                promote: false,
            },
        );
        let before = position.clone();
        let side_to_move = position.side_to_move();
        let illegal = Move::Step {
            from: sq(11, 11),
            to: sq(11, 10),
            promote: false,
        };

        assert_eq!(
            position.try_make_move(illegal, &MoveGenerator::standard()),
            Err(crate::IllegalMove(illegal)),
        );
        assert_eq!(position, before);
        assert_eq!(position.side_to_move(), side_to_move);
    }
}
