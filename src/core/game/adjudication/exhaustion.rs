//! 駒枯れの裁定と1手猶予の状態遷移。

use super::super::result::{DrawReason, GameResult, WinReason};
use crate::core::board::{BOARD_RANKS, Square};
use crate::core::movegen::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceKind};
use crate::core::position::{Position, Undo};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// 駒枯れ条件の判定結果。
pub(crate) enum PieceExhaustionOutcome {
    /// 駒枯れの条件(第22条第1項)が成立していない。
    ConditionNotMet,
    /// 双方が王駒だけとなる引き分け(第22条第8項)。
    Draw,
    /// 余分な駒を持つ側の勝ち(第22条第1項)。
    Win(Color),
    /// 余分な駒を取れる側への1手猶予の開始(第22条第5項)。
    GraceStart,
}

/// 駒枯れ判定による裁定結果と猶予の次状態の組。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct PieceExhaustionTransition {
    /// 裁定が確定すればその結果。
    pub(super) result: Option<GameResult>,
    /// 次の着手時点で猶予が進行中かどうか。
    pub(super) next_grace: bool,
}

/// 駒枯れ条件と1手猶予の適用結果を返す。
pub(crate) fn piece_exhaustion_outcome(
    position: &Position,
    generator: &MoveGenerator,
    promoted_waiting_piece: Option<Square>,
    grace_pending: bool,
) -> PieceExhaustionOutcome {
    // 第22条第8項: 双方に王駒以外の駒がなければ引き分け。ただし王駒が
    // 1枚ずつでなければ(太子併存など)駒枯れの対象外として対局を続ける。
    let royals = Color::ALL.map(|color| position.royal_pieces(color));
    let non_royals = position.occupied() & !(royals[0] | royals[1]);
    if non_royals.is_empty() {
        return if royals.into_iter().all(|royal| royal.popcount() == 1) {
            PieceExhaustionOutcome::Draw
        } else {
            PieceExhaustionOutcome::ConditionNotMet
        };
    }
    // 第22条第1項: 双方に王駒が1枚ずつ残り、王駒以外の駒が一方に1枚だけ。
    if royals.into_iter().any(|royal| royal.popcount() != 1) || non_royals.popcount() != 1 {
        return PieceExhaustionOutcome::ConditionNotMet;
    }

    let extra_square = non_royals
        .lsb()
        .expect("one non-royal piece must have a square");
    let extra_piece = position
        .piece_at(extra_square)
        .expect("the non-royal square must contain a piece");
    let extra_color = extra_piece
        .color()
        .expect("the extra piece must have an owner");
    let extra_kind = extra_piece
        .kind()
        .expect("the extra piece must have a kind");

    // 第22条第4項: 最奥段で移動不能となった歩兵・香車は余分な駒として数えない。
    if !extra_piece.is_promoted()
        && matches!(extra_kind, PieceKind::Pawn | PieceKind::Lance)
        && is_last_rank(extra_color, extra_square)
    {
        return PieceExhaustionOutcome::ConditionNotMet;
    }
    // 第22条第2項・第3項: 余分な駒が不成の歩兵・仲人なら、成るまで勝利は成立しない。
    if !extra_piece.is_promoted() && matches!(extra_kind, PieceKind::Pawn | PieceKind::GoBetween) {
        return PieceExhaustionOutcome::ConditionNotMet;
    }

    // 非捕獲で成った直後、猶予消化後、または余分な駒を持つ側の手番なら、
    // 相手に取り返す機会はなく勝利が確定する(第22条第1項・第7項)。
    if promoted_waiting_piece == Some(extra_square)
        || grace_pending
        || position.side_to_move() == extra_color
    {
        return PieceExhaustionOutcome::Win(extra_color);
    }

    // 第22条第5項: 王駒だけとなった側が余分な駒を次の着手で取れる場合に限り、
    // その着手のための猶予を与える。
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    if moves.into_iter().any(|candidate| {
        position
            .captured_squares(candidate)
            .into_iter()
            .flatten()
            .any(|capture| capture == extra_square)
    }) {
        PieceExhaustionOutcome::GraceStart
    } else {
        PieceExhaustionOutcome::Win(extra_color)
    }
}

/// 駒枯れ判定の結果を裁定結果と猶予の次状態へ写す。E2採用時は判定自体を行わない。
pub(super) fn piece_exhaustion_transition(
    position: &Position,
    generator: &MoveGenerator,
    promoted_waiting_piece: Option<Square>,
    grace_pending: bool,
    disabled: bool,
) -> PieceExhaustionTransition {
    if disabled {
        return PieceExhaustionTransition {
            result: None,
            next_grace: grace_pending,
        };
    }

    match piece_exhaustion_outcome(position, generator, promoted_waiting_piece, grace_pending) {
        PieceExhaustionOutcome::ConditionNotMet => PieceExhaustionTransition {
            result: None,
            next_grace: false,
        },
        PieceExhaustionOutcome::Draw => PieceExhaustionTransition {
            result: Some(GameResult::Draw {
                reason: DrawReason::PieceExhaustion,
            }),
            next_grace: false,
        },
        PieceExhaustionOutcome::Win(winner) => PieceExhaustionTransition {
            result: Some(GameResult::Win {
                winner,
                reason: WinReason::PieceExhaustion,
            }),
            next_grace: false,
        },
        PieceExhaustionOutcome::GraceStart => PieceExhaustionTransition {
            result: None,
            next_grace: true,
        },
    }
}

/// 非捕獲で歩または仲人が成った場合に、駒枯れの即時勝利候補升を返す。
pub(crate) fn promoted_waiting_square(mv: Move, undo: &Undo) -> Option<Square> {
    (mv.promote
        && undo.captured.iter().all(Option::is_none)
        && matches!(
            undo.moved_piece_before.kind(),
            Some(PieceKind::Pawn | PieceKind::GoBetween)
        ))
    .then_some(mv.to)
}

/// 指定升が指定対局者から見た相手側の最奥段にあるかを返す。
pub(super) fn is_last_rank(color: Color, square: Square) -> bool {
    match color {
        Color::Black => square.rank() == BOARD_RANKS - 1,
        Color::White => square.rank() == 0,
    }
}
