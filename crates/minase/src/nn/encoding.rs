//! 局面からネットの入力平面への符号化。
//!
//! 入力は手番側から見た12×12の盤の96面であり、面優先の並び
//! （`out[plane * 144 + square]`）で`f32`の配列へ書く。
//! 面0〜46は手番側の駒、面47〜93は相手側の駒を、
//! [`piece_state_of`]の47状態（29駒種と成れる18駒種の「まだ成れる」状態）の番号順に割り当てる。
//! 面94は、先獅子（RULES.md第15条）で直後に取れない獅子を取った駒の升を示す。
//! 面95は、成り権を保留している駒（規則P1、P2、P5、`Position::promotion_deferred`）の升を示す。
//! 手番側と相手側の駒を1面にまとめ、どちらの駒かは駒の面で区別する。
//! 升の番号は手番側から見た相対座標の`段 * 12 + 筋`であり、先手番では盤の座標のまま、
//! 後手番では盤を180度回転させた(11 − 筋, 11 − 段)を使う。

use minase_core::{BOARD_SQUARE_COUNT, Color, Position, Square};

use super::INPUT_PLANES;
use crate::eval::pst::{PIECE_STATE_COUNT, piece_state_of};

/// 入力平面全体の要素数（96面×144升）。
const INPUT_SIZE: usize = INPUT_PLANES * BOARD_SQUARE_COUNT;

/// 先獅子の升を示す面の番号。
const LION_CAPTURE_PLANE: usize = 2 * PIECE_STATE_COUNT;

/// 成り権を保留している駒の升を示す面の番号。
const PROMOTION_DEFERRED_PLANE: usize = LION_CAPTURE_PLANE + 1;

/// 升を手番側から見た相対座標へ写す。先手番はそのまま、後手番は盤を180度回転させる。
pub(super) fn relative_square(square: Square, side_to_move: Color) -> Square {
    match side_to_move {
        Color::Black => square,
        Color::White => Square::new(11 - square.file(), 11 - square.rank())
            .expect("180-degree rotation keeps a square on the board"),
    }
}

/// 局面を手番側から見た96面の入力平面として`out`へ書く。
///
/// `out`は先に0で埋めるので、バッチの領域を使い回してよい。
///
/// # Panics
///
/// `out`の長さが96×144でない場合にパニックする。
pub fn encode_into(position: &Position, out: &mut [f32]) {
    assert_eq!(
        out.len(),
        INPUT_SIZE,
        "input buffer must hold {INPUT_PLANES} planes of {BOARD_SQUARE_COUNT} squares"
    );
    out.fill(0.0);
    let side_to_move = position.side_to_move();
    for square in position.occupied() {
        let piece = position
            .piece_at(square)
            .expect("occupied square must contain a piece");
        let owner = usize::from(piece.color() != Some(side_to_move));
        let plane = owner * PIECE_STATE_COUNT + piece_state_of(piece);
        out[plane * BOARD_SQUARE_COUNT + relative_square(square, side_to_move).dense_index()] = 1.0;
    }
    if let Some(square) = position.lion_capture_square() {
        out[LION_CAPTURE_PLANE * BOARD_SQUARE_COUNT
            + relative_square(square, side_to_move).dense_index()] = 1.0;
    }
    for square in position.promotion_deferred() {
        out[PROMOTION_DEFERRED_PLANE * BOARD_SQUARE_COUNT
            + relative_square(square, side_to_move).dense_index()] = 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nn::test_support::{deferred_positions, random_positions, rotate_position};
    use minase_core::test_util::sq;
    use minase_core::{PieceCode, PieceKind, PositionBuilder};

    /// 局面の入力平面を新しい配列へ書いて返す。
    fn encode(position: &Position) -> Vec<f32> {
        let mut out = vec![f32::NAN; INPUT_SIZE];
        encode_into(position, &mut out);
        out
    }

    /// 値が1の要素の（面, 升）を返す。
    fn ones(planes: &[f32]) -> Vec<(usize, usize)> {
        planes
            .iter()
            .enumerate()
            .filter(|&(_, &value)| value == 1.0)
            .map(|(index, _)| (index / BOARD_SQUARE_COUNT, index % BOARD_SQUARE_COUNT))
            .collect()
    }

    /// 初期局面の先手の歩兵と後手の歩兵が、手番側と相手側の面の手計算した升に立つことを検査する。
    #[test]
    fn initial_position_marks_pawns_on_expected_planes() {
        let planes = encode(&Position::initial());
        assert!(planes.iter().all(|&value| value == 0.0 || value == 1.0));
        assert_eq!(ones(&planes).len(), 92);

        // 成っていない歩兵の状態番号は29（evaluation.md「入力特徴」）。
        let own_pawn = 29;
        let opponent_pawn = PIECE_STATE_COUNT + 29;
        for file in 0..12 {
            // 先手の歩兵は段3、後手の歩兵は段8にある。
            assert_eq!(planes[own_pawn * 144 + 3 * 12 + file], 1.0);
            assert_eq!(planes[opponent_pawn * 144 + 8 * 12 + file], 1.0);
        }
    }

    /// 後手番の局面と、それを180度回転させて陣営を入れ替えた先手番の局面の入力平面が一致することを検査する。
    #[test]
    fn white_to_move_matches_rotated_black_to_move() {
        let positions = random_positions();
        assert!(
            positions
                .iter()
                .any(|position| position.side_to_move() == Color::White)
        );
        for position in positions {
            let rotated = rotate_position(&position);
            assert_eq!(encode(&position), encode(&rotated));
        }
    }

    /// 先獅子の升が回転後の相対座標で面94に立つことを検査する。
    #[test]
    fn lion_capture_square_is_marked_in_relative_coordinates() {
        let mut builder = PositionBuilder::new(Color::White);
        let pieces = [
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(4, 5), Color::Black, PieceKind::GoldGeneral),
            (sq(8, 9), Color::White, PieceKind::Lion),
        ];
        for (square, color, kind) in pieces {
            builder
                .put(square, PieceCode::new(color, kind).unwrap())
                .unwrap();
        }
        let mut position = builder.finish().unwrap();
        position.set_lion_capture(Some(sq(4, 5))).unwrap();

        let planes = encode(&position);
        // 後手番なので(筋4, 段5)は(筋7, 段6)へ写る。
        assert_eq!(planes[LION_CAPTURE_PLANE * 144 + 6 * 12 + 7], 1.0);
        assert_eq!(
            ones(&planes)
                .iter()
                .filter(|&&(plane, _)| plane == LION_CAPTURE_PLANE)
                .count(),
            1
        );
    }

    /// 値が1の要素のうち、成り権保留の面にある升を返す。
    fn deferred_squares(planes: &[f32]) -> Vec<usize> {
        ones(planes)
            .into_iter()
            .filter(|&(plane, _)| plane == PROMOTION_DEFERRED_PLANE)
            .map(|(_, square)| square)
            .collect()
    }

    /// 成り権を保留した手番側と相手側の駒の升が、回転後の相対座標で面95に立つことを検査する。
    #[test]
    fn promotion_deferred_squares_are_marked_in_relative_coordinates() {
        let mut builder = PositionBuilder::new(Color::White);
        let pieces = [
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(5, 8), Color::Black, PieceKind::Pawn),
            (sq(2, 1), Color::White, PieceKind::SilverGeneral),
        ];
        for (square, color, kind) in pieces {
            builder
                .put(square, PieceCode::new(color, kind).unwrap())
                .unwrap();
        }
        builder.mark_promotion_deferred(sq(5, 8)).unwrap();
        builder.mark_promotion_deferred(sq(2, 1)).unwrap();
        let position = builder.finish().unwrap();

        let planes = encode(&position);
        // 後手番なので(筋5, 段8)は(筋6, 段3)へ、(筋2, 段1)は(筋9, 段10)へ写る。
        assert_eq!(deferred_squares(&planes), vec![3 * 12 + 6, 10 * 12 + 9]);
        assert_eq!(ones(&planes).len(), 4 + 2);
    }

    /// 成り権の保留を追跡しない規則の局面では、面95がすべて0であることを検査する。
    #[test]
    fn promotion_deferred_plane_is_empty_without_deferred_pieces() {
        for position in random_positions() {
            assert!(deferred_squares(&encode(&position)).is_empty());
        }
    }

    /// 規則P2の対局で成り権を保留した局面について、面95が保留の升だけに立ち、
    /// 180度回転させて陣営を入れ替えた局面と入力平面が一致することを検査する。
    #[test]
    fn promotion_deferred_positions_encode_symmetrically() {
        let positions = deferred_positions();
        assert!(
            positions
                .iter()
                .any(|position| position.side_to_move() == Color::White)
        );
        assert!(
            positions
                .iter()
                .any(|position| position.side_to_move() == Color::Black)
        );
        for position in positions {
            let planes = encode(&position);
            let side_to_move = position.side_to_move();
            let mut expected: Vec<_> = position
                .promotion_deferred()
                .into_iter()
                .map(|square| relative_square(square, side_to_move).dense_index())
                .collect();
            expected.sort_unstable();
            assert_eq!(deferred_squares(&planes), expected);
            assert_eq!(planes, encode(&rotate_position(&position)));
        }
    }
}
