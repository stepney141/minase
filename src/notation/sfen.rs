//! shogiops互換SFENと、先獅子・手数・成り権保留(P1・P2・P5)を加えた拡張SFENの読み書き。

use core::fmt;

use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::{Position, PositionBuildError, PositionBuilder};
use crate::core::rules::{RuleCode, Rules};
use crate::core::square::{BOARD_FILES, BOARD_RANKS, Square};

/// 拡張SFENが表す対局開始局面。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SetupPosition {
    /// 盤面、手番および成り権保留状態(P1・P2・P5)。
    pub position: Position,
    /// 直前の非獅子による獅子捕獲升。先獅子(第15条)の復元に使う。
    pub lion_capture: Option<Square>,
    /// 次の着手の手数。表記だけに保持し、裁定には使わない。
    pub next_move_number: u32,
}

/// SFENの構文または局面構築のエラー。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SfenError {
    /// 盤面欄がない。
    MissingBoard,
    /// 手番欄がない。
    MissingSideToMove,
    /// 手番欄が`b`でも`w`でもない。
    InvalidSideToMove,
    /// 基本形の手番欄の後に余分な欄がある。
    UnexpectedFields,
    /// 拡張SFENの欄数が4または5ではない。
    InvalidExtendedFieldCount {
        /// 入力にあった欄数。
        found: usize,
    },
    /// 獅子捕獲升の表記が盤上の升を表さない。
    InvalidLionCaptureSquare,
    /// 獅子捕獲升に手番側の駒があるため、直前着手後の状態として成立しない。
    LionCaptureOccupiedBySideToMove {
        /// 問題の獅子捕獲升。
        square: Square,
    },
    /// 手数が1以上9999以下の整数ではない。
    InvalidMoveNumber,
    /// 成り権保留升(P1・P2・P5)の表記が盤上の升を表さない。
    InvalidPromotionDeferredSquare,
    /// 成り権保留升(P1・P2・P5)が内部密番号の狭義昇順ではない。
    PromotionDeferredNotStrictlyAscending {
        /// 直前に読んだ升。
        previous: Square,
        /// 順序に反した升。
        current: Square,
    },
    /// 成り権保留欄はP1・P2・P5のいずれかの採用を要する。
    PromotionDeferredRequiresRule,
    /// 盤面の段数が12ではない。
    WrongRowCount {
        /// 入力にあった段数。
        found: usize,
    },
    /// 空升数の表記が0または桁あふれである。
    InvalidEmptyCount {
        /// 問題の1起算の段番号。
        row: usize,
    },
    /// 段の升数が12ではない。
    WrongRowWidth {
        /// 問題の1起算の段番号。
        row: usize,
        /// その段にあった升数。
        found: usize,
    },
    /// 成り接頭辞`+`の後に駒文字がない。
    MissingPromotedPiece {
        /// 問題の1起算の段番号。
        row: usize,
        /// 問題の1起算の列番号。
        column: usize,
    },
    /// 駒文字がSFENの駒文字表にない。
    UnsupportedPiece {
        /// 問題の1起算の段番号。
        row: usize,
        /// 問題の1起算の列番号。
        column: usize,
        /// 問題の駒文字。
        letter: char,
    },
    /// 成らない駒に成り接頭辞が付いている。
    UnpromotablePiece {
        /// 問題の1起算の段番号。
        row: usize,
        /// 問題の1起算の列番号。
        column: usize,
        /// 問題の駒文字。
        letter: char,
    },
    /// 局面構築の不変条件に反した。
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
            Self::PromotionDeferredRequiresRule => formatter
                .write_str("extended SFEN promotion-deferred squares require rule P1, P2, or P5"),
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
            | Self::PromotionDeferredRequiresRule
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

/// 対局開始局面を先獅子と成り権保留(P1・P2・P5)を含む5欄の拡張SFENへ書き出す。
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
/// 4欄入力では成り権保留欄(P1・P2・P5)を`-`とみなす。獅子捕獲升は検証して
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
    // 手数欄は数字列だけを受理する。u32のFromStrは先頭の`+`を受理するため、
    // 拡張SFEN文法（protocol-layer.md）の整数表記に合わせて事前に検査する。
    let next_move_number = Some(fields[3])
        .filter(|text| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|text| text.parse::<u32>().ok())
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

/// 升を筋数字と段英字の表記(例: `6f`)へ変換する。
pub(super) fn square_to_text(square: Square) -> String {
    let file = BOARD_FILES - square.file();
    let rank = char::from(b'a' + (BOARD_RANKS - 1 - square.rank()));
    format!("{file}{rank}")
}

/// 筋数字と段英字の表記を升へ変換する。盤外や先行ゼロは`None`を返す。
pub(super) fn parse_square(text: &str) -> Option<Square> {
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

/// 手番欄(`b`または`w`)を解析する。
fn parse_side_to_move(field: &str) -> Result<Color, SfenError> {
    match field {
        "b" => Ok(Color::Black),
        "w" => Ok(Color::White),
        _ => Err(SfenError::InvalidSideToMove),
    }
}

/// 成り権保留欄(P1・P2・P5)を解析する。`-`は空を表し、升は内部密番号の狭義昇順を要求する。
fn parse_promotion_deferred(field: &str, rules: Rules) -> Result<Vec<Square>, SfenError> {
    if field == "-" {
        return Ok(Vec::new());
    }
    if ![RuleCode::P1, RuleCode::P2, RuleCode::P5]
        .into_iter()
        .any(|code| rules.contains(code))
    {
        return Err(SfenError::PromotionDeferredRequiresRule);
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
/// 手数および成り権保留(P1・P2・P5)は拡張SFENで扱う。
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

/// 盤面欄を解析し、手番と成り権保留升とあわせて局面を構築する。
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
    use crate::test_util::sq;

    const EMPTY_BOARD: &str = "12/12/12/12/12/12/12/12/12/12/12/12";

    // lishogi正準4欄の初期局面SFEN（引き継ぎ資産。[USI]「SFENの全体構文」、盤面部は
    // [CECP]第7章のHaChu内蔵FENと文字列一致）。
    const LISHOGI_INITIAL_SFEN: &str = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b - 1";

    fn board(rows: [&str; 12]) -> String {
        rows.join("/")
    }

    fn board_with_row(row_index: usize, row: &str) -> String {
        let mut rows = ["12"; 12];
        rows[row_index] = row;
        rows.join("/")
    }

    // D5-SFEN-01: 升名は筋数字（1〜12）＋段英字（a〜l）。[USI]「座標系」の変換式に従う。
    #[test]
    fn square_names_follow_lishogi_coordinates_and_round_trip_all_144_squares() {
        for square in Square::all() {
            // 筋番号 = 12 − 内部file、段英字 = 'a' + (11 − 内部rank)（[USI]「座標系」）。
            let expected = format!(
                "{}{}",
                BOARD_FILES - square.file(),
                char::from(b'a' + (BOARD_RANKS - 1 - square.rank()))
            );
            assert_eq!(square_to_text(square), expected);
            assert_eq!(parse_square(&expected), Some(square));
        }

        // 12aは先手から見て左上隅であり、2桁筋は1桁筋と混同されない（[USI]「座標系」）。
        assert_eq!(parse_square("12a"), Some(sq(0, 11)));
        assert_eq!(parse_square("1a"), Some(sq(11, 11)));
        assert_eq!(parse_square("10l"), Some(sq(2, 0)));

        // 筋0・筋13以上・段m以降などは升名として不正（D5-SFEN-01境界）。
        for invalid in ["0a", "13a", "7m", "a1", "7", "", "01a"] {
            assert_eq!(parse_square(invalid), None, "{invalid}");
        }
    }

    // D5-SFEN-02: 基底駒21文字と先後の大小文字（[USI]「駒種の文字表記」、[HACHU]第11節）。
    #[test]
    fn all_21_base_piece_letters_parse_and_round_trip_in_both_cases() {
        let letters = [
            ('p', PieceKind::Pawn),
            ('i', PieceKind::GoBetween),
            ('l', PieceKind::Lance),
            ('a', PieceKind::ReverseChariot),
            ('f', PieceKind::FerociousLeopard),
            ('c', PieceKind::CopperGeneral),
            ('s', PieceKind::SilverGeneral),
            ('g', PieceKind::GoldGeneral),
            ('t', PieceKind::BlindTiger),
            ('e', PieceKind::DrunkElephant),
            ('x', PieceKind::Phoenix),
            ('o', PieceKind::Kirin),
            ('m', PieceKind::SideMover),
            ('v', PieceKind::VerticalMover),
            ('b', PieceKind::Bishop),
            ('r', PieceKind::Rook),
            ('h', PieceKind::DragonHorse),
            ('d', PieceKind::DragonKing),
            ('k', PieceKind::King),
            ('n', PieceKind::Lion),
            ('q', PieceKind::FreeKing),
        ];
        assert_eq!(letters.len(), 21);

        for (letter, kind) in letters {
            // 大文字が先手、小文字が後手（[USI]「駒種の文字表記」）。
            for (text, color) in [
                (letter.to_ascii_uppercase(), Color::Black),
                (letter, Color::White),
            ] {
                let sfen = format!("{} b", board_with_row(0, &format!("{text}11")));
                let position = parse_sfen(&sfen).unwrap();
                assert_eq!(
                    position.piece_at(sq(0, 11)),
                    Some(PieceCode::new(color, kind)),
                    "{text}"
                );
                assert_eq!(to_sfen(&position), sfen, "{text}");
            }
        }

        // J・U・W・Y・Zは未割当であり拒否する（[HACHU]第11節）。
        for letter in ['j', 'u', 'w', 'y', 'z', 'J', 'U', 'W', 'Y', 'Z'] {
            let sfen = format!("{} b", board_with_row(0, &format!("{letter}11")));
            assert!(
                matches!(parse_sfen(&sfen), Err(SfenError::UnsupportedPiece { .. })),
                "{letter}"
            );
        }
    }

    // D5-SFEN-03: 成駒18種の`+`前置と出自保存、成れない3種k・n・qへの`+`の拒否
    // （[USI]「駒種の文字表記」、[RULES]第17条）。引き継ぎ資産の駒文字表。
    #[test]
    fn promoted_prefix_covers_18_kinds_and_unpromotable_pieces_reject_it() {
        let promoted = [
            ('p', PieceKind::GoldGeneral),
            ('i', PieceKind::DrunkElephant),
            ('l', PieceKind::WhiteHorse),
            ('a', PieceKind::Whale),
            ('f', PieceKind::Bishop),
            ('c', PieceKind::SideMover),
            ('s', PieceKind::VerticalMover),
            ('g', PieceKind::Rook),
            ('t', PieceKind::FlyingStag),
            ('e', PieceKind::CrownPrince),
            ('x', PieceKind::FreeKing),
            ('o', PieceKind::Lion),
            ('m', PieceKind::FreeBoar),
            ('v', PieceKind::FlyingOx),
            ('b', PieceKind::DragonHorse),
            ('r', PieceKind::DragonKing),
            ('h', PieceKind::HornedFalcon),
            ('d', PieceKind::SoaringEagle),
        ];
        assert_eq!(promoted.len(), 18);

        for (letter, kind) in promoted {
            for (text, color) in [
                (letter.to_ascii_uppercase(), Color::Black),
                (letter, Color::White),
            ] {
                let sfen = format!("{} b", board_with_row(0, &format!("+{text}11")));
                let position = parse_sfen(&sfen).unwrap();
                assert_eq!(
                    position.piece_at(sq(0, 11)),
                    PieceCode::new_promoted(color, kind),
                    "+{text}"
                );
                assert_eq!(to_sfen(&position), sfen, "+{text}");
            }
        }

        // 猛豹成りの角行相当+fと生の角行bは出自別に区別される（[USI]「駒種の文字表記」末尾）。
        let promoted_leopard = parse_sfen(&format!("{} b", board_with_row(0, "+f11"))).unwrap();
        let raw_bishop = parse_sfen(&format!("{} b", board_with_row(0, "b11"))).unwrap();
        assert_ne!(
            promoted_leopard.piece_at(sq(0, 11)),
            raw_bishop.piece_at(sq(0, 11))
        );
        assert_ne!(to_sfen(&promoted_leopard), to_sfen(&raw_bishop));

        // 成らない駒への`+`の拒否（[RULES]第17条第1項）。
        for letter in ['k', 'n', 'q', 'K', 'N', 'Q'] {
            let sfen = format!("{} b", board_with_row(0, &format!("+{letter}11")));
            assert!(
                matches!(parse_sfen(&sfen), Err(SfenError::UnpromotablePiece { .. })),
                "+{letter}"
            );
        }

        // `+`の直後に駒文字が続かない入力の拒否（D5-SFEN-03境界）。
        for row in ["11+", "+111", "++p10", "+"] {
            assert!(
                parse_sfen(&format!("{} b", board_with_row(0, row))).is_err(),
                "{row}"
            );
        }
    }

    // D5-SFEN-04: 盤面欄の走査順（段a→段l、12筋→1筋）と空升の10進圧縮
    // （[USI]「SFENの全体構文」第1項・「to_sfenへの要求事項」）。
    #[test]
    fn board_scan_order_and_empty_run_compression_follow_shogiops() {
        // 先頭行の左端は12a（内部file 0・rank 11）、最終行の右端は1l（内部file 11・rank 0）。
        let corners = parse_sfen(&format!(
            "{} b",
            board([
                "P11", "12", "12", "12", "12", "12", "12", "12", "12", "12", "12", "11p"
            ])
        ))
        .unwrap();
        assert_eq!(
            corners.piece_at(sq(0, 11)),
            Some(PieceCode::new(Color::Black, PieceKind::Pawn))
        );
        assert_eq!(
            corners.piece_at(sq(11, 0)),
            Some(PieceCode::new(Color::White, PieceKind::Pawn))
        );

        // 連続する数字トークンの累積解釈: 3i4i3は3空＋仲人＋4空＋仲人＋3空。
        let go_betweens = parse_sfen(&format!("{} b", board_with_row(0, "3i4i3"))).unwrap();
        assert_eq!(
            go_betweens.piece_at(sq(3, 11)),
            Some(PieceCode::new(Color::White, PieceKind::GoBetween))
        );
        assert_eq!(
            go_betweens.piece_at(sq(8, 11)),
            Some(PieceCode::new(Color::White, PieceKind::GoBetween))
        );

        // `1`の直後の`2`は12と解釈される（`12`のみ＝全空の段の受理）。
        let empty = parse_sfen(&format!("{EMPTY_BOARD} b")).unwrap();
        assert!(Square::all().all(|square| empty.piece_at(square).is_none()));

        // 書き出しは最長連続をひとつの数字で圧縮する（非圧縮の`111`等を生成しない）。
        assert_eq!(
            to_sfen(&go_betweens),
            format!("{} b", board_with_row(0, "3i4i3"))
        );
        assert_eq!(to_sfen(&empty), format!("{EMPTY_BOARD} b"));

        // 段数・升数の不一致、空升数0の拒否（D5-SFEN-04境界）。
        assert!(matches!(
            parse_sfen("12/12/12/12/12/12/12/12/12/12/12 b"),
            Err(SfenError::WrongRowCount { .. })
        ));
        assert!(matches!(
            parse_sfen(&format!("{} b", board_with_row(0, "11"))),
            Err(SfenError::WrongRowWidth { .. })
        ));
        assert!(matches!(
            parse_sfen(&format!("{} b", board_with_row(0, "13"))),
            Err(SfenError::WrongRowWidth { .. })
        ));
        assert!(matches!(
            parse_sfen(&format!("{} b", board_with_row(0, "0P11"))),
            Err(SfenError::InvalidEmptyCount { .. })
        ));
    }

    // D5-SFEN-05: 手番欄はbが先手、wが後手（[USI]「SFENの全体構文」第2項）。
    #[test]
    fn side_to_move_letter_maps_b_to_black_and_w_to_white() {
        assert_eq!(
            parse_sfen(&format!("{EMPTY_BOARD} b"))
                .unwrap()
                .side_to_move(),
            Color::Black
        );
        assert_eq!(
            parse_sfen(&format!("{EMPTY_BOARD} w"))
                .unwrap()
                .side_to_move(),
            Color::White
        );
        assert!(to_sfen(&Position::empty(Color::Black)).ends_with(" b"));
        assert!(to_sfen(&Position::empty(Color::White)).ends_with(" w"));

        for invalid in ["B", "W", "x"] {
            assert_eq!(
                parse_sfen(&format!("{EMPTY_BOARD} {invalid}")),
                Err(SfenError::InvalidSideToMove),
                "{invalid}"
            );
        }
    }

    // D5-SFEN-06: lishogi正準4欄構文と初期局面SFENの厳密一致（引き継ぎ資産。
    // [USI]「SFENの全体構文」、[RULES]第5条、[CECP]第7章）。
    #[test]
    fn lishogi_initial_four_field_sfen_matches_the_initial_position() {
        let setup = parse_extended_sfen(LISHOGI_INITIAL_SFEN, Rules::engine_default()).unwrap();
        assert_eq!(setup.position, Position::initial());
        assert_eq!(setup.lion_capture, None);
        assert_eq!(setup.next_move_number, 1);

        // 王将Kが先手、玉将kが後手にある（[RULES]第5条）。
        assert_eq!(
            setup.position.piece_at(sq(5, 0)),
            Some(PieceCode::new(Color::Black, PieceKind::King))
        );
        assert_eq!(
            setup.position.piece_at(sq(6, 11)),
            Some(PieceCode::new(Color::White, PieceKind::King))
        );

        // 書き出しの4欄導出形（5欄から第5欄を落とす）が初期SFEN文字列と一致する。
        assert_eq!(
            to_extended_sfen(&setup),
            format!("{LISHOGI_INITIAL_SFEN} -")
        );
    }

    // D5-SFEN-07: minase基本形式は「盤面部 手番部」の2欄だけを読み書きする
    // （[USI]「to_sfenへの要求事項」、[PL]「拡張SFEN」）。
    #[test]
    fn basic_form_reads_and_writes_exactly_two_fields() {
        let position = Position::initial();
        let sfen = to_sfen(&position);
        assert_eq!(sfen.split_whitespace().count(), 2);
        assert_eq!(parse_sfen(&sfen).unwrap(), position);

        // 獅子捕獲升欄への部分対応（`-`だけの受理）は行わない。3欄以上と1欄は拒否する。
        for extra in [format!("{sfen} -"), format!("{sfen} - 1")] {
            assert_eq!(parse_sfen(&extra), Err(SfenError::UnexpectedFields));
        }
        assert_eq!(parse_sfen(EMPTY_BOARD), Err(SfenError::MissingSideToMove));

        // 2欄形式は成り権保留を運ばない（仕様上の制限）。盤面と手番は往復で保存される。
        let deferred_square = sq(7, 9);
        let mut builder = PositionBuilder::new(Color::White);
        builder
            .put(
                deferred_square,
                PieceCode::new(Color::Black, PieceKind::SilverGeneral),
            )
            .unwrap();
        builder.mark_promotion_deferred(deferred_square).unwrap();
        let with_deferred = builder.finish().unwrap();
        let restored = parse_sfen(&to_sfen(&with_deferred)).unwrap();
        assert!(restored.promotion_deferred().is_empty());
        assert_eq!(restored.side_to_move(), with_deferred.side_to_move());
        assert!(
            Square::all().all(|square| restored.piece_at(square) == with_deferred.piece_at(square))
        );
    }

    // D5-SFEN-08: 不正入力の非パニック拒否（[PL]「フェーズ2の確定設計」のエンジン境界の
    // 厳格性）。エラー分類の粒度は規範文書に定めがないため（SU-3）、拒否だけを断定する。
    #[test]
    fn malformed_inputs_are_rejected_without_panicking() {
        let inputs = [
            String::new(),
            "   ".to_owned(),
            "b".to_owned(),
            format!("{EMPTY_BOARD} z"),
            // 2桁数字の途中で行が終わる。
            "12/12/12/12/12/12/12/12/12/12/12/1 b".to_owned(),
            // 段区切りの連続と末尾の段区切り。
            "12/12//12/12/12/12/12/12/12/12/12 b".to_owned(),
            format!("{EMPTY_BOARD}/ b"),
            // 非ASCII文字。
            "１2/12/12/12/12/12/12/12/12/12/12/12 b".to_owned(),
            format!("{} b", board_with_row(0, "11＋p")),
            // 空升数の桁あふれ。
            format!("{} b", board_with_row(0, &"9".repeat(30))),
            // 極端に長い入力。
            format!("{} b", "1".repeat(65536)),
        ];
        for input in inputs {
            assert!(parse_sfen(&input).is_err(), "{input:.40}");
            assert!(
                parse_extended_sfen(&format!("{input} - 1"), Rules::engine_default()).is_err(),
                "{input:.40}"
            );
        }
    }

    // D5-SFEN-09: 第3欄は`-`または升名1つ。空升は受理し、手番側の駒がある升は拒否する
    // （[PL]「拡張SFEN」、[USI]「SFENの全体構文」第3項の経由升捕獲を含む訂正後条件）。
    #[test]
    fn lion_capture_field_accepts_dash_empty_and_non_mover_squares_only() {
        let rules = Rules::engine_default();
        // 7f = 内部(5, 6)。
        let capture_square = sq(5, 6);
        let occupied_board = board_with_row(5, "5p6");

        // `-`は獅子捕獲なし。
        assert_eq!(
            parse_extended_sfen(&format!("{occupied_board} b - 1"), rules)
                .unwrap()
                .lion_capture,
            None
        );
        // 非手番側の駒がいる升（通常の到達升捕獲）。
        let occupied = parse_extended_sfen(&format!("{occupied_board} b 7f 1"), rules).unwrap();
        assert_eq!(occupied.lion_capture, Some(capture_square));
        // 空升も受理する（飛鷲・角鷹の経由升捕獲では捕獲升が空になる。[PL]フェーズ3の契約訂正）。
        let empty = parse_extended_sfen(&format!("{EMPTY_BOARD} b 7f 1"), rules).unwrap();
        assert_eq!(empty.lion_capture, Some(capture_square));

        // 書き出し⇒解析の往復で捕獲升が保存される。
        assert_eq!(
            parse_extended_sfen(&to_extended_sfen(&occupied), rules).unwrap(),
            occupied
        );

        // 手番側の駒がある升は、直前の着手の結果として成立しないため拒否する。
        let own_board = board_with_row(5, "5P6");
        assert!(matches!(
            parse_extended_sfen(&format!("{own_board} b 7f 1"), rules),
            Err(SfenError::LionCaptureOccupiedBySideToMove { .. })
        ));

        // 不正な升名の拒否。
        for invalid in ["13a", "0a", "7m", "--"] {
            assert_eq!(
                parse_extended_sfen(&format!("{EMPTY_BOARD} b {invalid} 1"), rules),
                Err(SfenError::InvalidLionCaptureSquare),
                "{invalid}"
            );
        }
    }

    // D5-SFEN-10: L2判定情報は第3欄の升にいる非手番側の駒が麒麟由来の成獅子（+o）か
    // どうかから導出される（[PL]「拡張SFEN」、[RULES]第29条L2）。表記層は捕獲升と
    // 盤面の出自を保存し、導出の前提（+oとnの区別）を担う。
    #[test]
    fn lion_capture_square_preserves_kirin_promoted_lion_identity_for_l2() {
        let rules = Rules::engine_default();
        let capture_square = sq(5, 6);

        // 非手番側の+o（麒麟由来の成獅子）・n（生の獅子）・他の駒のいずれも受理する
        // （拒否条件は手番側の駒だけ。D5-SFEN-10境界）。
        let mut pieces = Vec::new();
        for row in ["5+o6", "5n6", "5p6"] {
            let setup =
                parse_extended_sfen(&format!("{} b 7f 1", board_with_row(5, row)), rules).unwrap();
            assert_eq!(setup.lion_capture, Some(capture_square), "{row}");
            pieces.push(setup.position.piece_at(capture_square).unwrap());
        }

        // +oとnは解析後も区別され（D5-SFEN-03の出自保存）、L2導出の一意性が成り立つ。
        assert_eq!(
            Some(pieces[0]),
            PieceCode::new_promoted(Color::White, PieceKind::Lion)
        );
        assert_eq!(pieces[1], PieceCode::new(Color::White, PieceKind::Lion));
        assert_ne!(pieces[0], pieces[1]);
    }

    // D5-SFEN-11: 第4欄は次の着手の手数で、1以上9999以下の整数だけを受理する
    // （[PL]「拡張SFEN」、[USI]「SFENの全体構文」第4項）。
    #[test]
    fn move_number_field_accepts_one_through_9999_and_round_trips() {
        let rules = Rules::engine_default();
        for valid in [1_u32, 9999] {
            let setup = parse_extended_sfen(&format!("{EMPTY_BOARD} b - {valid}"), rules).unwrap();
            assert_eq!(setup.next_move_number, valid);
            // 受理した手数は書き出しで保存される（往復一致）。
            assert!(to_extended_sfen(&setup).contains(&format!(" {valid} ")));
        }
        // `+1`はu32のFromStrが受理するが、拡張SFEN文法の整数表記ではないため拒否する
        // （spec-first再構築で検出した乖離の修正を固定）。
        for invalid in ["0", "10000", "-1", "+1", "1.5", "x"] {
            assert_eq!(
                parse_extended_sfen(&format!("{EMPTY_BOARD} b - {invalid}"), rules),
                Err(SfenError::InvalidMoveNumber),
                "{invalid}"
            );
        }
    }

    // D5-SFEN-12: 第5欄は成り権保留升(P1・P2・P5)のコンマ区切り列（[PL]「拡張SFEN」、
    // [RULES]第30条）。順序キー「内部密番号」の文書定義は未補修のため（SU-2）、決定性・往復保存・
    // 順序の単調性だけを断定し、特定の並びの直値は固定しない。
    #[test]
    fn deferred_field_is_a_strictly_ordered_comma_list_that_round_trips() {
        let p1 = Rules::from_codes(&[RuleCode::P1, RuleCode::R1]).unwrap();
        // 黒銀8c＝内部(4, 9)、白銀7j＝内部(5, 2)。どちらも自軍から見た敵陣内の未成駒。
        let mut rows = ["12"; 12];
        rows[2] = "4S7";
        rows[9] = "5s6";
        let board = rows.join("/");

        // 1升だけの列の受理と往復。
        let single = parse_extended_sfen(&format!("{board} b - 1 8c"), p1).unwrap();
        assert!(single.position.promotion_deferred().contains(sq(4, 9)));
        assert_eq!(to_extended_sfen(&single), format!("{board} b - 1 8c"));

        // 2升の列は一方の順序だけが昇順として受理され、逆順は拒否される（順序の単調性）。
        let forward = parse_extended_sfen(&format!("{board} b - 1 7j,8c"), p1);
        let backward = parse_extended_sfen(&format!("{board} b - 1 8c,7j"), p1);
        let (accepted, accepted_text) = match (forward, backward) {
            (Ok(setup), Err(SfenError::PromotionDeferredNotStrictlyAscending { .. })) => {
                (setup, format!("{board} b - 1 7j,8c"))
            }
            (Err(SfenError::PromotionDeferredNotStrictlyAscending { .. }), Ok(setup)) => {
                (setup, format!("{board} b - 1 8c,7j"))
            }
            other => panic!("exactly one order must be accepted: {other:?}"),
        };
        // 受理した保留集合は同一の列（同一順序）として再現される。
        assert_eq!(to_extended_sfen(&accepted), accepted_text);

        // 重複・空要素・不正升名の拒否。
        assert!(matches!(
            parse_extended_sfen(&format!("{board} b - 1 8c,8c"), p1),
            Err(SfenError::PromotionDeferredNotStrictlyAscending { .. })
        ));
        for invalid in ["7j,,8c", ",8c", "8c,", "13a"] {
            assert_eq!(
                parse_extended_sfen(&format!("{board} b - 1 {invalid}"), p1),
                Err(SfenError::InvalidPromotionDeferredSquare),
                "{invalid}"
            );
        }
    }

    // 第30条P1・P2・P5: 第5欄は3規則のいずれかを採用するときだけ受理する。
    #[test]
    fn deferred_field_requires_p1_p2_or_p5_and_round_trips_for_each() {
        let mut rows = ["12"; 12];
        rows[2] = "4S7";
        let board = rows.join("/");

        // 第30条P1・P2・P5: いずれもなければ、構文が正しい升名列も拒否する。
        assert!(parse_extended_sfen(&format!("{board} b - 1 -"), Rules::engine_default()).is_ok());
        assert_eq!(
            parse_extended_sfen(&format!("{board} b - 1 8c"), Rules::engine_default()),
            Err(SfenError::PromotionDeferredRequiresRule)
        );

        // 第30条P1・P2・P5: 各規則の単独採用で第5欄を受理し、そのまま往復する。
        let text = format!("{board} b - 1 8c");
        for code in [RuleCode::P1, RuleCode::P2, RuleCode::P5] {
            let rules = Rules::from_codes(&[code, RuleCode::R1]).unwrap();
            let setup = parse_extended_sfen(&text, rules).unwrap();
            assert!(setup.position.promotion_deferred().contains(sq(4, 9)));
            assert_eq!(to_extended_sfen(&setup), text, "{code:?}");
        }
    }

    // D5-SFEN-14: 保留升には成り得る未成駒の存在を要求し、空升・成駒・成れない駒の升を
    // 拒否する（[PL]「拡張SFEN」の不変条件4類型、[RULES]第17条第1項）。
    #[test]
    fn deferred_squares_must_hold_an_unpromoted_promotable_piece() {
        let p1 = Rules::from_codes(&[RuleCode::P1, RuleCode::R1]).unwrap();
        // 段c（内部rank 9）: 空升・王将・成銀・歩兵・獅子・奔王を並べる。
        let mut rows = ["12"; 12];
        rows[2] = "1K+SPNQ6";
        let board = rows.join("/");

        // 未成の歩兵がいる升（9c）は受理する。
        let ok = parse_extended_sfen(&format!("{board} b - 1 9c"), p1).unwrap();
        assert!(ok.position.promotion_deferred().contains(sq(3, 9)));

        // 空升12c・王将11c・成駒10c・獅子8c・奔王7cはいずれも拒否する。
        for invalid in ["12c", "11c", "10c", "8c", "7c"] {
            assert!(
                matches!(
                    parse_extended_sfen(&format!("{board} b - 1 {invalid}"), p1),
                    Err(SfenError::PositionBuild(_))
                ),
                "{invalid}"
            );
        }
    }

    // D5-SFEN-15: 拡張形式は5欄と4欄（第5欄`-`扱い）だけを受理する（[PL]「拡張SFEN」）。
    #[test]
    fn extended_parser_accepts_only_four_or_five_fields() {
        let rules = Rules::engine_default();
        // 4欄入力は第5欄`-`を明示した5欄入力と同じ解析結果になる。
        let four = parse_extended_sfen(&format!("{EMPTY_BOARD} b - 1"), rules).unwrap();
        let five = parse_extended_sfen(&format!("{EMPTY_BOARD} b - 1 -"), rules).unwrap();
        assert_eq!(four, five);
        // lishogi正準の4欄（初期SFEN）が受理される。
        assert!(parse_extended_sfen(LISHOGI_INITIAL_SFEN, rules).is_ok());

        // 1〜3欄と6欄以上は拒否する。2欄基本形も拡張解析では拒否される。
        for invalid in [
            EMPTY_BOARD.to_owned(),
            format!("{EMPTY_BOARD} b"),
            format!("{EMPTY_BOARD} b -"),
            format!("{EMPTY_BOARD} b - 1 - extra"),
        ] {
            assert!(matches!(
                parse_extended_sfen(&invalid, rules),
                Err(SfenError::InvalidExtendedFieldCount { .. })
            ));
        }
    }

    // D5-SFEN-16: 書き出しは常に5欄で、lishogi互換4欄は第5欄を落とした導出形。
    // 全5欄の情報が往復で一致する（[PL]「拡張SFEN」）。
    #[test]
    fn extended_output_always_writes_five_fields_and_round_trips_all_state() {
        // 捕獲なし・保留なしでも第3欄`-`と第5欄`-`は省略されない。
        let plain = SetupPosition {
            position: Position::initial(),
            lion_capture: None,
            next_move_number: 1,
        };
        let plain_text = to_extended_sfen(&plain);
        assert_eq!(plain_text.split_whitespace().count(), 5);
        assert!(plain_text.ends_with(" b - 1 -"));

        // 盤面・手番・獅子捕獲升・手数・保留集合のすべてが往復で一致する。
        let p1 = Rules::from_codes(&[RuleCode::P1, RuleCode::R1]).unwrap();
        let mut rows = ["12"; 12];
        rows[2] = "4S7";
        rows[9] = "5s6";
        let board = rows.join("/");
        let rich_text = format!("{board} b 7j 4321 8c");
        let rich = parse_extended_sfen(&rich_text, p1).unwrap();
        assert_eq!(to_extended_sfen(&rich), rich_text);
        assert_eq!(
            parse_extended_sfen(&to_extended_sfen(&rich), p1).unwrap(),
            rich
        );
    }
}
