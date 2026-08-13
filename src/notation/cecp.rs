//! CECPの多段階指し手表記。
//!
//! 獅子、角鷹および飛鷲の2段階移動も1手として扱う(第11条・第12条)。

use core::fmt;

use crate::core::mv::Move;
use crate::core::position::Position;
use crate::core::square::Square;

/// CECP指し手の解析エラー(第3条・第11条・第12条)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CecpError {
    /// CECP升名の不正。盤は12×12升である(第4条)。
    InvalidSquare {
        /// 問題の升名がある1起算のレグ番号。
        leg: usize,
        /// レグ内の1起算の升名位置。
        field: usize,
    },
    /// CECP着手のレグ数不正。2段階移動も1手である(第3条・第11条・第12条)。
    InvalidLegCount {
        /// 入力にあったレグ数。
        found: usize,
    },
    /// CECPレグの構文不正。2段階移動は連続する2段階である(第11条・第12条)。
    MalformedLeg {
        /// 問題の1起算のレグ番号。
        leg: usize,
    },
    /// CECPレグ間の不連続。第2段階は第1段階の到達升から始まる(第11条・第12条)。
    LegDiscontinuity,
    /// CECP接尾辞の不正。成りは着手終了時に行う(第18条)。
    InvalidSuffix,
    /// CECP単一レグじっとの不正。じっとは2段階移動である(第11条・第12条)。
    InvalidJittoForm,
    /// CECPの`@@@@`入力の不正。じっとの成立には局面上の移動が必要である(第11条・第12条)。
    MalformedJittoInput,
}

impl fmt::Display for CecpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSquare { leg, field } => {
                write!(formatter, "invalid CECP square in leg {leg}, field {field}")
            }
            Self::InvalidLegCount { found } => {
                write!(formatter, "CECP move has {found} legs; expected 1 or 2")
            }
            Self::MalformedLeg { leg } => write!(formatter, "malformed CECP move leg {leg}"),
            Self::LegDiscontinuity => formatter.write_str("CECP move legs are not continuous"),
            Self::InvalidSuffix => formatter.write_str("invalid CECP move suffix"),
            Self::InvalidJittoForm => {
                formatter.write_str("CECP jitto cannot be written as one same-square leg")
            }
            Self::MalformedJittoInput => {
                formatter.write_str("CECP @@@@ jitto requires legal-move context")
            }
        }
    }
}

impl std::error::Error for CecpError {}

/// CECPの多段階指し手表記を指し手へ変換する。
///
/// 2レグの中間升に相手駒がなければ、終点にかかわらず`mid: None`へ
/// 正規化する。移動元と移動先が等しい2レグは正準じっとになる
/// (第11条・第12条)。着手そのものの合法性は検査しない。
pub fn parse(position: &Position, input: &str) -> Result<Move, CecpError> {
    if input.bytes().any(|byte| byte == b'@') {
        return Err(CecpError::MalformedJittoInput);
    }

    let (body, promote) = split_suffix(input)?;
    let leg_texts: Vec<_> = body.split(',').collect();
    if !matches!(leg_texts.len(), 1 | 2) {
        return Err(CecpError::InvalidLegCount {
            found: leg_texts.len(),
        });
    }

    let first = parse_leg(leg_texts[0], 1)?;
    let (mid, to) = if leg_texts.len() == 1 {
        if first.0 == first.1 {
            return Err(CecpError::InvalidJittoForm);
        }
        (None, first.1)
    } else {
        let second = parse_leg(leg_texts[1], 2)?;
        if first.1 != second.0 {
            return Err(CecpError::LegDiscontinuity);
        }
        (Some(first.1), second.1)
    };

    let mid = mid.filter(|&square| {
        position
            .piece_at(square)
            .is_some_and(|piece| piece.color() == Some(position.side_to_move().opposite()))
    });

    Ok(Move {
        from: first.0,
        mid,
        to,
        promote,
    })
}

/// 指し手をCECPの`move`行の本文列へ変換する。
///
/// 2段階移動は2レグに分け、正準じっとは`@@@@`で表す
/// (第11条・第12条)。
pub fn legs(mv: Move) -> Vec<String> {
    if mv.mid.is_none() && mv.to == mv.from {
        return vec!["@@@@".to_owned()];
    }

    let from = square_to_text(mv.from);
    let to = square_to_text(mv.to);
    let suffix = if mv.promote { "+" } else { "" };
    if let Some(mid) = mv.mid {
        let mid = square_to_text(mid);
        vec![format!("{from}{mid},"), format!("{mid}{to}{suffix}")]
    } else {
        vec![format!("{from}{to}{suffix}")]
    }
}

/// 末尾の接尾辞を切り離し、レグ本文と成りの選択を返す。
///
/// `+`は成り、`=`は不成を表す。接尾辞が末尾以外に現れる入力は拒否する。
fn split_suffix(input: &str) -> Result<(&str, bool), CecpError> {
    let Some(last) = input.as_bytes().last().copied() else {
        return Ok((input, false));
    };
    let (body, promote) = match last {
        b'+' => (&input[..input.len() - 1], true),
        b'=' => (&input[..input.len() - 1], false),
        _ => (input, false),
    };
    if body.bytes().any(|byte| matches!(byte, b'+' | b'=' | b'?')) {
        Err(CecpError::InvalidSuffix)
    } else {
        Ok((body, promote))
    }
}

/// 1レグ(始点升と終点升の連結)を解析する。
fn parse_leg(input: &str, leg: usize) -> Result<(Square, Square), CecpError> {
    let (from, remaining) = parse_square_prefix(input, leg, 1)?;
    let (to, remaining) = parse_square_prefix(remaining, leg, 2)?;
    if !remaining.is_empty() {
        return Err(CecpError::MalformedLeg { leg });
    }
    Ok((from, to))
}

/// 入力の先頭からCECP升名(筋英字1文字と段数字1〜2桁)を1個読み取り、残りを返す。
fn parse_square_prefix(input: &str, leg: usize, field: usize) -> Result<(Square, &str), CecpError> {
    let invalid = CecpError::InvalidSquare { leg, field };
    let bytes = input.as_bytes();
    let Some(&file_byte) = bytes.first() else {
        return Err(invalid);
    };
    if !(b'a'..=b'l').contains(&file_byte) {
        return Err(invalid);
    }

    let digit_count = bytes[1..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if !(1..=2).contains(&digit_count) || bytes[1] == b'0' {
        return Err(invalid);
    }
    let rank_number = if digit_count == 1 {
        bytes[1] - b'0'
    } else {
        (bytes[1] - b'0') * 10 + (bytes[2] - b'0')
    };
    let square = Square::new(file_byte - b'a', rank_number - 1).ok_or(invalid)?;
    Ok((square, &input[digit_count + 1..]))
}

/// 升をCECP升名(例: `f6`)へ変換する。
fn square_to_text(square: Square) -> String {
    let mut output = String::with_capacity(3);
    output.push(char::from(b'a' + square.file()));
    output.push_str(&(square.rank() + 1).to_string());
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::movegen::MoveGenerator;
    use crate::core::piece::{Color, PieceCode, PieceKind};
    use crate::core::position::PositionBuilder;
    use crate::notation::usi;
    use crate::test_util::sq;

    fn generated(position: &Position) -> Vec<Move> {
        let mut moves = Vec::new();
        MoveGenerator::standard().generate_moves(position, &mut moves);
        moves
    }

    fn single_piece_position(color: Color, kind: PieceKind, from: Square) -> Position {
        let mut builder = PositionBuilder::new(color);
        builder.put(from, PieceCode::new(color, kind)).unwrap();
        builder.finish().unwrap()
    }

    fn assert_all_moves_round_trip(position: &Position) {
        let moves = generated(position);
        assert!(!moves.is_empty());

        for mv in moves {
            let rendered = legs(mv);
            if mv.mid.is_none() && mv.to == mv.from {
                assert_eq!(rendered, ["@@@@"]);
            } else {
                let wire = rendered.concat();
                assert_eq!(parse(position, &wire), Ok(mv), "text={wire}");
            }
        }
    }

    #[test]
    fn every_legal_move_round_trips_in_representative_positions() {
        let mut special_builder = PositionBuilder::new(Color::Black);
        for (square, color, kind) in [
            (sq(5, 5), Color::Black, PieceKind::Lion),
            (sq(1, 1), Color::Black, PieceKind::HornedFalcon),
            (sq(9, 1), Color::Black, PieceKind::SoaringEagle),
            (sq(5, 6), Color::White, PieceKind::Pawn),
            (sq(6, 6), Color::White, PieceKind::SilverGeneral),
            (sq(1, 2), Color::White, PieceKind::Pawn),
            (sq(8, 2), Color::White, PieceKind::Pawn),
        ] {
            special_builder
                .put(square, PieceCode::new(color, kind))
                .unwrap();
        }
        let special = special_builder.finish().unwrap();

        let mut promotion_builder = PositionBuilder::new(Color::Black);
        promotion_builder
            .put(sq(4, 7), PieceCode::new(Color::Black, PieceKind::Pawn))
            .unwrap();
        let promotion = promotion_builder.finish().unwrap();

        let positions = [
            Position::initial(),
            special,
            single_piece_position(Color::Black, PieceKind::Lion, sq(5, 5)),
            single_piece_position(Color::Black, PieceKind::HornedFalcon, sq(5, 5)),
            single_piece_position(Color::White, PieceKind::HornedFalcon, sq(5, 5)),
            single_piece_position(Color::Black, PieceKind::SoaringEagle, sq(5, 5)),
            single_piece_position(Color::White, PieceKind::SoaringEagle, sq(5, 5)),
            promotion,
        ];

        for position in &positions {
            assert_all_moves_round_trip(position);
        }

        let special_moves = generated(&positions[1]);
        for origin in [sq(5, 5), sq(1, 1), sq(9, 1)] {
            assert!(
                special_moves
                    .iter()
                    .any(|mv| mv.from == origin && mv.mid.is_some() && mv.to != origin)
            );
        }
        assert!(generated(&positions[7]).iter().any(|mv| mv.promote));
    }

    #[test]
    fn concrete_multileg_jump_and_suffix_examples_match() {
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(sq(3, 5), PieceCode::new(Color::White, PieceKind::Pawn))
            .unwrap();
        builder
            .put(sq(6, 6), PieceCode::new(Color::White, PieceKind::Pawn))
            .unwrap();
        let position = builder.finish().unwrap();

        let two_stage = Move {
            from: sq(4, 5),
            mid: Some(sq(3, 5)),
            to: sq(2, 5),
            promote: false,
        };
        assert_eq!(parse(&position, "e6d6,d6c6"), Ok(two_stage));
        assert_eq!(legs(two_stage), ["e6d6,", "d6c6"]);

        let tsukegui = Move {
            from: sq(5, 5),
            mid: Some(sq(6, 6)),
            to: sq(5, 5),
            promote: false,
        };
        assert_eq!(parse(&position, "f6g7,g7f6"), Ok(tsukegui));
        assert_eq!(legs(tsukegui), ["f6g7,", "g7f6"]);

        let direct_jump = Move {
            from: sq(4, 5),
            mid: None,
            to: sq(2, 5),
            promote: false,
        };
        assert_eq!(parse(&position, "e6c6"), Ok(direct_jump));
        assert_eq!(legs(direct_jump), ["e6c6"]);

        assert_eq!(parse(&position, "e6c6="), Ok(direct_jump));
        assert_eq!(parse(&position, "e6c6"), Ok(direct_jump));
        let promoted = Move {
            promote: true,
            ..direct_jump
        };
        assert_eq!(parse(&position, "e6c6+"), Ok(promoted));
        assert_eq!(legs(promoted), ["e6c6+"]);
    }

    #[test]
    fn intermediate_square_normalization_matches_occupancy() {
        let from = sq(5, 5);
        let mid = sq(6, 6);
        let to = sq(7, 7);
        let expected_without_mid = Move {
            from,
            mid: None,
            to,
            promote: false,
        };

        let empty = Position::empty(Color::Black);
        assert_eq!(parse(&empty, "f6g7,g7h8"), Ok(expected_without_mid));

        let mut friendly_builder = PositionBuilder::new(Color::Black);
        friendly_builder
            .put(mid, PieceCode::new(Color::Black, PieceKind::Pawn))
            .unwrap();
        let friendly = friendly_builder.finish().unwrap();
        assert_eq!(parse(&friendly, "f6g7,g7h8"), Ok(expected_without_mid));

        let mut enemy_builder = PositionBuilder::new(Color::Black);
        enemy_builder
            .put(mid, PieceCode::new(Color::White, PieceKind::Pawn))
            .unwrap();
        let enemy = enemy_builder.finish().unwrap();
        assert_eq!(
            parse(&enemy, "f6g7,g7h8"),
            Ok(Move {
                mid: Some(mid),
                ..expected_without_mid
            })
        );
    }

    #[test]
    fn empty_intermediate_return_normalizes_to_canonical_jitto() {
        let position = Position::empty(Color::Black);
        let jitto = Move {
            from: sq(5, 5),
            mid: None,
            to: sq(5, 5),
            promote: false,
        };

        assert_eq!(parse(&position, "f6g7,g7f6"), Ok(jitto));
        assert_eq!(legs(jitto), ["@@@@"]);
    }

    #[test]
    fn discontinuous_and_excess_legs_are_rejected() {
        let position = Position::empty(Color::Black);

        assert_eq!(
            parse(&position, "e6d6,c6b6"),
            Err(CecpError::LegDiscontinuity)
        );
        assert_eq!(
            parse(&position, "e6d6,d6c6,c6b6"),
            Err(CecpError::InvalidLegCount { found: 3 })
        );
    }

    #[test]
    fn malformed_jitto_forms_are_rejected() {
        let position = Position::empty(Color::Black);

        assert_eq!(parse(&position, "e6e6"), Err(CecpError::InvalidJittoForm));
        assert_eq!(
            parse(&position, "@@@@"),
            Err(CecpError::MalformedJittoInput)
        );
    }

    #[test]
    fn invalid_square_names_are_rejected() {
        let position = Position::empty(Color::Black);

        for invalid in ["m1a1", "a0b1", "a13b1", "e01d1", "A1b1"] {
            assert!(matches!(
                parse(&position, invalid),
                Err(CecpError::InvalidSquare { .. })
            ));
        }
    }

    #[test]
    fn invalid_suffixes_are_rejected() {
        let position = Position::empty(Color::Black);

        for invalid in [
            "e6d6+,d6c6",
            "e6d6=,d6c6",
            "e6d6,d6c6++",
            "e6d6,d6c6=+",
            "e6d6?",
        ] {
            assert_eq!(parse(&position, invalid), Err(CecpError::InvalidSuffix));
        }
    }

    #[test]
    fn usi_and_cecp_coordinates_describe_the_same_move() {
        let position = Position::empty(Color::Black);
        let mv = Move {
            from: sq(5, 5),
            mid: None,
            to: sq(5, 8),
            promote: false,
        };

        assert_eq!(usi::text(&position, mv), "7g7d");
        assert_eq!(legs(mv), ["f6f9"]);
    }
}
