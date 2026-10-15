//! 反復・駒枯れによる即時勝利・引き分けと王駒の先取りによる詰み回避の試験。

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
    position.unmake_move(undo);
    assert!(is_mate(
        &mut position,
        &AdjudicationContext::new(rules, &state, &generator)
    ));
    assert_eq!(position, before);
}
