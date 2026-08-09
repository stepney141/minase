use core::fmt;

use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::{Position, PositionBuildError, PositionBuilder};
use crate::core::square::{BOARD_FILES, BOARD_RANKS, Square};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SfenError {
    MissingBoard,
    MissingSideToMove,
    InvalidSideToMove,
    UnexpectedFields,
    WrongRowCount {
        found: usize,
    },
    InvalidEmptyCount {
        row: usize,
    },
    WrongRowWidth {
        row: usize,
        found: usize,
    },
    MissingPromotedPiece {
        row: usize,
        column: usize,
    },
    UnsupportedPiece {
        row: usize,
        column: usize,
        letter: char,
    },
    UnpromotablePiece {
        row: usize,
        column: usize,
        letter: char,
    },
    PositionBuild(PositionBuildError),
}

impl fmt::Display for SfenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBoard => formatter.write_str("SFEN is missing the board field"),
            Self::MissingSideToMove => {
                formatter.write_str("SFEN is missing the side-to-move field")
            }
            Self::InvalidSideToMove => {
                formatter.write_str("invalid SFEN side to move; expected \"b\" or \"w\"")
            }
            Self::UnexpectedFields => {
                formatter.write_str("unexpected SFEN field after the side to move")
            }
            Self::WrongRowCount { found } => write!(
                formatter,
                "SFEN board has {found} rows; expected {BOARD_RANKS}"
            ),
            Self::InvalidEmptyCount { row } => {
                write!(
                    formatter,
                    "SFEN row {row} has an invalid empty-square count"
                )
            }
            Self::WrongRowWidth { row, found } => write!(
                formatter,
                "SFEN row {row} has width {found}; expected {BOARD_FILES}"
            ),
            Self::MissingPromotedPiece { row, column } => write!(
                formatter,
                "SFEN promotion marker at row {row}, column {column} has no piece"
            ),
            Self::UnsupportedPiece {
                row,
                column,
                letter,
            } => write!(
                formatter,
                "unsupported SFEN piece letter {letter:?} at row {row}, column {column}"
            ),
            Self::UnpromotablePiece {
                row,
                column,
                letter,
            } => write!(
                formatter,
                "SFEN piece {letter:?} at row {row}, column {column} has no promoted form"
            ),
            Self::PositionBuild(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SfenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PositionBuild(error) => Some(error),
            Self::MissingBoard
            | Self::MissingSideToMove
            | Self::InvalidSideToMove
            | Self::UnexpectedFields
            | Self::WrongRowCount { .. }
            | Self::InvalidEmptyCount { .. }
            | Self::WrongRowWidth { .. }
            | Self::MissingPromotedPiece { .. }
            | Self::UnsupportedPiece { .. }
            | Self::UnpromotablePiece { .. } => None,
        }
    }
}

impl From<PositionBuildError> for SfenError {
    fn from(error: PositionBuildError) -> Self {
        Self::PositionBuild(error)
    }
}

/// 局面の盤面と手番を2欄基本形のSFENへ書き出す。
///
/// 獅子捕獲升と手数を含む拡張形式はプロトコル層で扱う。
pub fn to_sfen(position: &Position) -> String {
    let mut sfen = String::new();

    for rank in (0..BOARD_RANKS).rev() {
        if rank != BOARD_RANKS - 1 {
            sfen.push('/');
        }

        let mut empty_count = 0;
        for file in 0..BOARD_FILES {
            let square = Square::new(file, rank).expect("board coordinates must be valid");
            let Some(piece) = position.piece_at(square) else {
                empty_count += 1;
                continue;
            };

            if empty_count != 0 {
                sfen.push_str(&empty_count.to_string());
                empty_count = 0;
            }
            if piece.is_promoted() {
                sfen.push('+');
            }

            let kind = piece.kind().expect("a board piece must have a kind");
            let base_kind = if piece.is_promoted() {
                kind.unpromoted()
                    .expect("a promoted piece must have an unpromoted kind")
            } else {
                kind
            };
            let mut letter = match base_kind {
                PieceKind::King => 'K',
                PieceKind::Pawn => 'P',
                PieceKind::Lance => 'L',
                PieceKind::SilverGeneral => 'S',
                PieceKind::GoldGeneral => 'G',
                PieceKind::Bishop => 'B',
                PieceKind::Rook => 'R',
                PieceKind::FerociousLeopard => 'F',
                PieceKind::CopperGeneral => 'C',
                PieceKind::DrunkElephant => 'E',
                PieceKind::ReverseChariot => 'A',
                PieceKind::BlindTiger => 'T',
                PieceKind::Kirin => 'O',
                PieceKind::Phoenix => 'X',
                PieceKind::SideMover => 'M',
                PieceKind::VerticalMover => 'V',
                PieceKind::DragonHorse => 'H',
                PieceKind::DragonKing => 'D',
                PieceKind::Lion => 'N',
                PieceKind::FreeKing => 'Q',
                PieceKind::GoBetween => 'I',
                PieceKind::CrownPrince
                | PieceKind::WhiteHorse
                | PieceKind::Whale
                | PieceKind::FlyingOx
                | PieceKind::FreeBoar
                | PieceKind::FlyingStag
                | PieceKind::HornedFalcon
                | PieceKind::SoaringEagle => {
                    unreachable!("an unpromoted piece must have an SFEN letter")
                }
            };
            if piece.color() == Some(Color::White) {
                letter = letter.to_ascii_lowercase();
            }
            sfen.push(letter);
        }

        if empty_count != 0 {
            sfen.push_str(&empty_count.to_string());
        }
    }

    sfen.push(' ');
    sfen.push(match position.side_to_move() {
        Color::Black => 'b',
        Color::White => 'w',
    });
    sfen
}

/// Parses the supported shogiops-compatible SFEN board and side-to-move fields.
///
/// Rows run from internal rank 11 to rank 0, and columns run from internal file
/// 0 to file 11. The supported piece letters are `N`, `O`, `H`, `D`, `B`, `S`,
/// `G`, and `L`, with lowercase letters for White and `+` for promotion. Hands,
/// move counters, and lion-capture state are unsupported.
///
/// This function validates syntax only. It does not check rule-level position
/// validity, such as the existence of king pieces, per-piece-type count upper
/// bounds, or mutually inconsistent placements. The caller is responsible for
/// those checks, which are intended to occur at the protocol layer.
pub fn parse_sfen(sfen: &str) -> Result<Position, SfenError> {
    let mut fields = sfen.split_whitespace();
    let board = fields.next().ok_or(SfenError::MissingBoard)?;
    let side_to_move = match fields.next().ok_or(SfenError::MissingSideToMove)? {
        "b" => Color::Black,
        "w" => Color::White,
        _ => return Err(SfenError::InvalidSideToMove),
    };
    if fields.next().is_some() {
        return Err(SfenError::UnexpectedFields);
    }

    let rows: Vec<_> = board.split('/').collect();
    if rows.len() != BOARD_RANKS as usize {
        return Err(SfenError::WrongRowCount { found: rows.len() });
    }

    let mut builder = PositionBuilder::new(side_to_move);
    for (row_index, row) in rows.into_iter().enumerate() {
        let display_row = row_index + 1;
        let rank = BOARD_RANKS as usize - 1 - row_index;
        let mut file = 0_usize;
        let mut characters = row.chars().peekable();

        while let Some(character) = characters.next() {
            if character.is_ascii_digit() {
                let mut empty = (character as u8 - b'0') as usize;
                while let Some(next) = characters.peek().copied() {
                    if !next.is_ascii_digit() {
                        break;
                    }
                    characters.next();
                    let digit = (next as u8 - b'0') as usize;
                    empty = empty
                        .checked_mul(10)
                        .and_then(|count| count.checked_add(digit))
                        .ok_or(SfenError::InvalidEmptyCount { row: display_row })?;
                }
                if empty == 0 {
                    return Err(SfenError::InvalidEmptyCount { row: display_row });
                }
                file = file
                    .checked_add(empty)
                    .ok_or(SfenError::InvalidEmptyCount { row: display_row })?;
                if file > BOARD_FILES as usize {
                    return Err(SfenError::WrongRowWidth {
                        row: display_row,
                        found: file,
                    });
                }
                continue;
            }

            let column = file + 1;
            let (promote, letter) = if character == '+' {
                (
                    true,
                    characters.next().ok_or(SfenError::MissingPromotedPiece {
                        row: display_row,
                        column,
                    })?,
                )
            } else {
                (false, character)
            };
            let color = if letter.is_ascii_uppercase() {
                Color::Black
            } else {
                Color::White
            };
            let kind = match letter.to_ascii_uppercase() {
                'K' => PieceKind::King,
                'P' => PieceKind::Pawn,
                'L' => PieceKind::Lance,
                'S' => PieceKind::SilverGeneral,
                'G' => PieceKind::GoldGeneral,
                'B' => PieceKind::Bishop,
                'R' => PieceKind::Rook,
                'F' => PieceKind::FerociousLeopard,
                'C' => PieceKind::CopperGeneral,
                'E' => PieceKind::DrunkElephant,
                'A' => PieceKind::ReverseChariot,
                'T' => PieceKind::BlindTiger,
                'O' => PieceKind::Kirin,
                'X' => PieceKind::Phoenix,
                'M' => PieceKind::SideMover,
                'V' => PieceKind::VerticalMover,
                'H' => PieceKind::DragonHorse,
                'D' => PieceKind::DragonKing,
                'N' => PieceKind::Lion,
                'Q' => PieceKind::FreeKing,
                'I' => PieceKind::GoBetween,
                _ => {
                    return Err(SfenError::UnsupportedPiece {
                        row: display_row,
                        column,
                        letter,
                    });
                }
            };
            let unpromoted = PieceCode::new(color, kind);
            let piece = if promote {
                unpromoted.promote().ok_or(SfenError::UnpromotablePiece {
                    row: display_row,
                    column,
                    letter,
                })?
            } else {
                unpromoted
            };
            let square = Square::new(file as u8, rank as u8).ok_or(SfenError::WrongRowWidth {
                row: display_row,
                found: file + 1,
            })?;
            builder.put(square, piece)?;
            file += 1;
        }

        if file != BOARD_FILES as usize {
            return Err(SfenError::WrongRowWidth {
                row: display_row,
                found: file,
            });
        }
    }

    builder.finish().map_err(SfenError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::sq;

    const EMPTY_BOARD: &str = "12/12/12/12/12/12/12/12/12/12/12/12";

    #[test]
    fn initial_position_round_trips_and_matches_lishogi_sfen() {
        let position = Position::initial();
        let sfen = to_sfen(&position);

        assert_eq!(
            sfen,
            "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b"
        );
        assert_eq!(parse_sfen(&sfen).unwrap(), position);
    }

    #[test]
    fn promoted_position_round_trips() {
        let position = parse_sfen("12/12/12/12/4+f2l4/4S7/5+O6/7n4/12/12/12/12 w").unwrap();
        let sfen = to_sfen(&position);

        assert_eq!(parse_sfen(&sfen).unwrap(), position);
    }

    #[test]
    fn promotion_deferred_position_preserves_board_and_side() {
        let deferred = sq(7, 9);
        let mut builder = PositionBuilder::new(Color::White);
        builder
            .put(
                deferred,
                PieceCode::new(Color::Black, PieceKind::SilverGeneral),
            )
            .unwrap();
        builder.mark_promotion_deferred(deferred).unwrap();
        let position = builder.finish().unwrap();
        let sfen = to_sfen(&position);
        let restored = parse_sfen(&sfen).unwrap();

        assert!(!position.promotion_deferred().is_empty());
        assert!(restored.promotion_deferred().is_empty());
        assert_eq!(to_sfen(&restored), sfen);
        assert_eq!(restored.side_to_move(), position.side_to_move());
        assert!(Square::all().all(|square| restored.piece_at(square) == position.piece_at(square)));
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

    #[test]
    fn lishogi_initial_position_matches_position_initial() {
        let position = parse_sfen(
            "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b",
        )
        .unwrap();

        assert_eq!(position, Position::initial());
    }

    #[test]
    fn parses_every_promoted_piece_and_rejects_unpromotable_pieces() {
        let promoted_pieces = [
            ('p', PieceKind::GoldGeneral),
            ('l', PieceKind::WhiteHorse),
            ('s', PieceKind::VerticalMover),
            ('g', PieceKind::Rook),
            ('b', PieceKind::DragonHorse),
            ('r', PieceKind::DragonKing),
            ('f', PieceKind::Bishop),
            ('c', PieceKind::SideMover),
            ('e', PieceKind::CrownPrince),
            ('a', PieceKind::Whale),
            ('t', PieceKind::FlyingStag),
            ('o', PieceKind::Lion),
            ('x', PieceKind::FreeKing),
            ('m', PieceKind::FreeBoar),
            ('v', PieceKind::FlyingOx),
            ('h', PieceKind::HornedFalcon),
            ('d', PieceKind::SoaringEagle),
            ('i', PieceKind::DrunkElephant),
        ];
        for (letter, promoted_kind) in promoted_pieces {
            let sfen = format!("11+{letter}/12/12/12/12/12/12/12/12/12/12/12 b");
            let position = parse_sfen(&sfen).unwrap();

            assert_eq!(
                position.piece_at(Square::new(11, 11).unwrap()),
                PieceCode::new_promoted(Color::White, promoted_kind),
                "+{letter}"
            );
        }

        for letter in ['k', 'n', 'q'] {
            let sfen = format!("11+{letter}/12/12/12/12/12/12/12/12/12/12/12 b");

            assert_eq!(
                parse_sfen(&sfen),
                Err(SfenError::UnpromotablePiece {
                    row: 1,
                    column: 12,
                    letter,
                }),
                "+{letter}"
            );
        }
    }

    #[test]
    fn parses_supported_pieces_promotions_coordinates_and_side() {
        let position = parse_sfen("12/12/12/12/4B2l4/4S7/5+O6/7n4/12/12/12/12 w").unwrap();

        assert_eq!(position.side_to_move(), Color::White);
        assert_eq!(
            position.piece_at(Square::new(4, 7).unwrap()),
            Some(PieceCode::new(Color::Black, PieceKind::Bishop))
        );
        assert_eq!(
            position.piece_at(Square::new(7, 7).unwrap()),
            Some(PieceCode::new(Color::White, PieceKind::Lance))
        );
        assert_eq!(
            position.piece_at(Square::new(5, 5).unwrap()),
            PieceCode::new(Color::Black, PieceKind::Kirin).promote()
        );
        assert_eq!(
            position.piece_at(Square::new(7, 4).unwrap()),
            Some(PieceCode::new(Color::White, PieceKind::Lion))
        );
    }

    #[test]
    fn rejects_missing_invalid_and_extra_fields() {
        assert_eq!(parse_sfen(""), Err(SfenError::MissingBoard));
        assert_eq!(parse_sfen(EMPTY_BOARD), Err(SfenError::MissingSideToMove));
        assert_eq!(
            parse_sfen(&format!("{EMPTY_BOARD} x")),
            Err(SfenError::InvalidSideToMove)
        );
        assert_eq!(
            parse_sfen(&format!("{EMPTY_BOARD} b -")),
            Err(SfenError::UnexpectedFields)
        );
    }

    #[test]
    fn rejects_malformed_board_without_panicking() {
        assert_eq!(
            parse_sfen("12/12/12/12/12/12/12/12/12/12/12 b"),
            Err(SfenError::WrongRowCount { found: 11 })
        );
        assert_eq!(
            parse_sfen("13/12/12/12/12/12/12/12/12/12/12/12 b"),
            Err(SfenError::WrongRowWidth { row: 1, found: 13 })
        );
        assert_eq!(
            parse_sfen("11J/12/12/12/12/12/12/12/12/12/12/12 b"),
            Err(SfenError::UnsupportedPiece {
                row: 1,
                column: 12,
                letter: 'J',
            })
        );
        assert_eq!(
            parse_sfen("11+/12/12/12/12/12/12/12/12/12/12/12 b"),
            Err(SfenError::MissingPromotedPiece { row: 1, column: 12 })
        );
        assert_eq!(
            parse_sfen("11+N/12/12/12/12/12/12/12/12/12/12/12 b"),
            Err(SfenError::UnpromotablePiece {
                row: 1,
                column: 12,
                letter: 'N',
            })
        );
        assert_eq!(
            parse_sfen("999999999999999999999999999999/12/12/12/12/12/12/12/12/12/12/12 b"),
            Err(SfenError::InvalidEmptyCount { row: 1 })
        );
    }
}
