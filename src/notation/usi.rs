//! lishogi系拡張USIの指し手表記。

use core::fmt;

use crate::core::direction::Direction;
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceKind};
use crate::core::position::Position;
use crate::core::square::Square;

use super::sfen::{parse_square, square_to_text};

/// lishogi系拡張USI指し手の解析エラー。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UsiError {
    /// 指定欄が盤上の升名ではない。
    InvalidSquare {
        /// 問題の1起算の升名欄位置。
        field: usize,
    },
    /// 升名欄が2個または3個ではない。
    InvalidFieldCount {
        /// 入力にあった升名欄の数。
        found: usize,
    },
    /// 末尾が成りまたは不成の接尾辞ではない。
    InvalidSuffix,
    /// 移動元と移動先が等しい着手が2升連結で表されている。
    InvalidJittoForm,
}

impl fmt::Display for UsiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSquare { field } => {
                write!(formatter, "invalid USI square in field {field}")
            }
            Self::InvalidFieldCount { found } => write!(
                formatter,
                "USI move has {found} square fields; expected 2 or 3"
            ),
            Self::InvalidSuffix => formatter.write_str("invalid USI move suffix"),
            Self::InvalidJittoForm => {
                formatter.write_str("USI jitto must contain an intermediate square")
            }
        }
    }
}

impl std::error::Error for UsiError {}

/// 指し手をlishogi系拡張USI形式へ変換する。
///
/// 正準じっとは中間升を保持しないため、局面上で第1段階として使える
/// 空升のうち内部密番号が最小の升を補う(第11条・第12条)。
/// 正準じっとは、局面の合法手として生成された値であることを前提とする。
pub fn text(position: &Position, mv: Move) -> String {
    let mid = if mv.mid.is_none() && mv.to == mv.from {
        Some(jitto_intermediate(position, mv.from))
    } else {
        mv.mid
    };

    let mut output = square_to_text(mv.from);
    if let Some(mid) = mid {
        output.push_str(&square_to_text(mid));
    }
    output.push_str(&square_to_text(mv.to));
    if mv.promote {
        output.push('+');
    }
    output
}

/// lishogi系拡張USI形式を指し手へ変換する。
///
/// 3升連結で中間升に相手駒がなければ、終点にかかわらず`mid: None`へ
/// 正規化する。移動元と移動先が等しい着手は正準じっと、異なる着手は
/// 直接の二段移動になる(第11条・第12条)。
/// 着手そのものの合法性は検査しない。
pub fn parse(position: &Position, input: &str) -> Result<Move, UsiError> {
    let (body, promote) = split_suffix(input)?;
    let squares = parse_squares(body)?;
    if !matches!(squares.len(), 2 | 3) {
        return Err(UsiError::InvalidFieldCount {
            found: squares.len(),
        });
    }

    let from = squares[0];
    let (mid, to) = if squares.len() == 2 {
        (None, squares[1])
    } else {
        (Some(squares[1]), squares[2])
    };
    if mid.is_none() && to == from {
        return Err(UsiError::InvalidJittoForm);
    }
    let mid = mid.filter(|&square| {
        position
            .piece_at(square)
            .is_some_and(|piece| piece.color() == Some(position.side_to_move().opposite()))
    });

    Ok(Move {
        from,
        mid,
        to,
        promote,
    })
}

/// 末尾の接尾辞を切り離し、升名部分と成りの選択を返す。
///
/// `+`は成り、`=`と`?`は不成を表す。接尾辞が末尾以外に現れる入力は拒否する。
fn split_suffix(input: &str) -> Result<(&str, bool), UsiError> {
    let Some(last) = input.as_bytes().last().copied() else {
        return Ok((input, false));
    };
    match last {
        b'+' => Ok((&input[..input.len() - 1], true)),
        b'=' | b'?' => Ok((&input[..input.len() - 1], false)),
        _ => {
            if input.bytes().any(|byte| matches!(byte, b'+' | b'=' | b'?')) {
                Err(UsiError::InvalidSuffix)
            } else {
                Ok((input, false))
            }
        }
    }
}

/// 区切りなしで連結された升名列を先頭から順に解析する。
///
/// 各升名は筋の数字1〜2桁と段の英字1文字からなる。
fn parse_squares(mut input: &str) -> Result<Vec<Square>, UsiError> {
    let mut squares = Vec::new();
    while !input.is_empty() {
        if !input.as_bytes()[0].is_ascii_digit() {
            return if squares.len() >= 2 {
                Err(UsiError::InvalidSuffix)
            } else {
                Err(UsiError::InvalidSquare {
                    field: squares.len() + 1,
                })
            };
        }

        let rank_index = input
            .bytes()
            .take(3)
            .position(|byte| !byte.is_ascii_digit())
            .ok_or(UsiError::InvalidSquare {
                field: squares.len() + 1,
            })?;
        if !(1..=2).contains(&rank_index) || input.len() <= rank_index {
            return Err(UsiError::InvalidSquare {
                field: squares.len() + 1,
            });
        }
        let end = rank_index + 1;
        let square = parse_square(&input[..end]).ok_or(UsiError::InvalidSquare {
            field: squares.len() + 1,
        })?;
        squares.push(square);
        input = &input[end..];
    }
    Ok(squares)
}

/// 正準じっとの表記へ補う中間升を返す。
///
/// 駒種と手番に応じた第1段階の方向のうち、空升で内部密番号が最小の升を選ぶ。
///
/// # Panics
///
/// 移動元にじっとが可能な駒(獅子・角鷹・飛鷲)がない場合、または第1段階に
/// 使える空升がない場合にパニックする。
fn jitto_intermediate(position: &Position, from: Square) -> Square {
    let piece = position
        .piece_at(from)
        .expect("a canonical jitto origin must contain a piece");
    let kind = piece.kind().expect("a board piece must have a piece kind");
    let color = piece.color().expect("a board piece must have a color");

    let directions: &[Direction] = match (kind, color) {
        (PieceKind::Lion, _) => &Direction::ALL,
        (PieceKind::HornedFalcon, Color::Black) => &[Direction::North],
        (PieceKind::HornedFalcon, Color::White) => &[Direction::South],
        (PieceKind::SoaringEagle, Color::Black) => &[Direction::NorthEast, Direction::NorthWest],
        (PieceKind::SoaringEagle, Color::White) => &[Direction::SouthEast, Direction::SouthWest],
        _ => panic!("a canonical jitto must move a lion, horned falcon, or soaring eagle"),
    };

    directions
        .iter()
        .filter_map(|direction| from.offset(direction.file_delta(), direction.rank_delta()))
        .filter(|&square| position.piece_at(square).is_none())
        .min_by_key(|square| square.dense_index())
        .expect("a canonical jitto must have an empty first-step square")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::movegen::MoveGenerator;
    use crate::test_util::{position, sq};

    fn generated(pos: &Position) -> Vec<Move> {
        let mut moves = Vec::new();
        MoveGenerator::standard().generate_moves(pos, &mut moves);
        moves
    }

    fn single_piece_position(color: Color, kind: PieceKind, from: Square) -> Position {
        position(color, &[(from, color, kind)])
    }

    fn canonical_jitto(from: Square) -> Move {
        Move {
            from,
            mid: None,
            to: from,
            promote: false,
        }
    }

    // D5-USI-01: 指し手は升名の無区切り連結で、駒打ち形式は存在しない
    // （[USI]「指し手の文字列表記」、[PL]「Move文字列表記2形式」）。
    #[test]
    fn move_strings_concatenate_square_names_without_separators() {
        let empty = Position::empty(Color::Black);

        // 2桁筋を含む指し手の正しい切り出し。
        assert_eq!(
            parse(&empty, "12a12b"),
            Ok(Move {
                from: sq(0, 11),
                mid: None,
                to: sq(0, 10),
                promote: false,
            })
        );
        assert_eq!(
            parse(&empty, "1a12b"),
            Ok(Move {
                from: sq(11, 11),
                mid: None,
                to: sq(0, 10),
                promote: false,
            })
        );

        // 4升以上の連結、駒打ち形式、区切り文字、盤外の段・筋の拒否（D5-USI-01境界）。
        for invalid in [
            "7g7d7e7f", "P*3d", "7g 7d", "7g-7d", "7m7d", "13a1a", "0a1a",
        ] {
            assert!(parse(&empty, invalid).is_err(), "{invalid}");
        }
    }

    // D5-USI-02: 1歩停止も2升跳び直行も2升連結で書き、mid: Noneの移動に対応する
    // （[USI]「指し手の文字列表記」第1項・第3項、[CANON]「新しいMove型」）。
    #[test]
    fn two_square_form_maps_to_mid_none_and_rejects_zero_distance() {
        let empty = Position::empty(Color::Black);
        let step = Move {
            from: sq(6, 6),
            mid: None,
            to: sq(7, 7),
            promote: false,
        };
        let jump = Move {
            from: sq(6, 6),
            mid: None,
            to: sq(5, 8),
            promote: false,
        };

        assert_eq!(parse(&empty, "6f5e"), Ok(step));
        assert_eq!(parse(&empty, "6f7d"), Ok(jump));
        assert_eq!(text(&empty, step), "6f5e");
        assert_eq!(text(&empty, jump), "6f7d");

        // 距離0の2升連結は不合法であり、じっとは必ず3升連結で表す（[USI]同節第4項）。
        assert_eq!(parse(&empty, "6f6f"), Err(UsiError::InvalidJittoForm));
    }

    // D5-USI-03: 3升連結の具体例（二段捕獲6f5e6d・居喰い6i5h6i・じっと6f6g6f）が
    // 実際の合法手として存在する（[USI]「指し手の文字列表記」第3項、[RULES]第3条・第12条）。
    #[test]
    fn lishogi_documented_three_square_examples_are_legal_moves() {
        // 6fの獅子が5eの相手駒を取って6dへ達する二段捕獲。
        let capture_position = position(
            Color::Black,
            &[
                (sq(6, 6), Color::Black, PieceKind::Lion),
                (sq(7, 7), Color::White, PieceKind::Pawn),
            ],
        );
        let capture = parse(&capture_position, "6f5e6d").unwrap();
        assert_eq!(
            capture,
            Move {
                from: sq(6, 6),
                mid: Some(sq(7, 7)),
                to: sq(6, 8),
                promote: false,
            }
        );
        assert!(generated(&capture_position).contains(&capture));
        assert_eq!(text(&capture_position, capture), "6f5e6d");

        // 5hの相手駒を取って6iへ戻る居喰い（[RULES]第3条第14項・第12条第8項）。
        let igui_position = position(
            Color::Black,
            &[
                (sq(6, 3), Color::Black, PieceKind::Lion),
                (sq(7, 4), Color::White, PieceKind::Pawn),
            ],
        );
        let igui = parse(&igui_position, "6i5h6i").unwrap();
        assert_eq!(
            igui,
            Move {
                from: sq(6, 3),
                mid: Some(sq(7, 4)),
                to: sq(6, 3),
                promote: false,
            }
        );
        assert!(generated(&igui_position).contains(&igui));
        assert_eq!(text(&igui_position, igui), "6i5h6i");

        // 空の中間升6gを経て元の升へ戻るじっと（[RULES]第3条第13項・第12条第9項）。
        let jitto_position = single_piece_position(Color::Black, PieceKind::Lion, sq(6, 6));
        let jitto = parse(&jitto_position, "6f6g6f").unwrap();
        assert_eq!(jitto, canonical_jitto(sq(6, 6)));
        assert!(generated(&jitto_position).contains(&jitto));
    }

    // D5-USI-04: 出力の成り記号は`+`だけであり、解析は`=`と`?`を不成として受理する
    // （[USI]「指し手の文字列表記」第1項・第2項、[PL]「Move文字列表記2形式」）。
    // `?`の用途の一次資料は未確認であり（SU-1）、shogiops互換の実装判断としての受理である。
    #[test]
    fn promotion_suffix_writes_plus_and_reads_equals_and_question_as_non_promotion() {
        let empty = Position::empty(Color::Black);
        let plain = Move {
            from: sq(5, 5),
            mid: None,
            to: sq(5, 8),
            promote: false,
        };
        let promoted = Move {
            promote: true,
            ..plain
        };

        assert_eq!(parse(&empty, "7g7d"), Ok(plain));
        assert_eq!(parse(&empty, "7g7d="), Ok(plain));
        assert_eq!(parse(&empty, "7g7d?"), Ok(plain));
        assert_eq!(parse(&empty, "7g7d+"), Ok(promoted));
        assert_eq!(text(&empty, plain), "7g7d");
        assert_eq!(text(&empty, promoted), "7g7d+");

        // 記号の重複の拒否。
        for invalid in ["7g7d++", "7g7d+=", "7g7d=?"] {
            assert_eq!(
                parse(&empty, invalid),
                Err(UsiError::InvalidSuffix),
                "{invalid}"
            );
        }

        // 成り選択のある局面（[RULES]第18条第5項）でも出力に`=`と`?`は現れない。
        let promotion_position =
            position(Color::Black, &[(sq(4, 7), Color::Black, PieceKind::Pawn)]);
        let moves = generated(&promotion_position);
        assert!(moves.iter().any(|mv| mv.promote));
        assert!(moves.iter().any(|mv| !mv.promote));
        for mv in moves {
            let rendered = text(&promotion_position, mv);
            assert!(
                !rendered.contains('=') && !rendered.contains('?'),
                "{rendered}"
            );
        }
    }

    // D5-USI-05: 3升連結は中間升に相手駒がなければ終点にかかわらずmid: Noneへ正規化する
    // （[PL]「Move文字列表記2形式」の2026-08-10契約一般化。lishogi実対局の冗長経路
    // SNjoPiHz第15手7j7i6h・2u7dwJf9第176手6c7c8cを受けた互換性契約）。
    #[test]
    fn three_square_input_normalizes_by_intermediate_occupancy() {
        let from = sq(6, 6);
        let mid = sq(7, 7);
        let to = sq(6, 8);
        let direct = Move {
            from,
            mid: None,
            to,
            promote: false,
        };

        // 中間升に相手駒がある: mid: Someのまま保持する。
        let enemy = position(
            Color::Black,
            &[
                (from, Color::Black, PieceKind::Lion),
                (mid, Color::White, PieceKind::Pawn),
            ],
        );
        assert_eq!(
            parse(&enemy, "6f5e6d"),
            Ok(Move {
                mid: Some(mid),
                ..direct
            })
        );

        // 空の中間升: 直接の二段移動へ正規化する。
        let alone = single_piece_position(Color::Black, PieceKind::Lion, from);
        assert_eq!(parse(&alone, "6f5e6d"), Ok(direct));

        // 自駒の中間升: 「相手駒がなければ正規化」に含まれ、直接移動として合法手照合に委ねる。
        let friendly = position(
            Color::Black,
            &[
                (from, Color::Black, PieceKind::Lion),
                (mid, Color::Black, PieceKind::Pawn),
            ],
        );
        assert_eq!(parse(&friendly, "6f5e6d"), Ok(direct));

        // 空升経由の往復は正準じっとへ、相手駒経由の往復は居喰い（mid: Some）のまま。
        assert_eq!(parse(&alone, "6f5e6f"), Ok(canonical_jitto(from)));
        assert_eq!(
            parse(&enemy, "6f5e6f"),
            Ok(Move {
                from,
                mid: Some(mid),
                to: from,
                promote: false,
            })
        );

        // 正規化の冪等性: 正準形の文字列化を再解析しても同じMoveになる（D5-USI-08の基礎）。
        let rendered = text(&alone, direct);
        assert_eq!(rendered, "6f6d");
        assert_eq!(parse(&alone, &rendered), Ok(direct));
    }

    // D5-USI-06: 正準じっとの文字列化は、第1段階として合法な空の隣接升を中間升に補った
    // 3升連結になる（[PL]「Move文字列表記2形式」。方向制約は[RULES]第11条・第12条）。
    // 代表升の選択キー「内部密番号」は文書定義が未補修のため（SU-2）、候補集合への所属・
    // 空升であること・選択の決定性だけを断定し、複数候補時の直値は固定しない。
    #[test]
    fn jitto_text_supplies_a_deterministic_legal_first_step_square() {
        // 獅子: 8方向の空隣接升のいずれかを代表升とし、同一局面では常に同じ文字列になる。
        let lion_from = sq(5, 5);
        let lion_position = single_piece_position(Color::Black, PieceKind::Lion, lion_from);
        let lion_rendered = text(&lion_position, canonical_jitto(lion_from));
        assert_eq!(
            lion_rendered,
            text(&lion_position, canonical_jitto(lion_from))
        );
        assert!(lion_rendered.starts_with("7g") && lion_rendered.ends_with("7g"));
        let lion_mid = parse_square(&lion_rendered[2..lion_rendered.len() - 2]).unwrap();
        assert!(lion_position.piece_at(lion_mid).is_none());
        let file_delta = i16::from(lion_mid.file()) - i16::from(lion_from.file());
        let rank_delta = i16::from(lion_mid.rank()) - i16::from(lion_from.rank());
        assert!(file_delta.abs() <= 1 && rank_delta.abs() <= 1);
        assert_ne!((file_delta, rank_delta), (0, 0));

        // 隣接升が1つを残して塞がれた獅子は、残る唯一の空升を代表升にする（候補一意）。
        let corner_position = position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::Lion),
                (sq(1, 0), Color::Black, PieceKind::Pawn),
                (sq(1, 1), Color::Black, PieceKind::Pawn),
            ],
        );
        assert_eq!(
            text(&corner_position, canonical_jitto(sq(0, 0))),
            "12l12k12l"
        );

        // 角鷹の第1段階は前方1方向だけ（[RULES]第11条第1項・第7項）。先後で向きが変わる。
        for (color, forward) in [(Color::Black, sq(5, 6)), (Color::White, sq(5, 4))] {
            let falcon = single_piece_position(color, PieceKind::HornedFalcon, sq(5, 5));
            let expected = format!(
                "{}{}{}",
                square_to_text(sq(5, 5)),
                square_to_text(forward),
                square_to_text(sq(5, 5))
            );
            assert_eq!(text(&falcon, canonical_jitto(sq(5, 5))), expected);
        }

        // 飛鷲は前斜め2方向のうち片方だけが空なら、その方向が代表升になる
        // （[RULES]第11条第2項・第8項。候補一意のため直値を断定できる）。
        let eagle_from = sq(5, 5);
        let blocked = position(
            Color::Black,
            &[
                (eagle_from, Color::Black, PieceKind::SoaringEagle),
                (sq(4, 6), Color::Black, PieceKind::Pawn),
            ],
        );
        let expected = format!(
            "{}{}{}",
            square_to_text(eagle_from),
            square_to_text(sq(6, 6)),
            square_to_text(eagle_from)
        );
        assert_eq!(text(&blocked, canonical_jitto(eagle_from)), expected);

        // 両方が空の場合も、代表升は前斜め2方向のいずれかで決定的である。
        let open = single_piece_position(Color::Black, PieceKind::SoaringEagle, eagle_from);
        let eagle_rendered = text(&open, canonical_jitto(eagle_from));
        assert_eq!(eagle_rendered, text(&open, canonical_jitto(eagle_from)));
        let eagle_mid = parse_square(&eagle_rendered[2..eagle_rendered.len() - 2]).unwrap();
        assert!(eagle_mid == sq(4, 6) || eagle_mid == sq(6, 6));
    }

    // D5-USI-07: 不正入力をパニックせず拒否する。USI表記に`@@@@`は存在しない
    // （[USI]「指し手の文字列表記」第4項・第5項。CECPとの差は[HACHU]第8節）。
    #[test]
    fn invalid_inputs_are_rejected_without_panicking() {
        let empty = Position::empty(Color::Black);
        for invalid in [
            "", "6f6f", "P*3d", "13a1a", "0a1a", "7m1a", "resign", "@@@@", "7g7d7e7f", "7g7d#",
        ] {
            assert!(parse(&empty, invalid).is_err(), "{invalid:?}");
        }
    }

    // D5-USI-08, D5-PROP-01, D5-PROP-02: 代表局面群の全合法手についてMove単位の往復一致
    // （じっと以外は文字列単位でも）を検証し、合法手集合が各類型を実際に含むことを断定する
    // （[PL]「Move文字列表記2形式」と「検証」）。
    #[test]
    fn all_legal_moves_round_trip_in_representative_positions() {
        // (b)(c) 獅子・角鷹・飛鷲の2段階移動・居喰いが合法な局面。
        let special = position(
            Color::Black,
            &[
                (sq(5, 5), Color::Black, PieceKind::Lion),
                (sq(1, 1), Color::Black, PieceKind::HornedFalcon),
                (sq(9, 1), Color::Black, PieceKind::SoaringEagle),
                (sq(5, 6), Color::White, PieceKind::Pawn),
                (sq(6, 6), Color::White, PieceKind::SilverGeneral),
                (sq(1, 2), Color::White, PieceKind::Pawn),
                (sq(8, 2), Color::White, PieceKind::Pawn),
            ],
        );
        // (d) 成りと不成が選択できる局面（[RULES]第18条）。
        let promotion = position(Color::Black, &[(sq(4, 7), Color::Black, PieceKind::Pawn)]);
        // (a) 初期局面と、じっとを含む単駒局面群。
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

        for pos in &positions {
            let moves = generated(pos);
            assert!(!moves.is_empty());
            for mv in moves {
                let rendered = text(pos, mv);
                // Move単位の往復一致（parse(position, text(position, m)) == m）。
                assert_eq!(parse(pos, &rendered), Ok(mv), "{rendered}");
                // じっと以外は文字列単位でも往復一致する。
                if !(mv.mid.is_none() && mv.to == mv.from) {
                    assert_eq!(text(pos, parse(pos, &rendered).unwrap()), rendered);
                }
                // 2段階移動を行う駒は成れないため、2段階の合法手に成りは現れない
                // （[USI]同節第6項、[RULES]第11条第10項。SU-4）。
                assert!(!(mv.promote && (mv.mid.is_some() || mv.to == mv.from)));
            }
        }

        // カバレッジ空振り防止（D5-PROP-02）: 類型(1)(2)を獅子・角鷹・飛鷲の各出自で確認する。
        let special_moves = generated(&positions[1]);
        for origin in [sq(5, 5), sq(1, 1), sq(9, 1)] {
            // (1) 経路捕獲つき2段階移動（mid: Some かつ to != from）。
            assert!(
                special_moves
                    .iter()
                    .any(|mv| mv.from == origin && mv.mid.is_some() && mv.to != origin)
            );
            // (2) 居喰い（mid: Some かつ to == from）。
            assert!(
                special_moves
                    .iter()
                    .any(|mv| mv.from == origin && mv.mid.is_some() && mv.to == origin)
            );
        }
        // (3) 正準じっと（mid: None かつ to == from）を3駒種×先後で確認する。
        for pos in &positions[2..=6] {
            assert!(
                generated(pos)
                    .iter()
                    .any(|mv| mv.mid.is_none() && mv.to == mv.from && !mv.promote)
            );
        }
        // (4) 成りと(5) 同一from/toの不成。
        let promotion_moves = generated(&positions[7]);
        let promote_move = promotion_moves
            .iter()
            .copied()
            .find(|mv| mv.promote)
            .unwrap();
        assert!(
            promotion_moves.iter().any(|mv| {
                !mv.promote && mv.from == promote_move.from && mv.to == promote_move.to
            })
        );

        // (e) 冗長経路入力: 空升経由の3升連結の解析結果を文字列化すると正準の2升連結になる
        // （文字列単位では非対称。[PL]「Move文字列表記2形式」）。
        let alone = &positions[2];
        let normalized = parse(alone, "7g7f7e").unwrap();
        assert_eq!(normalized.mid, None);
        assert_eq!(text(alone, normalized), "7g7e");
    }
}
