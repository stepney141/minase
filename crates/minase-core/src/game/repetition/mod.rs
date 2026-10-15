//! 反復規則R1・R2・R3に応じた履歴の切り替え。

mod prohibition;
mod r1;

use crate::position::Position;
use crate::rules::RepetitionRule;
use prohibition::{R2History, R3History};
pub(crate) use prohibition::{repetition_is_forbidden, retain_repetition_allowed_moves};
use r1::R1History;
pub(crate) use r1::move_is_irreversible;
pub use r1::move_was_attacking;

/// 採用中の反復規則の同一局面のキーを局面から作る（RULES.md第24条第1項、第31条）。
///
/// R1は盤面・手番・先獅子状態の[`Position::zobrist`]に成り権保留状態の[`Position::rights_zobrist`]を
/// 合成する（第24条第1項aからd）。R2とR3は成り権保留状態を含めず`zobrist`だけを使う（第31条R2・R3の補充規定）。
pub fn repetition_key(rule: RepetitionRule, position: &Position) -> u64 {
    match rule {
        RepetitionRule::R1 => position.zobrist() ^ position.rights_zobrist(),
        RepetitionRule::R2 | RepetitionRule::R3 => position.zobrist(),
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
    /// R1の攻撃連続数を返し、R2とR3では`None`を返す。
    pub(crate) const fn consecutive_attacking_moves(&self) -> Option<[u32; 2]> {
        match self {
            Self::R1(history) => Some(history.consecutive_attacking_moves()),
            Self::R2(_) | Self::R3(_) => None,
        }
    }

    /// 採用規則に対応する履歴を、開始局面を第1回として作る。
    pub(crate) fn new(rule: RepetitionRule, position: &Position) -> Self {
        match rule {
            RepetitionRule::R1 => Self::R1(R1History::new(position)),
            RepetitionRule::R2 => Self::R2(R2History::new(position)),
            RepetitionRule::R3 => Self::R3(R3History::new(position)),
        }
    }
}
