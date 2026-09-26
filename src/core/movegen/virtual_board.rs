//! 着手途中と着手後の占有状態を表す仮想盤面。

use crate::core::board::{Bitboard, Square};
use crate::core::mv::Move;
use crate::core::piece::Color;
use crate::core::position::Position;

/// 現局面に対する着手途中または着手後の占有状態を差分で表す仮想盤面。
/// 獅子などの2段階移動の生成と、相手獅子を取った直後の仮想的な盤面
/// (第13条第4項)での足の判定に使う。
#[derive(Clone, Copy)]
pub(crate) struct VirtualBoard {
    /// 駒がある升の集合。
    pub(crate) occupied: Bitboard,
    /// 動かす側の駒の集合。
    pub(crate) own: Bitboard,
    /// 相手側の駒の集合。
    pub(crate) enemy: Bitboard,
    /// 動かしている駒の現在升。
    current: Square,
}

impl VirtualBoard {
    /// 現局面から占有状態を作る。
    pub(crate) fn new(position: &Position, color: Color, current: Square) -> Self {
        Self {
            occupied: position.occupied(),
            own: position.pieces_of(color),
            enemy: position.pieces_of(color.opposite()),
            current,
        }
    }

    /// 着手を2段階とも適用した後の仮想盤面を作る。
    pub(crate) fn after_move(position: &Position, mv: Move) -> Self {
        let color = position
            .piece_at(mv.from)
            .and_then(|piece| piece.color())
            .expect("move origin must contain a piece");
        let board = Self::new(position, color, mv.from);

        if let Some(mid) = mv.mid {
            board.move_to(mid).move_to(mv.to)
        } else {
            board.move_to(mv.to)
        }
    }

    /// 駒を1段階動かした後の状態を返す。到達升にある相手駒は取り除く。
    pub(super) fn move_to(mut self, to: Square) -> Self {
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
