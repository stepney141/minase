//! 歩兵・仲人の一括選別が移動元順と規則判定を保つことを、変更前の生成と照合する。

use super::{MoveGenerator, generated_captures_with, mv, rule_variants};
use crate::board::{Bitboard, Square};
use crate::movegen::control::capture_targets_without_special;
use crate::movegen::{ORDINARY_KINDS, OrdinaryCapturer};
use crate::piece::{Color, PieceCode, PieceKind};
use crate::position::{Position, PositionBuilder};
use crate::rules::{MoveRules, Rules, parse_rule_set};
use crate::test_util::{bench_positions, position, sampled_random_positions, sq};

/// 一括選別前の駒別捕獲収集。生成順と捕獲升を含む契約の参照実装とする。
fn collect_before(
    generator: &MoveGenerator,
    board: &Position,
    allowed: Bitboard,
) -> Vec<OrdinaryCapturer> {
    let mut output = Vec::new();
    let color = board.side_to_move();
    for kind in ORDINARY_KINDS {
        for from in board.pieces_of_kind(color, kind) {
            let captures = capture_targets_without_special(
                generator.tables(),
                board.occupied(),
                color,
                kind,
                from,
                allowed,
            );
            if !captures.is_empty() {
                output.push(OrdinaryCapturer {
                    from,
                    kind,
                    piece: board.piece_at(from).unwrap(),
                    captures,
                });
            }
        }
    }
    output
}

fn assert_matches_before(generator: &MoveGenerator, board: &Position) {
    let mut expected = Vec::new();
    super::properties::generation_before::<true, false>(
        generator,
        board,
        &mut Vec::new(),
        &mut expected,
    );
    assert_eq!(generated_captures_with(generator, board), expected);
    expected.clear();
    super::properties::generation_before::<false, true>(
        generator,
        board,
        &mut Vec::new(),
        &mut expected,
    );
    let mut actual = Vec::new();
    generator.generate_quiets(board, &mut Vec::new(), &mut actual);
    assert_eq!(actual, expected);

    let enemy = board.pieces_of(board.side_to_move().opposite());
    for allowed in [
        enemy,
        Bitboard::from_squares(enemy.iter().step_by(2)),
        Bitboard::EMPTY,
    ] {
        let expected = collect_before(generator, board, allowed);
        let mut actual = Vec::new();
        let targets = generator.collect_ordinary_capturers(board, allowed, &mut actual);
        let fields = |c: &OrdinaryCapturer| (c.from, c.kind, c.piece, c.captures);
        assert_eq!(
            actual.iter().map(fields).collect::<Vec<_>>(),
            expected.iter().map(fields).collect::<Vec<_>>()
        );
        assert_eq!(
            targets,
            expected
                .iter()
                .fold(Bitboard::EMPTY, |bb, c| bb | c.captures)
        );
        let mut emitted = Vec::new();
        generator.emit_ordinary_captures(board, &actual, allowed, &mut |c| emitted.push(c.mv));
        let expected: Vec<_> = generated_captures_with(generator, board)
            .into_iter()
            .filter(|m| {
                ORDINARY_KINDS.contains(&board.piece_at(m.from).unwrap().kind().unwrap())
                    && allowed.contains(m.to)
            })
            .collect();
        assert_eq!(emitted, expected);
    }
}

#[test]
fn pawn_batch_matches_reference_in_all_rules_and_both_sides() {
    for rules in rule_variants() {
        let generator = MoveGenerator::new(rules);
        for board in bench_positions()
            .into_iter()
            .chain(sampled_random_positions(rules))
        {
            for color in Color::ALL {
                assert_matches_before(&generator, &board.clone_with_side_to_move(color));
            }
        }
    }
}

/// 第7条・第9条。全144升で前進、盤端、味方による遮蔽、および捕獲を検査する。
#[test]
fn pawn_batch_preserves_board_edges_and_destination_occupancy() {
    let generator = MoveGenerator::standard();
    for color in Color::ALL {
        let delta = if color == Color::Black { 1 } else { -1 };
        for kind in [PieceKind::Pawn, PieceKind::GoBetween] {
            for from in Square::all() {
                for blocker in [None, Some(color), Some(color.opposite())] {
                    let mut pieces = vec![(from, color, kind)];
                    if let Some(to) = from.offset(0, delta)
                        && let Some(side) = blocker
                    {
                        pieces.push((to, side, PieceKind::Pawn));
                    }
                    assert_matches_before(&generator, &position(color, &pieces));
                }
            }
        }
    }
}

/// 第15条。歩兵の疑似利きが届いても、先獅子で保護された獅子は取れない。
#[test]
fn pawn_batch_preserves_sente_lion_restrictions() {
    for color in Color::ALL {
        let delta = if color == Color::Black { 1 } else { -1 };
        let from = sq(5, 5);
        let to = from.offset(0, delta).unwrap();
        let mut board = position(
            color,
            &[
                (from, color, PieceKind::Pawn),
                (to, color.opposite(), PieceKind::Lion),
                (
                    to.offset(0, delta).unwrap(),
                    color.opposite(),
                    PieceKind::Rook,
                ),
            ],
        );
        assert!(board.piece_control(from).contains(to));
        for rules in rule_variants() {
            let generator = MoveGenerator::new(rules);
            board.set_lion_capture(None).unwrap();
            assert!(
                generated_captures_with(&generator, &board).contains(&mv(from, None, to, false))
            );
            board.set_lion_capture(Some(sq(0, 0))).unwrap();
            assert!(generated_captures_with(&generator, &board).is_empty());
            assert_matches_before(&generator, &board);
        }
    }
}

/// 第18・19条とP5・P6。保留した歩兵の最奥段での選択肢は規則と捕獲で変わる。
#[test]
fn pawn_batch_preserves_deferred_and_forced_promotion() {
    for (text, capture, promotes) in [
        ("L0,P0,R1,E0", false, vec![false, true]),
        ("L0,P0,P5,R1,E0", false, vec![false]),
        ("L0,P0,P5,R1,E0", true, vec![true]),
        ("L0,P2,P5,P6,R1,E0", false, vec![true]),
    ] {
        let rules: MoveRules = Rules::from_codes(&parse_rule_set(text).unwrap())
            .unwrap()
            .moves;
        for color in Color::ALL {
            let (from, to) = if color == Color::Black {
                (sq(5, 10), sq(5, 11))
            } else {
                (sq(5, 1), sq(5, 0))
            };
            let mut builder = PositionBuilder::new(color);
            builder
                .put(from, PieceCode::new(color, PieceKind::Pawn).unwrap())
                .unwrap();
            builder.mark_promotion_deferred(from).unwrap();
            if capture {
                builder
                    .put(
                        to,
                        PieceCode::new(color.opposite(), PieceKind::Pawn).unwrap(),
                    )
                    .unwrap();
            }
            let board = builder.finish().unwrap();
            let generator = MoveGenerator::new(rules);
            assert_matches_before(&generator, &board);
            let mut actual = Vec::new();
            if capture {
                generator.generate_captures(&board, &mut actual);
            } else {
                generator.generate_quiets(&board, &mut Vec::new(), &mut actual);
            }
            assert_eq!(
                actual,
                promotes
                    .iter()
                    .map(|&p| mv(from, None, to, p))
                    .collect::<Vec<_>>()
            );
        }
    }
}
