//! 反復規則R1・R2・R3(第31条)の局面出現履歴と裁定。

use std::collections::{HashMap, HashSet};

use crate::core::game::{DrawReason, GameResult, WinReason};
use crate::core::movegen::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::Color;
use crate::core::position::Position;
use crate::core::rules::RepetitionRule;

/// R1で同一局面を判定するキー。
///
/// 局面本体とP1成り権保留状態のZobrist値を別成分として保持する。
/// 各値の衝突は実用上無視できるものとし、完全な局面署名とは照合しない。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct R1Key {
    /// 盤面・手番・先獅子状態のZobrist値。
    position_zobrist: u64,
    /// P1成り権保留状態のZobrist値。
    rights_zobrist: u64,
}

impl R1Key {
    /// 局面からR1の同一局面キーを作る。
    fn from_position(position: &Position) -> Self {
        Self {
            position_zobrist: position.zobrist(),
            rights_zobrist: position.rights_zobrist(),
        }
    }
}

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

/// R1で追跡する1局面の出現状態。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct R1PositionState {
    /// この局面の出現回数。
    occurrences: u8,
    /// 最初に出現した時点の手数。
    first_ply: u32,
}

/// R1の局面出現履歴と双方の攻撃連続数。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct R1History {
    /// 出現済み局面ごとの出現状態。
    positions: HashMap<R1Key, R1PositionState>,
    /// 対局者ごとの、直近まで連続した攻撃的着手の数。
    consecutive_attacking_moves: [u32; 2],
}

impl R1History {
    /// 開始局面を第1回として記録した履歴を作る。
    fn new(position: &Position) -> Self {
        Self {
            positions: HashMap::from([(
                R1Key::from_position(position),
                R1PositionState {
                    occurrences: 1,
                    first_ply: 0,
                },
            )]),
            consecutive_attacking_moves: [0; 2],
        }
    }

    /// 着手後の局面を記録し、4回目の出現なら裁定結果を返す(第31条R1)。
    pub(crate) fn record_move(
        &mut self,
        position: &Position,
        ply: u32,
        mover: Color,
        is_attacking: bool,
    ) -> Option<GameResult> {
        self.consecutive_attacking_moves =
            updated_attacking_counters(self.consecutive_attacking_moves, mover, is_attacking);
        let state = self
            .positions
            .entry(R1Key::from_position(position))
            .or_insert(R1PositionState {
                occurrences: 0,
                first_ply: ply,
            });
        state.occurrences += 1;
        if state.occurrences < 4 {
            return None;
        }

        Some(r1_repetition_result(
            ply,
            state.first_ply,
            self.consecutive_attacking_moves,
        ))
    }

    /// 仮想着手が4回目の出現を生じさせる場合の裁定結果を、履歴を変更せずに返す。
    ///
    /// 詰み判定(第21条第3項c)が回避手の即時勝利を調べるために使う。
    pub(crate) fn candidate_result(
        &self,
        position: &Position,
        ply: u32,
        mover: Color,
        is_attacking: bool,
    ) -> Option<GameResult> {
        let state = self.positions.get(&R1Key::from_position(position))?;
        if u16::from(state.occurrences) + 1 < 4 {
            return None;
        }
        let candidate_ply = ply
            .checked_add(1)
            .expect("a game cannot exceed u32::MAX plies");
        let counters =
            updated_attacking_counters(self.consecutive_attacking_moves, mover, is_attacking);
        Some(r1_repetition_result(
            candidate_ply,
            state.first_ply,
            counters,
        ))
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
    fn new(position: &Position) -> Self {
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
    fn new(position: &Position) -> Self {
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

/// 採用中の反復規則に必要な履歴だけを保持する内部状態。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum RepetitionHistory {
    /// R1の出現履歴。
    R1(R1History),
    /// R2の既出集合。
    R2(R2History),
    /// R3の出現回数。
    R3(R3History),
}

impl RepetitionHistory {
    /// 採用規則に対応する履歴を、開始局面を第1回として作る。
    pub(crate) fn new(rule: RepetitionRule, position: &Position) -> Self {
        match rule {
            RepetitionRule::R1 => Self::R1(R1History::new(position)),
            RepetitionRule::R2 => Self::R2(R2History::new(position)),
            RepetitionRule::R3 => Self::R3(R3History::new(position)),
        }
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

/// 4回目の同一局面出現に対するR1の裁定結果を返す(第31条R1)。
///
/// 最初の出現から4回目までの自分の全着手が攻撃的着手であった対局者が
/// 一方だけならその側の負け、それ以外は引き分けとする。
pub(crate) fn r1_repetition_result(
    ply: u32,
    first_ply: u32,
    consecutive_attacking_moves: [u32; 2],
) -> GameResult {
    let attackers = Color::ALL.map(|color| {
        // 最初の出現以降にその対局者が指した手数を、攻撃連続数が覆うかで判定する。
        let distance =
            moves_by_color_through(ply, color) - moves_by_color_through(first_ply, color);
        consecutive_attacking_moves[color.index()] >= distance
    });

    match attackers {
        [true, false] => GameResult::Win {
            winner: Color::White,
            reason: WinReason::Repetition,
        },
        [false, true] => GameResult::Win {
            winner: Color::Black,
            reason: WinReason::Repetition,
        },
        [false, false] | [true, true] => GameResult::Draw {
            reason: DrawReason::Repetition,
        },
    }
}

/// 着手側の攻撃連続数を、攻撃的着手なら1増やし、そうでなければ0へ戻す。
pub(crate) fn updated_attacking_counters(
    mut counters: [u32; 2],
    mover: Color,
    is_attacking: bool,
) -> [u32; 2] {
    let counter = &mut counters[mover.index()];
    if is_attacking {
        *counter = counter
            .checked_add(1)
            .expect("an attack sequence cannot exceed u32::MAX plies");
    } else {
        *counter = 0;
    }
    counters
}

/// 指定手数までに指定対局者が指した着手数を返す。先手が奇数手目を指す。
fn moves_by_color_through(ply: u32, color: Color) -> u32 {
    match color {
        Color::Black => ply / 2 + ply % 2,
        Color::White => ply / 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn article_31_r1_attacking_moves_extend_the_run_and_others_reset_it() {
        // D3-031-05: 攻撃連続数の更新は共有の純粋関数で行い、攻撃的着手は
        // 手番側の連続数を1増やし、非攻撃的着手は0へ戻す。相手側の値は
        // 変更しない(adjudication-refactor.md「R1の攻撃連続数」)。
        assert_eq!(
            updated_attacking_counters([2, 5], Color::Black, true),
            [3, 5]
        );
        assert_eq!(
            updated_attacking_counters([2, 5], Color::Black, false),
            [0, 5]
        );
        assert_eq!(
            updated_attacking_counters([2, 5], Color::White, true),
            [2, 6]
        );
        assert_eq!(
            updated_attacking_counters([2, 5], Color::White, false),
            [2, 0]
        );
        assert_eq!(
            updated_attacking_counters([0, 0], Color::Black, true),
            [1, 0]
        );
    }
}
