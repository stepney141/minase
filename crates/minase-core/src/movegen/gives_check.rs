//! 着手が王手になるかの判定。

use super::MoveGenerator;
use super::control::piece_control_with_occupancy;
use crate::attacks::{AttackTables, SpecialMovement, movement_profile, movement_profile_data};
use crate::board::{Bitboard, Square};
use crate::mv::Move;
use crate::piece::{Color, PieceKind};
use crate::position::Position;

impl Position {
    /// 手番側の最後の王駒を、相手が次の1手で取れるかを返す（第8条）。
    /// 王駒が2枚なら両方を取る合法手を要求し、0枚または3枚以上なら王手ではない。
    pub fn is_in_check(&self, generator: &MoveGenerator) -> bool {
        let attacker = self.side_to_move().opposite();
        let royals = self.royal_pieces(self.side_to_move());
        match royals.popcount() {
            1 => !self
                .attackers_to_by(
                    attacker,
                    royals.lsb().expect("one royal must have a square"),
                    self.occupied(),
                )
                .is_empty(),
            2 => can_capture_all_royals(&self.clone_with_side_to_move(attacker), generator, royals),
            _ => false,
        }
    }

    /// `mv`を指した後に、着手側が相手の残存王駒を次の1手ですべて取れるかを返す。
    ///
    /// 相手の王駒が1枚なら、その王駒へ疑似利きが届くことと捕獲できることが一致する
    /// （chu-shogi.md第8条）。王将または玉将と太子の2枚なら、両方を取る合法手を要求する。
    /// `mv`で相手の王駒がなくなる場合は王手ではない。
    ///
    /// `mv`は、まさにこの局面に対して`generator`が生成した合法手でなければならない。
    /// このメソッドは`self`を変更しない。
    pub fn gives_check(&self, generator: &MoveGenerator, mv: Move) -> bool {
        CheckContext::default().gives_check(self, generator, mv)
    }
}

/// 同じ局面の候補手で、現在の王駒への利きを遅延計算して共有する文脈。
/// 子局面を探索した後は元の局面へ戻して使い、別局面には新しい文脈を作る。
#[derive(Default)]
pub struct CheckContext {
    known_royals: Bitboard,
    attacked_royals: Bitboard,
}

impl CheckContext {
    /// [`Position::gives_check`]と同じ判定を、同じ局面の候補手に繰り返し適用する。
    /// 固定利きと特殊移動は5×5近傍、走りの遮蔽は8方向の利き線の中だけに依存するので、
    /// 移動元・経由捕獲升・到達升がその和集合の外なら、着手前の逆引きの結果を再利用できる。
    pub fn gives_check(
        &mut self,
        position: &Position,
        generator: &MoveGenerator,
        mv: Move,
    ) -> bool {
        let mover = position.side_to_move();
        let captured = position.captured_squares(mv);
        let mut remaining_royals = position.royal_pieces(mover.opposite());
        for square in captured.into_iter().flatten() {
            remaining_royals.clear(square);
        }

        match remaining_royals.popcount() {
            0 => false,
            1 => {
                let royal = remaining_royals
                    .lsb()
                    .expect("one remaining royal must have a square");
                let affected = generator.tables().attack_dependencies(royal);
                if !affected.contains(mv.from)
                    && !affected.contains(mv.to)
                    && !captured[0].is_some_and(|mid| affected.contains(mid))
                {
                    if !self.known_royals.contains(royal) {
                        if !position
                            .attackers_to_by(mover, royal, position.occupied())
                            .is_empty()
                        {
                            self.attacked_royals.set(royal);
                        }
                        self.known_royals.set(royal);
                    }
                    return self.attacked_royals.contains(royal);
                }
                let mut occupied = position.occupied();
                occupied.clear(mv.from);
                if let Some(mid_capture) = captured[0] {
                    occupied.clear(mid_capture);
                }
                occupied.set(mv.to);

                let moving_piece = position
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
                if directly_attacks_royal(
                    generator.tables(),
                    occupied,
                    mover,
                    moving_kind,
                    mv.to,
                    royal,
                ) {
                    return true;
                }

                let mut other_attackers = position.attackers_to_by(mover, royal, occupied);
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

                let mut after = position.clone();
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

/// 固定利きと遮蔽なしの範囲で直接王手を絞り、王駒を含む走り線だけ遮蔽を調べる。
/// 特殊移動は専用の利き計算を使い、すべての駒で通常の疑似利きと同じ判定を返す。
fn directly_attacks_royal(
    tables: &AttackTables,
    occupied: Bitboard,
    color: Color,
    kind: PieceKind,
    from: Square,
    royal: Square,
) -> bool {
    let id = movement_profile(kind);
    let profile = movement_profile_data(id);
    if !matches!(profile.special, SpecialMovement::None) {
        return piece_control_with_occupancy(occupied, color, kind, from).contains(royal);
    }
    if tables.fixed(color, id, from).contains(royal) {
        return true;
    }
    if !tables.reach(color, id, from).contains(royal) {
        return false;
    }
    profile.slides.iter().any(|slide| {
        let direction = slide.direction.for_color(color);
        tables.ray(from, direction).contains(royal)
            && tables
                .sliding_control(from, direction, occupied)
                .contains(royal)
    })
}
