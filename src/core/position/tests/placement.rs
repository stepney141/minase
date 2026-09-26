//! 駒の配置と除去の基本操作の試験。

use crate::core::board::Square;
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::zobrist::zobrist_keys;
use crate::core::position::{Position, PositionBuildError, PositionBuilder};
use crate::test_util::sq;

// 実装契約(D4-IMP-01): 空盤への1枚配置と同じ升の除去は互いに逆操作であり、
// 全144升×全盤上駒コード(未成21種＋成駒18種×両所有者)で空盤へ完全に戻る。
#[test]
fn placing_then_removing_any_piece_restores_the_empty_position() {
    let pristine = Position::empty(Color::Black);
    let mut codes = Vec::new();
    for color in Color::ALL {
        for kind in PieceKind::ALL {
            if let Some(piece) = PieceCode::new(color, kind) {
                codes.push(piece);
            }
            if let Some(promoted) = PieceCode::new_promoted(color, kind) {
                codes.push(promoted);
            }
        }
    }

    let mut position = pristine.clone();
    for square in Square::all() {
        for &code in &codes {
            position.put_piece(square, code, zobrist_keys()).unwrap();
            assert!(position.occupied().contains(square));
            assert_eq!(position.remove_piece(square, zobrist_keys()), code);
            assert_eq!(position, pristine, "{square:?} {code:?}");
        }
    }

    // 既に駒がある升への二重配置と、空升・番兵コードの配置は拒否され状態を変えない。
    let mut builder = PositionBuilder::new(Color::Black);
    builder
        .put(
            sq(4, 4),
            PieceCode::new(Color::Black, PieceKind::King).unwrap(),
        )
        .unwrap();
    assert_eq!(
        builder.put(
            sq(4, 4),
            PieceCode::new(Color::White, PieceKind::King).unwrap(),
        ),
        Err(PositionBuildError::SquareOccupied { square: sq(4, 4) })
    );
    assert_eq!(
        builder.put(sq(5, 5), PieceCode::EMPTY),
        Err(PositionBuildError::EmptyOrWallPiece)
    );
    assert_eq!(
        builder.put(sq(5, 5), PieceCode::WALL),
        Err(PositionBuildError::EmptyOrWallPiece)
    );
}
