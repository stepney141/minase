use core::fmt;

use crate::mv::Move;
use crate::position::Position;

use super::MoveGenerator;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IllegalMove(pub Move);

impl fmt::Display for IllegalMove {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "the move is not legal: {:?}", self.0)
    }
}

impl std::error::Error for IllegalMove {}

impl Position {
    pub fn try_make_move(
        &mut self,
        mv: Move,
        generator: &MoveGenerator,
    ) -> Result<crate::Undo, IllegalMove> {
        let mut moves = Vec::new();
        generator.generate_moves(self, &mut moves);
        if moves.contains(&mv) {
            Ok(self.make_move_unchecked(mv))
        } else {
            Err(IllegalMove(mv))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::MoveGenerator;
    use crate::piece::{Color, PieceCode, PieceKind};
    use crate::position::PositionBuilder;
    use crate::test_util::sq;

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
            let undo = position.make_move_unchecked(mv);
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
            let undo = position.make_move_unchecked(mv);
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
    fn articles_11_1_d_and_11_2_lion_like_direct_jumps_ignore_intermediate_occupancy() {
        let generator = MoveGenerator::standard();
        for (kind, from, middle, target) in [
            (PieceKind::HornedFalcon, sq(5, 5), sq(5, 6), sq(5, 7)),
            (PieceKind::SoaringEagle, sq(5, 5), sq(6, 6), sq(7, 7)),
        ] {
            let jump = Move {
                from,
                mid: None,
                to: target,
                promote: false,
            };
            for middle_color in [None, Some(Color::White), Some(Color::Black)] {
                let mut builder = PositionBuilder::new(Color::Black);
                builder
                    .put(from, PieceCode::new_promoted(Color::Black, kind).unwrap())
                    .unwrap();
                if let Some(color) = middle_color {
                    builder
                        .put(middle, PieceCode::new(color, PieceKind::Pawn))
                        .unwrap();
                }
                let position = builder.finish().unwrap();
                let mut moves = Vec::new();
                generator.generate_moves(&position, &mut moves);

                assert!(
                    moves.contains(&jump),
                    "kind={kind:?}, middle_color={middle_color:?}",
                );
                assert_eq!(position.captured_squares(jump), [None, None]);
            }

            let mut builder = PositionBuilder::new(Color::Black);
            builder
                .put(from, PieceCode::new_promoted(Color::Black, kind).unwrap())
                .unwrap();
            builder
                .put(target, PieceCode::new(Color::Black, PieceKind::Pawn))
                .unwrap();
            let position = builder.finish().unwrap();
            let mut moves = Vec::new();
            generator.generate_moves(&position, &mut moves);

            assert!(!moves.contains(&jump), "kind={kind:?}");
        }
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
            let undo = position.make_move_unchecked(mv);
            history.push(undo);
        }

        while let Some(undo) = history.pop() {
            position.unmake_move(undo);
        }
        assert_eq!(position, before);
    }
}
