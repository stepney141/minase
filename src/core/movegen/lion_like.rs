//! 角鷹と飛鷲の2段階移動の生成。

use crate::core::attacks::LionLikeProfile;
use crate::core::board::{Square, step_square};
use crate::core::mv::Move;
use crate::core::piece::Color;
use crate::core::position::Position;

/// 角鷹・飛鷲の2段階移動・居喰い・じっと(第11条)を生成する。1升移動で停止する
/// 着手は通常経路で生成済みのため含めない。
pub(super) fn generate_lion_like_double_and_jumps<
    const CAPTURES_ONLY: bool,
    const QUIETS_ONLY: bool,
>(
    position: &Position,
    color: Color,
    from: Square,
    profile: LionLikeProfile,
    output: &mut impl FnMut(Move),
) {
    let own = position.pieces_of(color);
    let enemy = position.pieces_of(color.opposite());
    let mut can_jitto = false;

    for relative in profile.directions {
        let direction = relative.for_color(color);
        let Some(first) = step_square(from, direction) else {
            continue;
        };
        if !CAPTURES_ONLY && !position.occupied().contains(first) {
            can_jitto = true;
        } else if !QUIETS_ONLY && enemy.contains(first) {
            output(Move {
                from,
                mid: Some(first),
                to: from,
                promote: false,
            });
        }

        let Some(second) = step_square(first, direction) else {
            continue;
        };
        if own.contains(second) {
            continue;
        }
        if (!CAPTURES_ONLY || enemy.contains(second))
            && (!QUIETS_ONLY || !position.occupied().contains(second))
        {
            output(Move {
                from,
                mid: None,
                to: second,
                promote: false,
            });
        }
        if !QUIETS_ONLY && enemy.contains(first) {
            output(Move {
                from,
                mid: Some(first),
                to: second,
                promote: false,
            });
        }
    }

    if !CAPTURES_ONLY && can_jitto {
        output(Move {
            from,
            mid: None,
            to: from,
            promote: false,
        });
    }
}
