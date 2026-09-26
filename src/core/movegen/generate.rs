//! 合法手生成の駆動。

use super::MoveGenerator;
use super::control::{piece_control_without_special, special_step_destinations};
use super::expand::push_with_promotion;
use super::lion::generate_lion_double_and_jumps;
use super::lion_like::generate_lion_like_double_and_jumps;
use crate::core::attacks::{SpecialMovement, movement_profile, movement_profile_data};
use crate::core::board::Square;
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceKind};
use crate::core::position::Position;

/// 手番側の全駒について合法手を生成する。
pub(super) fn generate_moves<const CAPTURES_ONLY: bool, const QUIETS_ONLY: bool>(
    generator: &MoveGenerator,
    position: &Position,
    base_moves: &mut Vec<Move>,
    output: &mut Vec<Move>,
) {
    let color = position.side_to_move();
    for kind in PieceKind::ALL {
        for from in position.pieces_of_kind(color, kind) {
            generate_piece_moves::<CAPTURES_ONLY, QUIETS_ONLY>(
                generator, position, color, kind, from, base_moves, output,
            );
        }
    }
}

/// 指定した1枚の駒について合法手を生成する。
pub(super) fn generate_piece_moves<const CAPTURES_ONLY: bool, const QUIETS_ONLY: bool>(
    generator: &MoveGenerator,
    position: &Position,
    color: Color,
    kind: PieceKind,
    from: Square,
    base_moves: &mut Vec<Move>,
    output: &mut Vec<Move>,
) {
    let profile = movement_profile_data(movement_profile(kind));
    let own = position.pieces_of(color);
    let enemy = position.pieces_of(color.opposite());
    let destinations = if CAPTURES_ONLY {
        enemy
    } else if QUIETS_ONLY {
        !position.occupied()
    } else {
        !own
    };
    base_moves.clear();
    let step_destinations =
        (piece_control_without_special(generator.tables(), position.occupied(), color, kind, from)
            | special_step_destinations(generator.tables(), color, from, profile.special))
            & destinations;
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
            generate_lion_double_and_jumps::<CAPTURES_ONLY, QUIETS_ONLY>(
                generator.tables(),
                position,
                color,
                from,
                &mut |mv| base_moves.push(mv),
            );
        }
        SpecialMovement::LionLike(profile) => {
            generate_lion_like_double_and_jumps::<CAPTURES_ONLY, QUIETS_ONLY>(
                position,
                color,
                from,
                profile,
                &mut |mv| base_moves.push(mv),
            );
        }
    }
    for &base in base_moves.iter() {
        push_with_promotion(generator, position, kind, base, output);
    }
}
