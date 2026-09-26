//! 反復規則R2・R3の局面出現履歴と着手の禁止。

use super::RepetitionHistory;
use crate::core::movegen::MoveGenerator;
use crate::core::mv::Move;
use crate::core::position::Position;
use std::collections::{HashMap, HashSet};

/// R2とR3で同一局面を判定するキー。
///
/// P1成り権保留状態を除外し、局面本体のZobrist値だけを保持する
/// (第24条第1項d)。値の衝突は実用上無視できるものとし、完全な局面署名とは
/// 照合しない。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct R2R3Key(u64);

impl R2R3Key {
    /// 局面からR2・R3の同一局面キーを作る。
    fn from_position(position: &Position) -> Self {
        Self(position.zobrist())
    }
}

/// R2の既出局面集合。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct R2History {
    /// 出現済み局面のキー集合。
    positions: HashSet<R2R3Key>,
}

impl R2History {
    /// 開始局面を既出として記録した履歴を作る。
    pub(super) fn new(position: &Position) -> Self {
        Self {
            positions: HashSet::from([R2R3Key::from_position(position)]),
        }
    }

    /// 着手後の局面を既出集合へ記録する。
    pub(crate) fn record(&mut self, position: &Position) {
        let inserted = self.positions.insert(R2R3Key::from_position(position));
        debug_assert!(inserted, "R2 must reject repeated positions");
    }
}

/// R3の局面出現回数。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct R3History {
    /// 出現済み局面ごとの出現回数。
    positions: HashMap<R2R3Key, u8>,
}

impl R3History {
    /// 開始局面を第1回として記録した履歴を作る。
    pub(super) fn new(position: &Position) -> Self {
        Self {
            positions: HashMap::from([(R2R3Key::from_position(position), 1)]),
        }
    }

    /// 着手後の局面の出現回数を1増やす。
    pub(crate) fn record(&mut self, position: &Position) {
        let occurrences = self
            .positions
            .entry(R2R3Key::from_position(position))
            .or_insert(0);
        *occurrences = occurrences
            .checked_add(1)
            .expect("a position cannot occur more than u8::MAX times");
    }
}

/// 着手後の局面が採用中のR2またはR3で禁止されるかを返す。
pub(crate) fn repetition_is_forbidden(history: &RepetitionHistory, position: &Position) -> bool {
    match history {
        RepetitionHistory::R1(_) => false,
        RepetitionHistory::R2(history) => history
            .positions
            .contains(&R2R3Key::from_position(position)),
        RepetitionHistory::R3(history) => history
            .positions
            .get(&R2R3Key::from_position(position))
            .is_some_and(|&occurrences| occurrences >= 3),
    }
}

/// R2またはR3が禁止する着手を候補列から除く。
///
/// 各候補を一時的に適用して同一の禁止判定を呼び、判定後に局面を復元する。
pub(crate) fn retain_repetition_allowed_moves(
    position: &mut Position,
    generator: &MoveGenerator,
    history: &RepetitionHistory,
    moves: &mut Vec<Move>,
) {
    if matches!(history, RepetitionHistory::R1(_)) {
        return;
    }

    moves.retain(|candidate| {
        let undo = position.make_move_unchecked(*candidate, generator.rules());
        let allowed = !repetition_is_forbidden(history, position);
        position.unmake_move(undo);
        allowed
    });
}
