//! 獅子の2段階移動と跳躍の生成。

use super::virtual_board::VirtualBoard;
use crate::attacks::AttackTables;
use crate::board::{Bitboard, Square};
use crate::mv::Move;
use crate::piece::Color;
use crate::position::Position;

/// 獅子の2段階移動・跳び・じっと(第12条)を生成する。1升移動で停止する着手は
/// 通常経路で生成済みのため含めない。
///
/// 隣接する相手駒の升だけを経由升とし、その駒を取った仮想盤面で、
/// 経由升から王将の1升移動で届く自駒以外の升へ進む手を出す。
/// 移動元へ戻る居喰いも含む。
/// 直接の跳びは中間升の占有を問わず、`lion_jumps`の升へ出す
/// （RULES.md第12条第5項から第7項）。
/// 隣接8升に空升があれば、じっとを全体で1手だけ出す（第12条第9項、第10項）。
/// 空升を経由する2段階移動は[`Move`]の正準形に従って生成しない。
/// この表現を採る理由は`docs/adr/represent-moves-in-canonical-form.md`にある。
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
