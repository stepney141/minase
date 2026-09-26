//! 王駒の捕獲と詰み、および合法手の有無の判定。

use super::super::repetition::{RepetitionHistory, retain_repetition_allowed_moves};
use super::super::result::{GameResult, WinReason};
use super::AdjudicationContext;
use crate::core::movegen::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::Color;
use crate::core::position::Position;

/// 着手後に相手の王駒がなくなった場合の勝利を返す。
pub(super) fn royal_capture_result(position: &Position, mover: Color) -> Option<GameResult> {
    position
        .royal_pieces(mover.opposite())
        .is_empty()
        .then_some(GameResult::Win {
            winner: mover,
            reason: WinReason::RoyalCapture,
        })
}

/// 着手が相手の残存王駒をすべて取るかを返す。
pub(crate) fn captures_last_royal(position: &Position, mv: Move) -> bool {
    let opponent = position.side_to_move().opposite();
    let opponent_royals = position.royal_pieces(opponent);
    let royal_count = opponent_royals.popcount();
    let captured_royal_count = mv
        .capture_candidates()
        .into_iter()
        .flatten()
        .filter(|&square| opponent_royals.contains(square))
        .count();

    royal_count > 0 && captured_royal_count == royal_count as usize
}

/// 手番側が相手の残存王駒をすべて取れるかを返す。
pub(crate) fn can_capture_last_royal(position: &Position, generator: &MoveGenerator) -> bool {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    moves
        .into_iter()
        .any(|mv| captures_last_royal(position, mv))
}

/// 手番側に対局規則上の合法手がないかを返す。
///
/// 合法手なしは詰みと異なる敗北であり、同じ局面に対して[`is_mate`]は
/// `false`を返す。
pub(crate) fn has_no_legal_move(
    position: &mut Position,
    generator: &MoveGenerator,
    repetition: &RepetitionHistory,
) -> bool {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    retain_repetition_allowed_moves(position, generator, repetition, &mut moves);
    moves.is_empty()
}

/// 手番側が詰みかを返す。
///
/// 王駒捕獲、R2またはR3で許される回避手、および第21条第3項cの即時勝利を
/// 回避手として扱い、すべての仮想着手後に局面を復元する。
pub(crate) fn is_mate(position: &mut Position, context: &AdjudicationContext<'_>) -> bool {
    let mut moves = Vec::new();
    context.generator.generate_moves(position, &mut moves);
    retain_repetition_allowed_moves(
        position,
        context.generator,
        context.state.repetition(),
        &mut moves,
    );
    if moves.is_empty() {
        return false;
    }

    let mut has_legal_move = false;
    for mv in moves {
        if captures_last_royal(position, mv) {
            return false;
        }

        let undo = position.make_move_unchecked(mv, context.rules.moves);
        has_legal_move = true;
        if context.candidate_is_immediate_win(position, mv, &undo) {
            position.unmake_move(undo);
            return false;
        }
        let opponent_can_capture_last_royal = can_capture_last_royal(position, context.generator);
        position.unmake_move(undo);

        if !opponent_can_capture_last_royal {
            return false;
        }
    }

    has_legal_move
}
