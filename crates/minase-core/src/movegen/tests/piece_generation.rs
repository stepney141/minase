//! 駒1枚の生成が全件生成からの抽出と、要素、重複、順序まで一致する契約を検証する。

use super::{generated_captures_with, generated_with, moves_from, mv};
use crate::movegen::MoveGenerator;
use crate::piece::{Color, PieceKind};
use crate::position::Position;
use crate::test_util::{
    bench_positions, capture_test_rules, position, sampled_random_positions, sq,
};

fn assert_piece_generation_matches_full_generation(position: &Position, generator: &MoveGenerator) {
    let all_moves = generated_with(generator, position);
    let all_captures = generated_captures_with(generator, position);
    // 既存の出力を残して追記し、作業領域に残る手を混ぜないことも契約に含む。
    let sentinel = mv(sq(0, 0), None, sq(0, 0), true);
    let mut base_moves = vec![sentinel];
    for from in position.pieces_of(position.side_to_move()) {
        let mut moves = vec![sentinel];
        generator.generate_moves_from(position, from, &mut base_moves, &mut moves);
        let mut expected = vec![sentinel];
        expected.extend(moves_from(&all_moves, from));
        assert_eq!(
            moves,
            expected,
            "all moves: rules={:?}, side={:?}, from={from:?}",
            generator.rules(),
            position.side_to_move()
        );

        let mut captures = vec![sentinel];
        generator.generate_captures_from(position, from, &mut base_moves, &mut captures);
        let mut expected = vec![sentinel];
        expected.extend(moves_from(&all_captures, from));
        assert_eq!(
            captures,
            expected,
            "captures: rules={:?}, side={:?}, from={from:?}",
            generator.rules(),
            position.side_to_move()
        );
    }
}

/// 設計書の駒1枚の生成の契約を、全規則標本・局面標本の両手番で照合する。
#[test]
fn piece_generation_matches_full_generation_in_order_for_all_rules_and_sides() {
    for rules in capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        for sample in bench_positions()
            .into_iter()
            .chain(sampled_random_positions(rules))
        {
            for side in Color::ALL {
                let position = sample.clone_with_side_to_move(side);
                assert_piece_generation_matches_full_generation(&position, &generator);
            }
        }
    }
}

/// RULES.md第11条第1項・第2項。居喰いと経由升だけで取る手は捕獲列に残り、
/// 捕獲のない駒の生成は既存の出力をそのまま保つ。
#[test]
fn article_11_piece_generation_preserves_igui_mid_only_capture_and_empty_captures() {
    let from = sq(5, 5);
    let mid = sq(5, 6);
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (from, Color::Black, PieceKind::Lion),
            (mid, Color::White, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    for rules in capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        assert_piece_generation_matches_full_generation(&board, &generator);
        let mut base_moves = Vec::new();
        let mut captures = Vec::new();
        generator.generate_captures_from(&board, from, &mut base_moves, &mut captures);
        assert!(captures.contains(&mv(from, Some(mid), from, false)));
        assert!(captures.contains(&mv(from, Some(mid), sq(5, 7), false)));
        assert!(!captures.contains(&mv(from, None, from, false)));

        let before = captures.clone();
        generator.generate_captures_from(&board, sq(0, 0), &mut base_moves, &mut captures);
        assert_eq!(captures, before);
    }
}

/// 両入口は、空升または相手駒の升という呼び出し側の前提違反をパニックで知らせる。
#[test]
fn piece_generation_panics_unless_from_contains_a_side_to_move_piece() {
    let generator = MoveGenerator::standard();
    let board = position(Color::Black, &[(sq(1, 1), Color::White, PieceKind::King)]);
    for from in [sq(0, 0), sq(1, 1)] {
        assert!(
            std::panic::catch_unwind(|| {
                generator.generate_moves_from(&board, from, &mut Vec::new(), &mut Vec::new());
            })
            .is_err()
        );
        assert!(
            std::panic::catch_unwind(|| {
                generator.generate_captures_from(&board, from, &mut Vec::new(), &mut Vec::new());
            })
            .is_err()
        );
    }
}
