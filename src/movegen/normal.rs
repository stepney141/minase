use crate::attacks::{AttackTables, SpecialMovement, movement_profile, movement_profile_data};
use crate::bitboard::Bitboard;
use crate::mv::Move;
use crate::piece::{Color, PieceKind};
use crate::position::Position;
use crate::rules::PromotionChoice;
use crate::square::Square;

use super::{MoveGenerator, lion};

fn promoting_variant(mv: Move) -> Move {
    Move {
        promote: true,
        ..mv
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
                base_moves.push(Move {
                    from,
                    mid: None,
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
