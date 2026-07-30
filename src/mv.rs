use crate::piece::PieceCode;
use crate::square::Square;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Move {
    Step {
        from: Square,
        to: Square,
        promote: bool,
    },
    Double {
        from: Square,
        mid: Square,
        to: Square,
        promote: bool,
    },
    Jump {
        from: Square,
        to: Square,
        promote: bool,
    },
}

impl Move {
    #[inline]
    pub(crate) const fn origin(self) -> Square {
        match self {
            Self::Step { from, .. } | Self::Double { from, .. } | Self::Jump { from, .. } => from,
        }
    }

    #[inline]
    pub(crate) const fn destination(self) -> Square {
        match self {
            Self::Step { to, .. } | Self::Double { to, .. } | Self::Jump { to, .. } => to,
        }
    }

    #[inline]
    pub(crate) const fn is_promoting(self) -> bool {
        match self {
            Self::Step { promote, .. }
            | Self::Double { promote, .. }
            | Self::Jump { promote, .. } => promote,
        }
    }

    pub(crate) const fn capture_candidates(self) -> [Option<Square>; 2] {
        match self {
            Self::Step { to, .. } | Self::Jump { to, .. } => [Some(to), None],
            Self::Double { from, mid, to, .. } => [
                Some(mid),
                if to.raw() == from.raw() {
                    None
                } else {
                    Some(to)
                },
            ],
        }
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
