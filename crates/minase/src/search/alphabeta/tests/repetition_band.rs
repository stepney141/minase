//! RULES.md第31条R1の攻撃側の負けと、R2・R3および第23条の禁止手しかない負けを保護する。
//! 専用の帯は対局履歴と探索経路に依存する。経路に依存する値を置換表や補正履歴へ保存せず、枝刈りから保護する。

use super::*;
use crate::eval::nnue::EVALUATION_LIMIT;
use crate::search::alphabeta::negamax::{can_prune_moves, null_move_cutoff_score};
use crate::search::alphabeta::pruning::{late_move_limit, see_margin};
use crate::search::{REPETITION_WIN, in_repetition_band};

#[test]
fn r1_repetition_band_boundaries_exclude_normal_evaluation_and_mates() {
    // 設計書の勝ち29,743、最大256手、通常評価の上限28,999から定まる境界。
    // eval::nnue::testsのfinal_sum_is_clamped_once_and_linear_division_truncates_toward_zero
    // も、Nnueが返す最大・最小値を±28,999として検査する。
    for sign in [-1, 1] {
        for (magnitude, expected) in [
            (0, false),
            (EVALUATION_LIMIT, false),
            (29_486, false),
            (29_487, true),
            (29_740, true),
            (29_743, true),
            (29_744, false),
            (30_000, false),
        ] {
            assert_eq!(in_repetition_band(sign * magnitude), expected);
        }
    }
}

#[test]
fn r1_repetition_band_negamax_ignores_every_bound_but_searches_tt_move_first() {
    let board = bare_kings();
    let moves = legal_moves(&board);
    let tt_move = *moves.last().unwrap();
    assert_ne!(tt_move, moves[0]);
    for score in [REPETITION_WIN - 3, -(REPETITION_WIN - 3)] {
        for bound in [Bound::Exact, Bound::Lower, Bound::Upper] {
            with_root_searcher(&board, &[], |searcher| {
                searcher.tt.store(
                    search_key(&board),
                    32,
                    score,
                    bound,
                    Some(tt_move),
                    0,
                    false,
                );
                for &mv in &moves {
                    let mut child = board.clone();
                    child.make_move_unchecked(mv, engine_rules());
                    searcher
                        .tt
                        .store(search_key(&child), 32, 0, Bound::Exact, None, 1, false);
                }
                let (alpha, beta) = match bound {
                    Bound::Exact => (-INFINITY, INFINITY),
                    Bound::Lower => (score - 1, score),
                    Bound::Upper => (score, score + 1),
                };
                // 子を1手だけ読ませ、2手目の適用前に中断して先頭の主変化を観測する。
                searcher.interrupt_at = Some((1, StopReason::NodeLimit));
                let actual = searcher.negamax(&mut board.clone(), 2, alpha, beta, 0, false);
                assert_eq!(actual, if beta <= 0 { Some(0) } else { None });
                assert_eq!(searcher.nodes, 1, "{score}, {bound:?}");
                assert_eq!(searcher.pv[0], [tt_move], "{score}, {bound:?}");
            });
        }
    }
}

fn capture_board() -> (Position, Move) {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(0, 0), Color::Black, PieceKind::Rook),
            (sq(0, 2), Color::White, PieceKind::Pawn),
            (sq(4, 0), Color::Black, PieceKind::Rook),
            (sq(4, 2), Color::White, PieceKind::Pawn),
        ],
    );
    let tt_move = Move {
        from: sq(4, 0),
        mid: None,
        to: sq(4, 2),
        promote: false,
    };
    assert!(legal_moves(&board).contains(&tt_move));
    (board, tt_move)
}

#[test]
fn r1_repetition_band_quiesce_ignores_every_bound_but_searches_tt_capture_first() {
    let (board, tt_move) = capture_board();
    for score in [REPETITION_WIN - 3, -(REPETITION_WIN - 3)] {
        for bound in [Bound::Exact, Bound::Lower, Bound::Upper] {
            with_root_searcher(&board, &[], |searcher| {
                searcher.tt.store(
                    search_key(&board),
                    32,
                    score,
                    bound,
                    Some(tt_move),
                    0,
                    false,
                );
                let (alpha, beta) = match bound {
                    Bound::Exact => (-INFINITY, INFINITY),
                    Bound::Lower if score > 0 => (-INFINITY, score),
                    // 負の下界よりstand-patが必ず高いので、負のLower自体による打ち切りは起こらない。
                    Bound::Lower => (-INFINITY, INFINITY),
                    Bound::Upper => (score, INFINITY),
                };
                // 高いαでも候補を残し、置換表と順序付けの契約をdelta pruningから独立に検査する。
                searcher.delta_margin = INFINITY;
                searcher.interrupt_at = Some((1, StopReason::NodeLimit));
                assert_eq!(searcher.quiesce(&mut board.clone(), alpha, beta, 0), None);
                assert_eq!(searcher.nodes, 1, "{score}, {bound:?}");
                assert_eq!(
                    searcher.previous_capture[1],
                    Some(tt_move.to),
                    "{score}, {bound:?}"
                );
            });
        }
    }
}

#[test]
fn r1_repetition_band_protection_keeps_mate_tt_cutoffs() {
    let (board, tt_move) = capture_board();
    for (score, bound, alpha, beta) in [
        (29_990, Bound::Exact, -INFINITY, INFINITY),
        (-29_990, Bound::Exact, -INFINITY, INFINITY),
        (29_990, Bound::Lower, -INFINITY, 29_955),
        (-29_990, Bound::Upper, -29_955, INFINITY),
    ] {
        with_root_searcher(&board, &[], |searcher| {
            searcher.tt.store(
                search_key(&board),
                32,
                score,
                bound,
                Some(tt_move),
                5,
                false,
            );
            let expected = if score > 0 { 29_955 } else { -29_955 };
            assert_eq!(
                searcher.negamax(&mut board.clone(), 2, alpha, beta, 40, false),
                Some(expected)
            );
            assert_eq!(
                searcher.quiesce(&mut board.clone(), alpha, beta, 40),
                Some(expected)
            );
            assert_eq!(searcher.nodes, 0);
        });
    }
}

#[test]
fn r1_repetition_band_deep_mate_corrected_into_band_is_not_a_tt_cutoff() {
    let board = bare_kings();
    for sign in [-1, 1] {
        let table = small_tt();
        table.store(
            search_key(&board),
            32,
            sign * 29_744,
            Bound::Exact,
            None,
            5,
            false,
        );
        // 保存距離251手に読み出し位置40手を足すと291手となり、専用の帯へ入る。
        assert_eq!(
            table.probe(search_key(&board), 40).unwrap().score,
            sign * 29_709
        );
        let (score, nodes) = run_negamax(&board, 1, -INFINITY, INFINITY, 40, &table);
        assert!(score.abs() < 29_487);
        assert!(nodes > 0);
    }
}

#[test]
fn r1_repetition_band_is_not_reused_after_only_game_history_changes() {
    let board = repetition_fixture();
    let mut repeated_child = board.clone();
    repeated_child.make_move_unchecked(repetition_move(), engine_rules());
    let history = [search_key(&repeated_child), search_key(&board)];
    let (expected, _) = run_negamax(&board, 2, -INFINITY, INFINITY, 0, &small_tt());
    assert!(expected <= -29_744, "履歴なしでは王駒を失う");
    // R1の勝敗と、R2・R3で全手禁止だった負けのいずれも同じ保護を受ける。
    for score in [29_740, -29_740, -29_487] {
        with_root_searcher(&board, &history, |searcher| {
            assert_eq!(
                searcher.negamax(&mut board.clone(), 2, -INFINITY, INFINITY, 0, false),
                Some(0)
            );
            // 経路依存の勝敗を前の探索の記録として置き、値を再利用しないことを確かめる。
            searcher.tt.store(
                search_key(&board),
                32,
                score,
                Bound::Exact,
                Some(repetition_move()),
                0,
                false,
            );
            searcher.history_keys = &[];
            searcher.nodes = 0;
            assert_eq!(
                searcher.negamax(&mut board.clone(), 2, -INFINITY, INFINITY, 0, false),
                Some(expected)
            );
            assert!(searcher.nodes > 0);
        });
    }
}

#[test]
fn r1_repetition_band_protection_leaves_draw_reuse_as_a_known_approximation() {
    let board = repetition_fixture();
    let mut child = board.clone();
    child.make_move_unchecked(repetition_move(), engine_rules());
    let history = [search_key(&child), search_key(&board)];
    with_root_searcher(&board, &history, |searcher| {
        assert_eq!(
            searcher.negamax(&mut board.clone(), 2, -INFINITY, INFINITY, 0, false),
            Some(0)
        );
        searcher.history_keys = &[];
        searcher.nodes = 0;
        // DRAW_SCOREの0は通常評価と区別できない。履歴を変えても祖先に保存された0を
        // 再利用し得ることは、RULES.md第31条R1を探索で近似する際の既知の制約である。
        assert_eq!(
            searcher.negamax(&mut board.clone(), 2, -INFINITY, INFINITY, 0, false),
            Some(DRAW_SCORE)
        );
        assert_eq!(searcher.nodes, 0);
    });
}

#[test]
fn r1_repetition_band_windows_disable_reverse_futility_and_razoring() {
    let board = bare_kings();
    for depth in [1, 2] {
        for (alpha, beta) in [
            (-29_488, -29_487),
            (-29_487, -29_486),
            (29_486, 29_487),
            (29_487, 29_488),
        ] {
            let (_, nodes) = run_negamax(&board, depth, alpha, beta, 0, &small_tt());
            assert!(nodes > 0, "depth={depth}, alpha={alpha}, beta={beta}");
        }
    }
}

#[test]
fn r1_repetition_band_windows_disable_futility_and_late_move_pruning() {
    let board = super::late_move::quiet_board();
    let moves = legal_moves(&board);
    assert!(moves.len() > late_move_limit(3) as usize);
    let history = repeated_root_children(&board, &moves);
    for depth in [1, 2, 3] {
        for alpha in [29_486, 29_487, 29_740] {
            with_root_searcher(&board, &history, |searcher| {
                searcher.tt.store(
                    search_key(&board),
                    0,
                    0,
                    Bound::Upper,
                    Some(moves[0]),
                    0,
                    false,
                );
                assert_eq!(
                    searcher.negamax(&mut board.clone(), depth, alpha, alpha + 1, 0, false),
                    Some(0)
                );
                assert_eq!(
                    searcher.nodes,
                    moves.len() as u64,
                    "depth={depth}, alpha={alpha}"
                );
            });
        }
    }
}

#[test]
fn r1_repetition_band_windows_disable_see_pruning() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::Rook),
            (sq(5, 4), Color::White, PieceKind::Pawn),
            (sq(5, 5), Color::White, PieceKind::Pawn),
        ],
    );
    let capture = Move {
        from: sq(5, 3),
        mid: None,
        to: sq(5, 4),
        promote: false,
    };
    let quiet = Move {
        from: sq(11, 0),
        mid: None,
        to: sq(10, 0),
        promote: false,
    };
    let moves = legal_moves(&board);
    let history = repeated_root_children(&board, &moves);
    for depth in [2, 3] {
        with_root_searcher(&board, &history, |searcher| {
            assert!(!royal_under_attack(&board));
            assert!(see_prunes(
                &board,
                engine_rules(),
                searcher.nnue,
                capture,
                see_margin(searcher.nnue.pawn_value(), depth)
            ));
            searcher.tt.store(
                search_key(&board),
                0,
                0,
                Bound::Upper,
                Some(quiet),
                0,
                false,
            );
            assert_eq!(
                searcher.negamax(&mut board.clone(), depth, 29_486, 29_487, 0, false),
                Some(0)
            );
            assert_eq!(searcher.nodes, moves.len() as u64);
        });
    }
}

#[test]
fn r1_repetition_band_loss_keeps_later_quiets_and_captures_eligible() {
    // 反復による勝敗の値を、静かな手と捕獲手が共有する枝刈り判定へ渡す。
    for score in [-30_000, -29_744, -29_743, -29_740, -29_487] {
        assert!(!can_prune_moves(score), "score={score}");
    }
    for score in [-29_486, -28_999, 0, 29_487, 29_744] {
        assert!(can_prune_moves(score), "score={score}");
    }
}

#[test]
fn r1_repetition_band_beta_disables_null_move() {
    let board = position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 1), Color::Black, PieceKind::GoldGeneral),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    );
    let moves = legal_moves(&board);
    let history = repeated_root_children(&board, &moves);
    for beta in [-29_740, -29_487, 29_487, 29_740] {
        with_root_searcher(&board, &history, |searcher| {
            searcher.tt.store(
                search_key(&board),
                0,
                0,
                Bound::Upper,
                Some(moves[0]),
                0,
                false,
            );
            let mut passed = board.clone();
            passed.make_null_move();
            searcher.tt.store(
                search_key(&passed),
                32,
                -29_990,
                Bound::Exact,
                None,
                1,
                false,
            );
            assert_eq!(
                searcher.negamax(&mut board.clone(), 8, beta - 1, beta, 0, false),
                Some(0)
            );
            assert_eq!(
                searcher.nodes,
                if beta < 0 { 1 } else { moves.len() as u64 }
            );
        });
    }
}

#[test]
fn r1_repetition_band_null_move_cutoff_returns_beta_instead_of_a_win() {
    // 反復判定の実装前でも、null moveの結果に適用する制限は帯の境界で固定する。
    for score in [29_487, 29_740, 29_743, 29_744, 29_999] {
        assert_eq!(null_move_cutoff_score(score, 123), 123);
    }
    for score in [124, 28_999, 29_486] {
        assert_eq!(null_move_cutoff_score(score, 123), score);
    }
}

#[test]
fn r1_repetition_band_normalizes_initial_and_widened_aspiration_edges() {
    for distance in [49, 50, 51, 100] {
        let mut high = AspirationWindow::initial(5, Some(29_487 - distance), 50);
        let mut low = AspirationWindow::initial(5, Some(-29_487 + distance), 50);
        assert_eq!(high.alpha, 29_487 - distance - 50);
        assert_eq!(low.beta, -29_487 + distance + 50);
        if distance <= 50 {
            assert_eq!(high.beta, INFINITY);
            assert_eq!(low.alpha, -INFINITY);
        } else {
            assert_eq!(high.beta, 29_487 - distance + 50);
            assert_eq!(low.alpha, -29_487 + distance - 50);
        }
        let unchanged_alpha = high.alpha;
        let unchanged_beta = low.beta;
        high.widen_high();
        low.widen_low();
        assert_eq!((high.alpha, high.beta), (unchanged_alpha, INFINITY));
        assert_eq!((low.alpha, low.beta), (-INFINITY, unchanged_beta));
    }
}

#[test]
fn r1_repetition_band_scores_do_not_update_correction_history() {
    let board = bare_kings();
    for score in [29_487, 29_740, 29_743, -29_487, -29_740, -29_743] {
        for bound in [Bound::Exact, Bound::Lower, Bound::Upper] {
            with_root_searcher(&board, &[], |searcher| {
                let key = material_key(&board);
                searcher.correction.update(Color::Black, key, 64, 8);
                let before = searcher.correction.read(Color::Black, key);
                assert_ne!(before, 0);
                // 反復による勝敗の値を、negamaxの出口で使う更新処理へ渡す。
                searcher.update_correction(&board, 2, 0, score, bound, false, None, None);
                assert_eq!(
                    searcher.correction.read(Color::Black, key),
                    before,
                    "{score}, {bound:?}"
                );
            });
        }
    }
    // 直前の通常範囲は引き続き学習対象となる。
    for score in [-29_486, 29_486] {
        with_root_searcher(&board, &[], |searcher| {
            searcher.update_correction(&board, 2, 0, score, Bound::Exact, false, None, None);
            assert_ne!(
                searcher.correction.read(Color::Black, material_key(&board)),
                0
            );
        });
    }
}
