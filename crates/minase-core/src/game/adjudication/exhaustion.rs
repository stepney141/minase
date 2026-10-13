//! 駒枯れの裁定と1手猶予の状態遷移。

use super::super::result::{DrawReason, GameResult, WinReason};
use super::bare_king::can_capture_any_royal;
use crate::board::{BOARD_RANKS, Bitboard, Square};
use crate::movegen::MoveGenerator;
use crate::piece::{Color, PieceKind};
use crate::position::Position;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// 駒枯れ条件の判定結果。
pub(crate) enum PieceExhaustionOutcome {
    /// 駒枯れによる勝利・引き分けと猶予のいずれも成立していない。
    ConditionNotMet,
    /// 死に駒を除いて双方に王駒が1枚ずつだけ残り、いずれも取られ得ない引き分け(第22条第8項)。
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
/// 最奥段で移動不能の歩兵・香車は、すべての判定で盤上にないものとして扱う
/// （RULES.md第22条第4項）。双方に王駒を1枚ずつ残し、それ以外が片側に1枚だけある場合を対象とする
/// （RULES.md第22条第1項・第9項）。王将と太子を併せ持つ側では一方を余分な駒と数え、
/// どちらを取られても余分な駒を失ったと扱う。
/// 双方に王駒が1枚ずつだけ残り、いずれも相手の次の着手で取られ得なければ、第8項の引き分けとする。
/// 取られ得る間は対局を継続し、以後の着手の完了時に改めて判定する。
///
/// 余分な駒が不成の歩兵または仲人なら成るまで勝ちとせず、成った後は通常の猶予を適用する
/// （第2項・第3項）。猶予は、王駒だけの側に手番があり、余分な駒または相手の最後の王駒を
/// 次の1手で取れる場合に与える（第5項）。余分な駒を持つ側の手番なら直ちにその側の勝ちとなる。
/// 猶予中に余分な駒を取れば駒枯れによる勝利は成立せず、第8項の条件を改めて判定する。
/// 死に駒を取っても余分な駒は残るので、その所有者の勝ちとなる（第6項・第7項）。
/// 最後の王駒の捕獲による勝利は、呼び出し側が先に裁定する（第21条第1項）。
pub(crate) fn piece_exhaustion_outcome(
    position: &Position,
    generator: &MoveGenerator,
    grace_pending: bool,
) -> PieceExhaustionOutcome {
    let royals = Color::ALL.map(|color| position.royal_pieces(color));
    if royals.into_iter().any(|royal| royal.is_empty()) {
        return PieceExhaustionOutcome::ConditionNotMet;
    }

    // 第22条第4項: 枚数、余分な駒の特定、猶予の捕獲対象から死に駒を除く。
    let remaining =
        Color::ALL.map(|color| position.pieces_of(color) & !dead_pieces(position, color));

    // 第22条第8項・第9項: 王将と太子は2枚と数え、双方が1枚でも王駒を取れる間は引き分けない。
    if remaining.into_iter().all(|pieces| pieces.popcount() == 1)
        && Color::ALL
            .into_iter()
            .all(|color| !can_capture_any_royal(position, generator, color))
    {
        return PieceExhaustionOutcome::Draw;
    }

    // 第22条第1項・第9項: 王駒を1枚ずつ除いた残りが片側に1枚だけなら対象となる。
    let extra_color = match remaining.map(|pieces| pieces.popcount()) {
        [2, 1] => Color::Black,
        [1, 2] => Color::White,
        _ => return PieceExhaustionOutcome::ConditionNotMet,
    };

    let non_royals = remaining[extra_color.index()] & !royals[extra_color.index()];
    if let Some(extra_square) = non_royals.lsb() {
        let extra_piece = position
            .piece_at(extra_square)
            .expect("the non-royal square must contain a piece");
        let extra_kind = extra_piece
            .kind()
            .expect("the extra piece must have a kind");

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

    // 第22条第4項・第5項・第9項: 死に駒を除く相手の王駒または余分な駒を取れる場合に限る。
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    if moves.into_iter().any(|candidate| {
        position
            .captured_squares(candidate)
            .into_iter()
            .flatten()
            .any(|square| remaining[extra_color.index()].contains(square))
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

/// 最奥段で移動不能となった歩兵・香車の集合を返す（第22条第4項、第32条E3）。
pub(super) fn dead_pieces(position: &Position, color: Color) -> Bitboard {
    Bitboard::from_squares(
        (position.pieces_of_kind(color, PieceKind::Pawn)
            | position.pieces_of_kind(color, PieceKind::Lance))
        .into_iter()
        .filter(|&square| is_last_rank(color, square)),
    )
}

/// 指定升が指定対局者から見た相手側の最奥段にあるかを返す。
fn is_last_rank(color: Color, square: Square) -> bool {
    match color {
        Color::Black => square.rank() == BOARD_RANKS - 1,
        Color::White => square.rank() == 0,
    }
}
