//! 着手後の終局裁定。王駒捕獲、反復、駒枯れ、裸玉、合法手なし、詰みを判定する。

mod bare_king;
mod exhaustion;
mod mate;

#[cfg(test)]
mod tests;

use super::repetition::{RepetitionHistory, move_is_irreversible, move_was_attacking};
use super::result::{GameResult, WinReason};
use crate::core::board::Square;
use crate::core::movegen::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::Color;
use crate::core::position::{Position, Undo};
use crate::core::rules::{ExhaustionRule, RepetitionRule, Rules};
use bare_king::bare_king_result;
pub(crate) use exhaustion::promoted_waiting_square;
use exhaustion::{
    PieceExhaustionOutcome, PieceExhaustionTransition, piece_exhaustion_outcome,
    piece_exhaustion_transition,
};
use mate::{has_no_legal_move, is_mate, royal_capture_result};

/// 対局裁定に必要な履歴依存状態。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct AdjudicationState {
    /// 採用中の反復規則の局面出現履歴。
    repetition: RepetitionHistory,
    /// 駒枯れの1手猶予(第22条第5項)が進行中かどうか。
    piece_exhaustion_grace: bool,
    /// 対局開始からの手数。
    ply: u32,
}

impl AdjudicationState {
    /// 開始局面を反復履歴の第1回として記録した初期状態を作る。
    pub(crate) fn new(repetition_rule: RepetitionRule, position: &Position) -> Self {
        Self {
            repetition: RepetitionHistory::new(repetition_rule, position),
            piece_exhaustion_grace: false,
            ply: 0,
        }
    }

    /// 反復履歴を返す。
    pub(crate) const fn repetition(&self) -> &RepetitionHistory {
        &self.repetition
    }

    /// 駒枯れの1手猶予が進行中かどうかを返す。
    pub(crate) const fn piece_exhaustion_grace(&self) -> bool {
        self.piece_exhaustion_grace
    }

    /// 駒枯れの1手猶予の状態を更新する。
    pub(crate) fn set_piece_exhaustion_grace(&mut self, grace: bool) {
        self.piece_exhaustion_grace = grace;
    }

    /// 対局開始からの手数を返す。
    pub(crate) const fn ply(&self) -> u32 {
        self.ply
    }

    /// 手数と反復履歴を着手後の局面で更新し、R1の裁定結果を返す。
    ///
    /// R1の裁定は対局開始または直前の不可逆手から可逆手が12手以上続くことを
    /// 前提とする(第31条R1)。不可逆手の判定には着手前の駒と捕獲情報を使う。
    pub(crate) fn record_move(
        &mut self,
        position: &Position,
        generator: &MoveGenerator,
        mover: Color,
        played: Move,
        undo: &Undo,
    ) -> Option<GameResult> {
        self.ply = self
            .ply
            .checked_add(1)
            .expect("a game cannot exceed u32::MAX plies");

        match &mut self.repetition {
            RepetitionHistory::R1(history) => history.record_move(
                position,
                self.ply,
                mover,
                move_was_attacking(position, generator, mover, played),
                move_is_irreversible(played, undo),
            ),
            RepetitionHistory::R2(history) => {
                history.record(position);
                None
            }
            RepetitionHistory::R3(history) => {
                history.record(position);
                None
            }
        }
    }
}

/// 仮想着手を含む裁定が参照する対局文脈。
///
/// 審判状態は反復履歴と手数を供給し、生成器は合法手と採用規則を供給する。
/// 駒枯れ猶予は確定着手の純粋な次状態を仮想裁定へ渡すために保持する。
pub(crate) struct AdjudicationContext<'a> {
    /// 反復履歴と手数を供給する審判状態。
    state: &'a AdjudicationState,
    /// 合法手を供給する生成器。
    generator: &'a MoveGenerator,
    /// 採用しているローカルルールの集合。
    rules: Rules,
    /// 仮想裁定へ渡す駒枯れ猶予の状態。
    piece_exhaustion_grace: bool,
}

impl<'a> AdjudicationContext<'a> {
    /// 完全な規則集合、審判状態、生成器から裁定文脈を作る。
    pub(crate) fn new(
        rules: Rules,
        state: &'a AdjudicationState,
        generator: &'a MoveGenerator,
    ) -> Self {
        Self {
            state,
            generator,
            rules,
            piece_exhaustion_grace: state.piece_exhaustion_grace(),
        }
    }

    /// 駒枯れ猶予だけを差し替えた文脈を返す。
    fn with_piece_exhaustion_grace(mut self, grace: bool) -> Self {
        self.piece_exhaustion_grace = grace;
        self
    }

    /// 仮想着手が着手側の即時勝利(第21条第3項c)を成立させるかを返す。
    ///
    /// R1の反復裁定と、裸玉または駒枯れによる勝利を即時勝利として調べる。
    fn candidate_is_immediate_win(
        &self,
        position: &Position,
        candidate: Move,
        undo: &Undo,
    ) -> bool {
        let mover = position.side_to_move().opposite();
        if let RepetitionHistory::R1(history) = self.state.repetition()
            && let Some(result) = history.candidate_result(
                position,
                self.state.ply(),
                mover,
                move_was_attacking(position, self.generator, mover, candidate),
                move_is_irreversible(candidate, undo),
            )
        {
            return matches!(result, GameResult::Win { winner, .. } if winner == mover);
        }

        if self.rules.exhaustion == ExhaustionRule::E3 {
            matches!(
                bare_king_result(position, self.generator),
                Some(GameResult::Win { winner, .. }) if winner == mover
            )
        } else {
            self.rules.exhaustion != ExhaustionRule::E2
                && matches!(
                    piece_exhaustion_outcome(
                        position,
                        self.generator,
                        promoted_waiting_square(candidate, undo),
                        self.piece_exhaustion_grace,
                    ),
                    PieceExhaustionOutcome::Win(winner) if winner == mover
                )
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// 確定着手後の裁定結果と駒枯れ猶予の次状態。
pub(crate) struct PostMoveAdjudication {
    /// 終局が確定すればその結果。
    result: Option<GameResult>,
    /// 次の着手時点で駒枯れ猶予が進行中かどうか。
    piece_exhaustion_grace: bool,
}

impl PostMoveAdjudication {
    /// 終局が確定していればその結果を返す。
    pub(crate) const fn result(self) -> Option<GameResult> {
        self.result
    }

    /// 次の着手時点で駒枯れ猶予が進行中かどうかを返す。
    pub(crate) const fn piece_exhaustion_grace(self) -> bool {
        self.piece_exhaustion_grace
    }
}

/// 確定した着手後の局面を、王駒捕獲、R1、駒枯れ、合法手なし、詰みの順で裁定する。
///
/// 仮想着手による局面変更は呼出し前の状態へ復元し、履歴依存状態は変更しない。
pub(crate) fn adjudicate_after_move(
    position: &mut Position,
    context: AdjudicationContext<'_>,
    mover: Color,
    promoted_waiting_piece: Option<Square>,
    repetition_result: Option<GameResult>,
) -> PostMoveAdjudication {
    let current_grace = context.state.piece_exhaustion_grace();
    if let Some(result) = royal_capture_result(position, mover) {
        return PostMoveAdjudication {
            result: Some(result),
            piece_exhaustion_grace: current_grace,
        };
    }
    if let Some(result) = repetition_result {
        return PostMoveAdjudication {
            result: Some(result),
            piece_exhaustion_grace: current_grace,
        };
    }

    let exhaustion = if context.rules.exhaustion == ExhaustionRule::E3 {
        PieceExhaustionTransition {
            result: bare_king_result(position, context.generator),
            next_grace: false,
        }
    } else {
        piece_exhaustion_transition(
            position,
            context.generator,
            promoted_waiting_piece,
            current_grace,
            context.rules.exhaustion == ExhaustionRule::E2,
        )
    };
    if exhaustion.result.is_some() {
        return PostMoveAdjudication {
            result: exhaustion.result,
            piece_exhaustion_grace: exhaustion.next_grace,
        };
    }

    let context = context.with_piece_exhaustion_grace(exhaustion.next_grace);
    let side_to_move = position.side_to_move();
    if has_no_legal_move(position, context.generator, context.state.repetition()) {
        return PostMoveAdjudication {
            result: Some(GameResult::Win {
                winner: side_to_move.opposite(),
                reason: WinReason::Stalemate,
            }),
            piece_exhaustion_grace: exhaustion.next_grace,
        };
    }

    let result = (!context.rules.e1 && is_mate(position, &context)).then_some(GameResult::Win {
        winner: side_to_move.opposite(),
        reason: WinReason::Mate,
    });
    PostMoveAdjudication {
        result,
        piece_exhaustion_grace: exhaustion.next_grace,
    }
}
