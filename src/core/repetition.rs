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
    position_zobrist: u64,
    rights_zobrist: u64,
}

impl R1Key {
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
    fn from_position(position: &Position) -> Self {
        Self(position.zobrist())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct R1PositionState {
    occurrences: u8,
    first_ply: u32,
}

/// R1の局面出現履歴と双方の攻撃連続数。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct R1History {
    positions: HashMap<R1Key, R1PositionState>,
    consecutive_attacking_moves: [u32; 2],
}

impl R1History {
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

    #[cfg(test)]
    pub(crate) fn occurrences(&self, position: &Position) -> Option<u8> {
        self.positions
            .get(&R1Key::from_position(position))
            .map(|state| state.occurrences)
    }

    #[cfg(test)]
    pub(crate) const fn consecutive_attacking_moves(&self) -> [u32; 2] {
        self.consecutive_attacking_moves
    }

    #[cfg(test)]
    pub(crate) fn set_state(&mut self, position: &Position, occurrences: u8, first_ply: u32) {
        assert_eq!(
            self.positions.insert(
                R1Key::from_position(position),
                R1PositionState {
                    occurrences,
                    first_ply,
                },
            ),
            None
        );
    }

    #[cfg(test)]
    pub(crate) fn set_attacking_counters(&mut self, counters: [u32; 2]) {
        self.consecutive_attacking_moves = counters;
    }
}

/// R2の既出局面集合。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct R2History {
    positions: HashSet<R2R3Key>,
}

impl R2History {
    fn new(position: &Position) -> Self {
        Self {
            positions: HashSet::from([R2R3Key::from_position(position)]),
        }
    }

    pub(crate) fn record(&mut self, position: &Position) {
        let inserted = self.positions.insert(R2R3Key::from_position(position));
        debug_assert!(inserted, "R2 must reject repeated positions");
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, position: &Position) -> bool {
        self.positions.contains(&R2R3Key::from_position(position))
    }
}

/// R3の局面出現回数。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct R3History {
    positions: HashMap<R2R3Key, u8>,
}

impl R3History {
    fn new(position: &Position) -> Self {
        Self {
            positions: HashMap::from([(R2R3Key::from_position(position), 1)]),
        }
    }

    pub(crate) fn record(&mut self, position: &Position) {
        let occurrences = self
            .positions
            .entry(R2R3Key::from_position(position))
            .or_insert(0);
        *occurrences = occurrences
            .checked_add(1)
            .expect("a position cannot occur more than u8::MAX times");
    }

    #[cfg(test)]
    pub(crate) fn occurrences(&self, position: &Position) -> Option<u8> {
        self.positions
            .get(&R2R3Key::from_position(position))
            .copied()
    }
}

/// 採用中の反復規則に必要な履歴だけを保持する内部状態。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum RepetitionHistory {
    R1(R1History),
    R2(R2History),
    R3(R3History),
}

impl RepetitionHistory {
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

pub(crate) fn r1_repetition_result(
    ply: u32,
    first_ply: u32,
    consecutive_attacking_moves: [u32; 2],
) -> GameResult {
    let attackers = Color::ALL.map(|color| {
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

fn moves_by_color_through(ply: u32, color: Color) -> u32 {
    match color {
        Color::Black => ply / 2 + ply % 2,
        Color::White => ply / 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::piece::{PieceCode, PieceKind};
    use crate::core::position::PositionBuilder;
    use crate::test_util::sq;

    #[test]
    fn r1_and_r2_r3_keys_keep_required_components_separate() {
        assert_ne!(
            R1Key {
                position_zobrist: 1,
                rights_zobrist: 2,
            },
            R1Key {
                position_zobrist: 2,
                rights_zobrist: 1,
            }
        );

        let deferred = sq(4, 9);
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(
                deferred,
                PieceCode::new(Color::Black, PieceKind::SilverGeneral),
            )
            .unwrap();
        builder.mark_promotion_deferred(deferred).unwrap();
        let position = builder.finish().unwrap();

        let r1 = R1Key::from_position(&position);
        let r2_r3 = R2R3Key::from_position(&position);

        assert_ne!(position.rights_zobrist(), 0);
        assert_eq!(r1.position_zobrist, position.zobrist());
        assert_eq!(r1.rights_zobrist, position.rights_zobrist());
        assert_eq!(r2_r3, R2R3Key(position.zobrist()));
    }
}
