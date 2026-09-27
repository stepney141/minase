//! 着手の順序付けと手の成績の更新。

use core::cmp::Reverse;

use crate::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::PieceCode;
use crate::core::position::Position;
use crate::eval::Pst;

use super::params;
use super::searcher::{HistoryTable, KILLER_COUNT, Searcher};

impl Searcher<'_> {
    /// 捕獲手、killer手、history値の順で着手を整列し、置換表の手を先頭へ置く。
    pub(super) fn order_moves(
        &self,
        position: &Position,
        moves: &mut [Move],
        tt_move: Option<Move>,
        ply: u32,
    ) {
        let killers = self.killers[ply as usize];
        let color = position.side_to_move().index();
        moves.sort_by_cached_key(|&mv| {
            if let Some(key) = move_order_key(position, self.pst, mv) {
                OrderedMoveKey::Capture {
                    captured_value: Reverse(key.captured_value),
                    attacker_value: key.attacker_value,
                }
            } else if Some(mv) == killers[0] {
                OrderedMoveKey::Killer(0)
            } else if Some(mv) == killers[1] {
                OrderedMoveKey::Killer(1)
            } else {
                OrderedMoveKey::Quiet(Reverse(
                    self.history[color][mv.from.dense_index()][mv.to.dense_index()],
                ))
            }
        });
        if let Some(index) = tt_move.and_then(|tt_move| moves.iter().position(|&mv| mv == tt_move))
        {
            moves.swap(0, index);
        }
    }

    /// βカットを起こした非捕獲手をkiller表とhistory表へ記録する。
    pub(super) fn record_quiet_beta_cutoff(
        &mut self,
        position: &Position,
        mv: Move,
        depth: u32,
        ply: u32,
    ) {
        let killers = &mut self.killers[ply as usize];
        if killers[0] != Some(mv) {
            killers[1] = killers[0];
            killers[0] = Some(mv);
        }

        let color = position.side_to_move().index();
        let from = mv.from.dense_index();
        let to = mv.to.dense_index();
        self.history[color][from][to] += (depth * depth) as i32;
        if self.history[color][from][to] > params::history_limit() {
            for color_history in self.history.iter_mut() {
                for from_history in color_history.iter_mut() {
                    for value in from_history.iter_mut() {
                        *value /= 2;
                    }
                }
            }
        }
    }
}

/// 捕獲手と整列キーのペアをMVV-LVA順で安定に整列する参照実装。
#[cfg(test)]
pub(super) fn order_captures(captures: &mut [(Move, MoveOrderKey)]) {
    captures.sort_by_key(|&(_, key)| (Reverse(key.captured_value), key.attacker_value));
}

/// 通常探索で使う着手の整列キー。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum OrderedMoveKey {
    /// 捕獲価値の降順、攻撃駒価値の昇順で並べる捕獲手。
    Capture {
        /// 取る駒の駒価値の合計。
        captured_value: Reverse<i32>,
        /// 動かす駒の駒価値。
        attacker_value: i32,
    },
    /// 添字が小さいほど新しいkiller手。
    Killer(usize),
    /// History値の降順で並べる残りの非捕獲手。
    Quiet(Reverse<i32>),
}

/// 通常探索で手を返す段階。
#[derive(Clone, Copy)]
pub(super) enum MovePickerStage {
    Tt,
    Captures,
    Killer0,
    Killer1,
    Quiets,
    Done,
}

/// TT手、捕獲手、killer手、静かな手の順に合法手を1回ずつ返す。
pub(super) struct MovePicker {
    stage: MovePickerStage,
    tt_move: Option<Move>,
    killers: [Option<Move>; KILLER_COUNT],
    captures: Vec<(Move, MoveOrderKey)>,
    capture_index: usize,
    captures_generated: bool,
    quiets: Vec<Move>,
    quiet_index: usize,
    base_moves: Vec<Move>,
    quiet_order: Vec<(Reverse<i32>, usize)>,
    used_quiets: [Option<usize>; KILLER_COUNT],
    quiets_generated: bool,
}

impl MovePicker {
    /// 助言手を保持した空の手選択器を作る。
    pub(super) fn new(tt_move: Option<Move>, killers: [Option<Move>; KILLER_COUNT]) -> Self {
        Self {
            stage: MovePickerStage::Tt,
            tt_move,
            killers,
            captures: Vec::new(),
            capture_index: 0,
            captures_generated: false,
            quiets: Vec::new(),
            quiet_index: 0,
            base_moves: Vec::new(),
            quiet_order: Vec::new(),
            used_quiets: [None; KILLER_COUNT],
            quiets_generated: false,
        }
    }

    /// 「段階6」（movegen-speedup-2.md）に従い、確保した領域を保って次のノードへ進む。
    pub(super) fn reset(&mut self, tt_move: Option<Move>, killers: [Option<Move>; KILLER_COUNT]) {
        self.stage = MovePickerStage::Tt;
        self.tt_move = tt_move;
        self.killers = killers;
        self.captures.clear();
        self.capture_index = 0;
        self.captures_generated = false;
        self.quiets.clear();
        self.quiet_index = 0;
        self.quiets_generated = false;
        self.quiet_order.clear();
        self.used_quiets = [None; KILLER_COUNT];
    }

    /// 現在の段階で次に探索する合法手と、捕獲手かどうかの組を返す。
    pub(super) fn next(
        &mut self,
        position: &Position,
        pst: &Pst,
        generator: &MoveGenerator,
        history: &HistoryTable,
    ) -> Option<(Move, bool)> {
        loop {
            match self.stage {
                MovePickerStage::Tt => {
                    self.stage = MovePickerStage::Captures;
                    if let Some(tt_move) = self.tt_move
                        && generator.is_legal_move(
                            position,
                            tt_move,
                            &mut self.base_moves,
                            &mut self.quiets,
                        )
                    {
                        return Some((tt_move, move_order_key(position, pst, tt_move).is_some()));
                    }
                }
                MovePickerStage::Captures => {
                    if !self.captures_generated {
                        self.quiets.clear();
                        generator.generate_captures_with_scratch(
                            position,
                            &mut self.base_moves,
                            &mut self.quiets,
                        );
                        self.captures.extend(
                            self.quiets
                                .drain(..)
                                .filter(|&mv| Some(mv) != self.tt_move)
                                .map(|mv| {
                                    let key = move_order_key(position, pst, mv)
                                        .expect("capture generator must not return a quiet move");
                                    (mv, key)
                                }),
                        );
                        self.captures.sort_by_key(|&(_, key)| {
                            (Reverse(key.captured_value), key.attacker_value)
                        });
                        self.captures_generated = true;
                    }
                    if let Some(&(mv, _)) = self.captures.get(self.capture_index) {
                        self.capture_index += 1;
                        return Some((mv, true));
                    }
                    self.stage = MovePickerStage::Killer0;
                }
                MovePickerStage::Killer0 | MovePickerStage::Killer1 => {
                    if !self.quiets_generated {
                        generator.generate_quiets(position, &mut self.base_moves, &mut self.quiets);
                        self.quiets.retain(|&mv| Some(mv) != self.tt_move);
                        self.quiets_generated = true;
                    }
                    let killer_index = usize::from(matches!(self.stage, MovePickerStage::Killer1));
                    self.stage = if killer_index == 0 {
                        MovePickerStage::Killer1
                    } else {
                        MovePickerStage::Quiets
                    };
                    if let Some(killer) = self.killers[killer_index]
                        && let Some(index) =
                            self.quiets.iter().enumerate().find_map(|(index, &mv)| {
                                (mv == killer && !self.used_quiets.contains(&Some(index)))
                                    .then_some(index)
                            })
                    {
                        self.used_quiets[killer_index] = Some(index);
                        return Some((self.quiets[index], false));
                    }
                }
                MovePickerStage::Quiets => {
                    let color = position.side_to_move().index();
                    self.quiet_order.extend(
                        self.quiets
                            .iter()
                            .enumerate()
                            .filter(|(index, _)| !self.used_quiets.contains(&Some(*index)))
                            .map(|(index, mv)| {
                                (
                                    Reverse(
                                        history[color][mv.from.dense_index()][mv.to.dense_index()],
                                    ),
                                    index,
                                )
                            }),
                    );
                    self.quiet_order.sort_unstable();
                    self.stage = MovePickerStage::Done;
                }
                MovePickerStage::Done => {
                    if let Some(&(_, index)) = self.quiet_order.get(self.quiet_index) {
                        self.quiet_index += 1;
                        return Some((self.quiets[index], false));
                    }
                    return None;
                }
            }
        }
    }
}

/// 捕獲手の整列キー。
#[derive(Clone, Copy)]
pub(super) struct MoveOrderKey {
    /// 取る駒の駒価値の合計。
    pub(super) captured_value: i32,
    /// 動かす駒の駒価値。
    pub(super) attacker_value: i32,
}

/// 捕獲手なら整列キーを返す。非捕獲手は`None`を返す。
pub(super) fn move_order_key(position: &Position, pst: &Pst, mv: Move) -> Option<MoveOrderKey> {
    let captured_value: i32 = position
        .captured_squares(mv)
        .into_iter()
        .flatten()
        .map(|square| pst.piece_value(piece_at_for_ordering(position, square)))
        .sum();
    (captured_value > 0).then(|| MoveOrderKey {
        captured_value,
        attacker_value: pst.piece_value(piece_at_for_ordering(position, mv.from)),
    })
}

/// 指定升の駒コードを返す。
///
/// # Panics
///
/// 升に駒がない場合にパニックする。
pub(super) fn piece_at_for_ordering(position: &Position, square: crate::Square) -> PieceCode {
    position
        .piece_at(square)
        .expect("move ordering square must contain a piece")
}
