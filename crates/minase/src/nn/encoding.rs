//! 手番側から見た局面を159面のネット入力へ符号化する。

use super::control::{CONTROL_PLANES, encode_control};
use super::{NnError, relative_square};
use crate::eval::features::{PIECE_STATE_COUNT, piece_state};
use minase_core::{BOARD_SQUARE_COUNT, Position};

/// 駒の94面、先獅子の1面、および疑似利きの64面からなる入力平面数。
pub const INPUT_PLANES: usize = 95 + CONTROL_PLANES;
/// 159面×144升の入力要素数。
pub const INPUT_SIZE: usize = INPUT_PLANES * BOARD_SQUARE_COUNT;

/// 局面を手番側から見た159面へ符号化し、`out`を上書きする。
///
/// 並びは`[面][升]`で、添字は`面 * 144 + 相対升`である。
/// 相対升は先手番なら[`minase_core::Square::dense_index`]、後手番なら
/// `143 - dense_index`とし、筋と段の両方を反転して盤を180度回転する。
/// 面0〜93は`相対陣営 * 47 + piece_state(piece)`で、手番側の駒を陣営0、相手の駒を陣営1とする。
/// 面94は、直前の着手で獅子以外の駒が相手獅子を取った升（[`Position::lion_capture_square`]）に1を置く。
/// 先獅子（RULES.md第15条）の禁止は、この升と盤面から決まる。
/// 面95〜158は[`encode_control`]が返す駒種ごとの疑似利きと利き数の面である。
/// 全要素を0にした後に該当箇所へ1を置くため、値は0.0か1.0だけである。
///
/// 規則セット`engine-default`（P0でP5なし）では、この159面で次の合法手が決まる。
/// 過去の局面の履歴と反復の回数は含めない。
///
/// # Errors
/// 成り権の保留がある局面は[`NnError::PromotionDeferred`]を返す。
/// 保留はP1・P2・P5で生じ得るが、この159面では表せない。エラー時の`out`の内容は未規定である。
///
/// # Panics
/// `out.len()`が[`INPUT_SIZE`]でなければパニックする。
pub fn encode(position: &Position, out: &mut [f32]) -> Result<(), NnError> {
    assert_eq!(out.len(), INPUT_SIZE);
    if !position.promotion_deferred().is_empty() {
        return Err(NnError::PromotionDeferred);
    }
    out.fill(0.0);
    let side = position.side_to_move();
    for square in position.occupied() {
        let piece = position
            .piece_at(square)
            .expect("occupied square has a piece");
        let opponent = usize::from(piece.color() != Some(side));
        let plane = opponent * PIECE_STATE_COUNT + piece_state(piece);
        out[plane * BOARD_SQUARE_COUNT + relative_square(square, side)] = 1.0;
    }
    if let Some(square) = position.lion_capture_square() {
        out[94 * BOARD_SQUARE_COUNT + relative_square(square, side)] = 1.0;
    }
    encode_control(position, &mut out[95 * BOARD_SQUARE_COUNT..]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nn::test_util::{random_positions, rotate};
    use minase_core::rules::PromotionRule;
    use minase_core::test_util::{position, sq};
    use minase_core::{Color, Move, MoveGenerator, PieceCode, PieceKind, PositionBuilder, Rules};

    // [`encode`]の契約: バッファを消去し、後手番では両座標を反転し、先獅子を面94へ置く。
    #[test]
    fn encoding_clears_buffer_and_rotates_both_coordinates() {
        let mut builder = PositionBuilder::new(Color::White);
        builder
            .put(
                sq(2, 3),
                PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
            )
            .unwrap();
        builder
            .put(
                sq(9, 8),
                PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap(),
            )
            .unwrap();
        let mut position = builder.finish().unwrap();
        position.set_lion_capture(Some(sq(5, 7))).unwrap();
        let mut input = vec![7.0; INPUT_SIZE];
        encode(&position, &mut input).unwrap();
        let active: Vec<_> = input[..95 * BOARD_SQUARE_COUNT]
            .iter()
            .enumerate()
            .filter_map(|(i, &value)| (value != 0.0).then_some((i, value)))
            .collect();
        assert_eq!(
            active,
            vec![
                (29 * 144 + 105, 1.0),
                (64 * 144 + 38, 1.0),
                (94 * 144 + 54, 1.0)
            ]
        );
        assert!(input.iter().all(|&value| value == 0.0 || value == 1.0));
        encode(&Position::empty(Color::Black), &mut input).unwrap();
        assert!(input.iter().all(|&value| value == 0.0));
    }

    /// 盤と先獅子の升を180度回転し、手番と駒の色を交換する。
    fn rotate_and_swap_colors(position: &Position) -> Position {
        let mut builder = PositionBuilder::new(position.side_to_move().opposite());
        for square in position.occupied() {
            let piece = position.piece_at(square).unwrap();
            let color = piece.color().unwrap().opposite();
            let kind = piece.kind().unwrap();
            let rotated = if piece.is_promoted() {
                PieceCode::new_promoted(color, kind)
            } else {
                PieceCode::new(color, kind)
            }
            .unwrap();
            builder.put(rotate(square), rotated).unwrap();
        }
        let mut rotated = builder.finish().unwrap();
        rotated
            .set_lion_capture(position.lion_capture_square().map(rotate))
            .unwrap();
        rotated
    }

    // [`encode`]の契約: 後手番の入力は、180度回転と陣営交換をした先手番の入力に等しい。
    #[test]
    fn white_inputs_match_rotation_with_colors_swapped_in_random_games() {
        let mut white_input = vec![0.0; INPUT_SIZE];
        let mut black_input = vec![0.0; INPUT_SIZE];
        let mut checked_positions = 0;
        for mut position in random_positions()
            .into_iter()
            .filter(|position| position.side_to_move() == Color::White)
        {
            // 先獅子の自然発生に依存せず、その面の回転も各盤面で確かめる。
            let trigger = position.pieces_of(Color::Black).into_iter().next();
            position.set_lion_capture(trigger).unwrap();
            encode(&position, &mut white_input).unwrap();
            encode(&rotate_and_swap_colors(&position), &mut black_input).unwrap();
            assert_eq!(white_input, black_input);
            checked_positions += 1;
        }
        assert!(checked_positions >= 100);
    }

    // [`encode`]の契約: 159面で表せない成り権の保留は、規則によらず専用エラーで拒む。
    #[test]
    fn deferred_promotion_is_rejected() {
        for (promotion, p5) in [
            (PromotionRule::P1, false),
            (PromotionRule::P2, false),
            (PromotionRule::P0, true),
        ] {
            let mut rules = Rules::ENGINE_DEFAULT.moves;
            rules.promotion = promotion;
            rules.p5 = p5;
            let generator = MoveGenerator::new(rules);
            let mut position = position(Color::Black, &[(sq(4, 7), Color::Black, PieceKind::Pawn)]);
            position
                .try_make_move(
                    Move {
                        from: sq(4, 7),
                        mid: None,
                        to: sq(4, 8),
                        promote: false,
                    },
                    &generator,
                )
                .unwrap();
            assert!(!position.promotion_deferred().is_empty());
            assert!(matches!(
                encode(&position, &mut vec![0.0; INPUT_SIZE]),
                Err(NnError::PromotionDeferred)
            ));
        }
    }
}
