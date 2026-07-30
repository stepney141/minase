#[cfg(test)]
mod adversarial;
mod legal;
mod lion;
mod normal;
#[cfg(test)]
mod verification;

use crate::attacks::{AttackTables, attack_tables};
use crate::bitboard::Bitboard;
use crate::mv::Move;
use crate::piece::{Color, PieceKind};
use crate::position::Position;
use crate::rules::Rules;
use crate::square::Square;

pub use legal::IllegalMove;

pub struct MoveGenerator<'a> {
    tables: &'a AttackTables,
    rules: Rules,
}

impl MoveGenerator<'static> {
    pub fn standard() -> Self {
        Self::new(Rules::standard())
    }
}

impl<'a> MoveGenerator<'a> {
    pub fn new(rules: Rules) -> Self {
        Self {
            tables: attack_tables(),
            rules,
        }
    }

    pub fn with_tables(tables: &'a AttackTables, rules: Rules) -> Self {
        Self { tables, rules }
    }

    pub fn generate_moves(&self, position: &Position, output: &mut Vec<Move>) {
        normal::generate_moves(self, position, output);
    }

    /// Counts legal-move paths to `depth` and restores `position` before returning.
    pub fn perft(&self, position: &mut Position, depth: u32) -> u64 {
        if depth == 0 {
            return 1;
        }

        let mut moves = Vec::new();
        self.generate_moves(position, &mut moves);
        moves
            .into_iter()
            .map(|mv| {
                let undo = position.make_move_unchecked(mv);
                let nodes = self.perft(position, depth - 1);
                position.unmake_move(undo);
                nodes
            })
            .sum()
    }

    pub(crate) const fn tables(&self) -> &AttackTables {
        self.tables
    }

    pub(crate) const fn rules(&self) -> Rules {
        self.rules
    }
}

pub(crate) fn piece_control_with_occupancy(
    occupied: Bitboard,
    color: Color,
    kind: PieceKind,
    from: Square,
) -> crate::Bitboard {
    normal::piece_control_with_occupancy(attack_tables(), occupied, color, kind, from)
}
