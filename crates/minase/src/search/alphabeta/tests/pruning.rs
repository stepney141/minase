//! 枝刈りと深さの削減を検査する。

use super::*;
use crate::search::REPETITION_BAND_START;
use crate::search::alphabeta::params;
use crate::search::alphabeta::pruning::{
    is_opponent_worsening, is_seek_mate, reverse_futility_margin, seek_mate_null_move_reduction,
};

use crate::search::alphabeta::pruning::{LmrContext, lmr_depth, lmr_research_depth, non_lmr_depth};

#[test]
fn seek_mate_threshold_matches_numeric_examples() {
    // F6の数値表。閾値と、その直前を両符号で検査する。
    for (depth, threshold) in [(2, 26803), (10, 1418), (20, 625), (100, 371)] {
        for sign in [-1, 1] {
            assert!(is_seek_mate(Some(sign * threshold), depth, 100));
            assert!(!is_seek_mate(Some(sign * (threshold - 1)), depth, 100));
        }
    }
    // 深さによる除算を先に切り捨て、歩兵価値で換算した後にも切り捨てる。
    for (pawn, threshold) in [(1000, 14180), (37, 524)] {
        assert!(is_seek_mate(Some(threshold), 10, pawn));
        assert!(!is_seek_mate(Some(threshold - 1), 10, pawn));
    }
}

#[test]
fn seek_mate_requires_previous_iteration_score() {
    for depth in 1..=MAX_PLY {
        assert!(!is_seek_mate(None, depth, 100));
    }
}

#[test]
fn seek_mate_null_move_reduction_subtracts_half_of_total() {
    for (total, reduced) in [(1, 1), (4, 2), (5, 3)] {
        assert_eq!(seek_mate_null_move_reduction(total, true), reduced);
        assert_eq!(seek_mate_null_move_reduction(total, false), total);
    }
}

#[test]
fn seek_mate_limits_reverse_futility_to_depth_five() {
    let board = bare_kings();
    let value = evaluate(test_nnue(), &board);
    // 置換表なしの深さ5,6の余裕値は102,136なので、どちらも200で足りる。
    let beta = value - 200;
    for (seek_mate, depth, pruned) in [(true, 5, true), (true, 6, false), (false, 6, true)] {
        with_root_searcher(&board, &[], |searcher| {
            searcher.seek_mate = seek_mate;
            let score = searcher
                .negamax(&mut board.clone(), depth, beta - 1, beta, 0, false)
                .unwrap();
            if pruned {
                assert_eq!((score, searcher.nodes), (value, 0));
                assert!(searcher.tt.probe(search_key(&board), 0).is_none());
            } else {
                assert!(searcher.nodes > 0, "残り深さ6では子を展開する");
            }
        });
    }
}

#[test]
fn seek_mate_disables_razoring() {
    let board = bare_kings();
    let value = evaluate(test_nnue(), &board);
    for (depth, margin) in [(1, 1606), (2, 2015)] {
        let alpha = value + margin;
        let (quiet_score, quiet_nodes) = run_quiesce(&board, alpha, alpha + 1, 0, &small_tt());
        for seek_mate in [false, true] {
            with_root_searcher(&board, &[], |searcher| {
                searcher.seek_mate = seek_mate;
                let score = searcher
                    .negamax(&mut board.clone(), depth, alpha, alpha + 1, 0, false)
                    .unwrap();
                if seek_mate {
                    assert!(searcher.nodes > quiet_nodes, "通常探索の子を展開する");
                } else {
                    assert_eq!((score, searcher.nodes), (quiet_score, quiet_nodes));
                }
            });
        }
    }
}

/// F4の数値例Aの入力。期待値は指示書の独立した計算による。
pub(super) fn lmr_context() -> LmrContext {
    LmrContext {
        depth: 8,
        index: 10,
        improving: false,
        pv_node: false,
        tt_pv: false,
        cut_node: false,
        tt_hit: None,
        tt_capture: None,
        is_tt_move: false,
        capture: false,
        history: Some(0),
        cutoff_count: 0,
        alpha: 0,
        eval: 0,
        pawn: 100,
    }
}

#[test]
fn lmr_table_follows_logarithmic_rule() {
    let table = lmr_table();
    assert_eq!(params::lmr_divisor(), 203);
    // 延長後の深さはnegamaxの入口で制限し、表はMAX_PLY + 1行に保つ。
    assert_eq!(table.len(), MAX_PLY as usize + 1);
    assert!(table[0].iter().all(|&value| value == 0));
    for row in table {
        assert_eq!(row[0], 0);
        assert_eq!(row[1], 0);
    }
    // 除数2.03で対数積を割り、100倍して切り捨てる。
    for (depth, index, expected) in [(3, 3, 59), (4, 8, 142), (5, 255, 439), (8, 10, 235)] {
        assert_eq!(table[depth][index], expected);
    }
}

#[test]
fn lmr_reduction_matches_f4_examples() {
    let a = lmr_context();
    for (case, context, reduction, depth) in [
        ("A", a, 469, 4),
        (
            "B",
            LmrContext {
                cut_node: true,
                tt_capture: Some(false),
                ..a
            },
            812,
            4,
        ),
        (
            "C",
            LmrContext {
                cut_node: true,
                ..a
            },
            903,
            4,
        ),
        (
            "D",
            LmrContext {
                pv_node: true,
                tt_pv: true,
                tt_hit: Some((1, 8)),
                tt_capture: Some(false),
                ..a
            },
            -99,
            8,
        ),
        (
            "E",
            LmrContext {
                cut_node: true,
                tt_pv: true,
                tt_hit: Some((0, 8)),
                tt_capture: Some(false),
                ..a
            },
            436,
            4,
        ),
        (
            "F",
            LmrContext {
                cutoff_count: 3,
                ..a
            },
            742,
            4,
        ),
        (
            "G",
            LmrContext {
                cutoff_count: 2,
                cut_node: true,
                tt_capture: Some(false),
                ..a
            },
            838,
            4,
        ),
        ("H", LmrContext { alpha: 80, ..a }, 500, 4),
        ("I", LmrContext { alpha: -50, ..a }, 448, 4),
        ("J", LmrContext { alpha: 20, ..a }, 482, 4),
        (
            "K",
            LmrContext {
                improving: true,
                history: Some(params::lmr_history_threshold()),
                ..a
            },
            257,
            5,
        ),
        (
            "L",
            LmrContext {
                tt_capture: Some(true),
                ..a
            },
            586,
            4,
        ),
        (
            "M",
            LmrContext {
                depth: 2,
                index: 1,
                pv_node: true,
                tt_pv: true,
                tt_capture: Some(false),
                ..a
            },
            -203,
            4,
        ),
    ] {
        assert_eq!(lmr_reduction(context), reduction, "{case}");
        assert_eq!(
            lmr_depth(context.depth, reduction, context.pv_node, 0, 8),
            depth,
            "{case}"
        );
    }
    assert_eq!(lmr_depth(2, -203, true, 16, 8), 2, "延長を許す範囲の境界");
    assert_eq!(lmr_depth(2, -203, true, 15, 8), 4);
    assert_eq!(lmr_depth(2, -203, true, 0, 0), 2, "反復深さの初期値0");
}

#[test]
fn non_lmr_depth_matches_f4_examples() {
    let a = LmrContext {
        depth: 6,
        index: 0,
        ..lmr_context()
    };
    for (case, context, reduction, depth) in [
        (
            "N",
            LmrContext {
                cut_node: true,
                is_tt_move: true,
                tt_capture: Some(false),
                history: None,
                ..a
            },
            335,
            5,
        ),
        (
            "O",
            LmrContext {
                cut_node: true,
                ..a
            },
            639,
            3,
        ),
        ("P", a, 178, 5),
        (
            "Q",
            LmrContext {
                is_tt_move: true,
                tt_capture: Some(false),
                history: None,
                ..a
            },
            -66,
            5,
        ),
        (
            "S",
            LmrContext {
                cut_node: true,
                is_tt_move: true,
                tt_capture: Some(false),
                history: None,
                cutoff_count: 2,
                ..a
            },
            574,
            3,
        ),
    ] {
        assert_eq!(lmr_reduction(context), reduction, "{case}");
        assert_eq!(
            non_lmr_depth(5, reduction, context.tt_capture.is_some()),
            depth,
            "{case}"
        );
    }
    assert_eq!(
        non_lmr_depth(2, 639, false),
        1,
        "通常深さ2では2手減らさない"
    );
    for (normal, reduction, expected) in [
        (5, 511, 5),
        (5, 512, 4),
        (5, 536, 4),
        (5, 537, 3),
        (0, 749, 0),
        (1, 749, 0),
    ] {
        assert_eq!(non_lmr_depth(normal, reduction, true), expected);
    }
}

#[test]
fn lmr_research_depth_matches_f4_examples() {
    for (reduced, value, expected) in [
        (4, 80, 8),
        (4, 60, 7),
        (4, 52, 6),
        (7, 80, 7),
        (4, 75, 7),
        (4, 54, 7),
    ] {
        assert_eq!(lmr_research_depth(7, reduced, value, 50, 100), expected);
    }
}

#[test]
fn lmr_history_and_alpha_eval_respect_eligibility() {
    let a = lmr_context();
    let threshold = params::lmr_history_threshold();
    // Aのr0=419に±100を足してからallNodeの項を加える。
    for (history, expected) in [
        (-threshold, 580),
        (-threshold + 1, 469),
        (threshold - 1, 469),
        (threshold, 357),
    ] {
        assert_eq!(
            lmr_reduction(LmrContext {
                history: Some(history),
                ..a
            }),
            expected
        );
    }
    assert_eq!(lmr_reduction(LmrContext { history: None, ..a }), 469);
    assert_eq!(
        lmr_reduction(LmrContext {
            capture: true,
            alpha: 80,
            ..a
        }),
        469
    );
    for alpha in [-REPETITION_BAND_START, REPETITION_BAND_START] {
        assert_eq!(lmr_reduction(LmrContext { alpha, ..a }), 469);
    }
    assert_eq!(
        lmr_reduction(LmrContext {
            cutoff_count: 1,
            ..a
        }),
        469
    );
}

#[test]
fn lmr_large_move_indices_clamp_only_table_column() {
    // 表の255列と256列は同じ。手番号の項だけ6減り、cutNodeにはallNodeの丸めがない。
    let a = LmrContext {
        index: 255,
        cut_node: true,
        ..lmr_context()
    };
    assert_eq!(
        lmr_reduction(LmrContext { index: 256, ..a }),
        lmr_reduction(a) - 6
    );
}

// 係数の変更はtuningの子プロセス内に閉じ込める。
#[cfg(feature = "tuning")]
pub(super) fn lmr_max_reduction_parameter_sets_cap() {
    let reduction = lmr_reduction(lmr_context());
    assert_eq!(lmr_depth(8, reduction, false, 0, 8), 4);
    params::set("LmrMaxReduction", 1).unwrap();
    assert_eq!(lmr_depth(8, reduction, false, 0, 8), 6);
    assert_eq!(
        lmr_depth(2, -203, true, 0, 8),
        4,
        "減深の上限は延長に掛けない"
    );
    params::set("LmrMaxReduction", 8).unwrap();
    assert_eq!(lmr_depth(8, reduction, false, 0, 8), 3, "例Aの上限8");
    params::set("LmrMaxReduction", 3).unwrap();
    params::set("LmrDeeperMargin", 0).unwrap();
    // 値52はbest50より高く、浅くする閾値54より低い。独立な両条件が相殺する。
    assert_eq!(lmr_research_depth(7, 4, 52, 50, 100), 7);
    params::set("LmrDeeperMargin", 25).unwrap();
}

// futilityは補正後の静的評価と余裕値の和がα以下なら後続の静かな手を飛ばす。
// 最善値が負の詰み帯にある間は少なくとも1手を読み、合法手なしの詰みと誤認しない。
#[test]
fn futility_reduces_quiet_nodes_without_false_mate() {
    let position = bare_kings();
    let nnue = test_nnue();
    let static_eval = evaluate(nnue, &position);
    let alpha = static_eval + 3 * nnue.pawn_value();
    assert!(!royal_under_attack(&position));
    assert!(
        legal_moves(&position)
            .iter()
            .all(|&mv| { !mv.promote && move_order_key(&position, nnue, mv).is_none() })
    );
    // 根付近とply=2の両方で、最初の手を保護することを検査する。
    for (depth, ply) in (1..=3).flat_map(|depth| [0, 2].map(|ply| (depth, ply))) {
        let table = small_tt();
        let (score, pruned_nodes) = run_negamax(&position, depth, alpha, alpha + 1, ply, &table);
        let (_, full_nodes) = run_negamax(&position, depth, alpha, alpha + 2, ply, &small_tt());
        assert!(pruned_nodes > 0, "最初の手は探索する: depth={depth}");
        assert!(
            pruned_nodes < full_nodes,
            "depth={depth}: {pruned_nodes} >= {full_nodes}"
        );
        assert!(score.abs() < MATE_THRESHOLD, "depth={depth}: score={score}");
        let hit = table.probe(search_key(&position), ply).unwrap();
        assert_eq!(hit.score, score);
        assert!(hit.score.abs() < MATE_THRESHOLD);
        if depth == 1 {
            let leaf_scores: Vec<_> = legal_moves(&position)
                .into_iter()
                .map(|mv| {
                    let mut child = position.clone();
                    child.make_move_unchecked(mv, engine_rules());
                    -evaluate(nnue, &child)
                })
                .collect();
            assert!(
                (*leaf_scores.iter().min().unwrap()..=*leaf_scores.iter().max().unwrap())
                    .contains(&score)
            );
        }
    }
}

// 先頭の記録手が王駒を失っても、
// 後続の安全な静かな手を読み、返り値にも置換表にも誤った詰み値を残さない。
#[test]
fn futility_searches_safe_quiets_after_losing_tt_move() {
    let position = position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(10, 6), Color::White, PieceKind::Rook),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    );
    let nnue = test_nnue();
    let alpha = evaluate(nnue, &position) + 3 * nnue.pawn_value();
    let losing_move = Move {
        from: sq(11, 0),
        mid: None,
        to: sq(10, 0),
        promote: false,
    };
    assert!(legal_moves(&position).contains(&losing_move));
    assert!(!royal_under_attack(&position));
    assert!(
        legal_moves(&position)
            .iter()
            .all(|&mv| { !mv.promote && move_order_key(&position, nnue, mv).is_none() })
    );
    let ply = 5;
    let mut child = position.clone();
    child.make_move_unchecked(losing_move, engine_rules());
    let (opponent_score, _) = run_negamax(&child, 1, -INFINITY, INFINITY, ply + 1, &small_tt());
    assert!(opponent_score >= MATE_THRESHOLD, "先頭手は王駒を失う");

    let table = small_tt();
    // 子の全窓探索で確認した詰み値を固定し、親で後続手を読む条件だけを検査する。
    // F5ではβを超える値に要求深さより1深い記録が要る。
    table.store(
        search_key(&child),
        2,
        opponent_score,
        Bound::Exact,
        None,
        ply + 1,
        false,
    );
    table.store(
        search_key(&position),
        0,
        0,
        Bound::Upper,
        Some(losing_move),
        ply,
        false,
    );
    let (score, _) = run_negamax(&position, 2, alpha, alpha + 1, ply, &table);
    assert!(score.abs() < MATE_THRESHOLD, "score={score}");
    let hit = table.probe(search_key(&position), ply).unwrap();
    assert!(hit.score.abs() < MATE_THRESHOLD);
    assert_ne!(hit.best_move, Some(losing_move));
}

// 静かな記録手を先に探索して条件を成立させても、
// 後続の捕獲手と非捕獲の成る手は展開し、記録手だけの場合より探索ノードが増える。
#[test]
fn futility_searches_captures_and_promotions_after_quiet_tt_move() {
    for (board, promotion) in [
        (
            position(
                Color::Black,
                &[
                    (sq(0, 11), Color::White, PieceKind::King),
                    (sq(5, 4), Color::White, PieceKind::Pawn),
                    (sq(5, 3), Color::Black, PieceKind::Rook),
                    (sq(11, 0), Color::Black, PieceKind::King),
                ],
            ),
            false,
        ),
        (
            position(
                Color::Black,
                &[
                    (sq(0, 11), Color::White, PieceKind::King),
                    (sq(5, 7), Color::Black, PieceKind::Pawn),
                    (sq(11, 0), Color::Black, PieceKind::King),
                ],
            ),
            true,
        ),
    ] {
        let position = board;
        let nnue = test_nnue();
        let alpha = evaluate(nnue, &position) + 3 * nnue.pawn_value();
        let tt_move = Move {
            from: sq(11, 0),
            mid: None,
            to: sq(11, 1),
            promote: false,
        };
        assert!(legal_moves(&position).contains(&tt_move));
        assert!(!royal_under_attack(&position));
        let protected: Vec<_> = legal_moves(&position)
            .into_iter()
            .filter(|&mv| {
                if promotion {
                    mv.promote && move_order_key(&position, nnue, mv).is_none()
                } else {
                    move_order_key(&position, nnue, mv).is_some()
                }
            })
            .collect();
        assert!(!protected.is_empty());
        let table = small_tt();
        table.store(
            search_key(&position),
            0,
            evaluate(nnue, &position),
            Bound::Upper,
            Some(tt_move),
            0,
            false,
        );
        let (score, nodes) = run_negamax(&position, 1, alpha, alpha + 1, 0, &table);
        assert!(score.abs() < MATE_THRESHOLD);
        assert!(nodes > 1, "記録手の後にも探索する");
    }
}

// SEEが負の捕獲しかなくても先頭手は読み、全手の枝刈りによる偽の詰みを防ぐ。
// この局面は王駒と周囲の歩兵が動けず、仲人が歩兵を取って取り返される2手だけが合法である。
// 判断の理由は docs/adr/prune-losing-captures-by-see-at-shallow-nodes.md にある。
#[test]
fn see_pruning_searches_first_capture_without_false_mate() {
    let board = position(
        Color::Black,
        &[
            (sq(0, 11), Color::Black, PieceKind::King),
            (sq(0, 10), Color::Black, PieceKind::Pawn),
            (sq(1, 10), Color::Black, PieceKind::Pawn),
            (sq(1, 11), Color::Black, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::GoBetween),
            (sq(5, 4), Color::White, PieceKind::Pawn),
            (sq(5, 6), Color::White, PieceKind::Pawn),
            (sq(5, 3), Color::White, PieceKind::GoBetween),
            (sq(5, 7), Color::White, PieceKind::Pawn),
        ],
    );
    let nnue = test_nnue();
    let moves = legal_moves(&board);
    assert_eq!(moves.len(), 2);
    assert!(!royal_under_attack(&board));
    assert!(moves.iter().all(|&mv| {
        move_order_key(&board, nnue, mv).is_some()
            && see_prunes(&board, engine_rules(), nnue, mv, 0)
    }));
    let alpha = evaluate(nnue, &board) + 3 * nnue.pawn_value();
    let table = small_tt();
    let ply = 5;
    let (score, nodes) = run_negamax(&board, 1, alpha, alpha + 1, ply, &table);
    assert!(score.abs() < MATE_THRESHOLD, "score={score}");
    let hit = table.probe(search_key(&board), ply).unwrap();
    // 保存された先頭捕獲の取り返しを別に数え、通常探索の展開が1手だけであることを検査する。
    let mut child = board.clone();
    child.make_move_unchecked(hit.best_move.unwrap(), engine_rules());
    let (_, reply_nodes) = run_quiesce(&child, -alpha - 1, -alpha, ply + 1, &small_tt());
    assert_eq!(nodes, 1 + reply_nodes, "後続の捕獲は枝刈りする");
    assert_eq!(hit.score, score);
    assert!(hit.score.abs() < MATE_THRESHOLD);
    assert_eq!(hit.bound, Bound::Upper);
}

// 奔王が飛車の筋を開ける記録手は負けるが、
// 飛車を取る手は交換で損をしても王将を守るので、負の詰み帯の後にも読む。
#[test]
fn see_pruning_searches_safe_capture_after_losing_tt_move() {
    let board = position(
        Color::Black,
        &[
            (sq(5, 0), Color::Black, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::FreeKing),
            (sq(4, 3), Color::White, PieceKind::Pawn),
            (sq(5, 10), Color::White, PieceKind::Rook),
            (sq(4, 10), Color::White, PieceKind::Rook),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    let nnue = test_nnue();
    let losing_move = Move {
        from: sq(5, 3),
        mid: None,
        to: sq(4, 3),
        promote: false,
    };
    let safe_capture = Move {
        to: sq(5, 10),
        ..losing_move
    };
    let moves = legal_moves(&board);
    assert!(moves.contains(&losing_move));
    assert!(moves.contains(&safe_capture));
    assert!(!royal_under_attack(&board));
    assert!(see_prunes(&board, engine_rules(), nnue, safe_capture, 200));
    let alpha = evaluate(nnue, &board) + 3 * nnue.pawn_value();
    let ply = 5;
    let mut child = board.clone();
    child.make_move_unchecked(losing_move, engine_rules());
    let (opponent_score, _) = run_negamax(&child, 1, -INFINITY, INFINITY, ply + 1, &small_tt());
    assert!(opponent_score >= MATE_THRESHOLD);
    let table = small_tt();
    // 子の全窓探索で確認した詰み値を固定し、親で後続手を読む条件だけを検査する。
    // F5ではβを超える値に要求深さより1深い記録が要る。
    table.store(
        search_key(&child),
        2,
        opponent_score,
        Bound::Exact,
        None,
        ply + 1,
        false,
    );
    table.store(
        search_key(&board),
        0,
        0,
        Bound::Upper,
        Some(losing_move),
        ply,
        false,
    );
    let (score, _) = run_negamax(&board, 2, alpha, alpha + 1, ply, &table);
    assert!(score.abs() < MATE_THRESHOLD, "score={score}");
    let hit = table.probe(search_key(&board), ply).unwrap();
    assert_eq!(hit.score, score);
    assert!(hit.score.abs() < MATE_THRESHOLD);
    assert_eq!(hit.best_move, Some(safe_capture));
    assert_eq!(hit.bound, Bound::Upper);
}

// 深さ2以上では、展開した捕獲の子局面に
// 通常探索の値が保存される。先頭の静かな記録手で負の詰み帯を脱した後を検査する。
#[test]
fn see_pruning_skips_losing_capture_only_at_eligible_nodes() {
    let nnue = test_nnue();
    for (depth, width, capture_is_tt, attacked, victim, expanded) in [
        (2, 1, false, false, PieceKind::Pawn, false),
        (3, 1, false, false, PieceKind::Pawn, false),
        (2, 1, true, false, PieceKind::Pawn, true),
        (2, 2, false, false, PieceKind::Pawn, true),
        (4, 1, false, false, PieceKind::Pawn, true),
        (2, 1, false, true, PieceKind::Pawn, true),
        (2, 1, false, false, PieceKind::Bishop, true),
        (3, 1, false, false, PieceKind::Bishop, false),
    ] {
        let mut pieces = vec![
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::Rook),
            (sq(5, 4), Color::White, victim),
            (sq(5, 5), Color::White, PieceKind::Pawn),
        ];
        if attacked {
            pieces.push((sq(11, 6), Color::White, PieceKind::Rook));
        }
        let board = position(Color::Black, &pieces);
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
        assert_eq!(royal_under_attack(&board), attacked);
        assert!(legal_moves(&board).contains(&capture));
        assert!(legal_moves(&board).contains(&quiet));
        assert!(see_prunes(&board, engine_rules(), nnue, capture, 0));
        assert_eq!(
            see_prunes(&board, engine_rules(), nnue, capture, 200),
            victim == PieceKind::Pawn,
        );
        let alpha = evaluate(nnue, &board) + 3 * nnue.pawn_value();
        let table = small_tt();
        let tt_move = if capture_is_tt { capture } else { quiet };
        table.store(
            search_key(&board),
            0,
            0,
            Bound::Upper,
            Some(tt_move),
            0,
            false,
        );
        let (score, _) = run_negamax(&board, depth, alpha, alpha + width, 0, &table);
        assert!(score.abs() < MATE_THRESHOLD);
        let mut child = board.clone();
        child.make_move_unchecked(capture, engine_rules());
        assert_eq!(
            table.probe(search_key(&child), 1).is_some(),
            expanded,
            "depth={depth}, width={width}, capture_is_tt={capture_is_tt}, attacked={attacked}, victim={victim:?}"
        );
        assert_eq!(table.probe(search_key(&board), 0).unwrap().score, score);
    }
}

// 経由升を持つ獅子の捕獲は規則依存なので展開する。
#[test]
fn see_pruning_searches_rule_dependent_capture() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::Lion),
            (sq(5, 4), Color::White, PieceKind::Pawn),
        ],
    );
    let capture = Move {
        from: sq(5, 3),
        mid: Some(sq(5, 4)),
        to: sq(5, 3),
        promote: false,
    };
    let quiet = Move {
        from: sq(11, 0),
        mid: None,
        to: sq(10, 0),
        promote: false,
    };
    let nnue = test_nnue();
    assert!(legal_moves(&board).contains(&capture));
    assert!(!see_prunes(&board, engine_rules(), nnue, capture, 200));
    assert!(!royal_under_attack(&board));
    let static_eval = evaluate(nnue, &board);
    let alpha = static_eval + 3 * nnue.pawn_value();
    let table = small_tt();
    // 他の捕獲による先行カットオフを避け、経由升を持つ捕獲まで探索を進める。
    // 対象と同じ盤面になる手は保存せず、実際に探索された結果を観測する。
    let mut target = board.clone();
    target.make_move_unchecked(capture, engine_rules());
    for mv in legal_moves(&board) {
        let mut child = board.clone();
        child.make_move_unchecked(mv, engine_rules());
        if search_key(&child) != search_key(&target) {
            table.store(
                search_key(&child),
                1,
                -static_eval,
                Bound::Exact,
                None,
                1,
                false,
            );
        }
    }
    table.store(
        search_key(&board),
        0,
        static_eval,
        Bound::Upper,
        Some(quiet),
        0,
        false,
    );
    let (score, _) = run_negamax(&board, 2, alpha, alpha + 1, 0, &table);
    assert!(score > static_eval);
    let hit = table.probe(search_key(&board), 0).unwrap();
    assert_eq!(hit.score, score);
    let best = hit.best_move.unwrap();
    assert!(best.mid.is_some(), "規則依存の捕獲を探索した値で更新する");
    let mut child = board.clone();
    child.make_move_unchecked(best, engine_rules());
    assert_eq!(search_key(&child), search_key(&target));
}

// 最後の王駒の捕獲は詰みとして返し、保存する。
#[test]
fn see_pruning_searches_last_royal_capture_after_quiet_tt_move() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(5, 4), Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::Rook),
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
    assert!(legal_moves(&board).contains(&capture));
    assert!(!royal_under_attack(&board));
    assert!(captures_last_royal(&board, capture));
    let ply = 5;
    let table = small_tt();
    table.store(
        search_key(&board),
        0,
        evaluate(test_nnue(), &board),
        Bound::Upper,
        Some(quiet),
        ply,
        false,
    );
    let alpha = evaluate(test_nnue(), &board) + 3 * test_nnue().pawn_value();
    let (score, _) = run_negamax(&board, 1, alpha, alpha + 1, ply, &table);
    assert_eq!(score, MATE - (ply + 1) as i32);
    let hit = table.probe(search_key(&board), ply).unwrap();
    assert_eq!(hit.score, score);
    assert_eq!(hit.best_move, Some(capture));
}

// F1の仕様表の数値例。2項の独立した丸めとi64の積も検査する。
#[test]
fn reverse_futility_margin_matches_numeric_examples() {
    let pawn = test_nnue().pawn_value();
    assert_eq!(pawn, 100);
    for (depth, tt_hit, improving, opponent_worsening, correction, expected) in [
        (1, true, false, false, 0, 24),
        (1, false, false, false, 0, 14),
        (2, true, false, true, 0, 43),
        (3, true, true, false, 0, 7),
        (1, true, true, false, 0, -41),
        (10, true, false, false, 0, 410),
        (18, false, false, false, 0, 558),
        (1, true, false, false, -50, 57),
        (1, true, false, false, 7, 28),
        (1, true, true, false, 7, -37),
    ] {
        assert_eq!(
            reverse_futility_margin(
                pawn,
                depth,
                tt_hit,
                improving,
                opponent_worsening,
                correction
            ),
            expected,
            "depth={depth}, tt_hit={tt_hit}, improving={improving}, opponent_worsening={opponent_worsening}, correction={correction}"
        );
    }
    // 1,000,000×41×1767/10000=7,244,700。積72,447,000,000はi32の範囲を超える。
    assert_eq!(
        reverse_futility_margin(1_000_000, 18, true, false, true, 0),
        7_244_700
    );
}

#[test]
fn opponent_worsening_compares_previous_ply_and_is_true_at_root() {
    for (previous, expected) in [
        (Some(-5), true),
        (Some(-10), false),
        (Some(-20), false),
        (None, true),
    ] {
        assert_eq!(is_opponent_worsening(10, previous), expected);
    }
}

// 根ではopponentWorseningが真。置換表なしの深さ1,2,4,18の余裕値は9,26,73,547。
#[test]
fn reverse_futility_returns_static_eval_at_margin_without_storing() {
    let board = bare_kings();
    let value = evaluate(test_nnue(), &board);
    // F5ではallNodeにIIRを適用しないので、要求深さをそのまま使う。
    for (requested_depth, margin) in [(1, 9), (2, 26), (4, 73), (18, 547)] {
        let table = small_tt();
        let beta = value - margin;
        assert_eq!(
            run_negamax(&board, requested_depth, beta - 1, beta, 0, &table),
            (value, 0)
        );
        assert!(table.probe(search_key(&board), 0).is_none());
    }
}

// 王駒への利き、窓の幅、帯の境界、余裕値不足、および残り深さ19では子を展開する。
#[test]
fn reverse_futility_exclusions_search_children() {
    use crate::search::REPETITION_BAND_START as B;
    let board = bare_kings();
    let value = evaluate(test_nnue(), &board);
    let beta = value - 9;
    let attacked = position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(11, 6), Color::White, PieceKind::Rook),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    );
    let attacked_beta = evaluate(test_nnue(), &attacked) - 9;
    for (position, attacked, depth, alpha, beta, reason) in [
        (&board, false, 1, beta - 2, beta, "幅2のPV窓"),
        (
            &board,
            false,
            1,
            -MATE_THRESHOLD - 1,
            -MATE_THRESHOLD,
            "負の詰み帯",
        ),
        (
            &board,
            false,
            1,
            MATE_THRESHOLD - 1,
            MATE_THRESHOLD,
            "正の詰み帯",
        ),
        (&board, false, 1, -B - 1, -B, "βが負の専用帯"),
        (&board, false, 1, -B, -B + 1, "αだけが専用帯"),
        (&board, false, 1, B - 1, B, "βが正の専用帯"),
        (&board, false, 1, beta, beta + 1, "余裕値に1不足"),
        (&board, false, 19, value - 579, value - 578, "残り深さ19"),
        (
            &attacked,
            true,
            1,
            attacked_beta - 1,
            attacked_beta,
            "王駒への利き",
        ),
    ] {
        assert_eq!(royal_under_attack(position), attacked, "{reason}");
        assert!(
            run_negamax(position, depth, alpha, beta, 0, &small_tt()).1 > 0,
            "{reason}"
        );
    }
}

#[test]
fn reverse_futility_validates_tt_move_and_excludes_tt_pv() {
    let board = position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 3), Color::White, PieceKind::Pawn),
            (sq(5, 1), Color::Black, PieceKind::Rook),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    );
    let capture = step(sq(5, 1), sq(5, 3));
    let quiet = step(sq(5, 1), sq(5, 2));
    // 敵駒が移動先にあっても、空升からの手は合法性の検査で除かれる。
    let invalid = step(sq(3, 3), sq(5, 3));
    let invalid_quiet = step(sq(3, 3), sq(3, 4));
    assert!(legal_moves(&board).contains(&capture));
    assert!(legal_moves(&board).contains(&quiet));
    assert!(!legal_moves(&board).contains(&invalid));
    assert!(!legal_moves(&board).contains(&invalid_quiet));
    assert!(!royal_under_attack(&board));
    let value = evaluate(test_nnue(), &board);
    // 深さ1、照合成功、opponentWorseningありで100×24×67/10000→16。
    let beta = value - 16;
    for (mv, is_pv, pruned) in [
        (None, false, true),
        (None, true, false),
        (Some(capture), false, true),
        (Some(capture), true, false),
        (Some(quiet), false, false),
        (Some(invalid), false, true),
        (Some(invalid_quiet), false, true),
    ] {
        let table = small_tt();
        table.store(search_key(&board), 0, value, Bound::Upper, mv, 0, is_pv);
        let (score, nodes) = run_negamax(&board, 1, beta - 1, beta, 0, &table);
        if pruned {
            assert_eq!((score, nodes), (value, 0));
            let hit = table.probe(search_key(&board), 0).unwrap();
            assert_eq!((hit.depth, hit.score, hit.best_move), (0, value, mv));
        } else {
            assert!(nodes > 0, "mv={mv:?}, is_pv={is_pv}");
        }
    }
}

#[test]
fn reverse_futility_requires_eval_at_least_beta_with_negative_margin() {
    let board = bare_kings();
    for at_beta in [false, true] {
        with_root_searcher(&board, &[], |searcher| {
            let value = evaluate(searcher.nnue, &board);
            searcher.accumulators[2] = searcher.accumulators[0].clone();
            searcher.material_keys[2] = searcher.material_keys[0];
            searcher.static_evals[0] = value - 1;
            searcher.static_evals[1] = -value;
            // improvingだけが真で、置換表なしの余裕値は100×14×(-172)/10000→-24。
            let beta = value + i32::from(!at_beta);
            assert!(value + 24 >= beta);
            let score = searcher
                .negamax(&mut board.clone(), 1, beta - 1, beta, 2, false)
                .unwrap();
            if at_beta {
                assert_eq!((score, searcher.nodes), (value, 0));
            } else {
                assert!(searcher.nodes > 0);
            }
        });
    }
}

// 置換表の打ち切りをreverse futilityより先に適用する。
#[test]
fn reverse_futility_preserves_tt_cutoff_priority() {
    let board = bare_kings();
    let beta = evaluate(test_nnue(), &board) - 16;
    let table = small_tt();
    table.store(
        search_key(&board),
        1,
        beta - 1,
        Bound::Exact,
        None,
        0,
        false,
    );
    assert_eq!(
        run_negamax(&board, 1, beta - 1, beta, 0, &table),
        (beta - 1, 0)
    );
}

// razoringは深さ1〜2で「静的評価 + 余裕値 ≤ α」なら静止探索を試し、その値もα以下なら通常探索を省く。
// 捕獲候補のない局面では静的評価を返し、置換表へ保存しない。
#[test]
fn razoring_returns_quiescence_at_the_margin_boundary() {
    assert_eq!(test_nnue().pawn_value(), 100, "固定ネットの歩兵価値");
    assert_eq!(params::razoring_margin1(), 1606, "数値例の前提");
    assert_eq!(params::razoring_margin2(), 2015, "数値例の前提");
    // 100×1606/100=1606、100×2015/100=2015。
    let position = bare_kings();
    let nnue = test_nnue();
    let static_eval = evaluate(nnue, &position);
    assert!(!royal_under_attack(&position));
    for (depth, margin) in [(1, 1606), (2, 2015)] {
        for excess in [0, nnue.pawn_value()] {
            let alpha = static_eval + margin + excess;
            let (quiet_score, quiet_nodes) =
                run_quiesce(&position, alpha, alpha + 1, 0, &small_tt());
            assert_eq!(quiet_score, static_eval);
            let table = small_tt();
            let (score, nodes) = run_negamax(&position, depth, alpha, alpha + 1, 0, &table);
            let (_, full_nodes) = run_negamax(&position, depth, alpha, alpha + 2, 0, &small_tt());
            assert_eq!(score, quiet_score);
            assert_eq!(nodes, quiet_nodes, "通常探索の手を展開しない");
            assert!(nodes < full_nodes);
            assert!(
                table.probe(search_key(&position), 0).is_none(),
                "捕獲のない静止探索は置換表に保存しない"
            );
        }
        let alpha = static_eval + margin - 1;
        let (_, quiet_nodes) = run_quiesce(&position, alpha, alpha + 1, 0, &small_tt());
        let (_, nodes) = run_negamax(&position, depth, alpha, alpha + 1, 0, &small_tt());
        assert!(nodes > quiet_nodes, "余裕値に1足りなければ通常探索する");
    }
}

// 静止探索の上界がαと等しい場合も打ち切る。
#[test]
fn razoring_accepts_quiescence_equal_to_alpha() {
    assert_eq!(test_nnue().pawn_value(), 100, "固定ネットの歩兵価値");
    assert_eq!(params::razoring_margin1(), 1606, "数値例の前提");
    assert_eq!(params::razoring_margin2(), 2015, "数値例の前提");
    // 100×1606/100=1606、100×2015/100=2015。
    let position = position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 4), Color::White, PieceKind::FreeKing),
            (sq(5, 3), Color::Black, PieceKind::Rook),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    );
    let nnue = test_nnue();
    for (depth, margin) in [(1, 1606), (2, 2015)] {
        let alpha = evaluate(nnue, &position) + margin;
        let table = small_tt();
        table.store(
            search_key(&position),
            0,
            alpha,
            Bound::Upper,
            None,
            0,
            false,
        );
        let (quiet_score, quiet_nodes) = run_quiesce(&position, alpha, alpha + 1, 0, &table);
        assert_eq!(quiet_score, alpha);
        let (score, nodes) = run_negamax(&position, depth, alpha, alpha + 1, 0, &table);
        assert_eq!(score, alpha);
        assert_eq!(nodes, quiet_nodes);
    }
}

// PV窓、詰み帯、深さ3では通常探索を行う。
#[test]
fn razoring_exclusions_search_children() {
    assert_eq!(test_nnue().pawn_value(), 100, "固定ネットの歩兵価値");
    assert_eq!(params::razoring_margin1(), 1606, "数値例の前提");
    assert_eq!(params::razoring_margin2(), 2015, "数値例の前提");
    // 100×1606/100=1606、100×2015/100=2015、共通の窓には大きい方の2015を使う。
    let position = bare_kings();
    let nnue = test_nnue();
    let alpha = evaluate(nnue, &position) + 2015;
    let attacked = minase_core::test_util::position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(11, 6), Color::White, PieceKind::Rook),
        ],
    );
    let attacked_eval = evaluate(nnue, &attacked);
    for (position, attacked, depth, alpha, beta) in [
        (&position, false, 1, alpha, alpha + 2),
        (&position, false, 2, alpha, alpha + 2),
        (&position, false, 1, MATE_THRESHOLD - 1, MATE_THRESHOLD),
        (&position, false, 2, MATE_THRESHOLD - 1, MATE_THRESHOLD),
        (&position, false, 1, MATE_THRESHOLD, MATE_THRESHOLD + 1),
        (&position, false, 2, MATE_THRESHOLD, MATE_THRESHOLD + 1),
        (&position, false, 1, -MATE_THRESHOLD, -MATE_THRESHOLD + 1),
        (&position, false, 2, -MATE_THRESHOLD, -MATE_THRESHOLD + 1),
        (&position, false, 3, alpha, alpha + 1),
        (
            &attacked,
            true,
            1,
            attacked_eval + 1606,
            attacked_eval + 1606 + 1,
        ),
        (
            &attacked,
            true,
            2,
            attacked_eval + 2015,
            attacked_eval + 2015 + 1,
        ),
    ] {
        let (quiet_score, quiet_nodes) = run_quiesce(position, alpha, beta, 0, &small_tt());
        assert_eq!(
            royal_under_attack(position),
            attacked,
            "depth={depth}, alpha={alpha}, attacked={attacked}"
        );
        if attacked {
            assert!(
                quiet_score <= alpha,
                "depth={depth}, alpha={alpha}, attacked={attacked}"
            );
        }
        let table = small_tt();
        if !attacked {
            table.store(
                search_key(position),
                0,
                0,
                Bound::Upper,
                Some(legal_moves(position)[0]),
                0,
                false,
            );
        }
        let (_, nodes) = run_negamax(position, depth, alpha, beta, 0, &table);
        assert!(
            nodes > quiet_nodes,
            "depth={depth}, alpha={alpha}, beta={beta}, attacked={attacked}: {nodes} <= {}",
            quiet_nodes
        );
    }
}

// 捕獲でαを上回れば通常探索へ進み、PV窓の探索と同じ値を返す。
#[test]
fn razoring_continues_normal_search_after_recovering_capture() {
    assert_eq!(test_nnue().pawn_value(), 100, "固定ネットの歩兵価値");
    assert_eq!(params::razoring_margin1(), 1606, "数値例の前提");
    // 100×1606/100=1606。
    let position = position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 4), Color::White, PieceKind::FreeKing),
            (sq(5, 3), Color::Black, PieceKind::Rook),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    );
    let nnue = test_nnue();
    let alpha = evaluate(nnue, &position) + 1606;
    assert!(!royal_under_attack(&position));
    let (quiet_score, quiet_nodes) = run_quiesce(&position, alpha, alpha + 1, 0, &small_tt());
    assert!(quiet_score > alpha, "無防備な奔王の捕獲で余裕値を取り戻す");
    // 深さ1なら子は静止探索となり、子ノードでの枝刈りの差を比較へ持ち込まない。
    let table = small_tt();
    let (score, nodes) = run_negamax(&position, 1, alpha, alpha + 1, 0, &table);
    let (full_score, _) = run_negamax(&position, 1, alpha, alpha + 2, 0, &small_tt());
    assert_eq!(score, full_score);
    assert!(nodes > quiet_nodes, "静止探索の後にも通常探索を行う");
    let hit = table.probe(search_key(&position), 0).unwrap();
    assert_eq!(hit.depth, 1);
    assert!(hit.best_move.is_some());
}

// null moveの追加減深は `(静的評価 − β) × NullMoveEvalScale / 100 / (2 × 歩兵価値)` を0〜3に制限する。
// 各整数境界の直前と一致で段階が変わり、深さによる基本量へ加算する。
#[test]
fn null_move_reduction_scales_with_eval_surplus_and_depth() {
    assert_eq!(params::null_move_base(), 3617, "数値例の前提");
    assert_eq!(params::null_move_slope(), 257, "数値例の前提");
    let beta = 137;
    for pawn_value in [37, 100] {
        let scale = params::null_move_eval_scale();
        assert!(scale > 0);
        let boundary = |extra| (extra * 2 * pawn_value * 100 + scale - 1) / scale;
        // (3617+d×257)/1200はd=1〜4で3、5〜9で4、10〜12で5。
        // 境界の分子はd=4で4645、d=5で4902、d=10で6187。
        for (depth, base) in [
            (1, 3),
            (2, 3),
            (3, 3),
            (4, 3),
            (5, 4),
            (6, 4),
            (7, 4),
            (8, 4),
            (9, 4),
            (10, 5),
            (11, 5),
            (12, 5),
        ] {
            for (difference, extra) in [
                (-6 * pawn_value, 0),
                (-1, 0),
                (0, 0),
                (boundary(1) - 1, 0),
                (boundary(1), 1),
                (boundary(2) - 1, 1),
                (boundary(2), 2),
                (boundary(3) - 1, 2),
                (boundary(3), 3),
                (boundary(4), 3),
            ] {
                assert_eq!(
                    null_move_reduction(depth, beta + difference, beta, pawn_value),
                    base + extra,
                    "depth={depth}, difference={difference}, pawn_value={pawn_value}"
                );
            }
        }
    }
}

// F5でIIRを行わないallNodeを深さ6で読み、静的評価の余裕による追加減深でnull move後の探索が静止探索まで縮む。
// 通常探索の実着手を省くためノード数も減る。
#[test]
fn null_move_reduction_reduces_nodes_with_large_eval_surplus() {
    assert_eq!(test_nnue().pawn_value(), 100, "固定ネットの歩兵価値");
    assert_eq!(params::null_move_eval_scale(), 56, "数値例の前提");
    // 追加減深1の境界はceil(2×100×100/56)=358。
    let position = position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 1), Color::Black, PieceKind::GoldGeneral),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    );
    let nnue = test_nnue();
    let beta = evaluate(nnue, &position) - 358;
    let mut passed = position.clone();
    passed.make_null_move();
    let (reply_score, reply_nodes) = run_negamax(&passed, 1, -beta, -beta + 1, 1, &small_tt());
    assert!(-reply_score >= beta);
    let table = small_tt();
    // 判定用の評価値だけをβ未満にしてreverse futilityを避け、null moveの減深を検査する。
    table.store(
        search_key(&position),
        0,
        beta - 1,
        Bound::Upper,
        None,
        0,
        false,
    );
    let (score, nodes) = run_negamax(&position, 6, beta - 1, beta, 0, &table);
    assert!(score >= beta && score.abs() < MATE_THRESHOLD);
    let hit = table.probe(search_key(&position), 0).unwrap();
    assert_eq!(
        (hit.depth, hit.score),
        (0, beta - 1),
        "null moveの打ち切りは保存しない"
    );
    assert!(
        nodes < reply_nodes,
        "nodes={nodes}, base_nodes={reply_nodes}"
    );
}

// 補正が判定用の評価値を動かすと、補正前の値では成立した枝刈りも抑止する。
#[test]
fn correction_can_prevent_reverse_futility_and_razoring() {
    assert_eq!(test_nnue().pawn_value(), 100, "固定ネットの歩兵価値");
    assert_eq!(params::razoring_margin1(), 1606, "数値例の前提");
    // reverse futilityの余裕値は100×14×67/10000→9、razoringは100×1606/100=1606。
    let position = bare_kings();
    for reverse in [true, false] {
        with_root_searcher(&position, &[], |searcher| {
            let side = position.side_to_move();
            let key = material_key(&position);
            let difference = if reverse { -1000 } else { 1000 };
            searcher.correction.update(side, key, difference, 8);
            assert_ne!(searcher.correction.read(side, key), 0);
            let raw = evaluate(searcher.nnue, &position);
            let alpha = if reverse { raw - 9 - 1 } else { raw + 1606 };
            let mut board = position.clone();
            let score = searcher
                .negamax(&mut board, 1, alpha, alpha + 1, 0, false)
                .unwrap();
            assert!(searcher.nodes > 0, "補正により通常探索へ進む");
            let hit = searcher.tt.probe(search_key(&position), 0).unwrap();
            assert_eq!(hit.depth, 1);
            assert_eq!(hit.score, score);
        });
    }
}

// F2の数値例。補正後の評価は詰み帯と専用の帯へ入れない。
#[test]
fn corrected_static_eval_clamps_to_evaluation_limit() {
    use crate::eval::nnue::EVALUATION_LIMIT;
    use crate::search::alphabeta::pruning::corrected_static_eval;
    assert_eq!(EVALUATION_LIMIT, 28_999);
    for (raw, correction, expected) in [
        (100, 30, 130),
        (0, -30, -30),
        (28_990, 50, 28_999),
        (-28_990, -50, -28_999),
    ] {
        assert_eq!(corrected_static_eval(raw, correction), expected);
    }
}

#[test]
fn pruning_eval_selects_only_directional_bounds_outside_decisive_bands() {
    use crate::search::REPETITION_BAND_START as B;
    use crate::search::alphabeta::pruning::pruning_eval;
    for (hit, expected) in [
        (None, 100),
        (Some((150, Bound::Lower)), 150),
        (Some((150, Bound::Exact)), 150),
        (Some((150, Bound::Upper)), 100),
        (Some((50, Bound::Upper)), 50),
        (Some((50, Bound::Exact)), 50),
        (Some((50, Bound::Lower)), 100),
        (Some((100, Bound::Upper)), 100),
        (Some((B - 1, Bound::Lower)), B - 1),
        (Some((B, Bound::Lower)), 100),
        (Some((-B, Bound::Upper)), 100),
        (Some((MATE - 5, Bound::Lower)), 100),
        (Some((-(MATE - 5), Bound::Upper)), 100),
    ] {
        assert_eq!(pruning_eval(100, hit), expected, "{hit:?}");
    }
}

#[test]
fn reverse_futility_uses_corrected_static_eval() {
    assert_eq!(test_nnue().pawn_value(), 100);
    let board = bare_kings();
    for corrected in [false, true] {
        with_root_searcher(&board, &[], |searcher| {
            let raw = evaluate(searcher.nnue, &board);
            if corrected {
                searcher
                    .correction
                    .update(board.side_to_move(), material_key(&board), 64, 8);
                assert_eq!(
                    searcher
                        .correction
                        .read(board.side_to_move(), material_key(&board)),
                    18
                );
            }
            // 補正なしの余裕値9に1足りない。補正18では余裕値が9+11=20になり、
            // 判定用の評価値raw+18から引いてもβ以上になる。
            let beta = raw - 8;
            let score = searcher
                .negamax(&mut board.clone(), 1, beta - 1, beta, 0, false)
                .unwrap();
            if corrected {
                assert_eq!(score, raw + 18);
                assert_eq!(searcher.nodes, 0);
            } else {
                assert!(searcher.nodes > 0);
            }
        });
    }
}

#[test]
fn reverse_futility_uses_shallow_tt_lower_bound_but_not_upper_bound() {
    assert_eq!(test_nnue().pawn_value(), 100);
    let board = bare_kings();
    for bound in [Bound::Lower, Bound::Upper] {
        with_root_searcher(&board, &[], |searcher| {
            let raw = evaluate(searcher.nnue, &board);
            let value = raw + 500;
            // 照合成功の余裕値は16。下界を採る場合だけβに達する。
            let beta = value - 16;
            searcher
                .tt
                .store(search_key(&board), 0, value, bound, None, 0, false);
            let score = searcher
                .negamax(&mut board.clone(), 1, beta - 1, beta, 0, false)
                .unwrap();
            if bound == Bound::Lower {
                assert_eq!(score, value);
                assert_eq!(searcher.nodes, 0);
            } else {
                assert!(searcher.nodes > 0);
            }
        });
    }
}

#[test]
fn razoring_uses_shallow_tt_upper_bound_but_not_lower_bound() {
    assert_eq!(test_nnue().pawn_value(), 100);
    assert_eq!(params::razoring_margin1(), 1606);
    let board = bare_kings();
    for bound in [Bound::Upper, Bound::Lower] {
        with_root_searcher(&board, &[], |searcher| {
            let raw = evaluate(searcher.nnue, &board);
            // 静的評価では余裕値に1足りず、低い上界を採れば静止探索へ移る。
            let alpha = raw + 1605;
            searcher
                .tt
                .store(search_key(&board), 0, raw - 18, bound, None, 0, false);
            let score = searcher
                .negamax(&mut board.clone(), 1, alpha, alpha + 1, 0, false)
                .unwrap();
            if bound == Bound::Upper {
                assert_eq!(searcher.nodes, 0);
                assert_eq!(score, raw, "捕獲のない静止探索は補正前の評価を返す");
            } else {
                assert!(searcher.nodes > 0);
            }
        });
    }
}

#[test]
fn null_move_reduction_uses_corrected_static_eval_not_tt_value() {
    assert_eq!(test_nnue().pawn_value(), 100);
    assert_eq!(params::null_move_base(), 3617);
    assert_eq!(params::null_move_slope(), 257);
    assert_eq!(params::null_move_eval_scale(), 56);
    let board = position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 1), Color::Black, PieceKind::GoldGeneral),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    );
    for corrected in [false, true] {
        with_root_searcher(&board, &[], |searcher| {
            let raw = evaluate(searcher.nnue, &board);
            let beta = raw - 357;
            if corrected {
                searcher
                    .correction
                    .update(board.side_to_move(), material_key(&board), 64, 8);
                assert_eq!(
                    searcher
                        .correction
                        .read(board.side_to_move(), material_key(&board)),
                    18
                );
            }
            // F5でIIRを行わないallNodeの深さ6で基本量は4。差358から追加1となり、補正18で境界を越す。
            // 判定用の評価値はβ未満だが、null moveの減深量には使わない。
            searcher.tt.store(
                search_key(&board),
                0,
                raw - 500,
                Bound::Upper,
                None,
                0,
                false,
            );
            let score = searcher
                .negamax(&mut board.clone(), 6, beta - 1, beta, 0, false)
                .unwrap();
            assert!(score >= beta);
            if corrected {
                assert_eq!(searcher.nodes, 0, "null moveの子は深さ0となる");
            } else {
                assert!(searcher.nodes > 0, "null moveの子は深さ1となる");
            }
        });
    }
}

// F5の指示書の数値例。零窓のβは100とする。
#[test]
fn zero_window_tt_cutoff_matches_f5_examples() {
    use super::super::pruning::zero_window_tt_cutoff;
    let cases = [
        (150, Bound::Lower, 5, 5, true, false),
        (150, Bound::Lower, 6, 5, true, true),
        (150, Bound::Lower, 6, 5, false, true),
        (150, Bound::Lower, 6, 4, false, false),
        (150, Bound::Lower, 5, 4, true, true),
        (100, Bound::Lower, 5, 5, true, true),
        (50, Bound::Upper, 5, 5, false, true),
        (50, Bound::Upper, 4, 4, true, false),
        (50, Bound::Upper, 5, 5, true, true),
        (50, Bound::Lower, 9, 5, false, false),
        (150, Bound::Upper, 9, 5, true, false),
        (150, Bound::Exact, 9, 5, true, true),
        (REPETITION_BAND_START, Bound::Lower, 9, 5, true, false),
        (-REPETITION_BAND_START, Bound::Upper, 9, 5, false, false),
        (50, Bound::Exact, 5, 5, false, true),
        (50, Bound::Upper, 4, 5, false, false),
    ];
    for (value, bound, tt_depth, depth, cut_node, expected) in cases {
        assert_eq!(
            zero_window_tt_cutoff(value, bound, tt_depth, depth, 100, cut_node),
            expected,
            "v={value}, bound={bound:?}, d_tt={tt_depth}, d={depth}, cutNode={cut_node}"
        );
    }
}
