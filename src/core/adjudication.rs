//! 着手後の終局裁定。王駒捕獲、反復、駒枯れ、裸玉、合法手なし、詰みを判定する。

use crate::core::bitboard::Bitboard;
use crate::core::game::{DrawReason, GameResult, WinReason};
use crate::core::movegen::MoveGenerator;
use crate::core::mv::{Move, Undo};
use crate::core::piece::{Color, PieceKind};
use crate::core::position::Position;
use crate::core::repetition::{RepetitionHistory, retain_repetition_allowed_moves};
use crate::core::rules::{ExhaustionRule, RepetitionRule, Rules};
use crate::core::square::{BOARD_RANKS, Square};

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
    pub(crate) fn record_move(
        &mut self,
        position: &Position,
        generator: &MoveGenerator,
        mover: Color,
        played: Move,
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
struct PieceExhaustionTransition {
    /// 裁定が確定すればその結果。
    result: Option<GameResult>,
    /// 次の着手時点で猶予が進行中かどうか。
    next_grace: bool,
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

/// 着手後に相手の王駒がなくなった場合の勝利を返す。
fn royal_capture_result(position: &Position, mover: Color) -> Option<GameResult> {
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

/// 裸玉即時裁定(第32条E3)の結果を現在局面から返す。
fn bare_king_result(position: &Position, generator: &MoveGenerator) -> Option<GameResult> {
    let bare_kings = Color::ALL.map(|color| bare_king_square(position, color));

    for bare_color in Color::ALL {
        let Some(bare_king) = bare_kings[bare_color.index()] else {
            continue;
        };
        let opponent = bare_color.opposite();
        let effective_pieces = effective_pieces(position, opponent);
        // 条件(a): 相手が王駒を持ち、価値ある駒を2枚以上持つ。
        // 条件(b): 裸玉側が相手王駒へ王手をかけていない。
        // 条件(c): 価値ある駒が3枚以上か、いずれも裸玉に隣接していない。
        if !position.royal_pieces(opponent).is_empty()
            && effective_pieces.popcount() >= 2
            && !pieces_give_check(position, generator, bare_color)
            && (effective_pieces.popcount() >= 3
                || effective_pieces
                    .into_iter()
                    .all(|square| !squares_are_adjacent(bare_king, square)))
        {
            return Some(GameResult::Win {
                winner: opponent,
                reason: WinReason::BareKing,
            });
        }
    }

    // 双方が裸玉で、いずれの王駒にも王手がかかっていなければ引き分けとする。
    if bare_kings.into_iter().all(|king| king.is_some())
        && Color::ALL
            .into_iter()
            .all(|color| !pieces_give_check(position, generator, color))
    {
        Some(GameResult::Draw {
            reason: DrawReason::BareKing,
        })
    } else {
        None
    }
}

/// 死に駒を除く自駒が王駒1枚だけなら、その升を返す。
fn bare_king_square(position: &Position, color: Color) -> Option<Square> {
    let remaining = position.pieces_of(color) & !dead_pieces(position, color);
    (remaining.popcount() == 1)
        .then(|| remaining.lsb())
        .flatten()
        .filter(|&square| position.royal_pieces(color).contains(square))
}

/// 歩兵・仲人と死んだ香車を除く価値ある駒の集合を返す。
fn effective_pieces(position: &Position, color: Color) -> Bitboard {
    position.pieces_of(color)
        & !position.pieces_of_kind(color, PieceKind::Pawn)
        & !position.pieces_of_kind(color, PieceKind::GoBetween)
        & !dead_pieces(position, color)
}

/// 最奥段で移動不能となった歩兵・香車の集合を返す。
fn dead_pieces(position: &Position, color: Color) -> Bitboard {
    Bitboard::from_squares(
        (position.pieces_of_kind(color, PieceKind::Pawn)
            | position.pieces_of_kind(color, PieceKind::Lance))
        .into_iter()
        .filter(|&square| is_last_rank(color, square)),
    )
}

/// 指定側の駒が相手のいずれかの王駒へ王手をかけているかを返す。
fn pieces_give_check(position: &Position, generator: &MoveGenerator, attacker: Color) -> bool {
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

/// 2升がチェビシェフ距離1で隣接するかを返す。
fn squares_are_adjacent(first: Square, second: Square) -> bool {
    first.file().abs_diff(second.file()) <= 1
        && first.rank().abs_diff(second.rank()) <= 1
        && first != second
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
fn piece_exhaustion_transition(
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
fn is_last_rank(color: Color, square: Square) -> bool {
    match color {
        Color::Black => square.rank() == BOARD_RANKS - 1,
        Color::White => square.rank() == 0,
    }
}

/// 着手後の局面で、着手側の攻撃が継続しているかを返す。
pub(crate) fn move_was_attacking(
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::core::game::{Game, GameStatus};
    use crate::core::piece::PieceCode;
    use crate::core::repetition::repetition_is_forbidden;
    use crate::core::rules::RuleCode;
    use crate::test_util::{position_from_codes as position, sq};

    fn piece(color: Color, kind: PieceKind) -> PieceCode {
        PieceCode::new(color, kind).expect("fixture uses an unpromoted-capable kind")
    }

    fn step(from: Square, to: Square) -> Move {
        Move {
            from,
            mid: None,
            to,
            promote: false,
        }
    }

    fn r1_state(position: &Position) -> AdjudicationState {
        AdjudicationState::new(RepetitionRule::R1, position)
    }

    #[test]
    fn article_21_3_mate_requires_every_escape_clause_to_fail() {
        // D3-021-02: 3項a(回避)・b(相手王駒の先取り)のいずれかが残れば詰みは
        // 成立しない。仮想着手の評価後に局面と裁定状態は完全に復元される
        // (adjudication-refactor.md「検証」)。
        let generator = MoveGenerator::standard();

        // 受けが尽きた局面は詰みである(第21条2項)。
        let mut mated = position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(0, 11), piece(Color::White, PieceKind::Rook)),
                (sq(11, 0), piece(Color::White, PieceKind::Rook)),
                (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
                (sq(10, 9), piece(Color::White, PieceKind::King)),
            ],
        );
        let mated_before = mated.clone();
        let state = r1_state(&mated);
        let state_before = state.clone();
        assert!(is_mate(
            &mut mated,
            &AdjudicationContext::new(Rules::ENGINE_DEFAULT, &state, &generator)
        ));
        assert_eq!(mated, mated_before);
        assert_eq!(state, state_before);

        // 3項a: (0,1)への逃げが残れば詰みではない。
        let mut escapable = position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(11, 0), piece(Color::White, PieceKind::Rook)),
                (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
                (sq(10, 9), piece(Color::White, PieceKind::King)),
            ],
        );
        let state = r1_state(&escapable);
        assert!(!is_mate(
            &mut escapable,
            &AdjudicationContext::new(Rules::ENGINE_DEFAULT, &state, &generator)
        ));

        // 3項b: 相手の最後の王駒を先に取れる着手が残れば詰みではない(第21条4項)。
        let mut counter_capture = position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(1, 0), piece(Color::White, PieceKind::King)),
            ],
        );
        let state = r1_state(&counter_capture);
        assert!(can_capture_last_royal(&counter_capture, &generator));
        assert!(!is_mate(
            &mut counter_capture,
            &AdjudicationContext::new(Rules::ENGINE_DEFAULT, &state, &generator)
        ));

        // 3項b境界: 先取りした王駒の升へ相手の取り返しの利き(飛車)が残っていても、
        // 最後の王駒を取った時点で勝ちが確定する(第21条4項)ため詰みではない。
        // 変異検証(フェーズ4)で検出したオラクル欠落の補強。
        let mut counter_capture_with_retaliation = position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(1, 1), piece(Color::White, PieceKind::King)),
                (sq(1, 11), piece(Color::White, PieceKind::Rook)),
            ],
        );
        let state = r1_state(&counter_capture_with_retaliation);
        assert!(!is_mate(
            &mut counter_capture_with_retaliation,
            &AdjudicationContext::new(Rules::ENGINE_DEFAULT, &state, &generator)
        ));

        // 第23条境界: 着手が1つもない局面は合法手なしであり、詰みではない。
        let mut stuck = position(
            Color::Black,
            &[
                (sq(4, 11), piece(Color::Black, PieceKind::Pawn)),
                (sq(0, 11), piece(Color::Black, PieceKind::Lance)),
                (sq(11, 0), piece(Color::White, PieceKind::King)),
            ],
        );
        let state = r1_state(&stuck);
        assert!(has_no_legal_move(
            &mut stuck,
            &generator,
            state.repetition()
        ));
        assert!(!is_mate(
            &mut stuck,
            &AdjudicationContext::new(Rules::ENGINE_DEFAULT, &state, &generator)
        ));
    }

    #[test]
    fn plan_adjudication_shared_functions_match_game_play() {
        // D3-PRP-03: 探索層が使う共有裁定関数と対局進行(Game::play)は、同一の
        // 局面・規則・履歴に対して同じ終局判定と同じ着手後局面を与える
        // (adjudication-refactor.md「探索部との境界」)。
        let scenarios: [(&[RuleCode], Position, Move); 4] = [
            (
                // 詰み(第21条2項)。
                &[RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E2],
                position(
                    Color::White,
                    &[
                        (sq(0, 0), piece(Color::Black, PieceKind::King)),
                        (sq(0, 11), piece(Color::White, PieceKind::Rook)),
                        (sq(11, 0), piece(Color::White, PieceKind::Rook)),
                        (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
                        (sq(10, 8), piece(Color::White, PieceKind::King)),
                    ],
                ),
                step(sq(10, 8), sq(10, 9)),
            ),
            (
                // 王駒捕獲(第21条1項)。
                &[RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E0],
                position(
                    Color::Black,
                    &[
                        (sq(0, 0), piece(Color::Black, PieceKind::King)),
                        (sq(5, 5), piece(Color::Black, PieceKind::Rook)),
                        (sq(5, 8), piece(Color::White, PieceKind::King)),
                    ],
                ),
                step(sq(5, 5), sq(5, 8)),
            ),
            (
                // 駒枯れ引き分け(第22条8項)。
                &[RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E0],
                position(
                    Color::Black,
                    &[
                        (sq(0, 0), piece(Color::Black, PieceKind::King)),
                        (sq(11, 11), piece(Color::White, PieceKind::King)),
                    ],
                ),
                step(sq(0, 0), sq(0, 1)),
            ),
            (
                // 終局しない通常の着手。
                &[RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E0],
                position(
                    Color::Black,
                    &[
                        (sq(0, 0), piece(Color::Black, PieceKind::King)),
                        (sq(3, 3), piece(Color::Black, PieceKind::GoldGeneral)),
                        (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
                        (sq(11, 11), piece(Color::White, PieceKind::King)),
                    ],
                ),
                step(sq(3, 3), sq(3, 4)),
            ),
        ];

        for (codes, start, mv) in scenarios {
            let rules = Rules::from_codes(codes).unwrap();
            let generator = MoveGenerator::new(rules.moves);

            // 探索層の経路: make/unmakeと共有裁定関数を直接使う。
            let mut low_level = start.clone();
            let mut state = AdjudicationState::new(rules.repetition, &low_level);
            let mover = low_level.side_to_move();
            let undo = low_level.try_make_move_with_undo(mv, &generator).unwrap();
            assert!(!repetition_is_forbidden(state.repetition(), &low_level));
            let waiting = promoted_waiting_square(mv, &undo);
            let repetition_result = state.record_move(&low_level, &generator, mover, mv);
            let outcome = adjudicate_after_move(
                &mut low_level,
                AdjudicationContext::new(rules, &state, &generator),
                mover,
                waiting,
                repetition_result,
            );

            // 対局進行の経路。
            let mut game = Game::from_position(rules, start.clone());
            let status = game.play(mv).unwrap();

            let expected = match outcome.result() {
                Some(result) => GameStatus::Finished(result),
                None => GameStatus::Ongoing,
            };
            assert_eq!(status, expected, "codes={codes:?}");
            assert_eq!(&low_level, game.position(), "codes={codes:?}");
        }
    }

    #[test]
    fn plan_adjudication_r2_filter_matches_game_legal_moves_and_restores_the_position() {
        // D3-PRP-03: 対局合法手の絞り込み(R2禁止フィルタ)は、共有関数の経路と
        // Game::legal_movesの経路で一致し、仮想評価の前後で局面が復元される。
        // D3-031-06: 既出局面を再現する着手だけが局面合法手から除かれる。
        let start = position(
            Color::Black,
            &[
                (sq(3, 3), piece(Color::Black, PieceKind::King)),
                (sq(8, 8), piece(Color::White, PieceKind::King)),
            ],
        );
        let rules =
            Rules::from_codes(&[RuleCode::L0, RuleCode::P0, RuleCode::R2, RuleCode::E2]).unwrap();
        let generator = MoveGenerator::new(rules.moves);
        let cycle = [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
        ];

        let mut game = Game::from_position(rules, start.clone());
        let mut low_level = start.clone();
        let mut state = AdjudicationState::new(RepetitionRule::R2, &low_level);
        for mv in cycle {
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
            let mover = low_level.side_to_move();
            low_level.try_make_move(mv, &generator).unwrap();
            state.record_move(&low_level, &generator, mover, mv);
        }

        let mut moves = Vec::new();
        generator.generate_moves(&low_level, &mut moves);
        let repeating = step(sq(8, 7), sq(8, 8));
        assert!(moves.contains(&repeating));

        let snapshot = low_level.clone();
        retain_repetition_allowed_moves(&mut low_level, &generator, state.repetition(), &mut moves);
        assert_eq!(low_level, snapshot);
        assert!(!moves.contains(&repeating));
        assert_eq!(
            moves.into_iter().collect::<HashSet<_>>(),
            game.legal_moves().into_iter().collect::<HashSet<_>>()
        );
    }
}
