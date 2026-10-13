//! 駒枯れの裁定と1手猶予の状態遷移。

use super::super::result::{DrawReason, GameResult, WinReason};
use crate::board::{BOARD_RANKS, Square};
use crate::movegen::MoveGenerator;
use crate::piece::{Color, PieceKind};
use crate::position::Position;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// 駒枯れ条件の判定結果。
pub(crate) enum PieceExhaustionOutcome {
    /// 駒枯れの条件(第22条第1項)が成立していない。
    ConditionNotMet,
    /// 双方に王駒が1枚ずつだけ残る引き分け(第22条第8項)。
    Draw,
    /// 余分な駒を持つ側の勝ち(第22条第1項)。
    Win(Color),
    /// 余分な駒または相手の最後の王駒を取れる側への1手猶予の開始(第22条第5項)。
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
///
/// 双方に王駒を1枚ずつ残し、それ以外が片側に1枚だけある場合を対象とする
/// （RULES.md第22条第1項・第9項）。王将と太子を併せ持つ側では一方を余分な駒と数え、
/// どちらを取られても余分な駒を失ったと扱う。
/// 唯一の余分な駒が最奥段で移動不能の不成の歩兵または香車なら、条件不成立とする（第4項）。
/// 引き分けは、双方に王駒が1枚ずつだけ残る場合に限る（第8項）。
///
/// 余分な駒が不成の歩兵または仲人なら成るまで勝ちとせず、成った後は通常の猶予を適用する
/// （第2項・第3項）。猶予は、王駒だけの側に手番があり、余分な駒または相手の最後の王駒を
/// 次の1手で取れる場合に与える（第5項）。余分な駒を持つ側の手番なら直ちにその側の勝ちとなる。
/// 猶予中の捕獲後に双方が王駒1枚ずつになれば引き分け、それ以外は余分な駒を持つ側の勝ちとなる
/// （第6項から第8項）。最後の王駒の捕獲による勝利は、呼び出し側が先に裁定する（第21条第1項）。
pub(crate) fn piece_exhaustion_outcome(
    position: &Position,
    generator: &MoveGenerator,
    grace_pending: bool,
) -> PieceExhaustionOutcome {
    let royals = Color::ALL.map(|color| position.royal_pieces(color));
    if royals.into_iter().any(|royal| royal.is_empty()) {
        return PieceExhaustionOutcome::ConditionNotMet;
    }

    // 第22条第1項・第9項: 王駒を1枚ずつ除いた残りが片側に1枚だけなら対象となる。
    // 王将と太子の併存も2枚と数え、双方1枚だけの場合は第8項の引き分けとする。
    let extra_color = match Color::ALL.map(|color| position.pieces_of(color).popcount()) {
        [1, 1] => return PieceExhaustionOutcome::Draw,
        [2, 1] => Color::Black,
        [1, 2] => Color::White,
        _ => return PieceExhaustionOutcome::ConditionNotMet,
    };

    let non_royals = position.pieces_of(extra_color) & !royals[extra_color.index()];
    if let Some(extra_square) = non_royals.lsb() {
        let extra_piece = position
            .piece_at(extra_square)
            .expect("the non-royal square must contain a piece");
        let extra_kind = extra_piece
            .kind()
            .expect("the extra piece must have a kind");

        // 第22条第4項: 最奥段で移動不能となった歩兵・香車は勝利を成立させない。
        if !extra_piece.is_promoted()
            && matches!(extra_kind, PieceKind::Pawn | PieceKind::Lance)
            && is_last_rank(extra_color, extra_square)
        {
            return PieceExhaustionOutcome::ConditionNotMet;
        }
        // 第22条第2項・第3項: 不成の歩兵・仲人なら、成るまで勝利は成立しない。
        if !extra_piece.is_promoted()
            && matches!(extra_kind, PieceKind::Pawn | PieceKind::GoBetween)
        {
            return PieceExhaustionOutcome::ConditionNotMet;
        }
    }

    // 猶予消化後、または余分な駒を持つ側の手番なら勝利が確定する(第22条第1項・第7項)。
    if grace_pending || position.side_to_move() == extra_color {
        return PieceExhaustionOutcome::Win(extra_color);
    }

    // 第22条第5項・第9項: 相手には王駒と余分な駒、または王将と太子だけが残る。
    // どの相手駒を取っても猶予の条件を満たす。
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    if moves.into_iter().any(|candidate| {
        position
            .captured_squares(candidate)
            .into_iter()
            .any(|capture| capture.is_some())
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
    grace_pending: bool,
    disabled: bool,
) -> PieceExhaustionTransition {
    if disabled {
        return PieceExhaustionTransition {
            result: None,
            next_grace: grace_pending,
        };
    }

    match piece_exhaustion_outcome(position, generator, grace_pending) {
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

/// 指定升が指定対局者から見た相手側の最奥段にあるかを返す。
pub(super) fn is_last_rank(color: Color, square: Square) -> bool {
    match color {
        Color::Black => square.rank() == BOARD_RANKS - 1,
        Color::White => square.rank() == 0,
    }
}
