//! 符号化の試験で共有する局面集と180度回転。

use core::num::{NonZeroU64, NonZeroUsize};

use minase_core::rng::XorShift64;
use minase_core::test_util::{bench_positions, sq};
use minase_core::{Move, MoveGenerator, PieceCode, Position, PositionBuilder, Square};

/// 升を180度回転させる。
pub(super) fn rotate_square(square: Square) -> Square {
    sq(11 - square.file(), 11 - square.rank())
}

/// 着手の各升を180度回転させる。
pub(super) fn rotate_move(mv: Move) -> Move {
    Move {
        from: rotate_square(mv.from),
        mid: mv.mid.map(rotate_square),
        to: rotate_square(mv.to),
        promote: mv.promote,
    }
}

/// 盤を180度回転させ、駒の陣営と手番を入れ替えた局面を返す。先獅子の升も回転させる。
///
/// 成り権の保留を持つ局面は扱わない。
pub(super) fn rotate_position(position: &Position) -> Position {
    assert!(position.promotion_deferred().is_empty());
    let mut builder = PositionBuilder::new(position.side_to_move().opposite());
    for square in position.occupied() {
        let piece = position.piece_at(square).unwrap();
        let color = piece.color().unwrap().opposite();
        let kind = piece.kind().unwrap();
        let rotated = if piece.is_promoted() {
            PieceCode::new_promoted(color, kind).unwrap()
        } else {
            PieceCode::new(color, kind).unwrap()
        };
        builder.put(rotate_square(square), rotated).unwrap();
    }
    let mut rotated = builder.finish().unwrap();
    rotated
        .set_lion_capture(position.lion_capture_square().map(rotate_square))
        .unwrap();
    rotated
}

/// bench局面と、固定シードの一様ランダム対局から先手番と後手番の両方を採った局面集を返す。
///
/// 規則は成り権の保留を追跡しない標準規則（engine-default）である。
pub(super) fn random_positions() -> Vec<Position> {
    let generator = MoveGenerator::standard();
    let rules = generator.rules();
    let mut positions = bench_positions();
    for seed in 1..=8_u64 {
        let mut rng = XorShift64::new(NonZeroU64::new(0x4e4e_5f45_4e43_0000 + seed).unwrap());
        let mut position = Position::initial();
        let mut moves = Vec::new();
        for ply in 0..240 {
            if ply % 3 == 0 {
                positions.push(position.clone());
            }
            moves.clear();
            generator.generate_moves(&position, &mut moves);
            let Some(length) = NonZeroUsize::new(moves.len()) else {
                break;
            };
            position.make_move_unchecked(moves[rng.index(length)], rules);
        }
    }
    positions
}
