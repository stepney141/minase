//! 着手から方策の出力位置（ラベル）への変換。
//!
//! ラベルは`移動元の相対升 * 249 + 面`であり、盤面を使わずに着手の値と手番だけから決まる。
//! 相対升は[`super::encoding`]と同じく、先手番では盤の座標のまま、後手番では盤を
//! 180度回転させた升の`段 * 12 + 筋`である。
//! 移動の差分(段差, 筋差)も回転後の相対座標で数える。
//! 段0は先手側の最下段であり、手番側の前方（相手陣の方向）は相対座標で段が増える向きにあたる。
//!
//! 8方向は、(段差, 筋差)で前(+1, 0)から時計回りに次の順で番号を振る。
//!
//! | 番号 | (段差, 筋差) |
//! |---|---|
//! | 0 | (+1, 0) |
//! | 1 | (+1, +1) |
//! | 2 | (0, +1) |
//! | 3 | (−1, +1) |
//! | 4 | (−1, 0) |
//! | 5 | (−1, −1) |
//! | 6 | (0, −1) |
//! | 7 | (+1, −1) |
//!
//! 249面は次のとおり割り当てる（docs/plans/alphazero.md「方策は移動元の升ごとの249面で符号化する」）。
//!
//! - 面0〜87：経由升のない不成の直進。`方向 * 11 + (距離 − 1)`、距離は1〜11。
//! - 面88〜175：同じ直進の成り。`88 + 方向 * 11 + (距離 − 1)`。
//! - 面176〜183：獅子の桂馬型の跳び（不成）。(段差, 筋差)が
//!   (+2, +1), (+1, +2), (−1, +2), (−2, +1), (−2, −1), (−1, −2), (+1, −2), (+2, −1)の順。
//! - 面184〜239：経由升で取ってから別の升へ進む2段階移動。
//!   `184 + 第1段の方向 * 7 + 第2段の順位`。第2段の順位は、第2段の方向番号から
//!   第1段の逆方向（`(第1段の方向 + 4) % 8`）を除いて詰めた番号である。
//! - 面240〜247：居喰い（経由升があり、移動先が移動元）。`240 + 第1段の方向`。
//! - 面248：じっと（経由升がなく、移動先が移動元）。
//!
//! 着手は正準化されており（docs/plans/move-canonicalization.md）、空いた経由升を通る2歩は
//! 直接の跳びと同じ値になるので、直進の距離2の面は同じ着手だけを表す。

use core::fmt;

use minase_core::{Color, Move, Square};

use super::POLICY_PLANES;
use super::encoding::relative_square;

/// 経由升のない成りの直進の面の始まり。
const PROMOTION_OFFSET: u16 = 88;
/// 桂馬型の跳びの面の始まり。
const KNIGHT_OFFSET: u16 = 176;
/// 経由升で取ってから別の升へ進む2段階移動の面の始まり。
const TWO_STEP_OFFSET: u16 = 184;
/// 居喰いの面の始まり。
const IGUI_OFFSET: u16 = 240;
/// じっとの面。
const JITTO_PLANE: u16 = 248;

/// 8方向の(段差, 筋差)。番号の順は前から時計回り。
const DIRECTIONS: [(i8, i8); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];

/// 桂馬型の跳びの(段差, 筋差)。
const KNIGHT_JUMPS: [(i8, i8); 8] = [
    (2, 1),
    (1, 2),
    (-1, 2),
    (-2, 1),
    (-2, -1),
    (-1, -2),
    (1, -2),
    (2, -1),
];

/// 方策の249面のどれにも割り当てられない着手を表すエラー。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PolicyLabelError {
    /// ラベルを持たない着手。
    pub mv: Move,
}

impl fmt::Display for PolicyLabelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "move {:?} has no policy label", self.mv)
    }
}

impl std::error::Error for PolicyLabelError {}

/// 着手の方策ラベル（`移動元の相対升 * 249 + 面`）を返す。
///
/// # Errors
///
/// 経由升のある成り、隣接しない経由升、桂馬型以外の非直線の跳び、
/// または成りを伴う桂馬型の跳びとじっとのように、249面のどれにも当たらない着手では
/// [`PolicyLabelError`]を返す。
pub fn label(mv: Move, side_to_move: Color) -> Result<u16, PolicyLabelError> {
    let error = PolicyLabelError { mv };
    let from = relative_square(mv.from, side_to_move);
    let to = relative_square(mv.to, side_to_move);
    let plane = match mv.mid {
        None if from == to => (!mv.promote).then_some(JITTO_PLANE),
        None => {
            let (rank_delta, file_delta) = delta(from, to);
            match straight(rank_delta, file_delta) {
                Some((direction, distance)) => {
                    let promotion = if mv.promote { PROMOTION_OFFSET } else { 0 };
                    Some(promotion + direction * 11 + (distance - 1))
                }
                None => KNIGHT_JUMPS
                    .iter()
                    .position(|&jump| jump == (rank_delta, file_delta))
                    .filter(|_| !mv.promote)
                    .map(|index| KNIGHT_OFFSET + index as u16),
            }
        }
        Some(mid) if !mv.promote => {
            let mid = relative_square(mid, side_to_move);
            step(from, mid).and_then(|first| {
                if to == from {
                    Some(IGUI_OFFSET + first)
                } else {
                    let second = step(mid, to)?;
                    let reverse = (first + 4) % 8;
                    // 第2段が第1段の逆方向なら移動先は移動元であり、居喰いとして上で扱っている。
                    debug_assert_ne!(second, reverse);
                    let order = if second < reverse { second } else { second - 1 };
                    Some(TWO_STEP_OFFSET + first * 7 + order)
                }
            })
        }
        Some(_) => None,
    }
    .ok_or(error)?;
    debug_assert!(usize::from(plane) < POLICY_PLANES);
    Ok(from.dense_index() as u16 * POLICY_PLANES as u16 + plane)
}

/// 合法手の集合から、方策ラベルが`label`に一致する着手を探す。
pub fn move_from_label(
    policy_label: u16,
    side_to_move: Color,
    legal_moves: &[Move],
) -> Option<Move> {
    legal_moves
        .iter()
        .copied()
        .find(|&mv| label(mv, side_to_move) == Ok(policy_label))
}

/// 相対座標の2升の(段差, 筋差)を返す。
fn delta(from: Square, to: Square) -> (i8, i8) {
    (
        to.rank() as i8 - from.rank() as i8,
        to.file() as i8 - from.file() as i8,
    )
}

/// 直線上の差分なら(方向番号, 距離)を返す。
fn straight(rank_delta: i8, file_delta: i8) -> Option<(u16, u16)> {
    let distance = rank_delta.abs().max(file_delta.abs());
    let is_line = rank_delta == 0 || file_delta == 0 || rank_delta.abs() == file_delta.abs();
    if distance == 0 || !is_line {
        return None;
    }
    let unit = (rank_delta / distance, file_delta / distance);
    let direction = DIRECTIONS.iter().position(|&d| d == unit)?;
    Some((direction as u16, distance as u16))
}

/// 隣接する2升なら、`from`から`to`への方向番号を返す。
fn step(from: Square, to: Square) -> Option<u16> {
    let (rank_delta, file_delta) = delta(from, to);
    straight(rank_delta, file_delta)
        .filter(|&(_, distance)| distance == 1)
        .map(|(direction, _)| direction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nn::POLICY_SIZE;
    use crate::nn::test_support::{random_positions, rotate_move, rotate_position};
    use minase_core::test_util::sq;
    use minase_core::{MoveGenerator, PieceCode, PieceKind, Position, PositionBuilder};
    use std::collections::HashSet;

    /// 局面の全合法手を返す。
    fn legal_moves(position: &Position) -> Vec<Move> {
        let mut moves = Vec::new();
        MoveGenerator::standard().generate_moves(position, &mut moves);
        moves
    }

    /// 着手の値を作る。
    fn mv(from: Square, mid: Option<Square>, to: Square, promote: bool) -> Move {
        Move {
            from,
            mid,
            to,
            promote,
        }
    }

    /// ランダム対局の局面集の全合法手について、ラベルが範囲内で互いに異なり、ラベルから着手へ戻せることを検査する。
    #[test]
    fn labels_of_legal_moves_are_injective_and_invertible() {
        let positions = random_positions();
        let mut checked_moves = 0;
        for position in &positions {
            let side_to_move = position.side_to_move();
            let moves = legal_moves(position);
            let mut labels = HashSet::new();
            for &legal in &moves {
                let value = label(legal, side_to_move).unwrap();
                assert!(usize::from(value) < POLICY_SIZE);
                assert!(
                    labels.insert(value),
                    "duplicate label {value} for {legal:?}"
                );
                assert_eq!(move_from_label(value, side_to_move, &moves), Some(legal));
            }
            checked_moves += moves.len();
        }
        assert!(checked_moves > 10_000, "checked only {checked_moves} moves");
    }

    /// 後手番の局面の着手のラベルが、180度回転させた先手番の局面の対応する着手のラベルと一致することを検査する。
    #[test]
    fn white_labels_match_rotated_black_labels() {
        for position in random_positions() {
            let rotated = rotate_position(&position);
            let moves = legal_moves(&position);
            let mut rotated_moves = legal_moves(&rotated);
            let mut mapped: Vec<_> = moves.iter().map(|&legal| rotate_move(legal)).collect();
            mapped.sort_by_key(|m| (m.from, m.mid, m.to, m.promote));
            rotated_moves.sort_by_key(|m| (m.from, m.mid, m.to, m.promote));
            assert_eq!(mapped, rotated_moves);
            for legal in moves {
                assert_eq!(
                    label(legal, position.side_to_move()),
                    label(rotate_move(legal), rotated.side_to_move())
                );
            }
        }
    }

    /// 初期局面の歩兵の前進のラベルを手計算した値と比べる。
    #[test]
    fn initial_pawn_advances_have_hand_computed_labels() {
        // 先手の筋5の歩兵は段3から段4へ進む。方向0（前）、距離1なので面0。
        let black = mv(sq(5, 3), None, sq(5, 4), false);
        assert_eq!(label(black, Color::Black), Ok((3 * 12 + 5) * 249));

        // 後手の筋5の歩兵は段8から段7へ進む。回転後は(筋6, 段3)から前へ1升で面0。
        let white = mv(sq(5, 8), None, sq(5, 7), false);
        assert_eq!(label(white, Color::White), Ok((3 * 12 + 6) * 249));

        let mut moves = Vec::new();
        MoveGenerator::standard().generate_moves(&Position::initial(), &mut moves);
        assert!(moves.contains(&black));
    }

    /// 獅子のじっと、居喰い、2段階移動、桂馬型の跳び、および成りのラベルを手計算した値と比べる。
    #[test]
    fn lion_and_promotion_moves_have_hand_computed_labels() {
        let lion = sq(5, 5);
        let from = (5 * 12 + 5) * 249;
        // じっと。
        assert_eq!(
            label(mv(lion, None, lion, false), Color::Black),
            Ok(from + 248)
        );
        // 前方(段6)の駒を取って戻る居喰い：方向0。
        assert_eq!(
            label(mv(lion, Some(sq(5, 6)), lion, false), Color::Black),
            Ok(from + 240)
        );
        // 右(筋6)の駒を取って前(段6)へ進む2段階移動：第1段は方向2、第2段は方向0。
        // 第1段の逆方向は6なので、第2段の順位は0のまま。
        assert_eq!(
            label(mv(lion, Some(sq(6, 5)), sq(6, 6), false), Color::Black),
            Ok(from + 184 + 2 * 7)
        );
        // 前の駒を取って左後ろ(方向5)へ進む2段階移動：逆方向4を除くので順位は4。
        assert_eq!(
            label(mv(lion, Some(sq(5, 6)), sq(4, 5), false), Color::Black),
            Ok(from + 184 + 4)
        );
        // 前へ2升、右へ1升の桂馬型の跳び。
        assert_eq!(
            label(mv(lion, None, sq(6, 7), false), Color::Black),
            Ok(from + 176)
        );
        // 左後ろへ3升進んで成る：方向5、距離3。
        assert_eq!(
            label(mv(lion, None, sq(2, 2), true), Color::Black),
            Ok(from + 88 + 5 * 11 + 2)
        );
    }

    /// 249面に割り当てのない着手をエラーとすることを検査する。
    #[test]
    fn moves_outside_the_249_planes_are_rejected() {
        let from = sq(5, 5);
        let invalid = [
            // 経由升のある成り。
            mv(from, Some(sq(5, 6)), sq(5, 7), true),
            // 隣接しない経由升。
            mv(from, Some(sq(5, 7)), sq(5, 8), false),
            // 桂馬型以外の非直線の跳び。
            mv(from, None, sq(6, 8), false),
            // 成りを伴う桂馬型の跳び。
            mv(from, None, sq(6, 7), true),
            // 成りを伴うじっと。
            mv(from, None, from, true),
        ];
        for invalid in invalid {
            assert_eq!(
                label(invalid, Color::Black),
                Err(PolicyLabelError { mv: invalid })
            );
        }
    }

    /// 獅子のじっと、居喰い、および2段階移動が合法な局面で、生成された着手のラベルが手計算と一致することを検査する。
    #[test]
    fn generated_lion_moves_use_the_two_step_planes() {
        let mut builder = PositionBuilder::new(Color::Black);
        let pieces = [
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Lion),
            (sq(5, 6), Color::White, PieceKind::GoldGeneral),
        ];
        for (square, color, kind) in pieces {
            builder
                .put(square, PieceCode::new(color, kind).unwrap())
                .unwrap();
        }
        let position = builder.finish().unwrap();
        let moves = legal_moves(&position);
        let from = (5 * 12 + 5) * 249;
        for (plane, expected) in [
            (248, mv(sq(5, 5), None, sq(5, 5), false)),
            (240, mv(sq(5, 5), Some(sq(5, 6)), sq(5, 5), false)),
            (184, mv(sq(5, 5), Some(sq(5, 6)), sq(5, 7), false)),
        ] {
            assert!(moves.contains(&expected), "{expected:?} is not generated");
            assert_eq!(
                move_from_label(from + plane, Color::Black, &moves),
                Some(expected)
            );
        }
    }
}
