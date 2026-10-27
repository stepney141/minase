//! 正準形の着手を249面の方策ラベルへ写し、合法手のlogitを正規化する。

use super::relative_square;
use minase_core::{BOARD_SQUARE_COUNT, Color, Move, Square};

/// 移動元の升ごとに割り当てる方策平面数。
pub const POLICY_PLANES: usize = 249;
/// 249面×144升の方策要素数。
pub const POLICY_SIZE: usize = POLICY_PLANES * BOARD_SQUARE_COUNT;

/// 手番側から見た筋と段の差を返す。
fn delta(from: Square, to: Square, side: Color) -> (i8, i8) {
    let dx = to.file() as i8 - from.file() as i8;
    let dy = to.rank() as i8 - from.rank() as i8;
    match side {
        Color::Black => (dx, dy),
        Color::White => (-dx, -dy),
    }
}

/// 8方向の単位移動を方策の方向番号へ写す。
fn direction(dx: i8, dy: i8) -> usize {
    match (dx, dy) {
        (0, 1) => 0,
        (1, 1) => 1,
        (1, 0) => 2,
        (1, -1) => 3,
        (0, -1) => 4,
        (-1, -1) => 5,
        (-1, 0) => 6,
        (-1, 1) => 7,
        _ => panic!("not an eight-direction unit step: ({dx}, {dy})"),
    }
}

/// 合法手の正準形を`面 * 144 + 相対升(移動元)`の方策ラベルへ写す。
///
/// 相対升は先手番なら[`Square::dense_index`]、後手番なら`143 - dense_index`である。
/// 方向は、手番側から見た`(筋の差, 段の差)`の単位ベクトルを
/// `(0,1)、(1,1)、(1,0)、(1,−1)、(0,−1)、(−1,−1)、(−1,0)、(−1,1)`の順に0〜7とする。
/// 後手番では差の符号を反転してから番号を付ける。
///
/// | 面 | 着手の形 | 面の計算 |
/// |---|---|---|
/// | 0〜87 | 中継升なし、縦横斜めの直線上へ進む不成 | `方向 * 11 + 距離 − 1`、距離1〜11 |
/// | 88〜175 | 同じ形の成り | `88 + 方向 * 11 + 距離 − 1` |
/// | 176〜183 | 中継升なしの桂馬型の跳び | `(1,2)、(2,1)、(2,−1)、(1,−2)、(−1,−2)、(−2,−1)、(−2,1)、(−1,2)`の順 |
/// | 184〜239 | 第1段で相手駒を取り、移動元以外へ進む2歩 | `184 + 第1歩の方向 * 7 + 第2歩の方向の詰め番号` |
/// | 240〜247 | 中継升で取り、移動元へ戻る居喰い | `240 + 第1歩の方向` |
/// | 248 | 中継升なしで移動先が移動元と同じじっと | `248` |
///
/// 第2歩の詰め番号は、第1歩の逆方向`(第1歩 + 4) % 8`を除いた0〜6の番号である。
/// 直進の距離2は獅子、麒麟、鳳凰、角鷹、飛鷲の跳びと、第1段の升が空いている2歩の両方を表す。
/// [`Move`]の正準形は空いた中継升を通る2歩を直接の跳びと同じ値にするため、異なる着手が同じラベルを持つことはない。
/// [`minase_core::MoveGenerator`]が返す着手を受け取る前提であり、中継升に相手駒があることは生成器が保証する。
///
/// # Panics
/// 正準形の経路に反する2歩、2歩系・じっと・桂馬型の成り、または符号化できない移動ではパニックする。
pub fn policy_label(mv: Move, side_to_move: Color) -> usize {
    let plane = if let Some(mid) = mv.mid {
        assert!(!mv.promote, "two-step moves cannot promote");
        let (dx, dy) = delta(mv.from, mid, side_to_move);
        let first = direction(dx, dy);
        if mv.to == mv.from {
            240 + first
        } else {
            let (dx, dy) = delta(mid, mv.to, side_to_move);
            let second = direction(dx, dy);
            let reverse = (first + 4) % 8;
            assert_ne!(second, reverse);
            184 + first * 7 + second - usize::from(second > reverse)
        }
    } else if mv.from == mv.to {
        assert!(!mv.promote, "stationary moves cannot promote");
        248
    } else {
        let (dx, dy) = delta(mv.from, mv.to, side_to_move);
        if dx == 0 || dy == 0 || dx.abs() == dy.abs() {
            let distance = dx.abs().max(dy.abs()) as usize;
            direction(dx.signum(), dy.signum()) * 11 + distance - 1 + usize::from(mv.promote) * 88
        } else {
            assert!(!mv.promote, "knight-shaped lion jumps cannot promote");
            176 + match (dx, dy) {
                (1, 2) => 0,
                (2, 1) => 1,
                (2, -1) => 2,
                (1, -2) => 3,
                (-1, -2) => 4,
                (-2, -1) => 5,
                (-2, 1) => 6,
                (-1, 2) => 7,
                _ => panic!("move cannot be encoded: {mv:?}"),
            }
        }
    };
    plane * BOARD_SQUARE_COUNT + relative_square(mv.from, side_to_move)
}

/// 最大値を引いてから指数をとり、和で割ってlogitをその場で確率へ正規化する。
///
/// 方策には合法手のlogitだけを渡し、非合法手には確率を割り当てない。
/// 入力は有限値とする。空のスライスでは何もしない。
pub fn softmax(logits: &mut [f32]) {
    if logits.is_empty() {
        return;
    }
    let maximum = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut sum = 0.0;
    for logit in logits.iter_mut() {
        *logit = (*logit - maximum).exp();
        sum += *logit;
    }
    for probability in logits {
        *probability /= sum;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nn::test_util::{random_positions, rotate};
    use minase_core::test_util::sq;
    use minase_core::{MoveGenerator, Rules};
    use std::collections::HashSet;

    /// 方策の仕様表から、盤面を参照せず着手を復元する。
    fn move_from_label(label: usize, side: Color) -> Move {
        const DIRECTIONS: [(i8, i8); 8] = [
            (0, 1),
            (1, 1),
            (1, 0),
            (1, -1),
            (0, -1),
            (-1, -1),
            (-1, 0),
            (-1, 1),
        ];
        const KNIGHT_JUMPS: [(i8, i8); 8] = [
            (1, 2),
            (2, 1),
            (2, -1),
            (1, -2),
            (-1, -2),
            (-2, -1),
            (-2, 1),
            (-1, 2),
        ];
        let from = Square::from_dense(label % 144).unwrap();
        let plane = label / 144;
        let mut mv = Move {
            from,
            mid: None,
            to: from,
            promote: false,
        };
        match plane {
            0..=175 => {
                let ray = plane % 88;
                let (dx, dy) = DIRECTIONS[ray / 11];
                let distance = (ray % 11 + 1) as i8;
                mv.to = from.offset(dx * distance, dy * distance).unwrap();
                mv.promote = plane >= 88;
            }
            176..=183 => {
                let (dx, dy) = KNIGHT_JUMPS[plane - 176];
                mv.to = from.offset(dx, dy).unwrap();
            }
            184..=239 => {
                let first = (plane - 184) / 7;
                let (dx, dy) = DIRECTIONS[first];
                let mid = from.offset(dx, dy).unwrap();
                let (_, &(dx, dy)) = DIRECTIONS
                    .iter()
                    .enumerate()
                    .filter(|&(index, _)| index != (first + 4) % 8)
                    .nth((plane - 184) % 7)
                    .unwrap();
                mv.mid = Some(mid);
                mv.to = mid.offset(dx, dy).unwrap();
            }
            240..=247 => {
                let (dx, dy) = DIRECTIONS[plane - 240];
                mv.mid = Some(from.offset(dx, dy).unwrap());
            }
            248 => {}
            _ => panic!("label outside the policy planes: {label}"),
        }
        if side == Color::White {
            mv.from = rotate(mv.from);
            mv.mid = mv.mid.map(rotate);
            mv.to = rotate(mv.to);
        }
        mv
    }

    // [`policy_label`]の契約: 合法手のラベルは範囲内で単射、180度回転で不変、盤面なしで逆変換できる。
    #[test]
    fn labels_are_injective_and_rotation_invariant_in_random_games() {
        let generator = MoveGenerator::new(Rules::ENGINE_DEFAULT.moves);
        let positions = random_positions();
        assert!(positions.len() >= 300);
        let mut moves = Vec::new();
        for position in positions {
            moves.clear();
            generator.generate_moves(&position, &mut moves);
            let side = position.side_to_move();
            let mut labels = HashSet::new();
            for &mv in &moves {
                let label = policy_label(mv, side);
                assert!(label < POLICY_SIZE, "{mv:?}: {label}");
                assert!(labels.insert(label), "ラベルが衝突した: {mv:?}");
                let rotated = Move {
                    from: rotate(mv.from),
                    mid: mv.mid.map(rotate),
                    to: rotate(mv.to),
                    promote: mv.promote,
                };
                assert_eq!(label, policy_label(rotated, side.opposite()));
                assert_eq!(move_from_label(label, side), mv);
            }
        }
    }

    // [`policy_label`]の契約: 全升の正準形の着手の族がラベルを共有せず、仕様表から元の着手へ戻せる。
    #[test]
    fn canonical_move_families_never_share_labels() {
        for from in Square::all() {
            let mut moves = Vec::new();
            for to in Square::all() {
                let dx = (to.file() as i8 - from.file() as i8).abs();
                let dy = (to.rank() as i8 - from.rank() as i8).abs();
                if from == to {
                    moves.push(Move {
                        from,
                        mid: None,
                        to,
                        promote: false,
                    });
                } else if dx == 0 || dy == 0 || dx == dy {
                    for promote in [false, true] {
                        moves.push(Move {
                            from,
                            mid: None,
                            to,
                            promote,
                        });
                    }
                } else if (dx, dy) == (1, 2) || (dx, dy) == (2, 1) {
                    moves.push(Move {
                        from,
                        mid: None,
                        to,
                        promote: false,
                    });
                }
            }
            for dx in -1..=1 {
                for dy in -1..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let Some(mid) = from.offset(dx, dy) else {
                        continue;
                    };
                    for ex in -1..=1 {
                        for ey in -1..=1 {
                            if (ex != 0 || ey != 0)
                                && let Some(to) = mid.offset(ex, ey)
                            {
                                moves.push(Move {
                                    from,
                                    mid: Some(mid),
                                    to,
                                    promote: false,
                                });
                            }
                        }
                    }
                }
            }
            for side in Color::ALL {
                let mut labels = HashSet::new();
                for &mv in &moves {
                    let label = policy_label(mv, side);
                    assert!(label < POLICY_SIZE);
                    assert!(labels.insert(label), "{mv:?}: {label}");
                    assert_eq!(move_from_label(label, side), mv);
                }
            }
        }
    }

    // [`policy_label`]の契約: 不正な経路と、成れない形の成りを内部の不変条件違反として拒む。
    #[test]
    fn invalid_move_shapes_panic() {
        for (mid, to, promote) in [
            (Some(sq(5, 5)), sq(5, 6), false),
            (Some(sq(5, 6)), sq(5, 6), false),
            (Some(sq(5, 7)), sq(5, 8), false),
            (Some(sq(5, 6)), sq(5, 7), true),
            (Some(sq(5, 6)), sq(5, 5), true),
            (None, sq(5, 5), true),
            (None, sq(6, 7), true),
            (None, sq(7, 8), false),
        ] {
            let mv = Move {
                from: sq(5, 5),
                mid,
                to,
                promote,
            };
            assert!(std::panic::catch_unwind(|| policy_label(mv, Color::Black)).is_err());
        }
    }

    // [`softmax`]の契約: 合法手の確率の和は1で、定数加算に不変、空入力はそのままになる。
    #[test]
    fn softmax_normalizes_and_is_invariant_under_constant_shifts() {
        let mut logits = [-2.0, 0.0, 1.0, 3.0];
        let mut shifted = logits.map(|logit| logit + 1000.0);
        softmax(&mut logits);
        softmax(&mut shifted);
        assert!((logits.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        for (&actual, &shifted) in logits.iter().zip(&shifted) {
            assert!((actual - shifted).abs() < 1e-6);
            assert!((0.0..=1.0).contains(&actual));
        }
        // 指数関数による比を保ち、一様化しただけの処理を受理しない。
        assert!((logits[2] / logits[1] - std::f32::consts::E).abs() < 1e-6);
        let mut singleton = [-1000.0];
        softmax(&mut singleton);
        assert_eq!(singleton, [1.0]);
        let mut empty = [];
        softmax(&mut empty);
        assert!(empty.is_empty());
    }
}
