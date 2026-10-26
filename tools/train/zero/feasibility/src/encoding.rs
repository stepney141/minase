use minase::eval::features::{PIECE_STATE_COUNT, piece_state};
use minase_core::{Color, Move, Position, Square};

pub const INPUT_SIZE: usize = 95 * 144;
pub const POLICY_SIZE: usize = 249 * 144;

fn relative_square(square: Square, side: Color) -> usize {
    match side {
        Color::Black => square.dense_index(),
        Color::White => 143 - square.dense_index(),
    }
}

pub fn encode(position: &Position, out: &mut [f32]) {
    assert_eq!(out.len(), INPUT_SIZE);
    out.fill(0.0);
    let side = position.side_to_move();
    for square in position.occupied() {
        let piece = position.piece_at(square).expect("占有升には駒がある");
        let opponent = usize::from(piece.color() != Some(side));
        let plane = opponent * PIECE_STATE_COUNT + piece_state(piece);
        out[plane * 144 + relative_square(square, side)] = 1.0;
    }
    if let Some(square) = position.lion_capture_square() {
        out[94 * 144 + relative_square(square, side)] = 1.0;
    }
}

fn delta(from: Square, to: Square, side: Color) -> (i8, i8) {
    let dx = to.file() as i8 - from.file() as i8;
    let dy = to.rank() as i8 - from.rank() as i8;
    match side {
        Color::Black => (dx, dy),
        Color::White => (-dx, -dy),
    }
}

// 方向は(筋, 段)の(0, 1)から(1, 1)、(1, 0)へ順に回る。
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
        _ => panic!("8方向の単位移動ではない: ({dx}, {dy})"),
    }
}

pub fn policy_label(mv: Move, side_to_move: Color) -> usize {
    let kind = if let Some(mid) = mv.mid {
        assert!(!mv.promote, "2歩の成りはない");
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
        assert!(!mv.promote, "じっとの成りはない");
        248
    } else {
        let (dx, dy) = delta(mv.from, mv.to, side_to_move);
        if dx == 0 || dy == 0 || dx.abs() == dy.abs() {
            let distance = dx.abs().max(dy.abs()) as usize;
            direction(dx.signum(), dy.signum()) * 11 + distance - 1 + usize::from(mv.promote) * 88
        } else {
            assert!(!mv.promote, "獅子の桂馬型の跳びは成らない");
            176 + match (dx, dy) {
                (1, 2) => 0,
                (2, 1) => 1,
                (2, -1) => 2,
                (1, -2) => 3,
                (-1, -2) => 4,
                (-2, -1) => 5,
                (-2, 1) => 6,
                (-1, 2) => 7,
                _ => panic!("符号化できない移動: {mv:?}"),
            }
        }
    };
    kind * 144 + relative_square(mv.from, side_to_move)
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase_core::rng::XorShift64;
    use minase_core::{MoveGenerator, PieceCode, PieceKind, PositionBuilder, Rules};
    use std::collections::HashSet;
    use std::num::{NonZeroU64, NonZeroUsize};

    fn sq(file: u8, rank: u8) -> Square {
        Square::new(file, rank).unwrap()
    }

    fn rotate(square: Square) -> Square {
        sq(11 - square.file(), 11 - square.rank())
    }

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
        encode(&position, &mut input);
        let active: Vec<_> = input
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
        encode(&Position::empty(Color::Black), &mut input);
        assert!(input.iter().all(|&value| value == 0.0));
    }

    #[test]
    fn labels_are_injective_and_rotation_invariant_in_random_games() {
        let rules = Rules::ENGINE_DEFAULT.moves;
        let generator = MoveGenerator::new(rules);
        let mut checked_positions = 0;
        for seed in [1, 71, 2026] {
            let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
            let mut position = Position::initial();
            let mut moves = Vec::new();
            for _ in 0..500 {
                moves.clear();
                generator.generate_moves(&position, &mut moves);
                let mut labels = HashSet::new();
                for &mv in &moves {
                    let label = policy_label(mv, position.side_to_move());
                    assert!(label < 35_856, "{mv:?}: {label}");
                    assert!(labels.insert(label), "ラベルが衝突した: {mv:?}");
                    let rotated = Move {
                        from: rotate(mv.from),
                        mid: mv.mid.map(rotate),
                        to: rotate(mv.to),
                        promote: mv.promote,
                    };
                    assert_eq!(
                        label,
                        policy_label(rotated, position.side_to_move().opposite())
                    );
                }
                checked_positions += 1;
                if moves.is_empty()
                    || Color::ALL
                        .iter()
                        .any(|&side| position.royal_pieces(side).is_empty())
                {
                    break;
                }
                let index = rng.index(NonZeroUsize::new(moves.len()).unwrap());
                position.make_move_unchecked(moves[index], rules);
            }
        }
        assert!(checked_positions >= 300);
    }

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
                }
            }
        }
    }
}
