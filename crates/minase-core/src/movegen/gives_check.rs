//! 着手が王手になるかの判定。

use super::MoveGenerator;
use super::control::piece_control_with_occupancy;
use crate::board::Bitboard;
use crate::mv::Move;
use crate::position::Position;

impl Position {
    /// `mv`を指した後に、着手側が相手の残存王駒を次の1手ですべて取れるかを返す。
    ///
    /// 相手の王駒が1枚なら、その王駒へ疑似利きが届くことと捕獲できることが一致する
    /// （RULES.md第8条）。王将または玉将と太子の2枚なら、両方を取る合法手を要求する。
    /// `mv`で相手の王駒がなくなる場合は王手ではない。
    ///
    /// `mv`は、まさにこの局面に対して`generator`が生成した合法手でなければならない。
    /// このメソッドは`self`を変更しない。
    pub fn gives_check(&self, generator: &MoveGenerator, mv: Move) -> bool {
        let mover = self.side_to_move();
        let captured = self.captured_squares(mv);
        let mut remaining_royals = self.royal_pieces(mover.opposite());
        for square in captured.into_iter().flatten() {
            remaining_royals.clear(square);
        }

        match remaining_royals.popcount() {
            0 => false,
            1 => {
                let royal = remaining_royals
                    .lsb()
                    .expect("one remaining royal must have a square");
                let mut occupied = self.occupied();
                occupied.clear(mv.from);
                if let Some(mid_capture) = captured[0] {
                    occupied.clear(mid_capture);
                }
                occupied.set(mv.to);

                let moving_piece = self
                    .piece_at(mv.from)
                    .expect("move origin must contain a piece");
                let moving_kind = if mv.promote {
                    moving_piece
                        .promote()
                        .expect("promoting move must have a promotable piece")
                        .kind()
                        .expect("promoted board piece must have a kind")
                } else {
                    moving_piece
                        .kind()
                        .expect("board piece must have a valid kind")
                };
                if piece_control_with_occupancy(occupied, mover, moving_kind, mv.to).contains(royal)
                {
                    return true;
                }

                let mut other_attackers = self.attackers_to_by(mover, royal, occupied);
                other_attackers.clear(mv.from);
                !other_attackers.is_empty()
            }
            2 => {
                let first = remaining_royals
                    .lsb()
                    .expect("two remaining royals must have a first square");
                let second = remaining_royals
                    .msb()
                    .expect("two remaining royals must have a second square");
                if !generator.tables().king_steps(first).contains(second) {
                    return false;
                }

                let mut after = self.clone();
                after.make_move_unchecked(mv, generator.rules());
                let checking_position = after.clone_with_side_to_move(mover);
                can_capture_all_royals(&checking_position, generator, remaining_royals)
            }
            // 1手の最大捕獲数は2枚なので、到達不能な構築局面に3枚以上あっても王手ではない。
            _ => false,
        }
    }
}

/// 手番側に`royals`の全駒を1手で取る合法手があるかを返す。
fn can_capture_all_royals(
    position: &Position,
    generator: &MoveGenerator,
    royals: Bitboard,
) -> bool {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    moves.into_iter().any(|mv| {
        let captured = position.captured_squares(mv);
        royals
            .into_iter()
            .all(|royal| captured.contains(&Some(royal)))
    })
}
