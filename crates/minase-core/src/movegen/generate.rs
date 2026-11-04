//! 合法手生成の駆動。

use super::MoveGenerator;
use super::control::{
    capture_targets_without_special, piece_control_without_special, special_step_destinations,
};
use super::expand::{promoting_variant, push_with_promotion};
use super::lion::generate_lion_double_and_jumps;
use super::lion_like::generate_lion_like_double_and_jumps;
use crate::attacks::{SpecialMovement, movement_profile, movement_profile_data};
use crate::board::{Bitboard, Square};
use crate::mv::Move;
use crate::piece::{Color, PieceKind};
use crate::position::Position;
use crate::promotion::PromotionChoice;

/// 手番側の全駒について合法手を生成する。
/// 走査と出力の順序は[`MoveGenerator::generate_moves`]に従い、
/// 捕獲限定と非捕獲限定でも該当する手の相対順序を保つ。
pub(super) fn generate_moves<const CAPTURES_ONLY: bool, const QUIETS_ONLY: bool>(
    generator: &MoveGenerator,
    position: &Position,
    base_moves: &mut Vec<Move>,
    output: &mut Vec<Move>,
) {
    let color = position.side_to_move();
    for kind in PieceKind::ALL {
        let origins = if matches!(kind, PieceKind::Pawn | PieceKind::GoBetween)
            && (CAPTURES_ONLY || QUIETS_ONLY)
        {
            let destinations = if CAPTURES_ONLY {
                position.pieces_of(color.opposite())
            } else {
                !position.occupied()
            };
            single_step_origins(position, kind, destinations)
        } else {
            position.pieces_of_kind(color, kind)
        };
        for from in origins {
            generate_piece_moves::<CAPTURES_ONLY, QUIETS_ONLY>(
                generator, position, color, kind, from, base_moves, output,
            );
        }
    }
}

/// 指定した到達升へ動ける歩兵・仲人の移動元を一括で求める。
/// 到達升を移動方向と逆に1段ずらした集合との積は、各駒の移動先を選別して戻す操作と等しい。
/// 移動元の昇順、成りの展開、および獅子捕獲の検査は駒別の生成に従う。
pub(super) fn single_step_origins(
    position: &Position,
    kind: PieceKind,
    destinations: Bitboard,
) -> Bitboard {
    let color = position.side_to_move();
    let origins = if kind == PieceKind::GoBetween {
        destinations.shift_south() | destinations.shift_north()
    } else {
        match color {
            Color::Black => destinations.shift_south(),
            Color::White => destinations.shift_north(),
        }
    };
    origins & position.pieces_of_kind(color, kind)
}

/// 指定した1枚の駒について合法手を生成する。
/// [`MoveGenerator::generate_moves`]の駒ごとの順序で基本の着手を作り、成りを展開する。
pub(super) fn generate_piece_moves<const CAPTURES_ONLY: bool, const QUIETS_ONLY: bool>(
    generator: &MoveGenerator,
    position: &Position,
    color: Color,
    kind: PieceKind,
    from: Square,
    base_moves: &mut Vec<Move>,
    output: &mut Vec<Move>,
) {
    base_moves.clear();
    if QUIETS_ONLY {
        generate_piece_quiets(generator, position, color, kind, from, output);
        return;
    }
    let profile = movement_profile_data(movement_profile(kind));
    let own = position.pieces_of(color);
    let enemy = position.pieces_of(color.opposite());
    let destinations = if CAPTURES_ONLY { enemy } else { !own };
    let ordinary = if CAPTURES_ONLY {
        capture_targets_without_special(
            generator.tables(),
            position.occupied(),
            color,
            kind,
            from,
            enemy,
        )
    } else {
        piece_control_without_special(generator.tables(), position.occupied(), color, kind, from)
    };
    let step_destinations = (ordinary
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

/// 非捕獲手は獅子捕獲の制限を満たすため、その検査を省いて生成順のまま追加する。
/// 駒の成否と保留状態は1枚ごとに求め、成れる駒だけ成りの選択肢を展開する。
fn generate_piece_quiets(
    generator: &MoveGenerator,
    position: &Position,
    color: Color,
    kind: PieceKind,
    from: Square,
    output: &mut Vec<Move>,
) {
    let profile = movement_profile_data(movement_profile(kind));
    let can_promote = kind.can_promote()
        && !position
            .piece_at(from)
            .expect("quiet origin has a piece")
            .is_promoted();
    let deferred = position.promotion_deferred().contains(from);
    let mut emit = |base: Move| {
        if !can_promote {
            output.push(base);
            return;
        }
        match generator
            .rules()
            .promotion_choice_for(color, kind, false, from, base.to, false, deferred)
        {
            PromotionChoice::NoPromotion => output.push(base),
            PromotionChoice::PromotionOptional => {
                output.push(base);
                output.push(promoting_variant(base));
            }
            PromotionChoice::PromotionForced => output.push(promoting_variant(base)),
        }
    };
    let destinations =
        (piece_control_without_special(generator.tables(), position.occupied(), color, kind, from)
            | special_step_destinations(generator.tables(), color, from, profile.special))
            & !position.occupied();
    for to in destinations {
        emit(Move {
            from,
            mid: None,
            to,
            promote: false,
        });
    }
    match profile.special {
        SpecialMovement::None => {}
        SpecialMovement::Lion => generate_lion_double_and_jumps::<false, true>(
            generator.tables(),
            position,
            color,
            from,
            &mut emit,
        ),
        SpecialMovement::LionLike(profile) => generate_lion_like_double_and_jumps::<false, true>(
            position, color, from, profile, &mut emit,
        ),
    }
}
