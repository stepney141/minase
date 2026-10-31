//! 手番側から見た駒種ごとの疑似利きと利き数を64面へ符号化する。

use minase_core::piece::PIECE_KIND_COUNT;
use minase_core::{BOARD_SQUARE_COUNT, Color, Position};

use super::relative_square;

const PLANES_PER_COLOR: usize = PIECE_KIND_COUNT + 3;
/// 両陣営の駒種ごとの利き29面と、利き数の閾値3面からなる平面数。
pub const CONTROL_PLANES: usize = 2 * PLANES_PER_COLOR;
/// 利きの64面×144升の要素数。
pub const CONTROL_SIZE: usize = CONTROL_PLANES * BOARD_SQUARE_COUNT;

/// 局面の疑似利きを、入力の面95〜158に対応する64面へ書き出す。
///
/// `out`を0で消去し、相対陣営`r`の駒種`k`の利きは面`32*r+k`へ1を置く。
/// 手番側は陣営0、相手は陣営1とし、駒種は成った後の[`minase_core::PieceKind`]の番号を使う。
/// 面`32*r+29+(c-1)`は、その升へ利く駒が`c`枚以上なら1とする（`c=1,2,3`）。
/// 疑似利きは[`Position::piece_control_with_occupancy`]に従い、味方の駒がある升も含む。
/// 獅子の捕獲制限は判定せず、同じ駒から複数の経路で届いても1枚と数える。
/// 並びは`[面][升]`で、後手番では盤を180度回転する。値は0.0か1.0だけである。
///
/// # Panics
/// `out.len()`が[`CONTROL_SIZE`]でなければパニックする。
pub fn encode_control(position: &Position, out: &mut [f32]) {
    assert_eq!(out.len(), CONTROL_SIZE);
    out.fill(0.0);
    let occupied = position.occupied();
    let side = position.side_to_move();
    for color in Color::ALL {
        let base = usize::from(color != side) * PLANES_PER_COLOR;
        let mut counts = [0_u8; BOARD_SQUARE_COUNT];
        for from in position.pieces_of(color) {
            let kind = position
                .piece_at(from)
                .and_then(|piece| piece.kind())
                .expect("occupied square has a piece kind");
            for target in Position::piece_control_with_occupancy(occupied, color, kind, from) {
                let square = relative_square(target, side);
                out[(base + kind.index()) * BOARD_SQUARE_COUNT + square] = 1.0;
                counts[square] += 1;
            }
        }
        for (square, count) in counts.into_iter().enumerate() {
            for threshold in 1..=3 {
                if usize::from(count) >= threshold {
                    out[(base + PIECE_KIND_COUNT + threshold - 1) * BOARD_SQUARE_COUNT + square] =
                        1.0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
