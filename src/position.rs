use core::fmt;

use crate::bitboard::Bitboard;
use crate::mv::{CapturedPiece, Move, Undo};
use crate::piece::{COLOR_COUNT, Color, PIECE_KIND_COUNT, PieceCode, PieceKind};
use crate::square::{BOARD_FILES, BOARD_RANKS, RAW_SQUARE_COUNT, Square};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Position {
    board: [PieceCode; RAW_SQUARE_COUNT],
    occupied: Bitboard,
    by_color: [Bitboard; COLOR_COUNT],
    by_kind: [[Bitboard; PIECE_KIND_COUNT]; COLOR_COUNT],
    side_to_move: Color,
    lion_taken_by_non_lion: Option<Square>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PositionError {
    PaddingIsNotWall {
        raw: u8,
    },
    ValidSquareIsWall {
        square: Square,
    },
    InvalidPieceCode {
        square: Square,
    },
    OccupancyMismatch {
        square: Square,
    },
    ColorOverlap,
    ColorAggregateMismatch {
        color: Color,
    },
    KindMismatch {
        square: Square,
        color: Color,
        kind: PieceKind,
    },
    PaddingBitSet,
}

impl fmt::Display for PositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "position invariant failed: {self:?}")
    }
}

impl std::error::Error for PositionError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PositionBuildError {
    SquareOccupied { square: Square },
    EmptyOrWallPiece,
    InvalidPosition(PositionError),
}

impl fmt::Display for PositionBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "position construction failed: {self:?}")
    }
}

impl std::error::Error for PositionBuildError {}

impl Position {
    pub fn empty(side_to_move: Color) -> Self {
        let mut board = [PieceCode::WALL; RAW_SQUARE_COUNT];
        for square in Square::all() {
            board[square.raw_index()] = PieceCode::EMPTY;
        }

        Self {
            board,
            occupied: Bitboard::EMPTY,
            by_color: [Bitboard::EMPTY; COLOR_COUNT],
            by_kind: [[Bitboard::EMPTY; PIECE_KIND_COUNT]; COLOR_COUNT],
            side_to_move,
            lion_taken_by_non_lion: None,
        }
    }

    pub fn initial() -> Self {
        let mut builder = PositionBuilder::new(Color::Black);
        let back_rank = [
            PieceKind::Lance,
            PieceKind::FerociousLeopard,
            PieceKind::CopperGeneral,
            PieceKind::SilverGeneral,
            PieceKind::GoldGeneral,
            PieceKind::King,
            PieceKind::DrunkElephant,
            PieceKind::GoldGeneral,
            PieceKind::SilverGeneral,
            PieceKind::CopperGeneral,
            PieceKind::FerociousLeopard,
            PieceKind::Lance,
        ];
        let second_rank = [
            (0, PieceKind::ReverseChariot),
            (2, PieceKind::Bishop),
            (4, PieceKind::BlindTiger),
            (5, PieceKind::Kirin),
            (6, PieceKind::Phoenix),
            (7, PieceKind::BlindTiger),
            (9, PieceKind::Bishop),
            (11, PieceKind::ReverseChariot),
        ];
        let third_rank = [
            PieceKind::SideMover,
            PieceKind::VerticalMover,
            PieceKind::Rook,
            PieceKind::DragonHorse,
            PieceKind::DragonKing,
            PieceKind::Lion,
            PieceKind::FreeKing,
            PieceKind::DragonKing,
            PieceKind::DragonHorse,
            PieceKind::Rook,
            PieceKind::VerticalMover,
            PieceKind::SideMover,
        ];

        for color in Color::ALL {
            let rotate = |file: u8, rank: u8| match color {
                Color::Black => (file, rank),
                Color::White => (BOARD_FILES - 1 - file, BOARD_RANKS - 1 - rank),
            };
            let mut put = |file, rank, kind| {
                let (file, rank) = rotate(file, rank);
                builder
                    .put(
                        Square::new(file, rank).unwrap(),
                        PieceCode::new(color, kind),
                    )
                    .unwrap();
            };

            for (file, kind) in back_rank.into_iter().enumerate() {
                put(file as u8, 0, kind);
            }
            for (file, kind) in second_rank {
                put(file, 1, kind);
            }
            for (file, kind) in third_rank.into_iter().enumerate() {
                put(file as u8, 2, kind);
            }
            for file in 0..BOARD_FILES {
                put(file, 3, PieceKind::Pawn);
            }
            for file in [3, 8] {
                put(file, 4, PieceKind::GoBetween);
            }
        }

        builder.finish().unwrap()
    }

    #[inline]
    pub fn piece_at(&self, square: Square) -> Option<PieceCode> {
        let piece = self.board[square.raw_index()];
        (!piece.is_empty()).then_some(piece)
    }

    #[inline]
    pub const fn occupied(&self) -> Bitboard {
        self.occupied
    }

    #[inline]
    pub const fn pieces_of(&self, color: Color) -> Bitboard {
        self.by_color[color.index()]
    }

    #[inline]
    pub const fn pieces_of_kind(&self, color: Color, kind: PieceKind) -> Bitboard {
        self.by_kind[color.index()][kind.index()]
    }

    #[inline]
    pub const fn side_to_move(&self) -> Color {
        self.side_to_move
    }

    pub fn royal_pieces(&self, color: Color) -> Bitboard {
        self.pieces_of_kind(color, PieceKind::King)
            | self.pieces_of_kind(color, PieceKind::CrownPrince)
    }

    pub(crate) const fn lion_taken_by_non_lion(&self) -> Option<Square> {
        self.lion_taken_by_non_lion
    }

    pub(crate) fn captured_squares(&self, mv: Move) -> [Option<Square>; 2] {
        let moving_color = self
            .piece_at(mv.origin())
            .and_then(PieceCode::color)
            .expect("move origin must contain a piece");
        mv.capture_candidates().map(|candidate| {
            candidate.filter(|&square| {
                self.piece_at(square)
                    .is_some_and(|piece| piece.color() == Some(moving_color.opposite()))
            })
        })
    }

    fn put_piece(&mut self, square: Square, piece: PieceCode) -> Result<(), PositionBuildError> {
        if piece.is_empty() || piece.is_wall() {
            return Err(PositionBuildError::EmptyOrWallPiece);
        }
        if self.piece_at(square).is_some() {
            return Err(PositionBuildError::SquareOccupied { square });
        }

        let color = piece.color().expect("validated piece must have a color");
        let kind = piece.kind().expect("validated piece must have a kind");
        self.board[square.raw_index()] = piece;
        self.occupied.set(square);
        self.by_color[color.index()].set(square);
        self.by_kind[color.index()][kind.index()].set(square);
        Ok(())
    }

    fn remove_piece(&mut self, square: Square) -> PieceCode {
        let piece = self.board[square.raw_index()];
        debug_assert!(!piece.is_empty() && !piece.is_wall());
        let color = piece.color().expect("occupied square must have a color");
        let kind = piece.kind().expect("occupied square must have a kind");

        self.board[square.raw_index()] = PieceCode::EMPTY;
        self.occupied.clear(square);
        self.by_color[color.index()].clear(square);
        self.by_kind[color.index()][kind.index()].clear(square);
        piece
    }

    pub fn validate(&self) -> Result<(), PositionError> {
        let union = self.by_color[0] | self.by_color[1];
        if union != self.occupied {
            return Err(PositionError::ColorAggregateMismatch {
                color: Color::Black,
            });
        }
        if self.by_color[0].intersects(self.by_color[1]) {
            return Err(PositionError::ColorOverlap);
        }

        for color in Color::ALL {
            let mut kinds = Bitboard::EMPTY;
            for kind in PieceKind::ALL {
                kinds |= self.by_kind[color.index()][kind.index()];
            }
            if kinds != self.by_color[color.index()] {
                return Err(PositionError::ColorAggregateMismatch { color });
            }
        }

        for raw in 0..RAW_SQUARE_COUNT {
            match Square::from_raw(raw as u8) {
                None => {
                    if !self.board[raw].is_wall() {
                        return Err(PositionError::PaddingIsNotWall { raw: raw as u8 });
                    }
                }
                Some(square) => {
                    let piece = self.board[raw];
                    if piece.is_wall() {
                        return Err(PositionError::ValidSquareIsWall { square });
                    }
                    if piece.is_empty() {
                        if self.occupied.contains(square) {
                            return Err(PositionError::OccupancyMismatch { square });
                        }
                    } else {
                        let color = piece
                            .color()
                            .ok_or(PositionError::InvalidPieceCode { square })?;
                        let kind = piece
                            .kind()
                            .ok_or(PositionError::InvalidPieceCode { square })?;
                        if !self.occupied.contains(square) {
                            return Err(PositionError::OccupancyMismatch { square });
                        }
                        if !self.by_kind[color.index()][kind.index()].contains(square) {
                            return Err(PositionError::KindMismatch {
                                square,
                                color,
                                kind,
                            });
                        }
                    }
                }
            }
        }

        let padding_mask = [!Bitboard::VALID_WORD; 3];
        let has_padding = |board: Bitboard| {
            board
                .words()
                .iter()
                .zip(padding_mask)
                .any(|(word, mask)| word & mask != 0)
        };
        if has_padding(self.occupied)
            || self.by_color.into_iter().any(has_padding)
            || self.by_kind.into_iter().flatten().any(has_padding)
        {
            return Err(PositionError::PaddingBitSet);
        }
        Ok(())
    }

    pub(crate) fn make_move_unchecked(&mut self, mv: Move) -> Undo {
        let previous_lion_taken = self.lion_taken_by_non_lion;
        let capture_squares = self.captured_squares(mv);
        let moved_piece_before = self.remove_piece(mv.origin());
        debug_assert_eq!(moved_piece_before.color(), Some(self.side_to_move));

        let mut captured = [None; 2];
        for (index, square) in capture_squares.into_iter().enumerate() {
            if let Some(square) = square {
                let piece = self.remove_piece(square);
                debug_assert_eq!(piece.color(), Some(self.side_to_move.opposite()));
                captured[index] = Some(CapturedPiece { square, piece });
            }
        }

        let moved_piece_after = if mv.is_promoting() {
            moved_piece_before
                .promote()
                .expect("promoting move must have a promotable piece")
        } else {
            moved_piece_before
        };
        self.put_piece(mv.destination(), moved_piece_after)
            .expect("generated move must end on an empty square");
        self.side_to_move = self.side_to_move.opposite();
        self.lion_taken_by_non_lion = (moved_piece_before.kind() != Some(PieceKind::Lion))
            .then(|| {
                captured
                    .into_iter()
                    .flatten()
                    .filter(|captured| captured.piece.kind() == Some(PieceKind::Lion))
                    .map(|captured| captured.square)
                    .next_back()
            })
            .flatten();

        Undo {
            mv,
            moved_piece_before,
            captured,
            previous_lion_taken,
        }
    }

    pub(crate) fn unmake_move(&mut self, undo: Undo) {
        self.side_to_move = self.side_to_move.opposite();
        self.remove_piece(undo.mv.destination());
        self.put_piece(undo.mv.origin(), undo.moved_piece_before)
            .expect("move origin must be empty while unmaking");
        for captured in undo.captured.into_iter().flatten() {
            self.put_piece(captured.square, captured.piece)
                .expect("capture square must be empty while unmaking");
        }
        self.lion_taken_by_non_lion = undo.previous_lion_taken;
    }
}

pub struct PositionBuilder {
    position: Position,
}

impl PositionBuilder {
    pub fn new(side_to_move: Color) -> Self {
        Self {
            position: Position::empty(side_to_move),
        }
    }

    pub fn put(&mut self, square: Square, piece: PieceCode) -> Result<(), PositionBuildError> {
        self.position.put_piece(square, piece)
    }

    pub fn finish(self) -> Result<Position, PositionBuildError> {
        self.position
            .validate()
            .map_err(PositionBuildError::InvalidPosition)?;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(file: u8, rank: u8) -> Square {
        Square::new(file, rank).unwrap()
    }

    fn position_with_pieces(
        side_to_move: Color,
        pieces: &[(Square, Color, PieceKind)],
    ) -> Position {
        let mut builder = PositionBuilder::new(side_to_move);
        for &(square, color, kind) in pieces {
            builder.put(square, PieceCode::new(color, kind)).unwrap();
        }
        builder.finish().unwrap()
    }

    fn position_after_non_lion_captures_lion() -> (Position, Square) {
        let captured_lion = sq(1, 1);
        let mut position = position_with_pieces(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::Bishop),
                (captured_lion, Color::White, PieceKind::Lion),
                (sq(10, 10), Color::White, PieceKind::Pawn),
            ],
        );
        position.make_move_unchecked(Move::Step {
            from: sq(0, 0),
            to: captured_lion,
            promote: false,
        });
        (position, captured_lion)
    }

    #[test]
    fn empty_and_initial_positions_validate() {
        assert!(Position::empty(Color::Black).validate().is_ok());
        let initial = Position::initial();
        assert!(initial.validate().is_ok());
        assert_eq!(initial.pieces_of(Color::Black).popcount(), 46);
        assert_eq!(initial.pieces_of(Color::White).popcount(), 46);
        assert_eq!(initial.occupied().popcount(), 92);
    }

    #[test]
    fn initial_position_matches_rules_article_7_on_every_square() {
        use Color::{Black, White};
        use PieceKind::{
            Bishop, BlindTiger, CopperGeneral, DragonHorse, DragonKing, DrunkElephant,
            FerociousLeopard, FreeKing, GoBetween, GoldGeneral, King, Kirin, Lance, Lion, Pawn,
            Phoenix, ReverseChariot, Rook, SideMover, SilverGeneral, VerticalMover,
        };

        let black = |kind| Some(PieceCode::new(Black, kind));
        let white = |kind| Some(PieceCode::new(White, kind));
        let expected_from_white_side = [
            [
                white(Lance),
                white(FerociousLeopard),
                white(CopperGeneral),
                white(SilverGeneral),
                white(GoldGeneral),
                white(DrunkElephant),
                white(King),
                white(GoldGeneral),
                white(SilverGeneral),
                white(CopperGeneral),
                white(FerociousLeopard),
                white(Lance),
            ],
            [
                white(ReverseChariot),
                None,
                white(Bishop),
                None,
                white(BlindTiger),
                white(Phoenix),
                white(Kirin),
                white(BlindTiger),
                None,
                white(Bishop),
                None,
                white(ReverseChariot),
            ],
            [
                white(SideMover),
                white(VerticalMover),
                white(Rook),
                white(DragonHorse),
                white(DragonKing),
                white(FreeKing),
                white(Lion),
                white(DragonKing),
                white(DragonHorse),
                white(Rook),
                white(VerticalMover),
                white(SideMover),
            ],
            [white(Pawn); BOARD_FILES as usize],
            [
                None,
                None,
                None,
                white(GoBetween),
                None,
                None,
                None,
                None,
                white(GoBetween),
                None,
                None,
                None,
            ],
            [None; BOARD_FILES as usize],
            [None; BOARD_FILES as usize],
            [
                None,
                None,
                None,
                black(GoBetween),
                None,
                None,
                None,
                None,
                black(GoBetween),
                None,
                None,
                None,
            ],
            [black(Pawn); BOARD_FILES as usize],
            [
                black(SideMover),
                black(VerticalMover),
                black(Rook),
                black(DragonHorse),
                black(DragonKing),
                black(Lion),
                black(FreeKing),
                black(DragonKing),
                black(DragonHorse),
                black(Rook),
                black(VerticalMover),
                black(SideMover),
            ],
            [
                black(ReverseChariot),
                None,
                black(Bishop),
                None,
                black(BlindTiger),
                black(Kirin),
                black(Phoenix),
                black(BlindTiger),
                None,
                black(Bishop),
                None,
                black(ReverseChariot),
            ],
            [
                black(Lance),
                black(FerociousLeopard),
                black(CopperGeneral),
                black(SilverGeneral),
                black(GoldGeneral),
                black(King),
                black(DrunkElephant),
                black(GoldGeneral),
                black(SilverGeneral),
                black(CopperGeneral),
                black(FerociousLeopard),
                black(Lance),
            ],
        ];
        let initial = Position::initial();

        for (diagram_rank, expected_rank) in expected_from_white_side.into_iter().enumerate() {
            let internal_rank = BOARD_RANKS - 1 - diagram_rank as u8;
            for (file, expected_piece) in expected_rank.into_iter().enumerate() {
                assert_eq!(
                    initial.piece_at(sq(file as u8, internal_rank)),
                    expected_piece,
                    "RULES.md 第7条の{}段目{}筋",
                    diagram_rank + 1,
                    file + 1,
                );
            }
        }
    }

    #[test]
    fn non_lion_capture_of_lion_sets_capture_square() {
        let (position, captured_lion) = position_after_non_lion_captures_lion();

        assert_eq!(position.lion_taken_by_non_lion(), Some(captured_lion));
    }

    #[test]
    fn lion_capture_of_lion_does_not_set_capture_square() {
        let mut position = position_with_pieces(
            Color::Black,
            &[
                (sq(4, 4), Color::Black, PieceKind::Lion),
                (sq(5, 4), Color::White, PieceKind::Lion),
            ],
        );
        position.make_move_unchecked(Move::Step {
            from: sq(4, 4),
            to: sq(5, 4),
            promote: false,
        });

        assert_eq!(position.lion_taken_by_non_lion(), None);
    }

    #[test]
    fn kirin_promoting_to_lion_still_sets_capture_square() {
        let captured_lion = sq(5, 8);
        let mut position = position_with_pieces(
            Color::Black,
            &[
                (sq(4, 7), Color::Black, PieceKind::Kirin),
                (captured_lion, Color::White, PieceKind::Lion),
            ],
        );
        position.make_move_unchecked(Move::Step {
            from: sq(4, 7),
            to: captured_lion,
            promote: true,
        });

        assert_eq!(position.lion_taken_by_non_lion(), Some(captured_lion));
        assert_eq!(
            position.piece_at(captured_lion),
            PieceCode::new_promoted(Color::Black, PieceKind::Lion),
        );
    }

    #[test]
    fn unrelated_next_move_clears_capture_square() {
        let (mut position, captured_lion) = position_after_non_lion_captures_lion();
        assert_eq!(position.lion_taken_by_non_lion(), Some(captured_lion));

        position.make_move_unchecked(Move::Step {
            from: sq(10, 10),
            to: sq(10, 9),
            promote: false,
        });

        assert_eq!(position.lion_taken_by_non_lion(), None);
    }

    #[test]
    fn unmake_move_restores_previous_capture_square() {
        let (mut position, captured_lion) = position_after_non_lion_captures_lion();
        let before_unrelated_move = position.clone();
        let undo = position.make_move_unchecked(Move::Step {
            from: sq(10, 10),
            to: sq(10, 9),
            promote: false,
        });
        assert_eq!(position.lion_taken_by_non_lion(), None);

        position.unmake_move(undo);

        assert_eq!(position.lion_taken_by_non_lion(), Some(captured_lion));
        assert_eq!(position, before_unrelated_move);
    }

    #[test]
    fn builder_rejects_double_placement() {
        let square = Square::new(4, 4).unwrap();
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(square, PieceCode::new(Color::Black, PieceKind::King))
            .unwrap();
        assert_eq!(
            builder.put(square, PieceCode::new(Color::White, PieceKind::King),),
            Err(PositionBuildError::SquareOccupied { square })
        );
    }

    #[test]
    fn exhaustive_placement_and_removal_returns_to_empty_position() {
        let mut position = Position::empty(Color::White);
        let empty = position.clone();

        for square in Square::all() {
            let color = if square.dense_index() % 2 == 0 {
                Color::Black
            } else {
                Color::White
            };
            let kind = PieceKind::ALL[square.dense_index() % PIECE_KIND_COUNT];
            position
                .put_piece(square, PieceCode::new(color, kind))
                .unwrap();
            assert!(position.validate().is_ok());
        }

        for square in Square::all().collect::<Vec<_>>().into_iter().rev() {
            position.remove_piece(square);
            assert!(position.validate().is_ok());
        }
        assert_eq!(position, empty);
    }

    #[test]
    fn mixed_placement_and_removal_sequence_preserves_invariants() {
        let mut position = Position::empty(Color::Black);
        let mut present = [false; crate::BOARD_SQUARE_COUNT];
        let mut state = 0xbb67_ae85_84ca_a73b_u64;

        for _ in 0..2_000 {
            state = state
                .wrapping_mul(2_862_933_555_777_941_757)
                .wrapping_add(3_037_000_493);
            let dense = state as usize % crate::BOARD_SQUARE_COUNT;
            let square = Square::from_dense(dense).unwrap();
            if present[dense] {
                position.remove_piece(square);
            } else {
                let color = if state & 1 == 0 {
                    Color::Black
                } else {
                    Color::White
                };
                let kind = PieceKind::ALL[(state as usize / 2) % PIECE_KIND_COUNT];
                position
                    .put_piece(square, PieceCode::new(color, kind))
                    .unwrap();
            }
            present[dense] = !present[dense];
            assert!(position.validate().is_ok());
        }
    }
}
