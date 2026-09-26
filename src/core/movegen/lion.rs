//! 獅子の2段階移動と跳躍の生成。

use super::virtual_board::VirtualBoard;
use crate::core::attacks::AttackTables;
use crate::core::board::{Bitboard, Square};
use crate::core::mv::Move;
use crate::core::piece::Color;
use crate::core::position::Position;

/// 獅子の2段階移動・跳び・じっと(第12条)を生成する。1升移動で停止する着手は
/// 通常経路で生成済みのため含めない。
pub(super) fn generate_lion_double_and_jumps<const CAPTURES_ONLY: bool, const QUIETS_ONLY: bool>(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    from: Square,
    output: &mut impl FnMut(Move),
) {
    let own = position.pieces_of(color);
    let enemy = position.pieces_of(color.opposite());
    let adjacent = tables.king_steps(from);

    for mid in adjacent & if QUIETS_ONLY { Bitboard::EMPTY } else { enemy } {
        let local = VirtualBoard::new(position, color, from).move_to(mid);
        let second = tables.king_steps(mid) & !local.own;
        for to in second {
            output(Move {
                from,
                mid: Some(mid),
                to,
                promote: false,
            });
        }
    }

    let jump_destinations = if CAPTURES_ONLY {
        enemy
    } else if QUIETS_ONLY {
        !position.occupied()
    } else {
        !own
    };
    for to in tables.lion_jumps(from) & jump_destinations {
        output(Move {
            from,
            mid: None,
            to,
            promote: false,
        });
    }

    if !CAPTURES_ONLY && !(adjacent & !position.occupied()).is_empty() {
        output(Move {
            from,
            mid: None,
            to: from,
            promote: false,
        });
    }
}
