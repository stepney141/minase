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
    use crate::notation::usi;
    use crate::test_util::{position, position_from_codes, sq};

    fn generated(pos: &Position) -> Vec<Move> {
        let mut moves = Vec::new();
        MoveGenerator::standard().generate_moves(pos, &mut moves);
        moves
    }

    fn single_piece_position(color: Color, kind: PieceKind, from: Square) -> Position {
        position(color, &[(from, color, kind)])
    }

    fn single_promoted_piece_position(color: Color, kind: PieceKind, from: Square) -> Position {
        position_from_codes(
            color,
            &[(from, PieceCode::new_promoted(color, kind).unwrap())],
        )
    }

    fn canonical_jitto(from: Square) -> Move {
        Move {
            from,
            mid: None,
            to: from,
            promote: false,
        }
    }

    // D5-CECP-01: CECP升名は筋英字a〜l＋段数字1〜12で、段1が先手側最下段
    // （[CECP]第5章、[PL]「Move文字列表記2形式」）。10段盤の0始まり特例は12段の中将棋に
    // 適用しない（[HACHU]第6節）。
    #[test]
    fn cecp_squares_cover_all_144_squares_and_correspond_to_usi_names() {
        for square in Square::all() {
            // 筋英字 = 'a' + 内部file、段番号 = 内部rank + 1。
            let expected = format!("{}{}", char::from(b'a' + square.file()), square.rank() + 1);
            assert_eq!(square_to_text(square), expected);
            let (parsed, rest) = parse_square_prefix(&expected, 1, 1).unwrap();
            assert_eq!(parsed, square);
            assert!(rest.is_empty());
        }

        // 引き継ぎ資産: USI 7g7d ＝ CECP f6f9（同一Moveの両表記。[CECP]第5章「筋aの左右の
        // 向き」のHaChu内蔵FENとlishogi初期SFENの盤面文字列一致からの導出）。
        let empty = Position::empty(Color::Black);
        let mv = Move {
            from: sq(5, 5),
            mid: None,
            to: sq(5, 8),
            promote: false,
        };
        assert_eq!(usi::text_generated(&empty, mv), "7g7d");
        assert_eq!(legs(mv), ["f6f9"]);
        assert_eq!(parse(&empty, "f6f9"), Ok(mv));

        // 2桁段（a10〜l12）の解析。
        assert_eq!(
            parse(&empty, "a10a11"),
            Ok(Move {
                from: sq(0, 9),
                mid: None,
                to: sq(0, 10),
                promote: false,
            })
        );
        // 盤外の升名の拒否。
        for invalid in ["a0b1", "a13b1", "m1a1", "e01d1"] {
            assert!(
                matches!(parse(&empty, invalid), Err(CecpError::InvalidSquare { .. })),
                "{invalid}"
            );
        }
    }

    // D5-CECP-02: 受信はコンマ区切りの1レグまたは2レグで、第2レグは第1レグの終点から
    // 始まる（[PL]「フェーズ5の確定設計」、[CECP]第5章「複数レグ指し手」）。
    #[test]
    fn comma_separated_legs_parse_with_continuity() {
        // d6とg7に相手駒を置き、2段階捕獲と居喰いを実経路で受ける。
        let board = position(
            Color::Black,
            &[
                (sq(3, 5), Color::White, PieceKind::Pawn),
                (sq(6, 6), Color::White, PieceKind::Pawn),
            ],
        );

        // 1レグはmid: None相当。
        assert_eq!(
            parse(&board, "e6c6"),
            Ok(Move {
                from: sq(4, 5),
                mid: None,
                to: sq(2, 5),
                promote: false,
            })
        );
        // 2レグは第1レグ終点を経由する2段階移動。
        assert_eq!(
            parse(&board, "e6d6,d6c6"),
            Ok(Move {
                from: sq(4, 5),
                mid: Some(sq(3, 5)),
                to: sq(2, 5),
                promote: false,
            })
        );
        // 居喰いの2レグ表記（[HACHU]第7節の実例）。
        assert_eq!(
            parse(&board, "f6g7,g7f6"),
            Ok(Move {
                from: sq(5, 5),
                mid: Some(sq(6, 6)),
                to: sq(5, 5),
                promote: false,
            })
        );

        // レグ不連続の拒否。
        assert_eq!(parse(&board, "e6d6,c6b6"), Err(CecpError::LegDiscontinuity));
        // 3レグ以上の拒否。関数契約は1〜2レグの解析だけを定め、中将棋では2レグで足りる
        // （[CECP]第5章）。明文の拒否規定はなく実装契約である（SU-5）。
        assert_eq!(
            parse(&board, "e6d6,d6c6,c6b6"),
            Err(CecpError::InvalidLegCount { found: 3 })
        );
        // 空レグの拒否。
        for invalid in ["e6d6,", ",d6e6"] {
            assert!(parse(&board, invalid).is_err(), "{invalid}");
        }
    }

    // D5-CECP-03: 送信はmid無しが1行、mid有りが2行（非最終レグ末尾コンマ）、正準じっとが
    // `@@@@`の1行（[PL]「フェーズ5の確定設計」、[CECP]第5章、[HACHU]第8節。引き継ぎ資産）。
    #[test]
    fn legs_output_splits_two_stage_moves_and_writes_jitto_as_null_move() {
        assert_eq!(
            legs(Move {
                from: sq(5, 5),
                mid: None,
                to: sq(5, 8),
                promote: false,
            }),
            ["f6f9"]
        );
        // 2段階捕獲: 非最終レグだけが末尾コンマを持つ。
        assert_eq!(
            legs(Move {
                from: sq(4, 5),
                mid: Some(sq(3, 5)),
                to: sq(2, 5),
                promote: false,
            }),
            ["e6d6,", "d6c6"]
        );
        // 居喰い。
        assert_eq!(
            legs(Move {
                from: sq(4, 5),
                mid: Some(sq(3, 5)),
                to: sq(4, 5),
                promote: false,
            }),
            ["e6d6,", "d6e6"]
        );
        // 正準じっとは`@@@@`の1行であり、往復2レグ（e6f6,＋f6e6）にはならない
        // （HaChuは往復2レグ表記を生成も受理もしない。[HACHU]第8節）。
        assert_eq!(legs(canonical_jitto(sq(4, 5))), ["@@@@"]);
    }

    // D5-CECP-04: 成り接尾辞`+`は最終レグの末尾だけに認め、`=`は不成として受理する。
    // 送信では不成は接尾辞なしで、`=`を出力しない（[PL]「フェーズ5の確定設計」、
    // [CECP]第5章「将棋式の成りと不成」）。
    #[test]
    fn promotion_suffix_attaches_to_the_final_leg_only() {
        let empty = Position::empty(Color::Black);
        let plain = Move {
            from: sq(2, 3),
            mid: None,
            to: sq(2, 4),
            promote: false,
        };
        let promoted = Move {
            promote: true,
            ..plain
        };

        assert_eq!(parse(&empty, "c4c5+"), Ok(promoted));
        assert_eq!(parse(&empty, "c4c5="), Ok(plain));
        assert_eq!(parse(&empty, "c4c5"), Ok(plain));
        assert_eq!(legs(promoted), ["c4c5+"]);
        assert_eq!(legs(plain), ["c4c5"]);

        // 2レグでは`+`が最終レグの末尾にだけ付く。
        let two_leg = Move {
            from: sq(4, 5),
            mid: Some(sq(3, 5)),
            to: sq(2, 5),
            promote: true,
        };
        let board = position(Color::Black, &[(sq(3, 5), Color::White, PieceKind::Pawn)]);
        assert_eq!(legs(two_leg), ["e6d6,", "d6c6+"]);
        assert_eq!(parse(&board, "e6d6,d6c6+"), Ok(two_leg));

        // 非最終レグへの`+`と、HaChu式の寛容判定（改行・`=`以外をすべて成り扱い）は
        // 採用しない（[HACHU]第6節。`+`・`=`・無印以外の接尾辞は拒否する）。
        for invalid in ["e6d6+,d6c6", "e6d6=,d6c6", "e6d6,d6c6++", "e6d6?", "e6d6q"] {
            assert!(parse(&board, invalid).is_err(), "{invalid}");
        }

        // 送信文字列に`=`は現れない。
        for leg in legs(plain).into_iter().chain(legs(two_leg)) {
            assert!(!leg.contains('='));
        }
    }

    // D5-CECP-05: 表記層は`@@@@`を拒否する。合法手リストという局面文脈を要する解決は
    // CECPプロトコルモジュール（D6の対象）の分担である（[PL]「フェーズ5の確定設計」）。
    #[test]
    fn null_move_input_is_rejected_by_the_notation_layer() {
        let empty = Position::empty(Color::Black);
        assert_eq!(parse(&empty, "@@@@"), Err(CecpError::MalformedJittoInput));
        assert!(parse(&empty, "@@@@,f6e6").is_err());
        // じっとに単一レグの綴りはない（HaChuも`@@@@`で符号化する。[HACHU]第8節）。
        assert_eq!(parse(&empty, "e6e6"), Err(CecpError::InvalidJittoForm));
    }

    // D5-CECP-06: 中間升の扱いはUSI形式と同一契約であり、相手駒がなければmid: Noneへ
    // 正規化し、始点＝終点は正準じっとへ潰す（[PL]「フェーズ5の確定設計」）。
    #[test]
    fn intermediate_normalization_matches_the_usi_contract() {
        let from = sq(5, 5);
        let mid = sq(6, 6);
        let to = sq(7, 7);
        let direct = Move {
            from,
            mid: None,
            to,
            promote: false,
        };

        // 空升・自駒の経由升はmid: Noneへ正規化する。
        let empty = Position::empty(Color::Black);
        assert_eq!(parse(&empty, "f6g7,g7h8"), Ok(direct));
        let friendly = position(Color::Black, &[(mid, Color::Black, PieceKind::Pawn)]);
        assert_eq!(parse(&friendly, "f6g7,g7h8"), Ok(direct));
        // 相手駒の経由升はmid: Someのまま保持する。
        let enemy = position(Color::Black, &[(mid, Color::White, PieceKind::Pawn)]);
        assert_eq!(
            parse(&enemy, "f6g7,g7h8"),
            Ok(Move {
                mid: Some(mid),
                ..direct
            })
        );

        // 空升経由の往復2レグは正準じっとへ潰れる。HaChuが受理しない綴りをminaseは
        // 受理して正準化する（表記差の吸収。[HACHU]第8節・第10章）。
        assert_eq!(parse(&empty, "f6g7,g7f6"), Ok(canonical_jitto(from)));
        // 相手駒経由の往復2レグは居喰いのままで、正準じっとにならない。
        assert_eq!(
            parse(&enemy, "f6g7,g7f6"),
            Ok(Move {
                from,
                mid: Some(mid),
                to: from,
                promote: false,
            })
        );

        // USI形式と同じ実経路の入力は同じ正準Moveへ落ちる（D5-USI-05と同一の正規化規則）。
        assert_eq!(
            usi::parse(&enemy, "7g6f5e").unwrap(),
            parse(&enemy, "f6g7,g7h8").unwrap()
        );
        assert_eq!(
            usi::parse(&empty, "7g6f5e").unwrap(),
            parse(&empty, "f6g7,g7h8").unwrap()
        );
    }

    // D5-CECP-07, D5-PROP-01, D5-PROP-02: じっと以外の全合法手について、legsの出力を
    // そのまま連結した文字列をparseへ渡すとMove単位で一致する（[PL]「フェーズ5の確定
    // 設計」）。じっとは`@@@@`が移動元を運ばないため往復対象外とし、解決はD6が担う。
    #[test]
    fn all_legal_moves_round_trip_via_concatenated_legs() {
        let special = position_from_codes(
            Color::Black,
            &[
                (
                    sq(5, 5),
                    PieceCode::new(Color::Black, PieceKind::Lion).unwrap(),
                ),
                (
                    sq(1, 1),
                    PieceCode::new_promoted(Color::Black, PieceKind::HornedFalcon).unwrap(),
                ),
                (
                    sq(9, 1),
                    PieceCode::new_promoted(Color::Black, PieceKind::SoaringEagle).unwrap(),
                ),
                (
                    sq(5, 6),
                    PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
                ),
                (
                    sq(6, 6),
                    PieceCode::new(Color::White, PieceKind::SilverGeneral).unwrap(),
                ),
                (
                    sq(1, 2),
                    PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
                ),
                (
                    sq(8, 2),
                    PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
                ),
            ],
        );
        let promotion = position(Color::Black, &[(sq(4, 7), Color::Black, PieceKind::Pawn)]);
        let positions = [
            Position::initial(),
            special,
            single_piece_position(Color::Black, PieceKind::Lion, sq(5, 5)),
            single_promoted_piece_position(Color::Black, PieceKind::HornedFalcon, sq(5, 5)),
            single_promoted_piece_position(Color::White, PieceKind::HornedFalcon, sq(5, 5)),
            single_promoted_piece_position(Color::Black, PieceKind::SoaringEagle, sq(5, 5)),
            single_promoted_piece_position(Color::White, PieceKind::SoaringEagle, sq(5, 5)),
            promotion,
        ];

        for pos in &positions {
            let moves = generated(pos);
            assert!(!moves.is_empty());
            for mv in moves {
                let rendered = legs(mv);
                if mv.mid.is_none() && mv.to == mv.from {
                    assert_eq!(rendered, ["@@@@"]);
                } else {
                    // 非最終レグが末尾コンマを持つため、連結だけで受信形式になる。
                    let wire = rendered.concat();
                    assert_eq!(parse(pos, &wire), Ok(mv), "{wire}");
                }
            }
        }

        // カバレッジ空振り防止（D5-PROP-02）。居喰い（to == from かつ mid: Some）が
        // じっとではなく往復対象に含まれることを、実在の断定で保証する。
        let special_moves = generated(&positions[1]);
        for origin in [sq(5, 5), sq(1, 1), sq(9, 1)] {
            assert!(
                special_moves
                    .iter()
                    .any(|mv| mv.from == origin && mv.mid.is_some() && mv.to != origin)
            );
            assert!(
                special_moves
                    .iter()
                    .any(|mv| mv.from == origin && mv.mid.is_some() && mv.to == origin)
            );
        }
        for pos in &positions[2..=6] {
            assert!(
                generated(pos)
                    .iter()
                    .any(|mv| mv.mid.is_none() && mv.to == mv.from && !mv.promote)
            );
        }
        assert!(generated(&positions[7]).iter().any(|mv| mv.promote));
        assert!(generated(&positions[7]).iter().any(|mv| !mv.promote));
    }
}
