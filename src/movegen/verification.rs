use std::collections::HashSet;

use super::MoveGenerator;
use crate::mv::Move;
use crate::piece::{Color, PieceCode, PieceKind};
use crate::position::{Position, PositionBuilder};
use crate::sfen::parse_sfen;
use crate::square::{BOARD_SQUARE_COUNT, Square};

fn sq(file: u8, rank: u8) -> Square {
    Square::new(file, rank).unwrap()
}

fn position(side_to_move: Color, pieces: &[(Square, Color, PieceKind)]) -> Position {
    let mut builder = PositionBuilder::new(side_to_move);
    for &(square, color, kind) in pieces {
        builder.put(square, PieceCode::new(color, kind)).unwrap();
    }
    builder.finish().unwrap()
}

#[test]
fn sfen_conversion_uses_shogiops_coordinates_and_piece_codes() {
    let position = parse_sfen("12/12/12/12/4B2l4/4S7/5+O6/7n4/12/12/12/12 b").unwrap();

    assert_eq!(
        position.piece_at(sq(4, 7)),
        Some(PieceCode::new(Color::Black, PieceKind::Bishop))
    );
    assert_eq!(
        position.piece_at(sq(7, 7)),
        Some(PieceCode::new(Color::White, PieceKind::Lance))
    );
    assert_eq!(
        position.piece_at(sq(4, 6)),
        Some(PieceCode::new(Color::Black, PieceKind::SilverGeneral))
    );
    assert_eq!(
        position.piece_at(sq(5, 5)),
        PieceCode::new(Color::Black, PieceKind::Kirin).promote()
    );
    assert_eq!(
        position.piece_at(sq(7, 4)),
        Some(PieceCode::new(Color::White, PieceKind::Lion))
    );
}

struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        assert_ne!(seed, 0);
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.state = value;
        value
    }

    fn index(&mut self, length: usize) -> usize {
        self.next() as usize % length
    }
}

fn random_piece(rng: &mut XorShift64, color: Color, kind: PieceKind) -> PieceCode {
    if rng.next() & 1 != 0
        && let Some(promoted) = PieceCode::new_promoted(color, kind)
    {
        promoted
    } else {
        PieceCode::new(color, kind)
    }
}

fn random_position(rng: &mut XorShift64, focus_kind: PieceKind) -> Position {
    let side_to_move = if rng.next() & 1 == 0 {
        Color::Black
    } else {
        Color::White
    };
    let mut builder = PositionBuilder::new(side_to_move);
    let mut occupied = [false; BOARD_SQUARE_COUNT];

    let focus_index = rng.index(BOARD_SQUARE_COUNT);
    let focus_square = Square::from_dense(focus_index).unwrap();
    builder
        .put(focus_square, random_piece(rng, side_to_move, focus_kind))
        .unwrap();
    occupied[focus_index] = true;

    for _ in 0..20 {
        let mut index = rng.index(BOARD_SQUARE_COUNT);
        while occupied[index] {
            index = rng.index(BOARD_SQUARE_COUNT);
        }
        occupied[index] = true;
        let square = Square::from_dense(index).unwrap();
        let color = if rng.next() & 1 == 0 {
            Color::Black
        } else {
            Color::White
        };
        let kind = PieceKind::ALL[rng.index(PieceKind::ALL.len())];
        builder.put(square, random_piece(rng, color, kind)).unwrap();
    }
    builder.finish().unwrap()
}

#[test]
fn seeded_random_moves_round_trip_validate_and_are_unique() {
    let generator = MoveGenerator::standard();
    let mut rng = XorShift64::new(0x554e_4d41_4b45_0004);

    for iteration in 0..64 {
        let kind = PieceKind::ALL[iteration % PieceKind::ALL.len()];
        let mut position = random_position(&mut rng, kind);
        let before = position.clone();
        let mut moves = Vec::new();
        generator.generate_moves(&position, &mut moves);
        assert_eq!(
            moves.iter().copied().collect::<HashSet<_>>().len(),
            moves.len(),
            "duplicate move in random position {iteration}",
        );
        let enemy = position.pieces_of(position.side_to_move().opposite());
        let own = position.pieces_of(position.side_to_move());
        let mut jitto_origins = HashSet::new();
        for &mv in &moves {
            if let Some(mid) = mv.mid {
                assert!(
                    enemy.contains(mid),
                    "non-enemy mid in random position {iteration}: move={mv:?}"
                );
            }
            if mv.mid.is_none() && mv.to == mv.from {
                assert!(
                    jitto_origins.insert(mv.from),
                    "multiple jitto moves from one origin in random position {iteration}: \
                     move={mv:?}"
                );
            }
            if mv.to != mv.from {
                assert!(
                    !own.contains(mv.to),
                    "own-occupied destination in random position {iteration}: move={mv:?}"
                );
            }
        }
        let before_zobrist = before.zobrist();
        for mv in moves {
            let undo = position.make_move_unchecked(mv);
            assert_eq!(position.validate(), Ok(()), "move={mv:?}");
            position.unmake_move(undo);
            assert_eq!(position.validate(), Ok(()), "unmade move={mv:?}");
            assert_eq!(position, before, "move={mv:?}");
            assert_eq!(position.zobrist(), before_zobrist, "move={mv:?}");
        }
    }
}

#[test]
fn seeded_random_playout_unmakes_to_initial_position() {
    let generator = MoveGenerator::standard();
    let mut rng = XorShift64::new(0x504c_4159_4f55_5404);
    let mut position = Position::initial();
    let initial = position.clone();
    let mut history = Vec::new();

    for ply in 0..64 {
        let mut moves = Vec::new();
        generator.generate_moves(&position, &mut moves);
        assert!(!moves.is_empty(), "no move at ply {ply}");
        let mv = moves[rng.index(moves.len())];
        history.push(position.make_move_unchecked(mv));
        assert_eq!(position.validate(), Ok(()), "ply={ply}, move={mv:?}");
    }
    while let Some(undo) = history.pop() {
        position.unmake_move(undo);
        assert_eq!(
            position.validate(),
            Ok(()),
            "invalid position while unmaking ply {}",
            history.len()
        );
    }
    assert_eq!(position, initial);
}

#[test]
fn articles_3_8_and_8_3_5_move_leaving_king_capturable_is_generated() {
    let position = position(
        Color::Black,
        &[
            (sq(4, 0), Color::Black, PieceKind::King),
            (sq(4, 2), Color::Black, PieceKind::SideMover),
            (sq(4, 5), Color::White, PieceKind::Rook),
        ],
    );
    let exposes_king = Move {
        from: sq(4, 2),
        mid: None,
        to: sq(5, 2),
        promote: false,
    };
    let mut moves = Vec::new();
    MoveGenerator::standard().generate_moves(&position, &mut moves);

    assert!(moves.contains(&exposes_king));
}

#[test]
fn article_12_8_igui_has_one_move_encoding() {
    let from = sq(5, 5);
    let mid = sq(5, 6);
    let position = position(
        Color::Black,
        &[
            (from, Color::Black, PieceKind::Lion),
            (mid, Color::White, PieceKind::SilverGeneral),
        ],
    );
    let expected = Move {
        from,
        mid: Some(mid),
        to: from,
        promote: false,
    };
    let mut moves = Vec::new();
    MoveGenerator::standard().generate_moves(&position, &mut moves);

    assert_eq!(moves.iter().filter(|&&mv| mv == expected).count(), 1);
    assert_eq!(
        moves
            .iter()
            .filter(|&&mv| {
                mv.origin() == from
                    && mv.destination() == from
                    && position.captured_squares(mv) == [Some(mid), None]
            })
            .count(),
        1
    );
}

#[test]
fn articles_16_1_and_16_11_tsukegui_requires_the_first_capture_on_mid() {
    let position = position(
        Color::Black,
        &[
            (sq(2, 2), Color::Black, PieceKind::Lion),
            (sq(3, 2), Color::White, PieceKind::SilverGeneral),
            (sq(4, 2), Color::White, PieceKind::Lion),
            (sq(4, 5), Color::White, PieceKind::Rook),
        ],
    );
    let capture_without_first_capture = Move {
        from: sq(2, 2),
        mid: None,
        to: sq(4, 2),
        promote: false,
    };
    let mut moves = Vec::new();
    MoveGenerator::standard().generate_moves(&position, &mut moves);

    assert!(!moves.contains(&capture_without_first_capture));
}

#[test]
fn article_24_equivalent_positions_ignore_quiet_move_history() {
    let initial = position(
        Color::Black,
        &[
            (sq(3, 3), Color::Black, PieceKind::Lion),
            (sq(8, 8), Color::White, PieceKind::Lion),
        ],
    );
    let mut first = initial.clone();
    first.make_move_unchecked(Move {
        from: sq(3, 3),
        mid: None,
        to: sq(3, 4),
        promote: false,
    });
    first.make_move_unchecked(Move {
        from: sq(8, 8),
        mid: None,
        to: sq(8, 7),
        promote: false,
    });
    first.make_move_unchecked(Move {
        from: sq(3, 4),
        mid: None,
        to: sq(3, 3),
        promote: false,
    });
    first.make_move_unchecked(Move {
        from: sq(8, 7),
        mid: None,
        to: sq(8, 8),
        promote: false,
    });

    let mut second = initial;
    second.make_move_unchecked(Move {
        from: sq(3, 3),
        mid: None,
        to: sq(4, 3),
        promote: false,
    });
    second.make_move_unchecked(Move {
        from: sq(8, 8),
        mid: None,
        to: sq(7, 8),
        promote: false,
    });
    second.make_move_unchecked(Move {
        from: sq(4, 3),
        mid: None,
        to: sq(3, 3),
        promote: false,
    });
    second.make_move_unchecked(Move {
        from: sq(7, 8),
        mid: None,
        to: sq(8, 8),
        promote: false,
    });

    assert_eq!(first, second);
}
