//! 角鷹と飛鷲の2段階移動の生成。

use crate::attacks::LionLikeProfile;
use crate::board::{Square, step_square};
use crate::mv::Move;
use crate::piece::Color;
use crate::position::Position;

/// 角鷹・飛鷲の2段階移動・居喰い・じっと(第11条)を生成する。1升移動で停止する
/// 着手は通常経路で生成済みのため含めない。
///
/// 許可方向ごとに、1升目が相手駒なら居喰いを出す。
/// 2升目が盤内で自駒でなければ、1升目の占有を問わず直接の跳びを出し、
/// 1升目が相手駒なら続けて2段階捕獲を出す。
/// いずれかの許可方向の1升目が空升なら、じっとを全体で1手だけ出す。
/// 1升目に相手駒がある方向は捕獲になるので、じっとの根拠にはしない
/// （RULES.md第11条第7項、第8項）。
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
