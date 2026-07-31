use crate::core::piece::PieceCode;
use crate::core::square::Square;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Move {
    pub from: Square,
    pub mid: Option<Square>,
    pub to: Square,
    pub promote: bool,
}

impl Move {
    #[inline]
    pub(crate) const fn origin(self) -> Square {
        self.from
    }

    #[inline]
    pub(crate) const fn destination(self) -> Square {
        self.to
    }

    #[inline]
    pub(crate) const fn is_promoting(self) -> bool {
        self.promote
    }

    pub(crate) const fn capture_candidates(self) -> [Option<Square>; 2] {
        [
            self.mid,
            if self.to.raw() == self.from.raw() {
                None
            } else {
                Some(self.to)
            },
        ]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CapturedPiece {
    pub square: Square,
    pub piece: PieceCode,
}

#[derive(PartialEq, Eq, Debug)]
pub struct Undo {
    pub(crate) mv: Move,
    pub(crate) moved_piece_before: PieceCode,
    pub(crate) captured: [Option<CapturedPiece>; 2],
    pub(crate) previous_lion_taken: Option<Square>,
    pub(crate) previous_zobrist: u64,
}
