//! 公開ビルダーによる駒の配置の試験。

use crate::board::Square;
use crate::piece::{Color, PieceCode, PieceKind};
use crate::position::{PositionBuildError, PositionBuilder};
use crate::test_util::sq;

// PositionBuilder::putとfinishの契約を、全144升×全盤上駒コードで検査する。
#[test]
fn builder_places_any_piece_on_any_square() {
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

    for square in Square::all() {
        for &code in &codes {
            let mut builder = PositionBuilder::new(Color::Black);
            builder
                .put(square, code)
                .unwrap_or_else(|error| panic!("{square:?} {code:?}: {error:?}"));
            let position = builder
                .finish()
                .unwrap_or_else(|error| panic!("{square:?} {code:?}: {error:?}"));
            assert_eq!(position.piece_at(square), Some(code), "{square:?} {code:?}");
            assert_eq!(position.occupied().popcount(), 1, "{square:?} {code:?}");
            assert!(position.occupied().contains(square), "{square:?} {code:?}");
            assert_eq!(position.side_to_move(), Color::Black, "{square:?} {code:?}");
            assert_eq!(position.validate(), Ok(()), "{square:?} {code:?}");
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
