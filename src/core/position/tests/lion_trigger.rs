//! 先獅子の状態の参照と設定の試験。

use super::position_after_non_lion_captures_lion;
use crate::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceKind};
use crate::core::position::Position;
use crate::test_util::{position as position_with_pieces, sq};

#[test]
fn lion_capture_square_tracks_explicit_state() {
    // 設計書predecessor-generator.md「Positionの公開面」
    let mut position = Position::empty(Color::Black);
    assert_eq!(position.lion_capture_square(), None);
    position.set_lion_capture(Some(sq(3, 3))).unwrap();
    assert_eq!(position.lion_capture_square(), Some(sq(3, 3)));
    position.set_lion_capture(None).unwrap();
    assert_eq!(position.lion_capture_square(), None);
}

#[test]
fn lion_capture_square_distinguishes_non_lion_and_lion_captures() {
    // 設計書predecessor-generator.md「Positionの公開面」
    let (non_lion_capture, captured_square) = position_after_non_lion_captures_lion();
    assert_eq!(
        non_lion_capture.lion_capture_square(),
        Some(captured_square)
    );

    let mut lion_capture = position_with_pieces(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::Lion),
            (captured_square, Color::White, PieceKind::Lion),
        ],
    );
    lion_capture
        .try_make_move(
            Move {
                from: sq(0, 0),
                mid: None,
                to: captured_square,
                promote: false,
            },
            &MoveGenerator::standard(),
        )
        .unwrap();
    assert_eq!(lion_capture.lion_capture_square(), None);
}
