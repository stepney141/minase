//! 合法性を検査してから着手を適用する入口。

use core::fmt;

use super::MoveGenerator;
use crate::mv::Move;
use crate::position::Position;

/// 合法手でない着手を渡されたことを表すエラー。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IllegalMove(pub Move);

impl fmt::Display for IllegalMove {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "the move is not legal: {:?}", self.0)
    }
}

impl std::error::Error for IllegalMove {}

impl Position {
    /// 着手が合法手に含まれるか検査してから適用する。不合法なら[`IllegalMove`]を返し、
    /// 局面は変更しない(第27条第1項)。
    pub fn try_make_move(
        &mut self,
        mv: Move,
        generator: &MoveGenerator,
    ) -> Result<(), IllegalMove> {
        self.try_make_move_with_undo(mv, generator).map(drop)
    }

    /// 着手が合法手に含まれるか検査してから適用し、内部の巻き戻しトークンを返す。
    ///
    /// [`MoveGenerator::generate_moves`]は、手番側の全駒について同じ`generate_piece_moves`を
    /// 呼ぶ和集合であり、獅子の捕獲制限と成りの展開は駒ごとの生成の中で着手ごとに行う。
    /// したがって、全件生成の列に含まれることと[`MoveGenerator::is_legal_move`]が
    /// 真を返すことは一致し、動かす駒1枚の生成で同じ着手集合を受理できる。
    pub(crate) fn try_make_move_with_undo(
        &mut self,
        mv: Move,
        generator: &MoveGenerator,
    ) -> Result<crate::position::Undo, IllegalMove> {
        let mut base_moves = Vec::new();
        let mut moves = Vec::new();
        if generator.is_legal_move(self, mv, &mut base_moves, &mut moves) {
            Ok(self.make_move_unchecked(mv, generator.rules()))
        } else {
            Err(IllegalMove(mv))
        }
    }
}
