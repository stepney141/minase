use crate::attacks::{AttackTables, SpecialMovement, movement_profile, movement_profile_data};
use crate::bitboard::Bitboard;
use crate::mv::Move;
use crate::piece::{Color, PieceKind};
use crate::position::Position;
use crate::rules::PromotionChoice;
use crate::square::Square;

use super::{MoveGenerator, lion};

fn promoting_variant(mv: Move) -> Move {
    match mv {
        Move::Step { from, to, .. } => Move::Step {
            from,
            to,
            promote: true,
        },
        Move::Double { from, mid, to, .. } => Move::Double {
            from,
            mid,
            to,
            promote: true,
        },
        Move::Jump { from, to, .. } => Move::Jump {
            from,
            to,
            promote: true,
        },
    }
}

pub(super) fn piece_control_with_occupancy(
    tables: &AttackTables,
    occupied: Bitboard,
    color: Color,
    kind: PieceKind,
    from: Square,
) -> Bitboard {
    let profile_id = movement_profile(kind);
    let profile = movement_profile_data(profile_id);
    let mut result = tables.fixed(color, profile_id, from);
    for slide in profile.slides {
        result |= tables.sliding_control(
            from,
            slide.direction.for_color(color),
            slide.max_steps,
            occupied,
        );
    }

    match profile.special {
        SpecialMovement::None => {}
        SpecialMovement::Lion => {
            result |= tables.king_steps(from) | tables.lion_jumps(from);
        }
        SpecialMovement::LionLike(lion_like) => {
            for direction in lion_like.directions {
                let direction = direction.for_color(color);
                if let Some(first) = crate::direction::step_square(from, direction) {
                    result.set(first);
                    if let Some(second) = crate::direction::step_square(first, direction) {
                        result.set(second);
                    }
                }
            }
        }
    }
    result
}

fn push_with_promotion(
    generator: &MoveGenerator,
    position: &Position,
    moving_kind: PieceKind,
    base: Move,
    output: &mut Vec<Move>,
) {
    if !generator.rules().special_move_is_legal(position, base) {
        return;
    }

    match generator
        .rules()
        .promotion_choice(position, &base, moving_kind)
    {
        PromotionChoice::NoPromotion => output.push(base),
        PromotionChoice::PromotionOptional => {
            output.push(base);
            output.push(promoting_variant(base));
        }
    }
}

pub(super) fn generate_moves(
    generator: &MoveGenerator,
    position: &Position,
    output: &mut Vec<Move>,
) {
    let color = position.side_to_move();
    let own = position.pieces_of(color);

    for kind in PieceKind::ALL {
        let profile = movement_profile_data(movement_profile(kind));
        for from in position.pieces_of_kind(color, kind) {
            let mut base_moves = Vec::new();
            let step_destinations =
                (piece_control_without_special(generator.tables(), position, color, kind, from)
                    | special_step_destinations(generator.tables(), color, from, profile.special))
                    & !own;
            for to in step_destinations {
                base_moves.push(Move::Step {
                    from,
                    to,
                    promote: false,
                });
            }

            match profile.special {
                SpecialMovement::None => {}
                SpecialMovement::Lion => {
                    lion::generate_lion_double_and_jumps(
                        generator.tables(),
                        position,
                        color,
                        from,
                        &mut base_moves,
                    );
                }
                SpecialMovement::LionLike(profile) => {
                    lion::generate_lion_like_double_and_jumps(
                        position,
                        color,
                        from,
                        profile,
                        &mut base_moves,
                    );
                }
            }
            for base in base_moves {
                push_with_promotion(generator, position, kind, base, output);
            }
        }
    }
}

fn special_step_destinations(
    tables: &AttackTables,
    color: Color,
    from: Square,
    special: SpecialMovement,
) -> Bitboard {
    match special {
        SpecialMovement::None => Bitboard::EMPTY,
        SpecialMovement::Lion => tables.king_steps(from),
        SpecialMovement::LionLike(profile) => {
            let mut destinations = Bitboard::EMPTY;
            for relative in profile.directions {
                if let Some(first) = crate::direction::step_square(from, relative.for_color(color))
                {
                    destinations.set(first);
                }
            }
            destinations
        }
    }
}

fn piece_control_without_special(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    kind: PieceKind,
    from: Square,
) -> Bitboard {
    let profile_id = movement_profile(kind);
    let profile = movement_profile_data(profile_id);
    let mut result = tables.fixed(color, profile_id, from);
    for slide in profile.slides {
        result |= tables.sliding_control(
            from,
            slide.direction.for_color(color),
            slide.max_steps,
            position.occupied(),
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attacks::{SpecialMovement, attack_tables};
    use crate::direction::step_square;
    use crate::piece::PieceCode;
    use crate::position::PositionBuilder;
    use crate::rules::Rules;

    fn reference_control(
        position: &Position,
        color: Color,
        kind: PieceKind,
        from: Square,
    ) -> Bitboard {
        let profile = movement_profile_data(movement_profile(kind));
        let mut result = Bitboard::EMPTY;
        for delta in profile.fixed_deltas {
            let (file, rank) = delta.for_color(color);
            if let Some(to) = from.offset(file, rank) {
                result.set(to);
            }
        }
        for slide in profile.slides {
            let direction = slide.direction.for_color(color);
            let mut current = from;
            let mut steps = 0;
            while slide.max_steps.is_none_or(|maximum| steps < maximum) {
                let Some(next) = step_square(current, direction) else {
                    break;
                };
                result.set(next);
                steps += 1;
                if position.occupied().contains(next) {
                    break;
                }
                current = next;
            }
        }

        match profile.special {
            SpecialMovement::None => {}
            SpecialMovement::Lion => {
                for file in -2_i8..=2 {
                    for rank in -2_i8..=2 {
                        if (file != 0 || rank != 0)
                            && let Some(to) = from.offset(file, rank)
                        {
                            result.set(to);
                        }
                    }
                }
            }
            SpecialMovement::LionLike(lion_like) => {
                for relative in lion_like.directions {
                    let direction = relative.for_color(color);
                    if let Some(first) = step_square(from, direction) {
                        result.set(first);
                        if let Some(second) = step_square(first, direction) {
                            result.set(second);
                        }
                    }
                }
            }
        }
        result
    }

    #[test]
    fn every_piece_control_matches_coordinate_reference() {
        let mut builder = PositionBuilder::new(Color::Black);
        for square in Square::all().filter(|square| square.dense_index() % 11 == 0) {
            builder
                .put(square, PieceCode::new(Color::White, PieceKind::Pawn))
                .unwrap();
        }
        let position = builder.finish().unwrap();

        for color in Color::ALL {
            for kind in PieceKind::ALL {
                for from in Square::all() {
                    assert_eq!(
                        piece_control_with_occupancy(
                            attack_tables(),
                            position.occupied(),
                            color,
                            kind,
                            from,
                        ),
                        reference_control(&position, color, kind, from),
                        "color={color:?}, kind={kind:?}, from={from:?}",
                    );
                }
            }
        }
    }

    #[test]
    fn kirin_jump_ignores_intermediate_occupancy() {
        let from = Square::new(4, 4).unwrap();
        let middle = Square::new(4, 5).unwrap();
        let target = Square::new(4, 6).unwrap();
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(from, PieceCode::new(Color::Black, PieceKind::Kirin))
            .unwrap();
        builder
            .put(middle, PieceCode::new(Color::Black, PieceKind::Pawn))
            .unwrap();
        let position = builder.finish().unwrap();

        assert!(
            piece_control_with_occupancy(
                attack_tables(),
                position.occupied(),
                Color::Black,
                PieceKind::Kirin,
                from,
            )
            .contains(target)
        );
    }

    #[test]
    fn articles_18_1_and_18_5_promotion_entry_generates_both_choices() {
        let from = Square::new(4, 7).unwrap();
        let to = Square::new(4, 8).unwrap();
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(from, PieceCode::new(Color::Black, PieceKind::Pawn))
            .unwrap();
        let position = builder.finish().unwrap();
        let generator = MoveGenerator::new(Rules::standard());
        let mut moves = Vec::new();
        generator.generate_moves(&position, &mut moves);

        assert!(moves.contains(&Move::Step {
            from,
            to,
            promote: false,
        }));
        assert!(moves.contains(&Move::Step {
            from,
            to,
            promote: true,
        }));
    }

    #[test]
    fn already_promoted_piece_does_not_promote_again() {
        let from = Square::new(4, 7).unwrap();
        let to = Square::new(4, 8).unwrap();
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(
                from,
                PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap(),
            )
            .unwrap();
        let position = builder.finish().unwrap();
        let generator = MoveGenerator::new(Rules::standard());
        let mut moves = Vec::new();
        generator.generate_moves(&position, &mut moves);

        assert!(moves.contains(&Move::Step {
            from,
            to,
            promote: false,
        }));
        assert!(!moves.contains(&Move::Step {
            from,
            to,
            promote: true,
        }));
    }
}
