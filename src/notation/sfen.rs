use core::fmt;

use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::{Position, PositionBuildError, PositionBuilder};
use crate::core::rules::{RuleCode, Rules};
use crate::core::square::{BOARD_FILES, BOARD_RANKS, Square};

/// 拡張SFENが表す対局開始局面。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SetupPosition {
    /// 盤面、手番およびP1成り権保留状態。
    pub position: Position,
    /// 直前の非獅子による獅子捕獲升。先獅子(第15条)の復元に使う。
    pub lion_capture: Option<Square>,
    /// 次の着手の手数。表記だけに保持し、裁定には使わない。
    pub next_move_number: u32,
}

/// SFENの構文または局面構築のエラー。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SfenError {
    MissingBoard,
    MissingSideToMove,
    InvalidSideToMove,
    UnexpectedFields,
    /// 拡張SFENの欄数が4または5ではない。
    InvalidExtendedFieldCount {
        found: usize,
    },
    /// 獅子捕獲升の表記が盤上の升を表さない。
    InvalidLionCaptureSquare,
    /// 獅子捕獲升に手番側の駒があるため、直前着手後の状態として成立しない。
    LionCaptureOccupiedBySideToMove {
        square: Square,
    },
    /// 手数が1以上9999以下の整数ではない。
    InvalidMoveNumber,
    /// P1成り権保留升の表記が盤上の升を表さない。
    InvalidPromotionDeferredSquare,
    /// P1成り権保留升が内部密番号の狭義昇順ではない。
    PromotionDeferredNotStrictlyAscending {
        previous: Square,
        current: Square,
    },
    /// P1を採用していない規則で成り権保留升が指定された(第30条P1)。
    PromotionDeferredRequiresP1,
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
            Self::InvalidExtendedFieldCount { found } => write!(
                formatter,
                "extended SFEN has {found} fields; expected 4 or 5"
            ),
            Self::InvalidLionCaptureSquare => {
                formatter.write_str("invalid extended SFEN lion-capture square")
            }
            Self::LionCaptureOccupiedBySideToMove { square } => write!(
                formatter,
                "extended SFEN lion-capture square {square:?} is occupied by the side to move"
            ),
            Self::InvalidMoveNumber => formatter
                .write_str("invalid extended SFEN move number; expected an integer from 1 to 9999"),
            Self::InvalidPromotionDeferredSquare => {
                formatter.write_str("invalid extended SFEN promotion-deferred square")
            }
            Self::PromotionDeferredNotStrictlyAscending { previous, current } => write!(
                formatter,
                "extended SFEN promotion-deferred squares are not strictly ascending: {previous:?}, {current:?}"
            ),
            Self::PromotionDeferredRequiresP1 => {
                formatter.write_str("extended SFEN promotion-deferred squares require rule P1")
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
            | Self::InvalidExtendedFieldCount { .. }
            | Self::InvalidLionCaptureSquare
            | Self::LionCaptureOccupiedBySideToMove { .. }
            | Self::InvalidMoveNumber
            | Self::InvalidPromotionDeferredSquare
            | Self::PromotionDeferredNotStrictlyAscending { .. }
            | Self::PromotionDeferredRequiresP1
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

/// 対局開始局面を先獅子とP1成り権保留を含む5欄の拡張SFENへ書き出す。
pub fn to_extended_sfen(setup: &SetupPosition) -> String {
    let lion_capture = setup
        .lion_capture
        .map_or_else(|| "-".to_owned(), square_to_text);
    let promotion_deferred = Square::all()
        .filter(|&square| setup.position.promotion_deferred().contains(square))
        .map(square_to_text)
        .collect::<Vec<_>>();
    let promotion_deferred = if promotion_deferred.is_empty() {
        "-".to_owned()
    } else {
        promotion_deferred.join(",")
    };

    format!(
        "{} {lion_capture} {} {promotion_deferred}",
        to_sfen(&setup.position),
        setup.next_move_number
    )
}

/// 4欄または5欄の拡張SFENを解析する。
///
/// 4欄入力ではP1成り権保留欄を`-`とみなす。獅子捕獲升は検証して
/// [`SetupPosition::lion_capture`]へ保持し、先獅子状態(第15条)の
/// [`Position`]への注入は対局管理層へ委ねる。
pub fn parse_extended_sfen(sfen: &str, rules: Rules) -> Result<SetupPosition, SfenError> {
    let fields: Vec<_> = sfen.split_whitespace().collect();
    if !matches!(fields.len(), 4 | 5) {
        return Err(SfenError::InvalidExtendedFieldCount {
            found: fields.len(),
        });
    }

    let side_to_move = parse_side_to_move(fields[1])?;
    let lion_capture = if fields[2] == "-" {
        None
    } else {
        Some(parse_square(fields[2]).ok_or(SfenError::InvalidLionCaptureSquare)?)
    };
    let next_move_number = fields[3]
        .parse::<u32>()
        .ok()
        .filter(|number| (1..=9999).contains(number))
        .ok_or(SfenError::InvalidMoveNumber)?;
    let promotion_deferred =
        parse_promotion_deferred(fields.get(4).copied().unwrap_or("-"), rules)?;
    let position = parse_position(fields[0], side_to_move, &promotion_deferred)?;

    if let Some(square) = lion_capture
        && position
            .piece_at(square)
            .is_some_and(|piece| piece.color() == Some(side_to_move))
    {
        return Err(SfenError::LionCaptureOccupiedBySideToMove { square });
    }

    Ok(SetupPosition {
        position,
        lion_capture,
        next_move_number,
    })
}

fn square_to_text(square: Square) -> String {
    let file = BOARD_FILES - square.file();
    let rank = char::from(b'a' + (BOARD_RANKS - 1 - square.rank()));
    format!("{file}{rank}")
}

fn parse_square(text: &str) -> Option<Square> {
    if !text.is_ascii() || !(2..=3).contains(&text.len()) {
        return None;
    }
    let (file_text, rank_text) = text.split_at(text.len() - 1);
    if file_text.starts_with('0') || !file_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let file_name = file_text.parse::<u8>().ok()?;
    if !(1..=BOARD_FILES).contains(&file_name) {
        return None;
    }
    let rank_name = rank_text.as_bytes()[0];
    if !(b'a'..b'a' + BOARD_RANKS).contains(&rank_name) {
        return None;
    }

    Square::new(
        BOARD_FILES - file_name,
        BOARD_RANKS - 1 - (rank_name - b'a'),
    )
}

fn parse_side_to_move(field: &str) -> Result<Color, SfenError> {
    match field {
        "b" => Ok(Color::Black),
        "w" => Ok(Color::White),
        _ => Err(SfenError::InvalidSideToMove),
    }
}

fn parse_promotion_deferred(field: &str, rules: Rules) -> Result<Vec<Square>, SfenError> {
    if field == "-" {
        return Ok(Vec::new());
    }
    if !rules.contains(RuleCode::P1) {
        return Err(SfenError::PromotionDeferredRequiresP1);
    }

    let mut squares: Vec<Square> = Vec::new();
    for text in field.split(',') {
        let square = parse_square(text).ok_or(SfenError::InvalidPromotionDeferredSquare)?;
        if let Some(&previous) = squares.last()
            && previous.dense_index() >= square.dense_index()
        {
            return Err(SfenError::PromotionDeferredNotStrictlyAscending {
                previous,
                current: square,
            });
        }
        squares.push(square);
    }
    Ok(squares)
}

/// shogiops互換の盤面と手番を2欄基本形のSFENから解析する。
///
/// 段は内部rank 11から0へ、筋は内部file 0から11へ進む。全21種の
/// 駒文字を受理し、後手は小文字、成駒は接頭辞`+`で表す。獅子捕獲升、
/// 手数およびP1成り権保留は拡張SFENで扱う。
///
/// 構文と[`PositionBuilder`]の不変条件だけを検証する。王駒の存在や
/// 駒種ごとの枚数上限など、規則上の局面合法性は呼出し側が検証する。
pub fn parse_sfen(sfen: &str) -> Result<Position, SfenError> {
    let mut fields = sfen.split_whitespace();
    let board = fields.next().ok_or(SfenError::MissingBoard)?;
    let side_to_move = parse_side_to_move(fields.next().ok_or(SfenError::MissingSideToMove)?)?;
    if fields.next().is_some() {
        return Err(SfenError::UnexpectedFields);
    }

    parse_position(board, side_to_move, &[])
}

fn parse_position(
    board: &str,
    side_to_move: Color,
    promotion_deferred: &[Square],
) -> Result<Position, SfenError> {
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

    for &square in promotion_deferred {
        builder.mark_promotion_deferred(square)?;
    }
    builder.finish().map_err(SfenError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PositionError;
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
    fn extended_sfen_initial_position_round_trips_and_four_fields_default_to_no_deferred_rights() {
        let setup = SetupPosition {
            position: Position::initial(),
            lion_capture: None,
            next_move_number: 1,
        };
        let extended = to_extended_sfen(&setup);

        assert_eq!(extended, format!("{} - 1 -", to_sfen(&setup.position)));
        assert_eq!(
            parse_extended_sfen(&extended, Rules::engine_default()).unwrap(),
            setup
        );

        let four_fields = format!("{} - 1", to_sfen(&setup.position));
        let parsed = parse_extended_sfen(&four_fields, Rules::engine_default()).unwrap();
        assert_eq!(parsed, setup);
        assert_eq!(to_extended_sfen(&parsed), extended);
    }

    #[test]
    fn extended_sfen_round_trips_occupied_and_empty_lion_capture_squares() {
        let occupied = sq(4, 4);
        let mut builder = PositionBuilder::new(Color::White);
        builder
            .put(
                occupied,
                PieceCode::new_promoted(Color::Black, PieceKind::Lion).unwrap(),
            )
            .unwrap();
        let occupied_setup = SetupPosition {
            position: builder.finish().unwrap(),
            lion_capture: Some(occupied),
            next_move_number: 42,
        };
        let occupied_text = to_extended_sfen(&occupied_setup);
        let parsed_occupied = parse_extended_sfen(&occupied_text, Rules::engine_default()).unwrap();
        assert_eq!(parsed_occupied, occupied_setup);
        assert_eq!(
            parsed_occupied.position.lion_taken_by_non_lion(),
            None,
            "解析は獅子捕獲升をPositionへ注入しない"
        );

        let empty = sq(6, 6);
        let empty_setup = SetupPosition {
            position: Position::empty(Color::Black),
            lion_capture: Some(empty),
            next_move_number: 9999,
        };
        let empty_text = to_extended_sfen(&empty_setup);
        assert_eq!(
            parse_extended_sfen(&empty_text, Rules::engine_default()).unwrap(),
            empty_setup
        );
    }

    #[test]
    fn extended_sfen_round_trips_p1_deferred_squares_in_dense_order() {
        let first = sq(4, 2);
        let second = sq(7, 9);
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(
                first,
                PieceCode::new(Color::White, PieceKind::SilverGeneral),
            )
            .unwrap();
        builder
            .put(
                second,
                PieceCode::new(Color::Black, PieceKind::SilverGeneral),
            )
            .unwrap();
        builder.mark_promotion_deferred(second).unwrap();
        builder.mark_promotion_deferred(first).unwrap();
        let setup = SetupPosition {
            position: builder.finish().unwrap(),
            lion_capture: None,
            next_move_number: 17,
        };
        let rules = Rules::from_codes(&[RuleCode::P1, RuleCode::R1]).unwrap();
        let extended = to_extended_sfen(&setup);

        assert!(extended.ends_with(" - 17 8j,5c"));
        assert_eq!(parse_extended_sfen(&extended, rules).unwrap(), setup);
    }

    #[test]
    fn extended_sfen_rejects_invalid_lion_capture_fields_and_side_to_move_occupancy() {
        let own = sq(11, 11);
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(own, PieceCode::new(Color::Black, PieceKind::Pawn))
            .unwrap();
        let board = to_sfen(&builder.finish().unwrap());

        for invalid in ["0a", "13a", "1m", "01a", "a1"] {
            assert_eq!(
                parse_extended_sfen(
                    &format!("{EMPTY_BOARD} b {invalid} 1 -"),
                    Rules::engine_default()
                ),
                Err(SfenError::InvalidLionCaptureSquare),
                "{invalid}"
            );
        }
        assert_eq!(
            parse_extended_sfen(&format!("{board} 1a 1 -"), Rules::engine_default()),
            Err(SfenError::LionCaptureOccupiedBySideToMove { square: own })
        );
        assert_eq!(
            parse_extended_sfen(&format!("{EMPTY_BOARD} b 7f 1 -"), Rules::engine_default())
                .unwrap()
                .lion_capture,
            Some(sq(5, 6))
        );
    }

    #[test]
    fn extended_sfen_accepts_only_move_numbers_from_one_through_9999() {
        for valid in [1, 9999] {
            assert_eq!(
                parse_extended_sfen(
                    &format!("{EMPTY_BOARD} b - {valid} -"),
                    Rules::engine_default()
                )
                .unwrap()
                .next_move_number,
                valid
            );
        }
        for invalid in ["0", "10000", "-1", "1.0", "x"] {
            assert_eq!(
                parse_extended_sfen(
                    &format!("{EMPTY_BOARD} b - {invalid} -"),
                    Rules::engine_default()
                ),
                Err(SfenError::InvalidMoveNumber),
                "{invalid}"
            );
        }
    }

    #[test]
    fn extended_sfen_rejects_invalid_deferred_lists_and_requires_p1() {
        let first = sq(4, 9);
        let second = sq(5, 10);
        let mut builder = PositionBuilder::new(Color::Black);
        for square in [first, second] {
            builder
                .put(
                    square,
                    PieceCode::new(Color::Black, PieceKind::SilverGeneral),
                )
                .unwrap();
        }
        let board = to_sfen(&builder.finish().unwrap());
        let p1 = Rules::from_codes(&[RuleCode::P1, RuleCode::R1]).unwrap();

        assert_eq!(
            parse_extended_sfen(&format!("{board} - 1 8c"), Rules::engine_default()),
            Err(SfenError::PromotionDeferredRequiresP1)
        );
        assert!(matches!(
            parse_extended_sfen(&format!("{board} - 1 8c,8c"), p1),
            Err(SfenError::PromotionDeferredNotStrictlyAscending { .. })
        ));
        assert!(matches!(
            parse_extended_sfen(&format!("{board} - 1 7b,8c"), p1),
            Err(SfenError::PromotionDeferredNotStrictlyAscending { .. })
        ));
        for invalid in ["13a", "8c,"] {
            assert_eq!(
                parse_extended_sfen(&format!("{board} - 1 {invalid}"), p1),
                Err(SfenError::InvalidPromotionDeferredSquare),
                "{invalid}"
            );
        }
    }

    #[test]
    fn extended_sfen_applies_position_builder_invariants_to_deferred_squares() {
        let empty = sq(0, 9);
        let king = sq(1, 9);
        let promoted = sq(2, 9);
        let outside_zone = sq(3, 7);
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(king, PieceCode::new(Color::Black, PieceKind::King))
            .unwrap();
        builder
            .put(
                promoted,
                PieceCode::new(Color::Black, PieceKind::SilverGeneral)
                    .promote()
                    .unwrap(),
            )
            .unwrap();
        builder
            .put(
                outside_zone,
                PieceCode::new(Color::Black, PieceKind::SilverGeneral),
            )
            .unwrap();
        let board = to_sfen(&builder.finish().unwrap());
        let p1 = Rules::from_codes(&[RuleCode::P1, RuleCode::R1]).unwrap();

        for square in [empty, king, promoted, outside_zone] {
            let text = square_to_text(square);
            assert!(matches!(
                parse_extended_sfen(&format!("{board} - 1 {text}"), p1),
                Err(SfenError::PositionBuild(
                    PositionBuildError::InvalidPosition(
                        PositionError::InvalidPromotionDeferred { square: rejected }
                    )
                )) if rejected == square
            ));
        }
    }

    #[test]
    fn extended_sfen_rejects_field_counts_other_than_four_or_five() {
        for input in [
            format!("{EMPTY_BOARD} b -"),
            format!("{EMPTY_BOARD} b - 1 - extra"),
        ] {
            assert!(matches!(
                parse_extended_sfen(&input, Rules::engine_default()),
                Err(SfenError::InvalidExtendedFieldCount { .. })
            ));
        }
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
