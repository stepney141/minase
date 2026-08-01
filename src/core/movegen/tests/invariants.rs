use std::collections::HashSet;

use super::super::{
    IllegalMove, MoveGenerator, generate_lion_double_and_jumps, piece_control_with_tables,
};
use crate::core::attacks::{
    SpecialMovement, attack_tables, movement_profile, movement_profile_data,
};
use crate::core::bitboard::Bitboard;
use crate::core::direction::step_square;
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::{Position, PositionBuilder};
use crate::core::rules::{RuleCode, Rules};
use crate::core::square::{BOARD_SQUARE_COUNT, Square};
use crate::test_util::sq;

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

fn assert_playout_position_invariants(
    position: &Position,
    rule_set_name: &str,
    phase: &str,
    ply: usize,
) {
    assert_eq!(
        position.validate(),
        Ok(()),
        "rule_set={rule_set_name}, phase={phase}, ply={ply}"
    );
    assert_eq!(
        position.zobrist(),
        position.recompute_zobrist(),
        "zobrist mismatch: rule_set={rule_set_name}, phase={phase}, ply={ply}"
    );
    assert_eq!(
        position.rights_zobrist(),
        position.recompute_rights_zobrist(),
        "rights zobrist mismatch: rule_set={rule_set_name}, phase={phase}, ply={ply}"
    );
}

fn generated_unique_moves(
    generator: &MoveGenerator,
    position: &Position,
    rule_set_name: &str,
    ply: usize,
) -> Vec<Move> {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    assert_eq!(
        moves.iter().copied().collect::<HashSet<_>>().len(),
        moves.len(),
        "duplicate move: rule_set={rule_set_name}, ply={ply}"
    );
    moves
}

#[test]
fn representative_rule_set_playouts_preserve_all_position_invariants() {
    const PLY_LIMIT: usize = 96;
    const RULE_SETS: [(&str, &[RuleCode], u64); 5] = [
        (
            "L1+L2+P3+R1+E1",
            &[
                RuleCode::L1,
                RuleCode::L2,
                RuleCode::P3,
                RuleCode::R1,
                RuleCode::E1,
            ],
            0x5255_4c45_5345_5401,
        ),
        (
            "L3+P4+R1",
            &[RuleCode::L3, RuleCode::P4, RuleCode::R1],
            0x5255_4c45_5345_5402,
        ),
        (
            "P1+R1",
            &[RuleCode::P1, RuleCode::R1],
            0x5255_4c45_5345_5403,
        ),
        (
            "P2+R1",
            &[RuleCode::P2, RuleCode::R1],
            0x5255_4c45_5345_5404,
        ),
        (
            "L1+L2+L3+P1+P3+P4+R2+E1+E2",
            &[
                RuleCode::L1,
                RuleCode::L2,
                RuleCode::L3,
                RuleCode::P1,
                RuleCode::P3,
                RuleCode::P4,
                RuleCode::R2,
                RuleCode::E1,
                RuleCode::E2,
            ],
            0x5255_4c45_5345_5405,
        ),
    ];

    for (rule_set_name, codes, seed) in RULE_SETS {
        let rules = Rules::from_codes(codes).unwrap();
        let generator = MoveGenerator::new(rules);
        let mut rng = XorShift64::new(seed);
        let mut position = Position::initial();
        let initial = position.clone();
        let mut history = Vec::new();
        let mut saw_promotion_deferred = false;

        for ply in 0..PLY_LIMIT {
            assert_playout_position_invariants(&position, rule_set_name, "forward", ply);
            let moves = generated_unique_moves(&generator, &position, rule_set_name, ply);
            if moves.is_empty() {
                break;
            }

            let mv = moves[rng.index(moves.len())];
            let before = position.clone();
            let undo = position.make_move_unchecked(mv, rules);
            history.push((undo, before));
            saw_promotion_deferred |= !position.promotion_deferred().is_empty();
            assert_playout_position_invariants(&position, rule_set_name, "forward", ply + 1);
        }

        assert_playout_position_invariants(
            &position,
            rule_set_name,
            "forward-final",
            history.len(),
        );
        let _ = generated_unique_moves(&generator, &position, rule_set_name, history.len());

        while let Some((undo, before)) = history.pop() {
            position.unmake_move(undo);
            assert_playout_position_invariants(&position, rule_set_name, "backward", history.len());
            assert_eq!(
                position,
                before,
                "round-trip mismatch: rule_set={rule_set_name}, ply={}",
                history.len()
            );
        }

        assert_eq!(position.zobrist(), initial.zobrist(), "{rule_set_name}");
        assert_eq!(
            position.rights_zobrist(),
            initial.rights_zobrist(),
            "{rule_set_name}"
        );
        assert_eq!(
            position.promotion_deferred(),
            initial.promotion_deferred(),
            "{rule_set_name}"
        );
        assert_eq!(
            position.lion_taken_by_non_lion(),
            initial.lion_taken_by_non_lion(),
            "{rule_set_name}"
        );
        assert_eq!(position, initial, "{rule_set_name}");
        if rules.contains(RuleCode::P1) {
            assert!(
                saw_promotion_deferred,
                "P1 promotion deferral did not occur: rule_set={rule_set_name}, seed={seed:#x}"
            );
        }
    }
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
            let undo = position.make_move_unchecked(mv, Rules::standard());
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
        history.push(position.make_move_unchecked(mv, Rules::standard()));
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
fn every_initial_generated_move_round_trips_exactly() {
    let generator = MoveGenerator::standard();
    let mut position = Position::initial();
    let before = position.clone();
    let mut moves = Vec::new();
    generator.generate_moves(&position, &mut moves);

    assert!(!moves.is_empty());
    assert_eq!(
        moves.iter().copied().collect::<HashSet<_>>().len(),
        moves.len(),
    );
    for mv in moves {
        let undo = position.make_move_unchecked(mv, Rules::standard());
        assert!(position.validate().is_ok());
        position.unmake_move(undo);
        assert_eq!(position, before);
    }
}

#[test]
fn initial_position_has_generated_moves_without_mutation() {
    let generator = MoveGenerator::standard();
    let position = Position::initial();
    let before = position.clone();
    let mut moves = Vec::new();
    generator.generate_moves(&position, &mut moves);

    assert!(!moves.is_empty());
    assert_eq!(position, before);
}

#[test]
fn all_generated_special_moves_round_trip() {
    let mut builder = PositionBuilder::new(Color::Black);
    for (square, color, kind) in [
        (sq(5, 5), Color::Black, PieceKind::Lion),
        (sq(1, 1), Color::Black, PieceKind::HornedFalcon),
        (sq(9, 1), Color::Black, PieceKind::SoaringEagle),
        (sq(5, 6), Color::White, PieceKind::Pawn),
        (sq(6, 6), Color::White, PieceKind::SilverGeneral),
        (sq(1, 2), Color::White, PieceKind::Pawn),
        (sq(8, 2), Color::White, PieceKind::Pawn),
    ] {
        builder.put(square, PieceCode::new(color, kind)).unwrap();
    }
    let mut position = builder.finish().unwrap();
    let before = position.clone();
    let mut moves = Vec::new();
    MoveGenerator::standard().generate_moves(&position, &mut moves);
    let special_origins = [sq(5, 5), sq(1, 1), sq(9, 1)];
    let special: Vec<_> = moves
        .into_iter()
        .filter(|mv| special_origins.contains(&mv.from))
        .collect();

    assert_eq!(
        special.iter().copied().collect::<HashSet<_>>().len(),
        special.len(),
    );
    assert!(special.iter().any(|&mv| {
        mv.from == mv.to && mv.mid.is_some() && position.captured_squares(mv)[0].is_some()
    }));
    assert!(special.iter().any(|mv| mv.mid.is_some()));
    assert!(special.iter().any(|mv| mv.mid.is_none()));
    for mv in special {
        let undo = position.make_move_unchecked(mv, Rules::standard());
        assert!(position.validate().is_ok());
        position.unmake_move(undo);
        assert_eq!(position, before);
    }
}

#[test]
fn lone_central_lion_generates_exactly_25_canonical_moves() {
    let from = sq(5, 5);
    let mut builder = PositionBuilder::new(Color::Black);
    builder
        .put(from, PieceCode::new(Color::Black, PieceKind::Lion))
        .unwrap();
    let position = builder.finish().unwrap();
    let mut moves = Vec::new();
    MoveGenerator::standard().generate_moves(&position, &mut moves);

    assert_eq!(moves.len(), 25);
    assert_eq!(moves.iter().filter(|mv| mv.to != from).count(), 24,);
    assert_eq!(moves.iter().filter(|mv| mv.to == from).count(), 1,);
    assert!(moves.iter().all(|mv| mv.mid.is_none()));
}

#[test]
fn generated_moves_have_no_duplicate_values() {
    let mut builder = PositionBuilder::new(Color::Black);
    for (square, piece) in [
        (sq(5, 5), PieceCode::new(Color::Black, PieceKind::Lion)),
        (
            sq(1, 1),
            PieceCode::new_promoted(Color::Black, PieceKind::HornedFalcon).unwrap(),
        ),
        (
            sq(9, 1),
            PieceCode::new_promoted(Color::Black, PieceKind::SoaringEagle).unwrap(),
        ),
        (sq(5, 6), PieceCode::new(Color::White, PieceKind::Pawn)),
        (
            sq(6, 6),
            PieceCode::new(Color::White, PieceKind::SilverGeneral),
        ),
        (sq(1, 2), PieceCode::new(Color::White, PieceKind::Pawn)),
        (sq(8, 2), PieceCode::new(Color::White, PieceKind::Pawn)),
    ] {
        builder.put(square, piece).unwrap();
    }
    let position = builder.finish().unwrap();
    let mut moves = Vec::new();
    MoveGenerator::standard().generate_moves(&position, &mut moves);

    assert_eq!(
        moves.iter().copied().collect::<HashSet<_>>().len(),
        moves.len(),
    );
}

#[test]
fn checked_move_application_accepts_only_generated_legal_moves() {
    let mut builder = PositionBuilder::new(Color::Black);
    for (square, color, kind) in [
        (sq(0, 0), Color::Black, PieceKind::King),
        (sq(11, 11), Color::White, PieceKind::King),
        (sq(4, 4), Color::Black, PieceKind::Pawn),
    ] {
        builder.put(square, PieceCode::new(color, kind)).unwrap();
    }
    let mut position = builder.finish().unwrap();
    let generator = MoveGenerator::standard();
    let legal = Move {
        from: sq(4, 4),
        mid: None,
        to: sq(4, 5),
        promote: false,
    };
    assert!(position.try_make_move(legal, &generator).is_ok());
    assert_eq!(position.side_to_move(), Color::White);

    let mut original = PositionBuilder::new(Color::Black);
    for (square, color, kind) in [
        (sq(0, 0), Color::Black, PieceKind::King),
        (sq(11, 11), Color::White, PieceKind::King),
        (sq(4, 4), Color::Black, PieceKind::Pawn),
    ] {
        original.put(square, PieceCode::new(color, kind)).unwrap();
    }
    let mut original = original.finish().unwrap();
    let illegal = Move {
        from: sq(4, 4),
        mid: None,
        to: sq(5, 4),
        promote: false,
    };
    assert_eq!(
        original.try_make_move(illegal, &generator),
        Err(IllegalMove(illegal))
    );
}

#[test]
fn a_move_sequence_can_be_unmade_in_reverse() {
    let generator = MoveGenerator::standard();
    let mut position = Position::initial();
    let before = position.clone();
    let mut history = Vec::new();

    for _ in 0..8 {
        let mut moves = Vec::new();
        generator.generate_moves(&position, &mut moves);
        let mv = moves
            .into_iter()
            .find(|&mv| {
                position
                    .captured_squares(mv)
                    .into_iter()
                    .all(|capture| capture.is_none())
            })
            .expect("initial sequence must have a quiet continuation");
        let undo = position.make_move_unchecked(mv, Rules::standard());
        history.push(undo);
    }

    while let Some(undo) = history.pop() {
        position.unmake_move(undo);
    }
    assert_eq!(position, before);
}

fn reference_control(position: &Position, color: Color, kind: PieceKind, from: Square) -> Bitboard {
    let profile = movement_profile_data(movement_profile(kind));
    let mut result = Bitboard::EMPTY;
    for delta in profile.fixed_deltas {
        let (file, rank) = delta.for_color(color);
        if let Some(to) = from.offset(file, rank) {
            result.set(to);
        }
    }
    for slide in profile.slides {
        let direction = slide.direction.for_color(color);
        let mut current = from;
        let mut steps = 0;
        while slide.max_steps.is_none_or(|maximum| steps < maximum) {
            let Some(next) = step_square(current, direction) else {
                break;
            };
            result.set(next);
            steps += 1;
            if position.occupied().contains(next) {
                break;
            }
            current = next;
        }
    }

    match profile.special {
        SpecialMovement::None => {}
        SpecialMovement::Lion => {
            for file in -2_i8..=2 {
                for rank in -2_i8..=2 {
                    if (file != 0 || rank != 0)
                        && let Some(to) = from.offset(file, rank)
                    {
                        result.set(to);
                    }
                }
            }
        }
        SpecialMovement::LionLike(lion_like) => {
            for relative in lion_like.directions {
                let direction = relative.for_color(color);
                if let Some(first) = step_square(from, direction) {
                    result.set(first);
                    if let Some(second) = step_square(first, direction) {
                        result.set(second);
                    }
                }
            }
        }
    }
    result
}

#[test]
fn every_piece_control_matches_coordinate_reference() {
    let mut builder = PositionBuilder::new(Color::Black);
    for square in Square::all().filter(|square| square.dense_index() % 11 == 0) {
        builder
            .put(square, PieceCode::new(Color::White, PieceKind::Pawn))
            .unwrap();
    }
    let position = builder.finish().unwrap();

    for color in Color::ALL {
        for kind in PieceKind::ALL {
            for from in Square::all() {
                assert_eq!(
                    piece_control_with_tables(
                        attack_tables(),
                        position.occupied(),
                        color,
                        kind,
                        from,
                    ),
                    reference_control(&position, color, kind, from),
                    "color={color:?}, kind={kind:?}, from={from:?}",
                );
            }
        }
    }
}

#[test]
fn kirin_jump_ignores_intermediate_occupancy() {
    let from = Square::new(4, 4).unwrap();
    let middle = Square::new(4, 5).unwrap();
    let target = Square::new(4, 6).unwrap();
    let mut builder = PositionBuilder::new(Color::Black);
    builder
        .put(from, PieceCode::new(Color::Black, PieceKind::Kirin))
        .unwrap();
    builder
        .put(middle, PieceCode::new(Color::Black, PieceKind::Pawn))
        .unwrap();
    let position = builder.finish().unwrap();

    assert!(
        piece_control_with_tables(
            attack_tables(),
            position.occupied(),
            Color::Black,
            PieceKind::Kirin,
            from,
        )
        .contains(target)
    );
}

#[test]
fn already_promoted_piece_does_not_promote_again() {
    let from = Square::new(4, 7).unwrap();
    let to = Square::new(4, 8).unwrap();
    let mut builder = PositionBuilder::new(Color::Black);
    builder
        .put(
            from,
            PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap(),
        )
        .unwrap();
    let position = builder.finish().unwrap();
    let generator = MoveGenerator::new(Rules::standard());
    let mut moves = Vec::new();
    generator.generate_moves(&position, &mut moves);

    assert!(moves.contains(&Move {
        from,
        mid: None,
        to,
        promote: false,
    }));
    assert!(!moves.contains(&Move {
        from,
        mid: None,
        to,
        promote: true,
    }));
}

#[test]
fn second_lion_stage_uses_updated_occupancy() {
    let mut builder = PositionBuilder::new(Color::Black);
    builder
        .put(sq(5, 5), PieceCode::new(Color::Black, PieceKind::Lion))
        .unwrap();
    builder
        .put(sq(5, 6), PieceCode::new(Color::White, PieceKind::Pawn))
        .unwrap();
    builder
        .put(sq(6, 6), PieceCode::new(Color::White, PieceKind::Pawn))
        .unwrap();
    let position = builder.finish().unwrap();
    let mut moves = Vec::new();
    generate_lion_double_and_jumps(
        attack_tables(),
        &position,
        Color::Black,
        sq(5, 5),
        &mut moves,
    );

    assert!(moves.contains(&Move {
        from: sq(5, 5),
        mid: Some(sq(5, 6)),
        to: sq(6, 6),
        promote: false,
    }));
    assert!(moves.contains(&Move {
        from: sq(5, 5),
        mid: Some(sq(5, 6)),
        to: sq(5, 5),
        promote: false,
    }));
}
