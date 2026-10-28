//! 王駒の捕獲と詰み、および合法手の有無の判定。

use super::super::repetition::{RepetitionHistory, retain_repetition_allowed_moves};
use super::super::result::{GameResult, WinReason};
use super::AdjudicationContext;
use crate::movegen::MoveGenerator;
use crate::mv::Move;
use crate::piece::Color;
use crate::position::Position;

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
///
/// 相手の獅子でない駒の升では、疑似利きが届くことと合法に取れることが一致する
/// （RULES.md第8条）。王駒が1枚ならこの補題で判定し、2枚なら1手で両方を取る
/// 必要があるため全合法手を生成する。0枚なら対象がなく、3枚以上なら1手の最大捕獲数
/// である2枚を超えるので、いずれも偽を返す。
/// 捕獲手は既出局面を再現しないので、[`is_mate`]の補題により反復のフィルタを掛けない。
pub(crate) fn can_capture_last_royal(position: &Position, generator: &MoveGenerator) -> bool {
    let mover = position.side_to_move();
    let royals = position.royal_pieces(mover.opposite());
    match royals.popcount() {
        1 => {
            let royal = royals.lsb().expect("one royal must be present");
            !position
                .attackers_to_by(mover, royal, position.occupied())
                .is_empty()
        }
        2 => {
            let mut moves = Vec::new();
            generator.generate_moves(position, &mut moves);
            moves
                .into_iter()
                .any(|mv| captures_last_royal(position, mv))
        }
        _ => false,
    }
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
/// 王駒捕獲、R2またはR3で許される捕獲回避手、および第21条第3項c・dの即時勝利・
/// 引き分けを回避手として扱う。着手側の即時敗北は回避手としない。
/// すべての仮想着手後に局面を復元する。
///
/// 捕獲を含む手は駒数、成りを含む手は駒種が不可逆に変わるので既出局面を再現せず、
/// R2とR3では禁じられない。
/// このため、受けの側の着手にはR2とR3のフィルタを掛けるが、
/// 相手が最後の王駒を取る応手には掛けない。
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

    let mover = position.side_to_move();
    for mv in moves {
        if captures_last_royal(position, mv) {
            return false;
        }

        let undo = position.make_move_unchecked(mv, context.rules.moves);
        let escapes_mate = match context.candidate_immediate_result(position, mv, &undo) {
            Some(GameResult::Win { winner, .. }) => winner == mover,
            Some(GameResult::Draw { .. }) => true,
            None => !can_capture_last_royal(position, context.generator),
        };
        position.unmake_move(undo);

        if escapes_mate {
            return false;
        }
    }

    true
}
