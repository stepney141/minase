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
    InvalidSquare { field: usize },
    /// 升名欄が2個または3個ではない。
    InvalidFieldCount { found: usize },
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
    use crate::core::piece::PieceCode;
    use crate::core::position::PositionBuilder;
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
            let rendered = text(position, mv);
            assert_eq!(parse(position, &rendered), Ok(mv), "text={rendered}");
            if !(mv.mid.is_none() && mv.to == mv.from) {
                let reparsed = parse(position, &rendered).unwrap();
                assert_eq!(text(position, reparsed), rendered);
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
                    .any(|mv| { mv.from == origin && mv.mid.is_some() && mv.to != origin })
            );
            assert!(
                special_moves
                    .iter()
                    .any(|mv| { mv.from == origin && mv.to == origin && mv.mid.is_some() })
            );
        }
        for position in &positions[2..=6] {
            assert!(
                generated(position)
                    .iter()
                    .any(|mv| { mv.mid.is_none() && mv.to == mv.from && !mv.promote })
            );
        }
        assert!(generated(&positions[7]).iter().any(|mv| mv.promote));
    }

    #[test]
    fn lion_jitto_uses_the_empty_adjacent_square_with_the_smallest_dense_index() {
        let from = sq(6, 6);
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(from, PieceCode::new(Color::Black, PieceKind::Lion))
            .unwrap();
        builder
            .put(sq(5, 5), PieceCode::new(Color::Black, PieceKind::Pawn))
            .unwrap();
        let position = builder.finish().unwrap();
        let mv = Move {
            from,
            mid: None,
            to: from,
            promote: false,
        };

        assert_eq!(text(&position, mv), "6f6g6f");
    }

    #[test]
    fn horned_falcon_jitto_uses_the_forward_square_for_each_side() {
        for (color, expected) in [(Color::Black, "7g7f7g"), (Color::White, "7g7h7g")] {
            let from = sq(5, 5);
            let position = single_piece_position(color, PieceKind::HornedFalcon, from);
            let mv = Move {
                from,
                mid: None,
                to: from,
                promote: false,
            };

            assert_eq!(text(&position, mv), expected);
        }
    }

    #[test]
    fn soaring_eagle_jitto_uses_the_forward_diagonal_with_the_smallest_dense_index() {
        for (color, expected) in [(Color::Black, "7g8f7g"), (Color::White, "7g8h7g")] {
            let from = sq(5, 5);
            let position = single_piece_position(color, PieceKind::SoaringEagle, from);
            let mv = Move {
                from,
                mid: None,
                to: from,
                promote: false,
            };

            assert_eq!(text(&position, mv), expected);
        }
    }

    #[test]
    fn suffixes_parse_as_promotion_or_non_promotion() {
        let position = Position::empty(Color::Black);
        let base = Move {
            from: sq(6, 6),
            mid: None,
            to: sq(7, 7),
            promote: false,
        };

        assert_eq!(parse(&position, "6f5e="), Ok(base));
        assert_eq!(parse(&position, "6f5e?"), Ok(base));
        assert_eq!(
            parse(&position, "6f5e+"),
            Ok(Move {
                promote: true,
                ..base
            })
        );
        assert_eq!(text(&position, base), "6f5e");
        assert_eq!(
            text(
                &position,
                Move {
                    promote: true,
                    ..base
                }
            ),
            "6f5e+"
        );
    }

    #[test]
    fn invalid_squares_field_counts_and_suffixes_are_rejected() {
        let position = Position::empty(Color::Black);

        for invalid in ["0a1a", "13a1a", "1m1a", "１a1a"] {
            assert!(matches!(
                parse(&position, invalid),
                Err(UsiError::InvalidSquare { .. })
            ));
        }
        assert_eq!(
            parse(&position, "6f"),
            Err(UsiError::InvalidFieldCount { found: 1 })
        );
        assert_eq!(
            parse(&position, "6f5e6d4c"),
            Err(UsiError::InvalidFieldCount { found: 4 })
        );
        for invalid in ["6f5e!", "6f5e++", "6f5e+x"] {
            assert_eq!(parse(&position, invalid), Err(UsiError::InvalidSuffix));
        }
    }

    #[test]
    fn lishogi_documented_examples_match_the_notation() {
        let lion_from = sq(6, 6);
        let empty = Position::empty(Color::Black);
        assert_eq!(
            text(
                &empty,
                Move {
                    from: lion_from,
                    mid: Some(sq(7, 7)),
                    to: sq(6, 8),
                    promote: false,
                }
            ),
            "6f5e6d"
        );
        assert_eq!(
            text(
                &empty,
                Move {
                    from: lion_from,
                    mid: None,
                    to: sq(7, 7),
                    promote: false,
                }
            ),
            "6f5e"
        );
        assert_eq!(
            text(
                &empty,
                Move {
                    from: lion_from,
                    mid: None,
                    to: sq(5, 8),
                    promote: false,
                }
            ),
            "6f7d"
        );

        let igui_from = sq(6, 3);
        let igui_mid = sq(7, 4);
        let mut igui_builder = PositionBuilder::new(Color::Black);
        igui_builder
            .put(igui_from, PieceCode::new(Color::Black, PieceKind::Lion))
            .unwrap();
        igui_builder
            .put(igui_mid, PieceCode::new(Color::White, PieceKind::Pawn))
            .unwrap();
        let igui_position = igui_builder.finish().unwrap();
        let igui = Move {
            from: igui_from,
            mid: Some(igui_mid),
            to: igui_from,
            promote: false,
        };
        assert_eq!(text(&igui_position, igui), "6i5h6i");
        assert_eq!(parse(&igui_position, "6i5h6i"), Ok(igui));

        let mut jitto_builder = PositionBuilder::new(Color::Black);
        jitto_builder
            .put(lion_from, PieceCode::new(Color::Black, PieceKind::Lion))
            .unwrap();
        jitto_builder
            .put(sq(5, 5), PieceCode::new(Color::Black, PieceKind::Pawn))
            .unwrap();
        let jitto_position = jitto_builder.finish().unwrap();
        let jitto = Move {
            from: lion_from,
            mid: None,
            to: lion_from,
            promote: false,
        };
        assert_eq!(text(&jitto_position, jitto), "6f6g6f");
        assert_eq!(parse(&jitto_position, "6f6g6f"), Ok(jitto));
        assert_eq!(
            parse(&jitto_position, "6f6g6f+"),
            Ok(Move {
                promote: true,
                ..jitto
            })
        );
        assert_eq!(
            parse(&jitto_position, "6f6f"),
            Err(UsiError::InvalidJittoForm)
        );

        let mut occupied_mid_builder = PositionBuilder::new(Color::Black);
        occupied_mid_builder
            .put(lion_from, PieceCode::new(Color::Black, PieceKind::Lion))
            .unwrap();
        occupied_mid_builder
            .put(sq(6, 5), PieceCode::new(Color::Black, PieceKind::Pawn))
            .unwrap();
        let occupied_mid = occupied_mid_builder.finish().unwrap();
        assert_eq!(parse(&occupied_mid, "6f6g6f"), Ok(jitto));
    }

    #[test]
    fn three_square_direct_move_with_empty_intermediate_normalizes_to_no_mid() {
        let position = single_piece_position(Color::Black, PieceKind::Lion, sq(6, 6));

        assert_eq!(
            parse(&position, "6f5e6d"),
            Ok(Move {
                from: sq(6, 6),
                mid: None,
                to: sq(6, 8),
                promote: false,
            })
        );
    }

    #[test]
    fn three_square_move_capturing_enemy_on_intermediate_keeps_mid() {
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(sq(6, 6), PieceCode::new(Color::Black, PieceKind::Lion))
            .unwrap();
        builder
            .put(sq(7, 7), PieceCode::new(Color::White, PieceKind::Pawn))
            .unwrap();
        let position = builder.finish().unwrap();

        assert_eq!(
            parse(&position, "6f5e6d"),
            Ok(Move {
                from: sq(6, 6),
                mid: Some(sq(7, 7)),
                to: sq(6, 8),
                promote: false,
            })
        );
    }

    #[test]
    fn three_square_direct_move_with_friendly_intermediate_normalizes_to_no_mid() {
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(sq(6, 6), PieceCode::new(Color::Black, PieceKind::Lion))
            .unwrap();
        builder
            .put(sq(7, 7), PieceCode::new(Color::Black, PieceKind::Pawn))
            .unwrap();
        let position = builder.finish().unwrap();

        assert_eq!(
            parse(&position, "6f5e6d"),
            Ok(Move {
                from: sq(6, 6),
                mid: None,
                to: sq(6, 8),
                promote: false,
            })
        );
    }
}
