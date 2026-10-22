//! 補正後の静的評価による良化判定を検査する。

use super::*;
use crate::search::alphabeta::pruning::{corrected_static_eval, is_improving};

// Stockfish src/search.cpp:883。等号と比較相手がない場合は偽とする。
#[test]
fn improving_requires_a_strict_increase_from_two_plies_ago() {
    for (current, previous, expected) in [
        (101, Some(100), true),
        (100, Some(100), false),
        (99, Some(100), false),
        (100, None, false),
    ] {
        assert_eq!(is_improving(current, previous), expected);
    }
}

#[test]
fn improving_compares_stored_evaluations_across_null_moves() {
    let board = bare_kings();
    with_root_searcher(&board, &[], |searcher| {
        for ply in [0, 1] {
            assert!(!searcher.improving(100, ply));
        }
        searcher.static_evals[2] = 100;
        for null_move_ply in [None, Some(4), Some(3), Some(2)] {
            searcher.null_move_ply = null_move_ply;
            for (current, expected) in [(101, true), (100, false), (99, false)] {
                assert_eq!(searcher.improving(current, 4), expected);
            }
        }
    });
}

// 静かな手のfutility pruningはimprovingを条件にも見積りにも使わない。
#[test]
fn futility_pruning_is_independent_of_improving() {
    let board = bare_kings();
    let moves = legal_moves(&board);
    let history = repeated_root_children(&board, &moves);
    let static_eval = evaluate(test_nnue(), &board);
    let alpha = static_eval + 10 * test_nnue().pawn_value();
    for improving in [false, true] {
        with_repetition_root_searcher(
            test_nnue(),
            &board,
            engine_rules(),
            SearchRepetition::R3,
            &history,
            |searcher| {
                searcher.tt.store(
                    search_key(&board),
                    0,
                    0,
                    None,
                    Bound::Upper,
                    Some(moves[0]),
                    2,
                    true,
                );
                searcher.static_evals[0] = static_eval - i32::from(improving);
                assert_eq!(searcher.improving(static_eval, 2), improving);
                assert!(!royal_under_attack(&board));
                set_test_path_ply(searcher, 2);
                let score = searcher
                    .negamax(&mut board.clone(), 1, alpha, alpha + 2, 2, false)
                    .unwrap();
                assert!(score < alpha && score.abs() < MATE_THRESHOLD);
                assert_eq!(searcher.nodes, 1);
            },
        );
    }
}

// Stockfish src/search.cpp:846-884。現在と2手前の双方に補正を含めて比べる。
#[test]
fn improving_uses_corrected_evaluations() {
    let board = bare_kings();
    for (previous_difference, expected_improving) in [(1, true), (0, false)] {
        with_root_searcher(&board, &[], |searcher| {
            let side = board.side_to_move();
            zero_position_correction(searcher, &board);
            store_position_correction(searcher, &board, 512);
            let correction = position_correction(searcher, side, 2);
            let raw = evaluate(searcher.nnue, &board);
            let corrected = corrected_static_eval(raw, correction);
            assert_ne!(corrected, raw);
            searcher.static_evals[0] = corrected - previous_difference;
            set_test_path_ply(searcher, 2);
            searcher
                .negamax(&mut board.clone(), 1, -INFINITY, INFINITY, 2, false)
                .unwrap();
            assert_eq!(searcher.static_evals[2], corrected);
            assert_eq!(
                searcher.improving(searcher.static_evals[2], 2),
                expected_improving
            );
        });
    }
}

// 根と浅い枝刈りの対象外の通常ノードも、補正後の値を保持する。
#[test]
fn root_and_full_window_nodes_store_corrected_evaluation() {
    let board = bare_kings();
    for root in [false, true] {
        with_root_searcher(&board, &[], |searcher| {
            let side = board.side_to_move();
            zero_position_correction(searcher, &board);
            store_position_correction(searcher, &board, 512);
            let correction = position_correction(searcher, side, 0);
            let raw = evaluate(searcher.nnue, &board);
            let corrected = corrected_static_eval(raw, correction);
            assert_ne!(corrected, raw);
            searcher.static_evals[0] = i32::MIN;
            if root {
                assert!(
                    searcher
                        .search_root(&board, &legal_moves(&board), 2, 2, -INFINITY, INFINITY)
                        .is_some()
                );
            } else {
                assert!(
                    searcher
                        .negamax(&mut board.clone(), 2, -INFINITY, INFINITY, 0, false)
                        .is_some()
                );
            }
            assert_eq!(searcher.static_evals[0], corrected);
        });
    }
}
