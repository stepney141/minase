//! フェーズ2で置き換える審判の判定を、現行の全件生成のまま固定する参照実装。
//!
//! 変更対象の本番関数は呼ばず、候補のR1裁定も履歴の複製への記録で求める。
//! AdjudicationContextからは規則・履歴・猶予を読むだけで、判定メソッドは使わない。

use super::super::exhaustion::{PieceExhaustionOutcome, PieceExhaustionTransition, dead_pieces};
use super::super::mate::{captures_last_royal, royal_capture_result};
use super::super::{AdjudicationContext, AdjudicationState, PostMoveAdjudication};
use crate::board::{Bitboard, Square};
use crate::game::repetition::{
    RepetitionHistory, move_is_irreversible, retain_repetition_allowed_moves,
};
use crate::game::result::{DrawReason, GameResult, WinReason};
use crate::movegen::MoveGenerator;
use crate::mv::Move;
use crate::piece::{Color, PieceKind};
use crate::position::{Position, Undo};
use crate::rules::{ExhaustionRule, Rules};

/// 現行の全件生成による判定の複製。フェーズ2の実装に依存しない参照実装である。
pub(super) fn oracle_can_capture_last_royal(
    position: &Position,
    generator: &MoveGenerator,
) -> bool {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    moves
        .into_iter()
        .any(|mv| captures_last_royal(position, mv))
}

/// 現行の全件生成による判定の複製。フェーズ2の実装に依存しない参照実装である。
pub(super) fn oracle_move_was_attacking(
    position: &Position,
    generator: &MoveGenerator,
    mover: Color,
    played: Move,
) -> bool {
    let probe = position.clone_with_side_to_move(mover);
    let opponent_royals = probe.royal_pieces(mover.opposite());
    let destination = played.to;
    let mut moves = Vec::new();
    generator.generate_moves(&probe, &mut moves);

    moves.into_iter().any(|candidate| {
        let captures = probe.captured_squares(candidate);
        captures
            .into_iter()
            .flatten()
            .any(|square| opponent_royals.contains(square))
            || (candidate.from == destination
                && captures.into_iter().any(|capture| capture.is_some()))
    })
}

/// 現行の全件生成による判定の複製。フェーズ2の実装に依存しない参照実装である。
pub(super) fn oracle_can_capture_any_royal(
    position: &Position,
    generator: &MoveGenerator,
    attacker: Color,
) -> bool {
    let probe = position.clone_with_side_to_move(attacker);
    let opponent_royals = probe.royal_pieces(attacker.opposite());
    let mut moves = Vec::new();
    generator.generate_moves(&probe, &mut moves);

    moves.into_iter().any(|candidate| {
        probe
            .captured_squares(candidate)
            .into_iter()
            .flatten()
            .any(|square| opponent_royals.contains(square))
    })
}

/// 現行の全件生成による判定の複製。フェーズ2の実装に依存しない参照実装である。
pub(super) fn oracle_piece_exhaustion_outcome(
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
            .all(|color| !oracle_can_capture_any_royal(position, generator, color))
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

/// 現行の全件生成による判定の複製。フェーズ2の実装に依存しない参照実装である。
pub(super) fn oracle_bare_king_result(
    position: &Position,
    generator: &MoveGenerator,
) -> Option<GameResult> {
    let bare_kings = Color::ALL.map(|color| oracle_bare_king_square(position, color));

    for bare_color in Color::ALL {
        let Some(bare_king) = bare_kings[bare_color.index()] else {
            continue;
        };
        let opponent = bare_color.opposite();
        let oracle_effective_pieces = oracle_effective_pieces(position, opponent);
        // 条件(a): 相手が王駒を持ち、価値ある駒を2枚以上持つ。
        // 条件(b): 裸玉側の駒が相手のいずれの王駒も次の着手で取れない。
        // 条件(c): 価値ある駒が3枚以上か、いずれも裸玉に隣接していない。
        if !position.royal_pieces(opponent).is_empty()
            && oracle_effective_pieces.popcount() >= 2
            && !oracle_can_capture_any_royal(position, generator, bare_color)
            && (oracle_effective_pieces.popcount() >= 3
                || oracle_effective_pieces
                    .into_iter()
                    .all(|square| !oracle_squares_are_adjacent(bare_king, square)))
        {
            return Some(GameResult::Win {
                winner: opponent,
                reason: WinReason::BareKing,
            });
        }
    }

    // 双方が裸玉で、いずれの王駒も相手の次の着手で取られ得なければ引き分けとする。
    if bare_kings.into_iter().all(|king| king.is_some())
        && Color::ALL
            .into_iter()
            .all(|color| !oracle_can_capture_any_royal(position, generator, color))
    {
        Some(GameResult::Draw {
            reason: DrawReason::BareKing,
        })
    } else {
        None
    }
}

fn oracle_bare_king_square(position: &Position, color: Color) -> Option<Square> {
    let remaining = position.pieces_of(color) & !dead_pieces(position, color);
    (remaining.popcount() == 1)
        .then(|| remaining.lsb())
        .flatten()
        .filter(|&square| position.royal_pieces(color).contains(square))
}

fn oracle_effective_pieces(position: &Position, color: Color) -> Bitboard {
    position.pieces_of(color)
        & !position.pieces_of_kind(color, PieceKind::Pawn)
        & !position.pieces_of_kind(color, PieceKind::GoBetween)
        & !dead_pieces(position, color)
}

fn oracle_squares_are_adjacent(first: Square, second: Square) -> bool {
    first.file().abs_diff(second.file()) <= 1
        && first.rank().abs_diff(second.rank()) <= 1
        && first != second
}

fn oracle_piece_exhaustion_transition(
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

    match oracle_piece_exhaustion_outcome(position, generator, grace_pending) {
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

fn oracle_exhaustion_transition(
    context: &AdjudicationContext<'_>,
    position: &Position,
) -> PieceExhaustionTransition {
    if context.rules.exhaustion == ExhaustionRule::E3 {
        PieceExhaustionTransition {
            result: oracle_bare_king_result(position, context.generator),
            next_grace: false,
        }
    } else {
        oracle_piece_exhaustion_transition(
            position,
            context.generator,
            context.piece_exhaustion_grace,
            context.rules.exhaustion == ExhaustionRule::E2,
        )
    }
}

/// 現行の全件生成による判定の複製。フェーズ2の実装に依存しない参照実装である。
pub(super) fn oracle_candidate_immediate_result(
    context: &AdjudicationContext<'_>,
    position: &Position,
    candidate: Move,
    undo: &Undo,
) -> Option<GameResult> {
    // record_moveは不可逆手なら可逆手数を0へ戻し、候補込み11手以下、
    // 未出現、候補込み3回以下ならNoneを返す。これらはcandidate_resultの
    // 早期終了と同じであり、残る場合も同じ攻撃連続数と手数で裁定する。
    // 複製だけを更新するので、候補の照合で本番のcandidate_resultを共有しない。
    let mover = position.side_to_move().opposite();
    if let RepetitionHistory::R1(history) = context.state.repetition()
        && let Some(result) = history.clone().record_move(
            position,
            context.state.ply() + 1,
            mover,
            oracle_move_was_attacking(position, context.generator, mover, candidate),
            move_is_irreversible(candidate, undo),
        )
    {
        return Some(result);
    }

    oracle_exhaustion_transition(context, position).result
}

/// 現行の全件生成による判定の複製。フェーズ2の実装に依存しない参照実装である。
pub(super) fn oracle_has_no_legal_move(
    position: &mut Position,
    generator: &MoveGenerator,
    repetition: &RepetitionHistory,
) -> bool {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    retain_repetition_allowed_moves(position, generator, repetition, &mut moves);
    moves.is_empty()
}

/// 現行の全件生成による判定の複製。フェーズ2の実装に依存しない参照実装である。
pub(super) fn oracle_is_mate(position: &mut Position, context: &AdjudicationContext<'_>) -> bool {
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
        let escapes_mate = match oracle_candidate_immediate_result(context, position, mv, &undo) {
            Some(GameResult::Win { winner, .. }) => winner == mover,
            Some(GameResult::Draw { .. }) => true,
            None => !oracle_can_capture_last_royal(position, context.generator),
        };
        position.unmake_move(undo);

        if escapes_mate {
            return false;
        }
    }

    true
}

/// 現行の全件生成による判定の複製。フェーズ2の実装に依存しない参照実装である。
pub(super) fn oracle_record_move(
    state: &mut AdjudicationState,
    position: &Position,
    generator: &MoveGenerator,
    mover: Color,
    played: Move,
    undo: &Undo,
) -> Option<GameResult> {
    state.ply = state
        .ply
        .checked_add(1)
        .expect("a game cannot exceed u32::MAX plies");

    match &mut state.repetition {
        RepetitionHistory::R1(history) => history.record_move(
            position,
            state.ply,
            mover,
            oracle_move_was_attacking(position, generator, mover, played),
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

/// 現行の全件生成による判定の複製。フェーズ2の実装に依存しない参照実装である。
pub(super) fn oracle_adjudicate_after_move(
    position: &mut Position,
    state: &AdjudicationState,
    rules: Rules,
    generator: &MoveGenerator,
    mover: Color,
    repetition_result: Option<GameResult>,
) -> PostMoveAdjudication {
    let mut context = AdjudicationContext::new(rules, state, generator);
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

    let exhaustion = oracle_exhaustion_transition(&context, position);
    // 第22条第5項・第9項の捕獲は、相手の王将と太子が互いを守っていても認める。
    // 猶予の開始時には捕獲手があるため、合法手なしの裁定も必要ない。
    if exhaustion.result.is_some() || exhaustion.next_grace {
        return PostMoveAdjudication {
            result: exhaustion.result,
            piece_exhaustion_grace: exhaustion.next_grace,
        };
    }

    context.piece_exhaustion_grace = exhaustion.next_grace;
    let side_to_move = position.side_to_move();
    if oracle_has_no_legal_move(position, context.generator, context.state.repetition()) {
        return PostMoveAdjudication {
            result: Some(GameResult::Win {
                winner: side_to_move.opposite(),
                reason: WinReason::Stalemate,
            }),
            piece_exhaustion_grace: exhaustion.next_grace,
        };
    }

    let result =
        (!context.rules.e1 && oracle_is_mate(position, &context)).then_some(GameResult::Win {
            winner: side_to_move.opposite(),
            reason: WinReason::Mate,
        });
    PostMoveAdjudication {
        result,
        piece_exhaustion_grace: exhaustion.next_grace,
    }
}
