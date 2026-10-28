//! 全件生成の集合を基準に、単独手の検査と検査付き適用の契約を照合する。

use super::generated_with;
use crate::board::Square;
use crate::movegen::MoveGenerator;
use crate::mv::Move;
use crate::piece::Color;
use crate::test_util::{bench_positions, capture_test_rules, sampled_random_positions};

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

/// RULES.md第6条から第19条、第29条・第30条の局面合法手を受理し、
/// 第27条第1項により不合法手では局面を変えない。
#[test]
fn article_27_legality_and_checked_application_match_full_generation() {
    let mut accepted = 0;
    let mut rejected = 0;
    for rules in capture_test_rules() {
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
