//! 全件生成の集合を基準に、単独手の検査と検査付き適用の契約を照合する。

use super::{generated_with, rule_variants};
use crate::board::Square;
use crate::movegen::MoveGenerator;
use crate::mv::Move;
use crate::piece::Color;
use crate::test_util::{bench_positions, sampled_random_positions};

/// 駒1枚の全手生成から候補を探す参照実装。直接判定の集合保存を照合する。
fn generated_legality(
    position: &crate::Position,
    generator: &MoveGenerator,
    candidate: Move,
) -> bool {
    if position
        .piece_at(candidate.from)
        .and_then(|piece| piece.color())
        != Some(position.side_to_move())
    {
        return false;
    }
    let mut moves = Vec::new();
    generator.generate_moves_from(position, candidate.from, &mut Vec::new(), &mut moves);
    moves.contains(&candidate)
}

/// 移動元、経由升、到達升、成りのいずれか1つだけを変える。
/// 変更後も合法な場合があるので、拒否を仮定せず全件生成の集合と照合する。
fn candidates(mv: Move) -> Vec<Move> {
    let mut candidates = vec![
        mv,
        Move {
            promote: !mv.promote,
            ..mv
        },
    ];
    if let Some(from) = Square::new(mv.from.file() + 1, mv.from.rank()) {
        candidates.push(Move { from, ..mv });
    }
    if let Some(to) = Square::new(mv.to.file(), mv.to.rank() + 1) {
        candidates.push(Move { to, ..mv });
    }
    match mv.mid {
        Some(mid) => {
            candidates.push(Move { mid: None, ..mv });
            if let Some(mid) = Square::new(mid.file() + 1, mid.rank()) {
                candidates.push(Move {
                    mid: Some(mid),
                    ..mv
                });
            }
        }
        None => {
            if let Some(mid) = Square::new(mv.from.file(), mv.from.rank() + 1) {
                candidates.push(Move {
                    mid: Some(mid),
                    ..mv
                });
            }
        }
    }
    candidates
}

/// chu-shogi.md第6条から第19条、第29条・第30条の局面合法手を受理し、
/// 第27条第1項により不合法手では局面を変えない。
#[test]
fn article_27_legality_and_checked_application_match_full_generation() {
    let mut accepted = 0;
    let mut rejected = 0;
    for rules in rule_variants() {
        let generator = MoveGenerator::new(rules);
        let mut base_moves = Vec::new();
        let mut piece_moves = Vec::new();
        for sample in bench_positions()
            .into_iter()
            .chain(sampled_random_positions(rules))
        {
            for side in Color::ALL {
                let position = sample.clone_with_side_to_move(side);
                let legal = generated_with(&generator, &position);
                for candidate in legal.iter().copied().flat_map(candidates) {
                    let expected = legal.contains(&candidate);
                    assert_eq!(
                        generated_legality(&position, &generator, candidate),
                        expected
                    );
                    assert_eq!(
                        generator.is_legal_move(
                            &position,
                            candidate,
                            &mut base_moves,
                            &mut piece_moves
                        ),
                        expected,
                        "rules={rules:?}, side={side:?}, candidate={candidate:?}"
                    );
                    let mut actual = position.clone();
                    assert_eq!(
                        actual.try_make_move(candidate, &generator).is_ok(),
                        expected,
                        "rules={rules:?}, side={side:?}, candidate={candidate:?}"
                    );
                    if expected {
                        let mut after = position.clone();
                        after.make_move_unchecked(candidate, rules);
                        assert_eq!(actual, after, "accepted candidate={candidate:?}");
                        accepted += 1;
                    } else {
                        assert_eq!(actual, position, "rejected candidate={candidate:?}");
                        rejected += 1;
                    }
                }
            }
        }
    }
    eprintln!("legality reference: accepted={accepted}, rejected={rejected}");
    assert!(accepted > 0 && rejected > 0);
}

/// 第15条の先獅子は通常駒の直接捕獲にも適用され、疑似利きだけでは受理できない。
#[test]
fn article_15_direct_legality_rejects_protected_sente_lion() {
    use crate::piece::PieceKind;
    use crate::test_util::{position, sq};
    let from = sq(5, 3);
    let to = sq(5, 5);
    let mut board = position(
        Color::Black,
        &[
            (from, Color::Black, PieceKind::Rook),
            (to, Color::White, PieceKind::Lion),
            (sq(5, 7), Color::White, PieceKind::Rook),
        ],
    );
    board.set_lion_capture(Some(sq(0, 0))).unwrap();
    assert!(board.piece_control(from).contains(to));
    let candidate = Move {
        from,
        mid: None,
        to,
        promote: false,
    };
    for rules in rule_variants() {
        let generator = MoveGenerator::new(rules);
        assert!(!generated_legality(&board, &generator, candidate));
        assert!(!generator.is_legal_move(&board, candidate, &mut Vec::new(), &mut Vec::new()));
    }
}

/// 第17〜19条と第30条P1・P2・P5・P6の選択肢を、保留状態と捕獲の有無で固定する。
#[test]
fn article_30_direct_legality_preserves_promotion_choices() {
    use crate::piece::{PieceCode, PieceKind};
    use crate::position::PositionBuilder;
    use crate::rules::{Rules, parse_rule_set};
    use crate::test_util::sq;
    let cases = [
        (
            "L0,P0,R1,E0",
            PieceKind::Lance,
            7,
            11,
            false,
            false,
            [true, true],
        ),
        (
            "L0,P0,P6,R1,E0",
            PieceKind::Lance,
            7,
            11,
            false,
            false,
            [false, true],
        ),
        (
            "L0,P0,P5,R1,E0",
            PieceKind::Pawn,
            10,
            11,
            true,
            true,
            [false, true],
        ),
        (
            "L0,P0,P5,R1,E0",
            PieceKind::Pawn,
            10,
            11,
            true,
            false,
            [true, false],
        ),
        (
            "L0,P0,P5,R1,E0",
            PieceKind::Pawn,
            8,
            9,
            true,
            true,
            [true, false],
        ),
        (
            "L0,P1,R1,E0",
            PieceKind::SilverGeneral,
            8,
            9,
            true,
            false,
            [true, false],
        ),
        (
            "L0,P1,R1,E0",
            PieceKind::SilverGeneral,
            8,
            9,
            false,
            false,
            [true, true],
        ),
        (
            "L0,P2,R1,E0",
            PieceKind::SilverGeneral,
            8,
            9,
            true,
            false,
            [true, false],
        ),
        (
            "L0,P2,R1,E0",
            PieceKind::SilverGeneral,
            8,
            9,
            false,
            false,
            [true, true],
        ),
        (
            "L0,P2,R1,E0",
            PieceKind::SilverGeneral,
            8,
            9,
            true,
            true,
            [true, true],
        ),
        (
            "L0,P2,P5,R1,E0",
            PieceKind::Pawn,
            10,
            11,
            true,
            false,
            [false, true],
        ),
    ];
    for (text, kind, from_rank, to_rank, deferred, capture, expected) in cases {
        let generator = MoveGenerator::new(
            Rules::from_codes(&parse_rule_set(text).unwrap())
                .unwrap()
                .moves,
        );
        let from = sq(5, from_rank);
        let to = sq(5, to_rank);
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(from, PieceCode::new(Color::Black, kind).unwrap())
            .unwrap();
        if deferred {
            builder.mark_promotion_deferred(from).unwrap();
        }
        if capture {
            builder
                .put(to, PieceCode::new(Color::White, PieceKind::Pawn).unwrap())
                .unwrap();
        }
        let board = builder.finish().unwrap();
        for (promote, expected) in [false, true].into_iter().zip(expected) {
            let candidate = Move {
                from,
                mid: None,
                to,
                promote,
            };
            if !capture {
                let mut quiets = Vec::new();
                generator.generate_quiets(&board, &mut Vec::new(), &mut quiets);
                assert_eq!(
                    quiets.contains(&candidate),
                    expected,
                    "quiet {text}: {candidate:?}"
                );
            }
            assert_eq!(
                generated_legality(&board, &generator, candidate),
                expected,
                "{text}: {candidate:?}"
            );
            assert_eq!(
                generator.is_legal_move(&board, candidate, &mut Vec::new(), &mut Vec::new()),
                expected,
                "{text}: {candidate:?}"
            );
        }
    }
}

/// 第7条4〜7項: 走りは最初の駒まで、固定の跳びは中間駒に依存せず受理する。
/// 遮蔽の省略と、固定利きに遮蔽を適用する誤りをそれぞれ固定例で検出する。
#[test]
fn article_7_direct_legality_preserves_blockers_and_jumps() {
    use crate::piece::PieceKind;
    use crate::test_util::{position, sq};
    let generator = MoveGenerator::standard();
    for color in Color::ALL {
        for kind in [PieceKind::Rook, PieceKind::Kirin] {
            let from = sq(5, 5);
            let board = position(
                color,
                &[
                    (from, color, kind),
                    (sq(5, 6), color.opposite(), PieceKind::Pawn),
                    (sq(6, 5), color, PieceKind::Pawn),
                ],
            );
            for (to, rook, kirin) in [
                (sq(5, 6), true, false),
                (sq(5, 7), false, true),
                (sq(5, 8), false, false),
                (sq(6, 5), false, false),
                (sq(7, 5), false, true),
                (sq(4, 5), true, false),
                (sq(4, 4), false, true),
            ] {
                let candidate = Move {
                    from,
                    mid: None,
                    to,
                    promote: false,
                };
                let expected = if kind == PieceKind::Rook { rook } else { kirin };
                assert_eq!(generated_legality(&board, &generator, candidate), expected);
                assert_eq!(
                    generator.is_legal_move(&board, candidate, &mut Vec::new(), &mut Vec::new()),
                    expected
                );
            }
        }
    }
}
