//! 反復・駒枯れによる即時勝利と王駒の先取りによる詰み回避の試験。

use super::*;
use crate::board::Square;
use crate::piece::{PieceCode, PieceKind};
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

/// 第22条第2項・第3項・第5項・第9項により、成駒または王駒を取れる相手には猶予がある。
/// その成りは第21条第3項cの即時勝利にはならない。
#[test]
fn article_22_promotions_are_immediate_wins_only_without_a_grace_reply() {
    for kind in [
        PieceKind::Pawn,
        PieceKind::GoBetween,
        PieceKind::DrunkElephant,
    ] {
        for (bare_square, immediate_win) in
            [(sq(3, 8), false), (sq(0, 1), false), (sq(11, 11), true)]
        {
            let mut position = position(
                Color::Black,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(4, 7), piece(Color::Black, kind)),
                    (bare_square, piece(Color::White, PieceKind::King)),
                ],
            );
            let rules = Rules::ENGINE_DEFAULT;
            let generator = MoveGenerator::new(rules.moves);
            let state = r1_state(&position);
            let candidate = Move {
                promote: true,
                ..step(sq(4, 7), sq(4, 8))
            };
            let undo = position
                .try_make_move_with_undo(candidate, &generator)
                .unwrap();
            let context = AdjudicationContext::new(rules, &state, &generator);
            assert_eq!(
                context.candidate_is_immediate_win(&position, candidate, &undo),
                immediate_win,
                "{kind:?}, {bare_square:?}"
            );
        }
    }
}

/// 4回目の同一局面を生じさせる受けでも、仮想着手を含めて
/// 可逆手が11手なら即時勝利にならず詰みとなる
/// （RULES.md第21条第3項c、第31条R1、[`R1_MIN_REVERSIBLE_PLIES`](crate::game::repetition::r1::R1_MIN_REVERSIBLE_PLIES)）。
#[test]
fn article_31_r1_candidate_at_eleven_reversible_plies_does_not_rescue_mate() {
    let start = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(1, 8), piece(Color::Black, PieceKind::Rook)),
            (sq(3, 9), piece(Color::Black, PieceKind::Rook)),
            (sq(2, 9), piece(Color::Black, PieceKind::Pawn)),
            (sq(10, 10), piece(Color::Black, PieceKind::Lion)),
            (sq(0, 5), piece(Color::White, PieceKind::Rook)),
            (sq(2, 1), piece(Color::White, PieceKind::GoldGeneral)),
            (sq(1, 11), piece(Color::White, PieceKind::Pawn)),
            (sq(7, 10), piece(Color::White, PieceKind::Pawn)),
            (sq(8, 3), piece(Color::White, PieceKind::Lion)),
            (sq(11, 0), piece(Color::White, PieceKind::King)),
        ],
    );
    let rules = Rules::ENGINE_DEFAULT;
    let generator = MoveGenerator::new(rules.moves);
    let mut position = start;
    let mut state = r1_state(&position);
    let plies = [
        step(sq(10, 10), sq(10, 9)),
        step(sq(8, 3), sq(8, 3)),
        step(sq(10, 9), sq(10, 9)),
        step(sq(0, 5), sq(2, 5)),
        step(sq(1, 8), sq(1, 2)),
        step(sq(2, 5), sq(0, 5)),
        step(sq(1, 2), sq(1, 8)),
        step(sq(0, 5), sq(2, 5)),
        step(sq(3, 9), sq(3, 10)),
        step(sq(2, 5), sq(0, 5)),
    ];
    // 1・3・7手目の局面へ11手目の候補(3,10)→(3,9)で戻る。
    // 局面合法な着手を審判履歴へ直接記録し、仮想着手の裁定を単独で検証する。
    for mv in plies {
        let mover = position.side_to_move();
        let undo = position.try_make_move_with_undo(mv, &generator).unwrap();
        assert_eq!(
            state.record_move(&position, &generator, mover, mv, &undo),
            None
        );
    }
    let before = position.clone();
    let candidate = step(sq(3, 10), sq(3, 9));
    let undo = position
        .try_make_move_with_undo(candidate, &generator)
        .unwrap();
    let context = AdjudicationContext::new(rules, &state, &generator);
    assert!(!context.candidate_is_immediate_win(&position, candidate, &undo));
    position.unmake_move(undo);
    assert!(is_mate(&mut position, &context));
    assert_eq!(position, before);
}

/// 3項b(相手王駒の先取り)が残れば詰みは成立しない。
/// 仮想着手の評価後に局面は完全に復元される
/// （[`is_mate`](crate::game::adjudication::mate::is_mate)の復元の契約、RULES.md第21条第3項b）。
#[test]
fn article_21_3_b_royal_counter_capture_rescues_mate_and_restores_position() {
    let generator = MoveGenerator::standard();

    // 3項b境界: 先取りした王駒の升へ相手の取り返しの利き(飛車)が残っていても、
    // 最後の王駒を取った時点で勝ちが確定する(第21条4項)ため詰みではない。
    let mut counter_capture_with_retaliation = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(1, 1), piece(Color::White, PieceKind::King)),
            (sq(1, 11), piece(Color::White, PieceKind::Rook)),
        ],
    );
    let before = counter_capture_with_retaliation.clone();
    let state = r1_state(&counter_capture_with_retaliation);
    assert!(!is_mate(
        &mut counter_capture_with_retaliation,
        &AdjudicationContext::new(Rules::ENGINE_DEFAULT, &state, &generator)
    ));
    assert_eq!(counter_capture_with_retaliation, before);
}
