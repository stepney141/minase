//! 反復・駒枯れによる即時勝利・引き分けと王駒の先取りによる詰み回避の試験。

use super::*;
use crate::board::Square;
use crate::game::{DrawReason, Game, GameStatus};
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
fn article_21_3_c_promotions_are_immediate_wins_only_without_a_grace_reply() {
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
                context.candidate_immediate_result(&position, candidate, &undo),
                immediate_win.then_some(GameResult::Win {
                    winner: Color::Black,
                    reason: WinReason::PieceExhaustion,
                }),
                "{kind:?}, {bare_square:?}"
            );
        }
    }
}

/// 4回目の同一局面を生じさせる受けでも、仮想着手を含めて
/// 可逆手が11手なら勝利も引き分けも成立せず詰みとなる
/// （RULES.md第21条第3項c・d、第31条R1、[`R1_MIN_REVERSIBLE_PLIES`](crate::game::repetition::r1::R1_MIN_REVERSIBLE_PLIES)）。
#[test]
fn article_21_3_r1_candidate_at_eleven_reversible_plies_does_not_rescue_mate() {
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
    assert_eq!(
        context.candidate_immediate_result(&position, candidate, &undo),
        None
    );
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

/// 相手の王将と太子の一方を取って引き分けにできれば、両方の王駒に守られていても
/// 詰みではない（第21条第3項d、第22条第8項・第9項）。
#[test]
fn article_21_3_d_piece_exhaustion_draw_rescues_mate() {
    let mut position = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(1, 0), piece(Color::White, PieceKind::King)),
            (
                sq(1, 1),
                PieceCode::new_promoted(Color::White, PieceKind::CrownPrince).unwrap(),
            ),
        ],
    );
    let rules = Rules::ENGINE_DEFAULT;
    let generator = MoveGenerator::new(rules.moves);
    let state = r1_state(&position);
    let context = AdjudicationContext::new(rules, &state, &generator);
    let before = position.clone();
    let mut moves = Vec::new();
    generator.generate_moves(&position, &mut moves);
    assert!(!moves.is_empty());
    for mv in moves {
        assert!(!mate::captures_last_royal(&position, mv));
        let undo = position.make_move_unchecked(mv, rules.moves);
        assert!(mate::can_capture_last_royal(&position, &generator));
        position.unmake_move(undo);
    }

    assert!(!is_mate(&mut position, &context));
    assert_eq!(position, before);
    let mut game = Game::from_position(rules, position);
    assert_eq!(
        game.play(step(sq(0, 0), sq(1, 1))).unwrap(),
        GameStatus::Finished(GameResult::Draw {
            reason: DrawReason::PieceExhaustion
        })
    );
}

/// 4回目の同一局面を作る手が自分の反復負けになる場合、詰みの逃れにはならない
/// （第21条第3項c・d、第31条R1）。
#[test]
fn article_21_3_r1_repetition_loss_does_not_rescue_mate() {
    let mut position = position(
        Color::White,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(1, 8), piece(Color::Black, PieceKind::Rook)),
            (sq(3, 9), piece(Color::Black, PieceKind::Rook)),
            (sq(0, 5), piece(Color::White, PieceKind::Rook)),
            (sq(2, 1), piece(Color::White, PieceKind::GoldGeneral)),
            (sq(1, 11), piece(Color::White, PieceKind::Pawn)),
            (sq(7, 9), piece(Color::White, PieceKind::Pawn)),
            (sq(7, 10), piece(Color::White, PieceKind::Pawn)),
            (sq(11, 0), piece(Color::White, PieceKind::King)),
        ],
    );
    let rules = Rules::ENGINE_DEFAULT;
    let generator = MoveGenerator::new(rules.moves);
    let mut state = r1_state(&position);
    let w_out = step(sq(0, 5), sq(2, 5));
    let w_back = step(sq(2, 5), sq(0, 5));
    let z_out = step(sq(1, 8), sq(1, 2));
    let z_back = step(sq(1, 2), sq(1, 8));
    let y_out = step(sq(3, 9), sq(3, 10));
    // 白の(0,5)→(2,5)は非攻撃的で、黒の全着手は攻撃的である。
    // 途中の詰み裁定を行わず、局面合法な着手を審判履歴へ直接記録する。
    for mv in [
        w_out, z_out, w_back, z_back, w_out, z_out, w_back, z_back, w_out, y_out, w_back,
    ] {
        let mover = position.side_to_move();
        let undo = position.try_make_move_with_undo(mv, &generator).unwrap();
        assert_eq!(
            state.record_move(&position, &generator, mover, mv, &undo),
            None
        );
    }
    let before = position.clone();
    assert!(is_mate(
        &mut position,
        &AdjudicationContext::new(rules, &state, &generator)
    ));
    assert_eq!(position, before);

    let candidate = step(sq(3, 10), sq(3, 9));
    let undo = position
        .try_make_move_with_undo(candidate, &generator)
        .unwrap();
    assert_eq!(
        state.record_move(&position, &generator, Color::Black, candidate, &undo),
        Some(GameResult::Win {
            winner: Color::White,
            reason: WinReason::Repetition
        })
    );
}

/// E3でも着手の完了時に成立する引き分けを第21条第3項dの裁定結果として返す。
/// 最奥段の歩兵と香車は、E3の裸玉判定では除外される。
#[test]
fn article_21_3_d_e3_draw_is_an_immediate_result() {
    let mut position = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(5, 11), piece(Color::Black, PieceKind::Pawn)),
            (sq(0, 1), piece(Color::White, PieceKind::Rook)),
            (sq(5, 0), piece(Color::White, PieceKind::Lance)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    );
    let rules = Rules {
        exhaustion: ExhaustionRule::E3,
        ..Rules::ENGINE_DEFAULT
    };
    let generator = MoveGenerator::new(rules.moves);
    let state = r1_state(&position);
    let context = AdjudicationContext::new(rules, &state, &generator);
    let candidate = step(sq(0, 0), sq(0, 1));
    let before = position.clone();
    let expected = GameResult::Draw {
        reason: DrawReason::BareKing,
    };
    let undo = position
        .try_make_move_with_undo(candidate, &generator)
        .unwrap();
    assert_eq!(
        context.candidate_immediate_result(&position, candidate, &undo),
        Some(expected)
    );
    position.unmake_move(undo);
    assert!(!is_mate(&mut position, &context));
    assert_eq!(position, before);
    let mut game = Game::from_position(rules, position);
    assert_eq!(
        game.play(candidate).unwrap(),
        GameStatus::Finished(expected)
    );
}

/// 王駒の捕獲を避けても、その着手で自分の敗北が確定するなら逃れにはならない
/// （第21条第3項、第22条第1項）。
#[test]
fn article_21_3_immediate_loss_overrides_royal_capture_avoidance() {
    let mut position = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(0, 5), piece(Color::White, PieceKind::Rook)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    );
    let rules = Rules::ENGINE_DEFAULT;
    let generator = MoveGenerator::new(rules.moves);
    let state = r1_state(&position);
    let before = position.clone();
    let candidate = step(sq(0, 0), sq(1, 0));
    let undo = position
        .try_make_move_with_undo(candidate, &generator)
        .unwrap();
    assert!(!mate::can_capture_last_royal(&position, &generator));
    let mut game = Game::from_position(rules, before.clone());
    assert_eq!(
        game.play(candidate).unwrap(),
        GameStatus::Finished(GameResult::Win {
            winner: Color::White,
            reason: WinReason::PieceExhaustion
        })
    );
    position.unmake_move(undo);
    assert!(is_mate(
        &mut position,
        &AdjudicationContext::new(rules, &state, &generator)
    ));
    assert_eq!(position, before);
}
