//! 補正後の静的評価による良化判定を検査する。

use super::*;
use crate::search::alphabeta::params;
use crate::search::alphabeta::pruning::is_improving;

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

// 縮めたfutility余裕値の境界では、良化していないノードだけ後続の静かな手を刈る。
#[test]
fn futility_prunes_quiets_only_when_not_improving() {
    assert_eq!(test_nnue().pawn_value(), 100, "固定ネットの歩兵価値");
    // 良化していない深さ1の余裕値をαに合わせる。
    let margin = test_nnue().pawn_value() * params::futility_margin1() / 100
        * params::non_improving_futility1()
        / 100;
    let board = bare_kings();
    let quiets = legal_moves(&board);
    let history = repeated_root_children(&board, &quiets);
    let moves = legal_moves(&board);
    for improving in [false, true] {
        with_root_searcher(&board, &history, |searcher| {
            searcher.draw_value = -1000;
            searcher.seek_mate = true;
            searcher.killers[2] = [Some(quiets[1]), Some(quiets[2])];
            searcher.tt.store(
                search_key(&board),
                0,
                0,
                Bound::Upper,
                Some(quiets[0]),
                2,
                true,
            );
            let static_eval = evaluate(searcher.nnue, &board);
            let alpha = static_eval + margin;
            searcher.static_evals[0] = static_eval - i32::from(improving);
            assert_eq!(searcher.improving(static_eval, 2), improving);
            assert!(!royal_under_attack(&board));
            let score = searcher
                .negamax(&mut board.clone(), 1, alpha, alpha + 1, 2, false)
                .unwrap();
            assert!(score < alpha && score.abs() < MATE_THRESHOLD);
            assert_eq!(
                searcher.nodes,
                if improving && params::futility_margin1() > margin {
                    moves.len() as u64
                } else {
                    1
                }
            );
        });
    }
}

// Stockfish src/search.cpp:846-884。現在と2手前の双方に補正を含めて比べる。
#[test]
fn improving_uses_corrected_evaluations() {
    assert_eq!(test_nnue().pawn_value(), 100, "固定ネットの歩兵価値");
    // 良化していない深さ1の余裕値をαに合わせる。
    let margin = test_nnue().pawn_value() * params::futility_margin1() / 100
        * params::non_improving_futility1()
        / 100;
    let board = bare_kings();
    let quiets = legal_moves(&board);
    let history = repeated_root_children(&board, &quiets);
    for (previous_correction, expected_improving) in [(0, true), (100, false)] {
        with_root_searcher(&board, &history, |searcher| {
            searcher.draw_value = -1000;
            searcher.seek_mate = true;
            searcher.killers[2] = [Some(quiets[1]), Some(quiets[2])];
            searcher.tt.store(
                search_key(&board),
                0,
                0,
                Bound::Upper,
                Some(quiets[0]),
                2,
                true,
            );
            let side = board.side_to_move();
            let key = material_key(&board);
            searcher.correction.update(side, key, 100, 8);
            let correction = searcher.correction.read(side, key);
            assert!(correction > 0 && correction <= 100);
            let raw = evaluate(searcher.nnue, &board);
            searcher.static_evals[0] = raw + previous_correction;
            let alpha = raw + correction + margin;
            let score = searcher
                .negamax(&mut board.clone(), 1, alpha, alpha + 1, 2, false)
                .unwrap();
            assert_eq!(searcher.static_evals[2], raw + correction);
            assert!(score < alpha && score.abs() < MATE_THRESHOLD);
            assert_eq!(
                searcher.nodes,
                if expected_improving && params::futility_margin1() > margin {
                    legal_moves(&board).len() as u64
                } else {
                    1
                }
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
            searcher
                .correction
                .update(side, material_key(&board), 64, 8);
            let correction = searcher.correction.read(side, material_key(&board));
            assert!(correction > 0);
            let raw = evaluate(searcher.nnue, &board);
            searcher.static_evals[0] = i32::MIN;
            if root {
                assert!(
                    searcher
                        .search_root(&board, &legal_moves(&board), 2, -INFINITY, INFINITY)
                        .is_some()
                );
            } else {
                assert!(
                    searcher
                        .negamax(&mut board.clone(), 2, -INFINITY, INFINITY, 0, false)
                        .is_some()
                );
            }
            assert_eq!(searcher.static_evals[0], raw + correction);
        });
    }
}
