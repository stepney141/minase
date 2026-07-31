use crate::movegen::MoveGenerator;
use crate::mv::Move;
use crate::position::Position;

pub(crate) fn captures_last_royal(position: &Position, mv: Move) -> bool {
    let opponent = position.side_to_move().opposite();
    let opponent_royals = position.royal_pieces(opponent);
    let royal_count = opponent_royals.popcount();
    let captured_royal_count = mv
        .capture_candidates()
        .into_iter()
        .flatten()
        .filter(|&square| opponent_royals.contains(square))
        .count();

    royal_count > 0 && captured_royal_count == royal_count as usize
}

pub(crate) fn can_capture_last_royal(position: &Position, generator: &MoveGenerator) -> bool {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    moves
        .into_iter()
        .any(|mv| captures_last_royal(position, mv))
}

/// Returns whether Article 23 applies to the side to move.
///
/// Having no legal move is a loss distinct from mate; [`is_mate`] returns
/// `false` for the same position.
pub(crate) fn has_no_legal_move(position: &Position, generator: &MoveGenerator) -> bool {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    moves.is_empty()
}

pub(crate) fn is_mate(position: &mut Position, generator: &MoveGenerator) -> bool {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    if moves.is_empty() {
        return false;
    }

    for mv in moves {
        if captures_last_royal(position, mv) {
            return false;
        }

        let undo = position.make_move_unchecked(mv);
        let opponent_can_capture_last_royal = can_capture_last_royal(position, generator);
        position.unmake_move(undo);

        if !opponent_can_capture_last_royal {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::piece::{Color, PieceCode, PieceKind};
    use crate::position::PositionBuilder;
    use crate::square::Square;

    fn sq(file: u8, rank: u8) -> Square {
        Square::new(file, rank).unwrap()
    }

    fn piece(color: Color, kind: PieceKind) -> PieceCode {
        PieceCode::new(color, kind)
    }

    fn prince(color: Color) -> PieceCode {
        PieceCode::new_promoted(color, PieceKind::CrownPrince).unwrap()
    }

    fn position(side_to_move: Color, pieces: &[(Square, PieceCode)]) -> Position {
        let mut builder = PositionBuilder::new(side_to_move);
        for &(square, piece) in pieces {
            builder.put(square, piece).unwrap();
        }
        builder.finish().unwrap()
    }

    #[test]
    fn article_21_2_single_royal_with_no_escape_is_mate() {
        let mut position = position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(0, 11), piece(Color::White, PieceKind::Rook)),
                (sq(11, 0), piece(Color::White, PieceKind::Rook)),
                (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
                (sq(10, 9), piece(Color::White, PieceKind::King)),
            ],
        );
        let original = position.clone();

        assert!(is_mate(&mut position, &MoveGenerator::standard()));
        assert_eq!(position, original);
    }

    #[test]
    fn article_21_3_a_escape_square_prevents_mate() {
        let mut position = position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(11, 0), piece(Color::White, PieceKind::Rook)),
                (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
                (sq(10, 9), piece(Color::White, PieceKind::King)),
            ],
        );

        assert!(!is_mate(&mut position, &MoveGenerator::standard()));
    }

    #[test]
    fn article_21_3_b_capturing_opponents_last_royal_prevents_mate() {
        let pieces = [
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(1, 0), piece(Color::White, PieceKind::King)),
        ];
        let threatened = position(Color::White, &pieces);
        let mut position = position(Color::Black, &pieces);

        let generator = MoveGenerator::standard();
        assert!(can_capture_last_royal(&threatened, &generator));
        assert!(can_capture_last_royal(&position, &generator));
        assert!(!is_mate(&mut position, &MoveGenerator::standard()));
    }

    #[test]
    fn articles_20_3_to_20_5_capturing_one_of_two_royals_does_not_mate() {
        let mut position = position(
            Color::Black,
            &[
                (sq(0, 11), piece(Color::Black, PieceKind::King)),
                (sq(2, 11), prince(Color::Black)),
                (sq(0, 10), piece(Color::Black, PieceKind::Pawn)),
                (sq(1, 10), piece(Color::Black, PieceKind::Pawn)),
                (sq(2, 10), piece(Color::Black, PieceKind::Pawn)),
                (sq(3, 10), piece(Color::Black, PieceKind::Pawn)),
                (sq(1, 11), piece(Color::Black, PieceKind::Pawn)),
                (sq(3, 11), piece(Color::Black, PieceKind::Pawn)),
                (sq(5, 10), piece(Color::Black, PieceKind::Pawn)),
                (sq(0, 9), piece(Color::White, PieceKind::Kirin)),
                (sq(11, 0), piece(Color::White, PieceKind::King)),
            ],
        );
        let generator = MoveGenerator::standard();
        let mut moves = Vec::new();
        generator.generate_moves(&position, &mut moves);
        assert_eq!(moves.len(), 2);

        let capture_one_royal = Move {
            from: sq(0, 9),
            mid: None,
            to: sq(0, 11),
            promote: false,
        };
        for mv in moves {
            let undo = position.make_move_unchecked(mv);
            let mut replies = Vec::new();
            generator.generate_moves(&position, &mut replies);
            assert!(replies.contains(&capture_one_royal));
            assert!(!captures_last_royal(&position, capture_one_royal));

            let reply_undo = position.make_move_unchecked(capture_one_royal);
            assert_eq!(position.royal_pieces(Color::Black).popcount(), 1);
            position.unmake_move(reply_undo);
            position.unmake_move(undo);
        }

        assert!(!is_mate(&mut position, &generator));
    }

    #[test]
    fn article_21_5_lion_double_capture_takes_both_royals() {
        let position = position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(5, 5), piece(Color::Black, PieceKind::Lion)),
                (sq(5, 6), piece(Color::White, PieceKind::King)),
                (sq(6, 6), prince(Color::White)),
            ],
        );
        let double_capture = Move {
            from: sq(5, 5),
            mid: Some(sq(5, 6)),
            to: sq(6, 6),
            promote: false,
        };
        let mut moves = Vec::new();
        MoveGenerator::standard().generate_moves(&position, &mut moves);

        assert!(moves.contains(&double_capture));
        assert!(captures_last_royal(&position, double_capture));
        assert!(can_capture_last_royal(
            &position,
            &MoveGenerator::standard()
        ));
    }

    #[test]
    fn article_23_no_legal_move_is_not_mate() {
        let mut position = position(
            Color::Black,
            &[
                (sq(4, 11), piece(Color::Black, PieceKind::Pawn)),
                (sq(0, 11), piece(Color::Black, PieceKind::Lance)),
                (sq(11, 0), piece(Color::White, PieceKind::King)),
            ],
        );
        let generator = MoveGenerator::standard();

        assert!(has_no_legal_move(&position, &generator));
        assert!(!is_mate(&mut position, &generator));
    }
}
