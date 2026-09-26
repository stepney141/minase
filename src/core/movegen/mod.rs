//! 合法手の生成。

#[cfg(test)]
pub(crate) mod tests;

mod checked;
mod control;
mod expand;
mod generate;
mod lion;
mod lion_capture;
mod lion_like;
mod search_captures;
mod virtual_board;

pub use checked::IllegalMove;
pub(crate) use search_captures::{CaptureCache, CaptureCandidate, OrdinaryCapturer};

use crate::core::attacks::{AttackTables, attack_tables};
use crate::core::mv::Move;
use crate::core::position::Position;
use crate::core::rules::MoveRules;

/// 採用ルールの下で合法手を列挙する生成器。
#[derive(Clone)]
pub struct MoveGenerator {
    /// 利きの前計算テーブル。
    tables: &'static AttackTables,
    /// 採用しているローカルルールの集合。
    rules: MoveRules,
}

impl MoveGenerator {
    /// 標準規則の生成器を作る。
    pub fn standard() -> Self {
        Self::new(MoveRules::standard())
    }

    /// 指定ルールの生成器を作る。
    pub fn new(rules: MoveRules) -> Self {
        Self {
            tables: attack_tables(),
            rules,
        }
    }

    /// `position`の手番側の全合法手を`output`へ追加する。
    pub fn generate_moves(&self, position: &Position, output: &mut Vec<Move>) {
        generate::generate_moves::<false, false>(self, position, &mut Vec::new(), output);
    }

    /// `position`の手番側の捕獲を伴う合法手を`output`へ追加する。
    pub fn generate_captures(&self, position: &Position, output: &mut Vec<Move>) {
        self.generate_captures_with_scratch(position, &mut Vec::new(), output);
    }

    /// 「段階6」（movegen-speedup-2.md）の捕獲生成用領域を再利用する。
    pub(crate) fn generate_captures_with_scratch(
        &self,
        position: &Position,
        base_moves: &mut Vec<Move>,
        output: &mut Vec<Move>,
    ) {
        generate::generate_moves::<true, false>(self, position, base_moves, output);
    }

    /// 「段階6」（movegen-speedup-2.md）の非捕獲だけを全手生成と同じ順で追加する。
    pub(crate) fn generate_quiets(
        &self,
        position: &Position,
        base_moves: &mut Vec<Move>,
        output: &mut Vec<Move>,
    ) {
        generate::generate_moves::<false, true>(self, position, base_moves, output);
    }

    /// 候補手が`position`で手番側の合法手かを返す。
    pub(crate) fn is_legal_move(
        &self,
        position: &Position,
        candidate: Move,
        base_moves: &mut Vec<Move>,
        moves: &mut Vec<Move>,
    ) -> bool {
        moves.clear();
        let Some(piece) = position.piece_at(candidate.from) else {
            return false;
        };
        if piece.color() != Some(position.side_to_move()) {
            return false;
        }
        let kind = piece.kind().expect("board piece must have a valid kind");
        generate::generate_piece_moves::<false, false>(
            self,
            position,
            position.side_to_move(),
            kind,
            candidate.from,
            base_moves,
            moves,
        );
        moves.contains(&candidate)
    }

    /// 利きの前計算テーブルを返す。
    pub(crate) const fn tables(&self) -> &AttackTables {
        self.tables
    }

    /// 採用しているルールの集合を返す。
    pub const fn rules(&self) -> MoveRules {
        self.rules
    }
}
