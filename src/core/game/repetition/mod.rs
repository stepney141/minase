//! 反復規則R1・R2・R3に応じた履歴の切り替え。

mod prohibition;
mod r1;

use crate::core::position::Position;
use crate::core::rules::RepetitionRule;
use prohibition::{R2History, R3History};
pub(crate) use prohibition::{repetition_is_forbidden, retain_repetition_allowed_moves};
use r1::R1History;
pub(crate) use r1::{move_is_irreversible, move_was_attacking};

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
