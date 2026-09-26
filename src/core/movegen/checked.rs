//! 合法性を検査してから着手を適用する入口。

use core::fmt;

use super::MoveGenerator;
use crate::core::mv::Move;
use crate::core::position::Position;

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
    pub(crate) fn try_make_move_with_undo(
        &mut self,
        mv: Move,
        generator: &MoveGenerator,
    ) -> Result<crate::core::position::Undo, IllegalMove> {
        let mut moves = Vec::new();
        generator.generate_moves(self, &mut moves);
        if moves.contains(&mv) {
            Ok(self.make_move_unchecked(mv, generator.rules()))
        } else {
            Err(IllegalMove(mv))
        }
    }
}
