//! 枝刈りと深さの削減を検査する。

use super::*;
use crate::search::REPETITION_BAND_START;
use crate::search::alphabeta::history::CONTINUATION_HISTORY_LIMIT;
use crate::search::alphabeta::ordering::continuation_index;
use crate::search::alphabeta::params;
use crate::search::alphabeta::pruning::{
    CaptureFutilityCoefficients, FutilityCoefficients, SeeCoefficients, applies_lmr,
    continuation_history_prunes, history_lmr_divisor, history_pruning_lmr_depth,
    is_opponent_worsening, is_seek_mate, lmr_correction_adjustment, lmr_history_adjustment,
    lmr_history_value, pruning_lmr_depth, quiet_see_margin, quiet_see_margin_with_coefficient,
    razoring_margin, razoring_margin_with_coefficient, reverse_futility_margin,
    reverse_futility_value, seek_mate_null_move_reduction, should_raise_futility_best_score,
};

use crate::search::alphabeta::pruning::{LmrCoefficients, LmrContext};
#[cfg(feature = "tuning")]
use crate::search::alphabeta::pruning::{late_move_limit, lmr_depth, lmr_research_depth};

// F1の成立境界の窓を作る。根はopponentWorseningが真でimprovingは偽。
// eval >= betaも必要なので、負の余裕値ではevalとbetaを一致させる。
fn root_reverse_margin(depth: u32, tt_hit: bool) -> i32 {
    let mult = (params::reverse_futility_mult_base()
        + params::reverse_futility_mult_slope() * depth as i32)
        .min(41)
        - params::reverse_futility_no_tt_hit() * i32::from(!tt_hit);
    (test_nnue().pawn_value()
        * mult
        * (100 * depth as i32 - params::reverse_futility_opponent_worsening())
        / 10000)
        .max(0)
}

/// S8で定めたreverse futility pruningの返り値を、探索テストの期待値として求める。
fn expected_reverse_futility_value(beta: i32, eval: i32) -> i32 {
    ((661 * i64::from(beta) + 363 * i64::from(eval)) / 1024) as i32
}

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
fn razoring_margin_matches_numeric_examples() {
    for (depth, expected) in [(1, 164), (4, 656)] {
        assert_eq!(razoring_margin_with_coefficient(100, depth, 164), expected);
    }
}

#[test]
fn seek_mate_limits_reverse_futility_to_depth_five() {
    let board = bare_kings();
    let value = evaluate(test_nnue(), &board);
    // 深さ5と6の両方で余裕値を満たす窓にする。
    let beta = value
        - root_reverse_margin(5, false)
            .max(root_reverse_margin(6, false))
            .max(0);
    for (seek_mate, depth, pruned) in [(true, 5, true), (true, 6, false), (false, 6, true)] {
        with_root_searcher(&board, &[], |searcher| {
            zero_position_correction(searcher, &board);
            searcher.seek_mate = seek_mate;
            let score = searcher
                .negamax(&mut board.clone(), depth, beta - 1, beta, 0, false)
                .unwrap();
            if pruned {
                assert_eq!(
                    (score, searcher.nodes),
                    (expected_reverse_futility_value(beta, value), 0)
                );
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
    for depth in [1, 4] {
        let alpha = value + razoring_margin(test_nnue().pawn_value(), depth) + 1;
        let (quiet_score, quiet_nodes) = run_quiesce(&board, alpha, alpha + 1, 0, &small_tt());
        for seek_mate in [false, true] {
            with_root_searcher(&board, &[], |searcher| {
                zero_position_correction(searcher, &board);
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
        stat_score: 0,
        history: Some(0),
        cutoff_count: 0,
        alpha: 0,
        eval: 0,
        pawn: 100,
        correction_weighted_sum: 0,
        seek_mate_gives_check: None,
    }
}

#[test]
fn lmr_table_follows_logarithmic_rule() {
    let table = lmr_table();

    // 延長後の深さはnegamaxの入口で制限し、表はMAX_PLY + 1行に保つ。
    assert_eq!(table.len(), MAX_PLY as usize + 1);
    assert!(table[0].iter().all(|&value| value == 0));
    for row in table {
        assert_eq!(row[0], 0);
        assert_eq!(row[1], 0);
    }
    // 除数1.47で対数積を割り、100倍して切り捨てる。
    for (depth, index, expected) in [(3, 3, 82), (4, 8, 196), (5, 255, 606), (8, 10, 325)] {
        assert_eq!(super::super::pruning::lmr_base(depth, index, 147), expected);
    }
}

#[test]
fn lmr_reduction_before_pruning_and_depth_match_numeric_examples() {
    let coefficients = lmr_coefficients();
    for (case, table, improving, pv_node, tt_pv, depth, reduction, lmr_depth) in [
        ("base", 0, false, false, false, 1, 127, -1),
        ("pv", 300, true, true, false, 8, 421, 3),
        ("tt pv", 300, true, false, true, 8, 497, 3),
        ("non improving", 300, false, false, false, 8, 436, 3),
        ("combined", 300, false, true, true, 8, 500, 2),
    ] {
        assert_eq!(
            coefficients.lmr_reduction_before_pruning(table, improving, pv_node, tt_pv),
            reduction,
            "{case}"
        );
        assert_eq!(
            coefficients.pruning_lmr_depth(table, depth, improving, pv_node, tt_pv),
            lmr_depth,
            "{case}"
        );
    }
}

#[test]
fn lmr_stat_score_adjustment_matches_numeric_examples_and_precedes_all_node_scaling() {
    assert_eq!(lmr_history_adjustment(10_000, 439), 104);
    assert_eq!(lmr_history_adjustment(-10_000, 439), -104);

    let coefficients = LmrCoefficients {
        lmr_history_scale: 439,
        lmr_all_node: 100,
        ..isolated_lmr_coefficients(1000)
    };
    let context = LmrContext {
        depth: 1,
        index: 0,
        improving: true,
        capture: true,
        stat_score: 10_000,
        history: None,
        ..lmr_context()
    };
    // 1000からq_hist=104を引いた896へ、allNodeの896×100/(100+105)=437を足す。
    assert_eq!(coefficients.lmr_reduction(0, context), 1333);
}

#[test]
fn lmr_correction_adjustment_matches_numeric_examples() {
    for (weighted_sum, expected) in [(1023, 1), (-102_400, 101), (537_i64 * 1024, 542), (0, 0)] {
        assert_eq!(lmr_correction_adjustment(weighted_sum, 101), expected);
    }
}

#[test]
fn lmr_correction_adjustment_precedes_all_node_scaling() {
    let coefficients = LmrCoefficients {
        lmr_correction: 101,
        lmr_all_node: 100,
        ..isolated_lmr_coefficients(1000)
    };
    let context = LmrContext {
        depth: 1,
        index: 0,
        improving: true,
        capture: true,
        history: None,
        correction_weighted_sum: 102_400,
        ..lmr_context()
    };

    let non_all_node = LmrContext {
        cut_node: true,
        ..context
    };
    let without_correction = LmrContext {
        correction_weighted_sum: 0,
        ..non_all_node
    };
    assert_eq!(
        coefficients.lmr_reduction(0, without_correction)
            - coefficients.lmr_reduction(0, non_all_node),
        101
    );

    // 1000から101を引いた899へ、allNodeの899×100/(100+105)=438を足す。
    assert_eq!(coefficients.lmr_reduction(0, context), 1337);
}

#[test]
fn lmr_correction_adjustment_is_equal_at_root_and_internal_nodes() {
    let coefficients = LmrCoefficients {
        lmr_correction: 101,
        ..isolated_lmr_coefficients(1000)
    };
    let base = LmrContext {
        capture: true,
        history: None,
        correction_weighted_sum: 102_400,
        ..lmr_context()
    };
    let root = LmrContext {
        pv_node: true,
        tt_pv: true,
        tt_capture: Some(false),
        is_tt_move: true,
        ..base
    };
    let internal = LmrContext {
        cut_node: true,
        ..base
    };

    for (case, context) in [("root", root), ("internal", internal)] {
        let without_correction = LmrContext {
            correction_weighted_sum: 0,
            ..context
        };
        assert_eq!(
            coefficients.lmr_reduction(0, without_correction)
                - coefficients.lmr_reduction(0, context),
            101,
            "{case}"
        );
    }
}

#[test]
fn lmr_targets_capture_killer_and_late_tt_moves_with_fixed_coefficients() {
    assert!(!applies_lmr(1, 1), "残り深さ1では対象外");
    assert!(!applies_lmr(8, 0), "先頭手は対象外");
    assert!(applies_lmr(2, 1), "残り深さ2の2手目から対象");

    let base = lmr_context();
    let threshold = lmr_coefficients().lmr_history_threshold;
    for (case, context, expected_reduction, expected_depth) in [
        (
            "capture",
            LmrContext {
                capture: true,
                history: None,
                ..base
            },
            655,
            4,
        ),
        (
            "killer",
            LmrContext {
                history: Some(threshold),
                ..base
            },
            540,
            4,
        ),
        (
            "late tt move",
            LmrContext {
                tt_capture: Some(false),
                is_tt_move: true,
                history: Some(threshold),
                ..base
            },
            229,
            5,
        ),
    ] {
        let reduction = example_lmr_reduction(context);
        assert_eq!(reduction, expected_reduction, "{case}");
        assert_eq!(
            example_lmr_depth(context.depth, reduction, false, 0, 8),
            expected_depth,
            "{case}"
        );
        assert!(expected_depth < context.depth - 1, "{case}");
    }
}

#[test]
fn lmr_history_step_includes_killer_and_quiet_tt_but_excludes_capture() {
    let butterfly = 417;
    for case in ["killer", "quiet tt move"] {
        assert_eq!(
            lmr_history_value(false, butterfly),
            Some(butterfly),
            "{case}"
        );
    }
    assert_eq!(lmr_history_value(true, butterfly), None, "capture");
}

#[test]
fn continuation_history_pruning_formulas_match_boundaries() {
    for (depth, expected) in [(1, 3343), (8, 3000), (16, 3448), (20, 3448)] {
        assert_eq!(history_lmr_divisor(depth), expected);
    }

    let coefficient = 4000;
    let depth = 3;
    assert!(continuation_history_prunes(-12_001, depth, coefficient));
    assert!(!continuation_history_prunes(-12_000, depth, coefficient));
    assert!(!continuation_history_prunes(-11_999, depth, coefficient));

    // H=6000+69×32/32=6069、D(8)=3000なので、2を足す。
    assert_eq!(history_pruning_lmr_depth(2, 8, 6000, 32), 4);
}

#[test]
fn futility_value_matches_numeric_examples() {
    let coefficients = FutilityCoefficients {
        futility_base: 79,
        futility_lmr_depth: 57,
        futility_above_alpha: 43,
    };
    for (static_eval, alpha, lmr_depth, expected) in
        [(0, 50, 2, 193), (60, 50, -1, 125), (50, 50, 0, 129)]
    {
        assert_eq!(
            coefficients.futility_value(100, static_eval, alpha, lmr_depth),
            expected
        );
    }
}

#[test]
fn futility_pruning_value_includes_equality_but_excludes_depth_twelve() {
    let coefficients = FutilityCoefficients {
        futility_base: 79,
        futility_lmr_depth: 57,
        futility_above_alpha: 43,
    };

    assert_eq!(
        coefficients.futility_pruning_value(100, 0, 193, 2),
        Some(193)
    );
    assert_eq!(coefficients.futility_pruning_value(100, 0, 192, 2), None);
    assert_eq!(
        coefficients.futility_pruning_value(100, 0, 706, 11),
        Some(706)
    );
    assert_eq!(coefficients.futility_pruning_value(100, 0, 1000, 12), None);
}

#[test]
fn futility_best_score_raise_requires_strict_order_and_values_inside_bands() {
    let band = REPETITION_BAND_START;
    for (best_score, futility_value, expected) in [
        (10, 11, true),
        (10, 10, false),
        (-band + 1, -band + 2, true),
        (-band, -band + 1, false),
        (band - 2, band - 1, true),
        (band - 2, band, false),
    ] {
        assert_eq!(
            should_raise_futility_best_score(best_score, futility_value),
            expected,
            "best_score={best_score}, futility_value={futility_value}"
        );
    }
}

// S3の数値例。2回の除算の独立した0方向の丸めと、0での切り詰めを検査する。
#[test]
fn see_margin_matches_numeric_examples() {
    let coefficients = SeeCoefficients {
        see_depth_margin: 85,
        see_capture_history: 34,
    };
    for (depth, history, expected) in [(1, 0, 85), (3, 1024, 271), (1, -8000, 0)] {
        assert_eq!(coefficients.see_margin(100, depth, history), expected);
    }
}

#[test]
fn quiet_see_margin_matches_numeric_examples() {
    for (lmr_depth, expected) in [(-3, 0), (0, 0), (3, 99), (5, 275)] {
        assert_eq!(
            quiet_see_margin_with_coefficient(100, lmr_depth, 11),
            expected
        );
    }
}

// S4の数値例。負のlmrDepthとcapture historyの2回の除算を0方向へ丸める。
#[test]
fn capture_futility_value_matches_numeric_examples() {
    let coefficients = CaptureFutilityCoefficients {
        capture_futility_base: 113,
        capture_futility_lmr_depth: 119,
        capture_futility_history: 134,
    };
    for (static_eval, lmr_depth, captured_value, history, expected) in
        [(-500, 1, 200, 0, -68), (0, -2, 100, 1024, 39)]
    {
        assert_eq!(
            coefficients.capture_futility_value(
                100,
                static_eval,
                lmr_depth,
                captured_value,
                history,
            ),
            expected
        );
    }
}

#[test]
fn lmr_reduction_matches_f4_examples() {
    let a = lmr_context();
    for (case, context, reduction, depth) in [
        ("A", a, 655, 4),
        (
            "B",
            LmrContext {
                cut_node: true,
                tt_capture: Some(false),
                ..a
            },
            731,
            4,
        ),
        (
            "C",
            LmrContext {
                cut_node: true,
                ..a
            },
            790,
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
            109,
            7,
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
            297,
            5,
        ),
        (
            "F",
            LmrContext {
                cutoff_count: 3,
                ..a
            },
            1129,
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
            779,
            4,
        ),
        ("H", LmrContext { alpha: 80, ..a }, 680, 4),
        ("I", LmrContext { alpha: -50, ..a }, 639, 4),
        ("J", LmrContext { alpha: 20, ..a }, 666, 4),
        (
            "K",
            LmrContext {
                improving: true,
                history: Some(lmr_coefficients().lmr_history_threshold),
                ..a
            },
            529,
            4,
        ),
        (
            "L",
            LmrContext {
                tt_capture: Some(true),
                ..a
            },
            655,
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
            -53,
            2,
        ),
    ] {
        assert_eq!(example_lmr_reduction(context), reduction, "{case}");
        assert_eq!(
            example_lmr_depth(context.depth, reduction, context.pv_node, 0, 8),
            depth,
            "{case}"
        );
    }
    assert_eq!(
        example_lmr_depth(2, -203, true, 16, 8),
        2,
        "延長を許す範囲の境界"
    );
    assert_eq!(example_lmr_depth(2, -203, true, 15, 8), 4);
    assert_eq!(
        example_lmr_depth(2, -203, true, 0, 0),
        2,
        "反復深さの初期値0"
    );
}

#[test]
fn seek_mate_check_adjusts_positive_lmr_reduction_by_one_quarter() {
    let context = LmrContext {
        depth: 1,
        index: 0,
        improving: true,
        pv_node: true,
        capture: true,
        history: None,
        ..lmr_context()
    };
    for (reduction, gives_check, expected) in [
        (400, None, 400),
        (400, Some(true), 300),
        (400, Some(false), 500),
        (-200, Some(true), -200),
        (-200, Some(false), -200),
        (0, Some(true), 0),
        (0, Some(false), 0),
        (3, Some(true), 3),
        (3, Some(false), 3),
        (7, Some(true), 6),
        (7, Some(false), 8),
    ] {
        assert_eq!(
            isolated_lmr_coefficients(reduction).lmr_reduction(
                0,
                LmrContext {
                    seek_mate_gives_check: gives_check,
                    ..context
                },
            ),
            expected,
            "reduction={reduction}, gives_check={gives_check:?}"
        );
    }
}

#[test]
fn seek_mate_check_adjustment_precedes_no_tt_move_increment() {
    let context = LmrContext {
        depth: 1,
        index: 0,
        improving: true,
        pv_node: true,
        capture: true,
        history: None,
        ..lmr_context()
    };
    for (reduction, gives_check, expected_adjusted, expected_depth) in
        [(360, false, 450, 5), (612, true, 459, 4)]
    {
        let adjusted = isolated_lmr_coefficients(reduction).lmr_reduction(
            0,
            LmrContext {
                seek_mate_gives_check: Some(gives_check),
                ..context
            },
        );
        assert_eq!(adjusted, expected_adjusted);
        assert_eq!(
            super::super::pruning::non_lmr_depth_with_coefficient(5, adjusted, false, 57),
            expected_depth,
            "reduction={reduction}, gives_check={gives_check}"
        );
    }
}

#[test]
fn seek_mate_check_adjustment_follows_all_node_term() {
    let coefficients = LmrCoefficients {
        lmr_all_node: 100,
        ..isolated_lmr_coefficients(400)
    };
    let context = LmrContext {
        depth: 1,
        index: 0,
        improving: true,
        capture: true,
        history: None,
        seek_mate_gives_check: Some(true),
        ..lmr_context()
    };

    // allNodeの項で400から595に増やした後、その1/4を0方向へ丸めて引く。
    assert_eq!(coefficients.lmr_reduction(0, context), 447);
}

#[test]
fn lmr_check_value_is_present_only_when_seeking_mate() {
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 2), Color::Black, PieceKind::Rook),
            (sq(5, 4), Color::Black, PieceKind::SideMover),
            (sq(5, 8), Color::White, PieceKind::King),
        ],
    );
    let generator = MoveGenerator::standard();
    let moves = legal_moves(&board);
    let checking = moves
        .iter()
        .copied()
        .find(|&mv| board.gives_check(&generator, mv))
        .expect("fixture must have a checking move");
    let non_checking = moves
        .iter()
        .copied()
        .find(|&mv| !board.gives_check(&generator, mv))
        .expect("fixture must have a non-checking move");

    with_root_searcher(&board, &[], |searcher| {
        searcher.seek_mate = false;
        assert_eq!(searcher.lmr_seek_mate_gives_check(&board, checking), None);
        assert_eq!(
            searcher.lmr_seek_mate_gives_check(&board, non_checking),
            None
        );

        searcher.seek_mate = true;
        assert_eq!(
            searcher.lmr_seek_mate_gives_check(&board, checking),
            Some(true)
        );
        assert_eq!(
            searcher.lmr_seek_mate_gives_check(&board, non_checking),
            Some(false)
        );
    });
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
            128,
            5,
        ),
        (
            "O",
            LmrContext {
                cut_node: true,
                ..a
            },
            456,
            4,
        ),
        ("P", a, 279, 5),
        (
            "Q",
            LmrContext {
                is_tt_move: true,
                tt_capture: Some(false),
                history: None,
                ..a
            },
            -43,
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
            445,
            5,
        ),
    ] {
        assert_eq!(example_lmr_reduction(context), reduction, "{case}");
        assert_eq!(
            example_non_lmr_depth(5, reduction, context.tt_capture.is_some()),
            depth,
            "{case}"
        );
    }
    assert_eq!(
        example_non_lmr_depth(2, 639, false),
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
        assert_eq!(example_non_lmr_depth(normal, reduction, true), expected);
    }
}

#[test]
fn lmr_research_depth_matches_f4_examples() {
    let deeper = 50 + 82;
    let shallower = 50 + 26;
    for (reduced, value, expected) in [
        (4, deeper + 1, 8),
        (4, shallower + 1, 7),
        (4, shallower - 1, 6),
        (7, deeper + 1, 7),
        (4, deeper, 7),
        (4, shallower, 7),
    ] {
        assert_eq!(
            super::super::pruning::lmr_research_depth_with_margins(
                7, reduced, value, 50, 100, 82, 26
            ),
            expected
        );
    }
}

#[test]
fn lmr_history_and_alpha_eval_respect_eligibility() {
    let a = lmr_context();
    let threshold = lmr_coefficients().lmr_history_threshold;
    // Aのr0=567に±100を足してからallNodeの項を加える。
    for (history, expected) in [
        (-threshold, 771),
        (-threshold + 1, 655),
        (threshold - 1, 655),
        (threshold, 540),
    ] {
        assert_eq!(
            example_lmr_reduction(LmrContext {
                history: Some(history),
                ..a
            }),
            expected
        );
    }
    assert_eq!(
        example_lmr_reduction(LmrContext { history: None, ..a }),
        655
    );
    assert_eq!(
        example_lmr_reduction(LmrContext {
            capture: true,
            alpha: 80,
            ..a
        }),
        655
    );
    for alpha in [-REPETITION_BAND_START, REPETITION_BAND_START] {
        assert_eq!(example_lmr_reduction(LmrContext { alpha, ..a }), 655);
    }
    assert_eq!(
        example_lmr_reduction(LmrContext {
            cutoff_count: 1,
            ..a
        }),
        655
    );
}

#[test]
fn lmr_large_move_indices_clamp_only_table_column() {
    // 表の255列と256列は同じ。手番号の項だけLmrMoveCount分減り、cutNodeにはallNodeの丸めがない。
    let a = LmrContext {
        index: 255,
        cut_node: true,
        ..lmr_context()
    };
    assert_eq!(
        lmr_reduction(LmrContext { index: 256, ..a }),
        lmr_reduction(a) - params::lmr_move_count()
    );
}

// 係数の変更はtuningの子プロセス内に閉じ込める。
#[cfg(feature = "tuning")]
pub(super) fn lmr_max_reduction_parameter_sets_cap() {
    let reduction = 900;
    params::set("LmrMaxReduction", 3).unwrap();
    assert_eq!(lmr_depth(8, reduction, false, 0, 8), 4);
    params::set("LmrMaxReduction", 1).unwrap();
    assert_eq!(lmr_depth(8, reduction, false, 0, 8), 6);
    assert_eq!(
        lmr_depth(2, -203, true, 0, 8),
        4,
        "減深の上限は延長に掛けない"
    );
    params::set("LmrMaxReduction", 8).unwrap();
    assert_eq!(lmr_depth(8, reduction, false, 0, 8), 1, "例Aの上限8");
    params::set("LmrMaxReduction", 3).unwrap();
    params::set("LmrDeeperMargin", 0).unwrap();
    params::set("LmrShallowerMargin", 26).unwrap();
    // 値52はbest50より高く、浅くする閾値76より低い。独立な両条件が相殺する。
    assert_eq!(lmr_research_depth(7, 4, 52, 50, 100), 7);
    params::set("LmrDeeperMargin", 82).unwrap();
}

// futilityはPVノードでも後続の静かな手を飛ばすが、followPVでは全手を読む。
// どちらも少なくとも1手を読み、合法手なしの詰みと誤認しない。
#[test]
fn futility_prunes_pv_quiets_without_false_mate_except_on_follow_pv() {
    let board = bare_kings();
    let nnue = test_nnue();
    let static_eval = evaluate(nnue, &board);
    let moves = legal_moves(&board);
    let history = repeated_root_children(&board, &moves);
    let alpha = static_eval + 10 * nnue.pawn_value();

    assert!(!royal_under_attack(&board));
    assert!(
        moves
            .iter()
            .all(|&mv| { !mv.promote && move_order_key(&board, nnue, mv).is_none() })
    );
    for depth in [1, 4] {
        let mut results = Vec::new();
        for follow_pv in [false, true] {
            with_root_searcher(&board, &history, |searcher| {
                searcher.follow_pv[0] = follow_pv;
                searcher.tt.store(
                    search_key(&board),
                    0,
                    0,
                    None,
                    Bound::Upper,
                    Some(moves[0]),
                    0,
                    true,
                );
                let score = searcher
                    .negamax(&mut board.clone(), depth, alpha, alpha + 2, 0, false)
                    .unwrap();
                assert!(searcher.nodes > 0, "最初の手は探索する");
                assert!(score.abs() < MATE_THRESHOLD, "score={score}");
                results.push((score, searcher.nodes));
            });
        }
        assert!(results[0].1 < results[1].1, "depth={depth}");
        assert_eq!(results[1].1, moves.len() as u64, "depth={depth}");
    }
}

// late move pruningの上限まではfutilityが王手になる静かな手を保護し、上限後は王手でも先に省く。
#[cfg(feature = "tuning")]
pub(super) fn futility_protects_quiet_checks_before_late_move_limit() {
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 2), Color::Black, PieceKind::Rook),
            (sq(5, 4), Color::Black, PieceKind::SideMover),
            (sq(5, 8), Color::White, PieceKind::King),
        ],
    );
    let moves = legal_moves(&board);
    let tt_move = Move {
        from: sq(0, 0),
        mid: None,
        to: sq(1, 0),
        promote: false,
    };
    assert!(moves.contains(&tt_move));
    assert!(!board.gives_check(&MoveGenerator::standard(), tt_move));
    assert!(!royal_under_attack(&board));
    assert!(
        moves
            .iter()
            .all(|&mv| !mv.promote && move_order_key(&board, test_nnue(), mv).is_none())
    );
    let checking: Vec<_> = moves
        .iter()
        .copied()
        .filter(|&mv| board.gives_check(&MoveGenerator::standard(), mv))
        .collect();
    assert!(!checking.is_empty());
    let history = repeated_root_children(&board, &moves);
    with_root_searcher(&board, &history, |searcher| {
        let limit = usize::try_from(late_move_limit(1, false)).unwrap();
        let mut picker = MovePicker::new(Some(tt_move), [None; KILLER_COUNT]);
        let mut ordered = Vec::new();
        while let Some((mv, capture)) = picker.next(
            &board,
            searcher.nnue,
            &searcher.generator,
            searcher_move_picker_context(searcher, 0),
        ) {
            assert!(!capture && !mv.promote);
            ordered.push(mv);
        }
        assert_eq!(ordered[0], tt_move);
        let counted = &ordered[1..];
        assert!(counted.len() > limit);
        let checking_before_limit = counted[..limit]
            .iter()
            .filter(|&&mv| board.gives_check(&searcher.generator, mv))
            .count();
        assert!(checking_before_limit > 0);
        assert!(
            counted[limit..]
                .iter()
                .any(|&mv| board.gives_check(&searcher.generator, mv)),
            "late move pruningが王手になる静かな手も対象にする局面"
        );
        searcher.tt.store(
            search_key(&board),
            0,
            0,
            None,
            Bound::Upper,
            Some(tt_move),
            0,
            true,
        );
        let alpha = evaluate(searcher.nnue, &board) + 10 * searcher.nnue.pawn_value();
        let score = searcher
            .negamax(&mut board.clone(), 1, alpha, alpha + 2, 0, false)
            .unwrap();
        assert!(score.abs() < MATE_THRESHOLD);
        assert_eq!(searcher.nodes, 1 + checking_before_limit as u64);
    });
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
    let (opponent_score, _) =
        run_move_pruning(&child, 1, -INFINITY, INFINITY, ply + 1, &small_tt());
    assert!(opponent_score >= MATE_THRESHOLD, "先頭手は王駒を失う");

    let table = small_tt();
    // 子の全窓探索で確認した詰み値を固定し、親で後続手を読む条件だけを検査する。
    // 親を深さ3にし、先頭手の追加減深後も通常探索が残るようにする。
    // 子の詰み値は延長があっても深さ不足にならない記録にする。
    table.store(
        search_key(&child),
        MAX_PLY,
        opponent_score,
        None,
        Bound::Exact,
        None,
        ply + 1,
        false,
    );
    table.store(
        search_key(&position),
        0,
        0,
        None,
        Bound::Upper,
        Some(losing_move),
        ply,
        false,
    );
    let (score, _) = run_move_pruning(&position, 3, alpha, alpha + 1, ply, &table);
    assert!(score.abs() < MATE_THRESHOLD, "score={score}");
    let hit = table.probe(search_key(&position), ply).unwrap();
    assert!(hit.value.unwrap().score.abs() < MATE_THRESHOLD);
    assert_ne!(hit.best_move, Some(losing_move));
}

// 静かな記録手を先に探索して条件を成立させても、
// 後続の王手になる捕獲手と非捕獲の成る手は展開し、記録手だけの場合より探索ノードが増える。
#[test]
fn futility_searches_checking_captures_and_promotions_after_quiet_tt_move() {
    for (board, promotion) in [
        (
            position(
                Color::Black,
                &[
                    (sq(5, 8), Color::White, PieceKind::King),
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
        if !promotion {
            assert!(
                protected
                    .iter()
                    .all(|&mv| position.gives_check(&MoveGenerator::standard(), mv))
            );
        }
        let table = small_tt();
        table.store(
            search_key(&position),
            0,
            evaluate(nnue, &position),
            None,
            Bound::Upper,
            Some(tt_move),
            0,
            false,
        );
        let (score, nodes) = run_move_pruning(&position, 1, alpha, alpha + 1, 0, &table);
        assert!(score.abs() < MATE_THRESHOLD);
        assert!(nodes > 1, "記録手の後にも探索する");
    }
}

/// 捕獲手のfutility見積りと、その手がLMR表で使う番号を返す。
fn capture_futility_context(
    board: &Position,
    target: Move,
    tt_move: Move,
    depth: u32,
) -> (i32, i32) {
    let nnue = test_nnue();
    let mut target_index = None;
    with_root_searcher(board, &[], |searcher| {
        let mut picker = MovePicker::new(Some(tt_move), [None; KILLER_COUNT]);
        let mut index = 0;
        while let Some((mv, _)) = picker.next(
            board,
            searcher.nnue,
            &searcher.generator,
            searcher_move_picker_context(searcher, 0),
        ) {
            if mv == target {
                target_index = Some(index);
                break;
            }
            index += 1;
        }
    });
    let index = target_index.expect("target capture must be returned by the move picker");
    let lmr_depth = super::super::pruning::pruning_lmr_depth(depth, index, false, false, true);
    let captured_value = board
        .captured_squares(target)
        .into_iter()
        .flatten()
        .map(|square| nnue.piece_value(board.piece_at(square).unwrap()))
        .sum();
    let value = super::super::pruning::capture_futility_value(
        nnue.pawn_value(),
        evaluate(nnue, board),
        lmr_depth,
        captured_value,
        0,
    );
    (lmr_depth, value)
}

/// 対象手だけがβへ届く子の値を置換表へ置き、対象手を読んだかを根の結果で検査する。
/// αは現在の係数から求めた見積りを基準にするため、係数の既定値には依存しない。
fn assert_capture_futility_search(
    board: &Position,
    target: Move,
    tt_move: Move,
    depth: u32,
    alpha_offset: i32,
    expected_check: bool,
    searched: bool,
) -> i32 {
    let nnue = test_nnue();
    let moves = legal_moves(board);
    assert!(moves.contains(&target));
    assert!(moves.contains(&tt_move));
    assert!(move_order_key(board, nnue, target).is_some());
    assert!(!captures_last_royal(board, target));
    assert!(!royal_under_attack(board));
    assert_eq!(
        board.gives_check(&MoveGenerator::new(engine_rules()), target),
        expected_check
    );

    let (lmr_depth, futility_value) = capture_futility_context(board, target, tt_move, depth);
    let alpha = futility_value + alpha_offset;
    let beta = alpha + 1;
    assert!(alpha > evaluate(nnue, board));
    assert!(alpha.abs() < REPETITION_BAND_START && beta.abs() < REPETITION_BAND_START);
    assert_eq!(futility_value <= alpha, alpha_offset >= 0);
    assert!(!see_prunes(
        board,
        engine_rules(),
        nnue,
        target,
        -super::super::pruning::see_margin(nnue.pawn_value(), depth, 0),
    ));

    let mut target_child = board.clone();
    target_child.make_move_unchecked(target, engine_rules());
    let target_key = search_key(&target_child);
    assert_eq!(
        moves
            .iter()
            .filter(|&&mv| {
                let mut child = board.clone();
                child.make_move_unchecked(mv, engine_rules());
                search_key(&child) == target_key
            })
            .count(),
        1,
        "対象手の子局面を置換表で一意に識別できる"
    );

    let table = TranspositionTable::new(16).unwrap();
    for &mv in &moves {
        let mut child = board.clone();
        child.make_move_unchecked(mv, engine_rules());
        let parent_score = if mv == target { beta } else { alpha - 1 };
        table.store(
            search_key(&child),
            MAX_PLY,
            -parent_score,
            None,
            Bound::Exact,
            None,
            1,
            false,
        );
    }
    table.store(
        search_key(board),
        0,
        0,
        None,
        Bound::Upper,
        Some(tt_move),
        0,
        false,
    );
    let (score, _) = run_move_pruning(board, depth, alpha, beta, 0, &table);
    let hit = table.probe(search_key(board), 0).unwrap();
    if searched {
        assert_eq!(score, beta);
        assert_eq!(hit.best_move, Some(target));
        assert_eq!(hit.value.unwrap().bound, Bound::Lower);
    } else {
        assert_eq!(score, alpha - 1);
        assert_ne!(hit.best_move, Some(target));
    }
    lmr_depth
}

fn capture_futility_fixture(white_king: Square) -> (Position, Move, Move) {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (white_king, Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::Rook),
            (sq(5, 4), Color::White, PieceKind::Pawn),
        ],
    );
    let target = Move {
        from: sq(5, 3),
        mid: None,
        to: sq(5, 4),
        promote: false,
    };
    let tt_move = Move {
        from: sq(11, 0),
        mid: None,
        to: sq(10, 0),
        promote: false,
    };
    (board, target, tt_move)
}

#[test]
fn capture_futility_prunes_eligible_capture_at_equality() {
    let (board, target, tt_move) = capture_futility_fixture(sq(0, 11));
    let lmr_depth = assert_capture_futility_search(&board, target, tt_move, 7, 0, false, false);
    assert!(lmr_depth < super::super::pruning::CAPTURE_FUTILITY_LMR_DEPTH_LIMIT);
}

#[test]
fn capture_futility_searches_promoting_checking_and_tt_captures() {
    let promotion_board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 7), Color::Black, PieceKind::Pawn),
            (sq(5, 8), Color::White, PieceKind::Pawn),
        ],
    );
    let promotion = Move {
        from: sq(5, 7),
        mid: None,
        to: sq(5, 8),
        promote: true,
    };
    let quiet = Move {
        from: sq(11, 0),
        mid: None,
        to: sq(10, 0),
        promote: false,
    };
    assert_capture_futility_search(&promotion_board, promotion, quiet, 7, 0, false, true);

    let (checking_board, checking, quiet) = capture_futility_fixture(sq(5, 8));
    assert_capture_futility_search(&checking_board, checking, quiet, 7, 0, true, true);

    let (tt_board, tt_capture, _) = capture_futility_fixture(sq(0, 11));
    assert_capture_futility_search(&tt_board, tt_capture, tt_capture, 7, 0, false, true);
}

#[test]
fn capture_futility_excludes_lmr_depth_eight() {
    let (board, target, tt_move) = capture_futility_fixture(sq(0, 11));
    let depth = (1..=MAX_PLY)
        .find(|&depth| capture_futility_context(&board, target, tt_move, depth).0 >= 8)
        .unwrap();
    let lmr_depth = assert_capture_futility_search(&board, target, tt_move, depth, 0, false, true);
    assert_eq!(lmr_depth, 8);
}

#[test]
fn capture_futility_applies_to_two_stage_captures_and_sums_both_victims() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::Lion),
            (sq(5, 4), Color::White, PieceKind::Pawn),
            (sq(5, 5), Color::White, PieceKind::Pawn),
        ],
    );
    let target = Move {
        from: sq(5, 3),
        mid: Some(sq(5, 4)),
        to: sq(5, 5),
        promote: false,
    };
    let tt_move = Move {
        from: sq(11, 0),
        mid: None,
        to: sq(10, 0),
        promote: false,
    };
    assert_eq!(
        board.captured_squares(target),
        [Some(sq(5, 4)), Some(sq(5, 5))]
    );

    assert_capture_futility_search(&board, target, tt_move, 7, 0, false, false);
    assert_capture_futility_search(&board, target, tt_move, 7, -1, false, true);
}

// SEEが負の捕獲しかなくても先頭手は読み、全手の枝刈りによる偽の詰みを防ぐ。
// この局面は王駒と周囲の歩兵が動けず、仲人が歩兵を取って取り返される2手だけが合法である。
// 判断の理由は docs/adr/prune-captures-and-quiet-moves-by-see-and-capture-futility-with-depth-scaled-margins.md にある。
#[cfg(feature = "tuning")]
pub(super) fn see_pruning_searches_first_capture_without_false_mate() {
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
    let alpha = moves
        .iter()
        .map(|&mv| {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            -evaluate(nnue, &child)
        })
        .max()
        .unwrap()
        .max(evaluate(nnue, &board))
        + 1;
    let table = small_tt();
    let ply = MAX_PLY - 1;
    let mut result = None;
    with_root_searcher(&board, &[], |searcher| {
        searcher.seek_mate = true;
        searcher.accumulators[ply as usize] = searcher.accumulators[0].clone();
        searcher.material_keys[ply as usize] = searcher.material_keys[0];
        searcher.pawn_keys[ply as usize] = searcher.pawn_keys[0];
        set_test_path_ply(searcher, ply);
        searcher.tt.store(
            search_key(&board),
            0,
            alpha,
            None,
            Bound::Upper,
            None,
            ply,
            true,
        );
        let score = searcher
            .negamax(&mut board.clone(), 1, alpha, alpha + 1, ply, false)
            .unwrap();
        let hit = searcher.tt.probe(search_key(&board), ply).unwrap();
        table.store(
            search_key(&board),
            1,
            hit.value.unwrap().score,
            None,
            hit.value.unwrap().bound,
            hit.best_move,
            ply,
            false,
        );
        result = Some((score, searcher.nodes));
    });
    let (score, nodes) = result.unwrap();
    assert!(score.abs() < MATE_THRESHOLD, "score={score}");
    let hit = table.probe(search_key(&board), ply).unwrap();
    // 先頭手はcan_prune_movesが偽なので必ず読み、後続手はS4の式が成立するので読まない。
    let static_eval = evaluate(nnue, &board);
    let improving = static_eval > 0;
    let lmr_depth = super::super::pruning::pruning_lmr_depth(1, 1, improving, false, true);
    assert!(lmr_depth < super::super::pruning::CAPTURE_FUTILITY_LMR_DEPTH_LIMIT);
    for &mv in &moves {
        let captured_value = board
            .captured_squares(mv)
            .into_iter()
            .flatten()
            .map(|square| nnue.piece_value(board.piece_at(square).unwrap()))
            .sum();
        assert!(
            super::super::pruning::capture_futility_value(
                nnue.pawn_value(),
                static_eval,
                lmr_depth,
                captured_value,
                0,
            ) <= alpha
        );
        assert!(!board.gives_check(&MoveGenerator::standard(), mv));
    }
    assert_eq!(nodes, 1);
    assert_eq!(hit.value.unwrap().score, score);
    assert!(hit.value.unwrap().score.abs() < MATE_THRESHOLD);
    assert_eq!(hit.value.unwrap().bound, Bound::Upper);
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
    assert!(see_prunes(&board, engine_rules(), nnue, safe_capture, -200));
    let alpha = evaluate(nnue, &board) + 3 * nnue.pawn_value();
    let ply = 5;
    let mut child = board.clone();
    child.make_move_unchecked(losing_move, engine_rules());
    let (opponent_score, _) =
        run_move_pruning(&child, 1, -INFINITY, INFINITY, ply + 1, &small_tt());
    assert!(opponent_score >= MATE_THRESHOLD);
    let table = small_tt();
    // 子の全窓探索で確認した詰み値を固定し、親で後続手を読む条件だけを検査する。
    // 親を深さ3にし、先頭手の追加減深後も通常探索が残るようにする。
    // 子の詰み値は延長があっても深さ不足にならない記録にする。
    table.store(
        search_key(&child),
        MAX_PLY,
        opponent_score,
        None,
        Bound::Exact,
        None,
        ply + 1,
        false,
    );
    table.store(
        search_key(&board),
        0,
        0,
        None,
        Bound::Upper,
        Some(losing_move),
        ply,
        false,
    );
    let (score, _) = run_move_pruning(&board, 3, alpha, alpha + 1, ply, &table);
    assert!(score.abs() < MATE_THRESHOLD, "score={score}");
    let hit = table.probe(search_key(&board), ply).unwrap();
    assert_eq!(hit.value.unwrap().score, score);
    assert!(hit.value.unwrap().score.abs() < MATE_THRESHOLD);
    let best = hit.best_move.unwrap();
    assert_ne!(best, losing_move);
    let mut safe = board.clone();
    safe.make_move_unchecked(best, engine_rules());
    assert!(
        !legal_moves(&safe)
            .into_iter()
            .any(|mv| captures_last_royal(&safe, mv))
    );
    assert_eq!(hit.value.unwrap().bound, Bound::Upper);
}

// 展開した捕獲の子局面に通常探索の値が保存されることを使い、
// SEEがPVノードと残り深さ4にも適用され、記録手と王駒への利きは除かれることを検査する。
#[cfg(feature = "tuning")]
pub(super) fn see_pruning_applies_without_depth_or_zero_window_limits() {
    let nnue = test_nnue();
    for (depth, width, capture_is_tt, attacked, attacker, victim) in [
        (2, 1, false, false, PieceKind::Phoenix, PieceKind::Pawn),
        (3, 1, false, false, PieceKind::Phoenix, PieceKind::Pawn),
        (2, 1, true, false, PieceKind::Phoenix, PieceKind::Pawn),
        (2, 2, false, false, PieceKind::Phoenix, PieceKind::Pawn),
        (4, 1, false, false, PieceKind::Rook, PieceKind::Pawn),
        (2, 1, false, true, PieceKind::Phoenix, PieceKind::Pawn),
        (
            2,
            1,
            false,
            false,
            PieceKind::Phoenix,
            PieceKind::GoldGeneral,
        ),
        (
            3,
            1,
            false,
            false,
            PieceKind::Phoenix,
            PieceKind::GoldGeneral,
        ),
    ] {
        let mut pieces = vec![
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, attacker),
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
            see_prunes(&board, engine_rules(), nnue, capture, -200),
            victim == PieceKind::Pawn,
        );
        let expanded = capture_is_tt
            || attacked
            || !see_prunes(
                &board,
                engine_rules(),
                nnue,
                capture,
                -super::super::pruning::see_margin(nnue.pawn_value(), depth, 0),
            );
        let alpha = evaluate(nnue, &board) + 3 * nnue.pawn_value();
        let table = small_tt();
        let tt_move = if capture_is_tt { capture } else { quiet };
        table.store(
            search_key(&board),
            0,
            0,
            None,
            Bound::Upper,
            Some(tt_move),
            0,
            false,
        );
        let (score, _) = run_move_pruning(&board, depth, alpha, alpha + width, 0, &table);
        assert!(score.abs() < MATE_THRESHOLD);
        let mut child = board.clone();
        child.make_move_unchecked(capture, engine_rules());
        assert_eq!(
            table.probe(search_key(&child), 1).is_some(),
            expanded,
            "depth={depth}, width={width}, capture_is_tt={capture_is_tt}, attacked={attacked}, attacker={attacker:?}, victim={victim:?}"
        );
        assert_eq!(
            table
                .probe(search_key(&board), 0)
                .unwrap()
                .value
                .unwrap()
                .score,
            score
        );
    }
}

fn continuation_pruning_fixture(white_king: Square) -> (Position, Move) {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (white_king, Color::White, PieceKind::King),
            (sq(4, 4), Color::Black, PieceKind::Rook),
        ],
    );
    (board, step(sq(4, 4), sq(5, 4)))
}

/// 先頭の置換表の手で枝刈りを可能にし、`target`が読まれた場合だけβ打ち切りを起こす。
fn assert_continuation_pruning_search(
    board: &Position,
    target: Move,
    expected_check: bool,
    follow_pv: bool,
    searched: bool,
) {
    let nnue = test_nnue();
    let moves = legal_moves(board);
    let leading_quiet = step(sq(11, 0), sq(10, 0));
    assert!(moves.contains(&leading_quiet));
    assert!(moves.contains(&target));
    assert!(!royal_under_attack(board));
    assert_eq!(
        board.gives_check(&MoveGenerator::new(engine_rules()), target),
        expected_check
    );
    assert!(board.captured_squares(target).iter().all(Option::is_none));

    let minimum_child_value = moves
        .iter()
        .map(|&mv| {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            -evaluate(nnue, &child)
        })
        .min()
        .unwrap();
    let alpha = minimum_child_value - 1;
    let beta = alpha + 2;
    assert!(alpha.abs() < REPETITION_BAND_START && beta.abs() < REPETITION_BAND_START);

    let table = TranspositionTable::new(16).unwrap();
    for &mv in &moves {
        let mut child = board.clone();
        child.make_move_unchecked(mv, engine_rules());
        let parent_score = if mv == target { beta } else { alpha - 1 };
        table.store(
            search_key(&child),
            MAX_PLY,
            -parent_score,
            None,
            Bound::Exact,
            None,
            2,
            false,
        );
    }
    table.store(
        search_key(board),
        0,
        0,
        None,
        Bound::Upper,
        Some(leading_quiet),
        1,
        false,
    );

    let context = MoveRecord {
        mv: step(sq(3, 3), sq(3, 4)),
        piece: PieceCode::new(Color::White, PieceKind::CopperGeneral).unwrap(),
        captured: None,
    };
    let piece = board.piece_at(target.from).unwrap();
    let continuation = continuation_index(context, piece, target, 1);
    let (score, _) = run_configured_negamax(board, 2, alpha, beta, 1, &table, |searcher| {
        searcher.path_moves[0].record = Some(context);
        searcher.follow_pv[1] = follow_pv;
        searcher.killers[1][0] = Some(target);
        searcher
            .continuation_history
            .store(continuation, -(CONTINUATION_HISTORY_LIMIT as i16));
    });
    let hit = table.probe(search_key(board), 1).unwrap();
    if searched {
        assert_eq!(hit.best_move, Some(target), "score={score}, beta={beta}");
        assert!(score >= beta);
        assert_eq!(hit.value.unwrap().bound, Bound::Lower);
    } else {
        assert_ne!(hit.best_move, Some(target));
    }
}

#[test]
fn continuation_history_pruning_skips_low_quiet_but_searches_checks_and_follow_pv() {
    let (quiet_board, quiet) = continuation_pruning_fixture(sq(11, 11));
    assert_continuation_pruning_search(&quiet_board, quiet, false, false, false);
    assert_continuation_pruning_search(&quiet_board, quiet, false, true, true);

    let (checking_board, checking) = continuation_pruning_fixture(sq(5, 8));
    assert_continuation_pruning_search(&checking_board, checking, true, false, true);
}

/// 先頭の置換表の手で枝刈りを可能にし、`target`が読まれた場合だけβ打ち切りを起こす。
/// 子の値は固定ネットの局面評価から作る窓に対する相対値とし、係数の既定値に依存させない。
fn assert_quiet_see_search(
    board: &Position,
    target: Move,
    expected_check: bool,
    follow_pv: bool,
    target_is_tt_move: bool,
    searched: bool,
) {
    let nnue = test_nnue();
    let moves = legal_moves(board);
    let leading_quiet = Move {
        from: sq(11, 0),
        mid: None,
        to: sq(10, 0),
        promote: false,
    };
    assert!(moves.contains(&leading_quiet));
    assert!(moves.contains(&target));
    assert!(
        moves
            .iter()
            .all(|&mv| move_order_key(board, nnue, mv).is_none())
    );
    assert!(!royal_under_attack(board));
    assert_eq!(
        board.gives_check(&MoveGenerator::new(engine_rules()), target),
        expected_check
    );
    assert!(see_prunes(
        board,
        engine_rules(),
        nnue,
        target,
        -super::super::pruning::see_margin(nnue.pawn_value(), 1, 0),
    ));
    let lmr_depth = pruning_lmr_depth(1, usize::from(!target_is_tt_move), false, true, true);
    if !expected_check && !target.promote && target.mid.is_none() {
        assert!(see_prunes(
            board,
            engine_rules(),
            nnue,
            target,
            -quiet_see_margin(nnue.pawn_value(), lmr_depth),
        ));
    }

    let minimum_child_value = moves
        .iter()
        .map(|&mv| {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            -evaluate(nnue, &child)
        })
        .min()
        .unwrap();
    let alpha = minimum_child_value - 1;
    let beta = alpha + 2;
    assert!(alpha.abs() < REPETITION_BAND_START && beta.abs() < REPETITION_BAND_START);
    if !follow_pv && !target_is_tt_move && !target.promote {
        assert_eq!(
            super::super::pruning::futility_pruning_value(
                nnue.pawn_value(),
                evaluate(nnue, board),
                alpha,
                lmr_depth,
            ),
            None,
            "対象手はS1でなくSEEの判定へ進む"
        );
    }

    let table = TranspositionTable::new(16).unwrap();
    for &mv in &moves {
        let mut child = board.clone();
        child.make_move_unchecked(mv, engine_rules());
        let mut captures = Vec::new();
        MoveGenerator::new(engine_rules()).generate_captures(&child, &mut captures);
        assert!(!captures.is_empty(), "move={mv:?}");
        let parent_score = if mv == target { beta } else { alpha - 1 };
        table.store(
            search_key(&child),
            MAX_PLY,
            -parent_score,
            None,
            Bound::Exact,
            None,
            1,
            false,
        );
    }
    let tt_move = if target_is_tt_move {
        target
    } else {
        leading_quiet
    };
    table.store(
        search_key(board),
        0,
        0,
        None,
        Bound::Upper,
        Some(tt_move),
        0,
        false,
    );
    let (score, _) = run_configured_negamax(board, 1, alpha, beta, 0, &table, |searcher| {
        searcher.follow_pv[0] = follow_pv;
        searcher.killers[0][0] = Some(target);
    });
    let hit = table.probe(search_key(board), 0).unwrap();
    if searched {
        assert_eq!(score, beta);
        assert_eq!(hit.best_move, Some(target));
        assert_eq!(hit.value.unwrap().bound, Bound::Lower);
    } else {
        // targetを読めばその場でβ打ち切りになる。後続手の値は問わない。
        assert_ne!(hit.best_move, Some(target));
    }
}

// 王手になる静かな飛車移動は、斜めの角行に取られるSEEの損により読まない。
#[test]
fn see_pruning_skips_losing_quiet_check() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(5, 8), Color::White, PieceKind::King),
            (sq(4, 4), Color::Black, PieceKind::Rook),
            (sq(6, 3), Color::White, PieceKind::Bishop),
            (sq(0, 11), Color::Black, PieceKind::Pawn),
            (sq(0, 5), Color::White, PieceKind::Rook),
        ],
    );
    let target = Move {
        from: sq(4, 4),
        mid: None,
        to: sq(5, 4),
        promote: false,
    };
    assert_quiet_see_search(&board, target, true, true, false, false);
}

fn losing_quiet_non_check() -> (Position, Move) {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(4, 4), Color::Black, PieceKind::Rook),
            (sq(6, 3), Color::White, PieceKind::Bishop),
            (sq(0, 11), Color::Black, PieceKind::Pawn),
            (sq(0, 5), Color::White, PieceKind::Rook),
        ],
    );
    let target = Move {
        from: sq(4, 4),
        mid: None,
        to: sq(5, 4),
        promote: false,
    };
    (board, target)
}

// 王手にならない静かな飛車移動は、斜めの角行に取られるSEEの損により読まない。
#[test]
fn quiet_see_pruning_skips_losing_non_check() {
    let (board, target) = losing_quiet_non_check();
    assert_quiet_see_search(&board, target, false, false, false, false);
}

// followPVのPVノードでは、同じ王手にならない静かな手を読む。
#[cfg(feature = "tuning")]
pub(super) fn quiet_see_pruning_searches_follow_pv_move() {
    let (board, target) = losing_quiet_non_check();
    assert_quiet_see_search(&board, target, false, true, false, true);
}

// 置換表の手では、同じ王手にならない静かな手を読む。
#[test]
fn quiet_see_pruning_searches_tt_move() {
    let (board, target) = losing_quiet_non_check();
    assert_quiet_see_search(&board, target, false, false, true, true);
}

// 成る角行の静かな手は王手になりSEEが負でも、成る手の保護により読む。
#[cfg(feature = "tuning")]
pub(super) fn see_pruning_searches_promoting_quiet_check() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(6, 8), Color::White, PieceKind::King),
            (sq(4, 7), Color::Black, PieceKind::Bishop),
            (sq(0, 11), Color::Black, PieceKind::Pawn),
            (sq(0, 5), Color::White, PieceKind::Rook),
        ],
    );
    let target = Move {
        from: sq(4, 7),
        mid: None,
        to: sq(5, 8),
        promote: true,
    };
    assert_quiet_see_search(&board, target, true, false, false, true);
}

// 経由升を持つ獅子の捕獲は規則依存なので展開する。
#[cfg(feature = "tuning")]
pub(super) fn see_pruning_searches_rule_dependent_capture() {
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
    assert!(!see_prunes(&board, engine_rules(), nnue, capture, -200));
    assert!(!royal_under_attack(&board));
    let static_eval = evaluate(nnue, &board);
    // S4の見積りを1だけ上回らせず、判定不能の手がS3へ進むことを検査する。
    let (_, futility_value) = capture_futility_context(&board, capture, quiet, 2);
    let alpha = futility_value - 1;
    assert!(alpha > static_eval);
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
                None,
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
        None,
        Bound::Upper,
        Some(quiet),
        0,
        false,
    );
    let (score, _) = run_move_pruning(&board, 2, alpha, alpha + 1, 0, &table);
    assert!(score > static_eval);
    let hit = table.probe(search_key(&board), 0).unwrap();
    assert_eq!(hit.value.unwrap().score, score);
    let best = hit.best_move.unwrap();
    assert!(best.mid.is_some(), "規則依存の捕獲を探索した値で更新する");
    let mut child = board.clone();
    child.make_move_unchecked(best, engine_rules());
    assert_eq!(search_key(&child), search_key(&target));
}

// 捕獲手のfutility式が成立しても、最後の王駒の捕獲は詰みとして返し、保存する。
#[test]
fn capture_futility_searches_last_royal_capture_after_quiet_tt_move() {
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
    let ply = 0;
    let (lmr_depth, futility_value) = capture_futility_context(&board, capture, quiet, 1);
    assert!(lmr_depth < super::super::pruning::CAPTURE_FUTILITY_LMR_DEPTH_LIMIT);
    let alpha = futility_value.max(evaluate(test_nnue(), &board) + 1);
    assert!(alpha.abs() < REPETITION_BAND_START && (alpha + 1).abs() < REPETITION_BAND_START);
    let table = small_tt();
    let mut quiet_child = board.clone();
    quiet_child.make_move_unchecked(quiet, engine_rules());
    table.store(
        search_key(&quiet_child),
        MAX_PLY,
        -(alpha - 1),
        None,
        Bound::Exact,
        None,
        ply + 1,
        false,
    );
    table.store(
        search_key(&board),
        0,
        evaluate(test_nnue(), &board),
        None,
        Bound::Upper,
        Some(quiet),
        ply,
        false,
    );
    let (score, _) = run_move_pruning(&board, 1, alpha, alpha + 1, ply, &table);
    assert_eq!(score, MATE - (ply + 1) as i32);
    let hit = table.probe(search_key(&board), ply).unwrap();
    assert_eq!(hit.value.unwrap().score, score);
    assert_eq!(hit.best_move, Some(capture));
}

// F1の仕様表の数値例。2項の独立した丸めとi64の積も検査する。
#[test]
fn reverse_futility_margin_matches_numeric_examples() {
    let pawn = test_nnue().pawn_value();
    assert_eq!(pawn, 100);
    for (depth, tt_hit, improving, opponent_worsening, correction, expected) in [
        (1, true, false, false, 0, 41),
        (1, false, false, false, 0, 41),
        (2, true, false, true, 0, 73),
        (3, true, true, false, 0, 63),
        (1, true, true, false, 0, -18),
        (10, true, false, false, 0, 410),
        (18, false, false, false, 0, 738),
        (1, true, false, false, -50, 103),
        (1, true, false, false, 7, 49),
        (1, true, true, false, 7, -10),
    ] {
        assert_eq!(
            reverse_coefficients().reverse_futility_margin(
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
    // 1,000,000×41×1780/10000=7,298,000。積72,980,000,000はi32の範囲を超える。
    assert_eq!(
        reverse_coefficients().reverse_futility_margin(1_000_000, 18, true, false, true, 0),
        7_298_000
    );
}

#[test]
fn reverse_futility_value_matches_numeric_examples() {
    for (beta, eval, expected) in [
        (100, 1124, 463),
        (-300, -300, -300),
        (-100, 1000, 289),
        (-1000, -500, -822),
    ] {
        assert_eq!(reverse_futility_value(beta, eval), expected);
    }
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

// 根ではopponentWorseningが真。各深さの余裕値に一致する窓で検査する。
#[test]
fn reverse_futility_returns_weighted_value_at_margin_without_storing() {
    let board = bare_kings();
    let value = evaluate(test_nnue(), &board);
    // F5ではallNodeにIIRを適用しないので、要求深さをそのまま使う。
    for requested_depth in [1, 2, 4, 18] {
        let margin = root_reverse_margin(requested_depth, false);
        let table = small_tt();
        let beta = value - margin.max(0);
        assert_eq!(
            run_configured_negamax(
                &board,
                requested_depth,
                beta - 1,
                beta,
                0,
                &table,
                |searcher| zero_position_correction(searcher, &board),
            ),
            (expected_reverse_futility_value(beta, value), 0)
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
    let beta = value - root_reverse_margin(1, false).max(0);
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
            run_configured_negamax(position, depth, alpha, beta, 0, &small_tt(), |s| {
                s.seek_mate = true;
                s.null_move_ply = Some(0);
            })
            .1 > 0,
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
    // 深さ1、照合成功、opponentWorseningありの余裕値を使う。
    let beta = value - root_reverse_margin(1, true);
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
        table.store(
            search_key(&board),
            0,
            value,
            None,
            Bound::Upper,
            mv,
            0,
            is_pv,
        );
        let (score, nodes) =
            run_configured_negamax(&board, 1, beta - 1, beta, 0, &table, |searcher| {
                zero_position_correction(searcher, &board)
            });
        if pruned {
            assert_eq!(
                (score, nodes),
                (expected_reverse_futility_value(beta, value), 0)
            );
            let hit = table.probe(search_key(&board), 0).unwrap();
            assert_eq!(
                (
                    hit.value.unwrap().depth,
                    hit.value.unwrap().score,
                    hit.best_move
                ),
                (0, value, mv)
            );
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
            searcher.seek_mate = true;
            zero_position_correction(searcher, &board);
            let value = evaluate(searcher.nnue, &board);
            searcher.accumulators[2] = searcher.accumulators[0].clone();
            searcher.material_keys[2] = searcher.material_keys[0];
            searcher.static_evals[0] = value - 1;
            searcher.static_evals[1] = -value;
            set_test_path_ply(searcher, 2);
            let margin = reverse_futility_margin(100, 1, false, true, false, 0);
            let beta = value + i32::from(!at_beta);

            let score = searcher
                .negamax(&mut board.clone(), 1, beta - 1, beta, 2, false)
                .unwrap();
            if at_beta && margin <= 0 {
                assert_eq!(
                    (score, searcher.nodes),
                    (expected_reverse_futility_value(beta, value), 0)
                );
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
        None,
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

// 判定用の評価値が閾値と等しければ通常探索し、1小さければ深さの上限なく静止探索の値を返す。
// 捕獲候補のない局面では静的評価を返し、置換表へ保存しない。
#[test]
fn razoring_uses_a_strict_boundary_without_a_depth_limit() {
    assert_eq!(test_nnue().pawn_value(), 100, "固定ネットの歩兵価値");
    let position = bare_kings();
    let nnue = test_nnue();
    let static_eval = evaluate(nnue, &position);
    assert!(!royal_under_attack(&position));
    for depth in [1, 2, 4] {
        let margin = razoring_margin(nnue.pawn_value(), depth);
        let boundary_alpha = static_eval + margin;
        let (_, quiet_nodes) = run_quiesce(
            &position,
            boundary_alpha,
            boundary_alpha + 1,
            0,
            &small_tt(),
        );
        let (_, boundary_nodes) = run_configured_negamax(
            &position,
            depth,
            boundary_alpha,
            boundary_alpha + 1,
            0,
            &small_tt(),
            |searcher| zero_position_correction(searcher, &position),
        );
        assert!(
            boundary_nodes > quiet_nodes,
            "evalが閾値と等しい場合は通常探索する: depth={depth}"
        );

        let alpha = boundary_alpha + 1;
        let (quiet_score, quiet_nodes) = run_quiesce(&position, alpha, alpha + 1, 0, &small_tt());
        assert_eq!(quiet_score, static_eval);
        let table = small_tt();
        let (score, nodes) =
            run_configured_negamax(&position, depth, alpha, alpha + 1, 0, &table, |searcher| {
                zero_position_correction(searcher, &position)
            });
        assert_eq!((score, nodes), (quiet_score, quiet_nodes));
        assert!(nodes < boundary_nodes);
        assert!(
            table.probe(search_key(&position), 0).is_none(),
            "捕獲のない静止探索は置換表に保存しない"
        );
    }
}

// allNodeだけが対象であり、同じ零窓でもcutNodeは対象にしない。
#[test]
fn razoring_only_applies_to_all_nodes() {
    let position = bare_kings();
    let nnue = test_nnue();
    let depth = 1;
    let alpha = evaluate(nnue, &position) + razoring_margin(nnue.pawn_value(), depth) + 1;
    let (quiet_score, quiet_nodes) = run_quiesce(&position, alpha, alpha + 1, 0, &small_tt());
    assert_eq!(
        run_negamax(&position, depth, alpha, alpha + 1, 0, &small_tt()),
        (quiet_score, quiet_nodes),
        "allNodeでは静止探索へ移る"
    );
    for (name, beta, cut_node) in [("PV", alpha + 2, false), ("cut", alpha + 1, true)] {
        with_root_searcher(&position, &[], |searcher| {
            searcher
                .negamax(&mut position.clone(), depth, alpha, beta, 0, cut_node)
                .unwrap();
            assert!(searcher.nodes > quiet_nodes, "{name}Nodeでは通常探索する");
        });
    }
}

// 王駒に相手の疑似利きが届く場合は通常探索を行う。
#[test]
fn razoring_does_not_run_when_a_royal_is_attacked() {
    let nnue = test_nnue();
    let position = minase_core::test_util::position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(11, 6), Color::White, PieceKind::Rook),
        ],
    );
    assert!(royal_under_attack(&position));
    let depth = 1;
    let alpha = evaluate(nnue, &position) + razoring_margin(nnue.pawn_value(), depth) + 1;
    let (_, quiet_nodes) = run_quiesce(&position, alpha, alpha + 1, 0, &small_tt());
    let (_, nodes) = run_negamax(&position, depth, alpha, alpha + 1, 0, &small_tt());
    assert!(nodes > quiet_nodes, "王駒への利きがあれば通常探索する");
}

// 静止探索の値がαを超えても、その値をそのまま返して通常探索の深さでは保存しない。
#[test]
fn razoring_returns_quiescence_above_alpha() {
    let position = bare_kings();
    let nnue = test_nnue();
    let raw = evaluate(nnue, &position);
    let depth = 1;
    let alpha = raw - 1;
    let beta = raw;
    let tt_eval = alpha - razoring_margin(nnue.pawn_value(), depth) - 1;
    assert!(!royal_under_attack(&position));
    let table = small_tt();
    table.store(
        search_key(&position),
        0,
        tt_eval,
        None,
        Bound::Upper,
        None,
        0,
        false,
    );
    let (score, nodes) = run_negamax(&position, depth, alpha, beta, 0, &table);
    assert_eq!((score, nodes), (raw, 0));
    assert!(score > alpha);
    let hit = table.probe(search_key(&position), 0).unwrap();
    assert_eq!(
        (
            hit.value.unwrap().depth,
            hit.value.unwrap().score,
            hit.value.unwrap().bound
        ),
        (0, tt_eval, Bound::Upper)
    );
}

// S7の数値例。調整中の既定値とは独立した固定係数で、2段階の整数除算と上限を持たない加算を検査する。
#[test]
fn null_move_reduction_matches_s7_numeric_examples() {
    let coefficients = null_coefficients();
    for (depth, difference, expected) in [(16, 0, 11), (16, 500, 15), (3, 2000, 23)] {
        assert_eq!(
            coefficients.null_move_reduction(depth, difference, 0, 100),
            expected,
            "depth={depth}, difference={difference}"
        );
    }
    assert_eq!(coefficients.null_move_reduction(16, -1, 0, 100), 11);
}

// NullMoveEvalScaleの範囲は0以上なので、補正後の静的評価が増えても減深量は減らない。
#[test]
fn null_move_reduction_is_monotone_in_corrected_static_eval() {
    let lower = null_move_reduction(16, 0, 0, 100);
    assert!(null_move_reduction(16, 1000, 0, 100) >= lower);
}

// S7の試行条件の数値例。余裕値151とimprovingの減算23、先行する打ち切り2回の加算48を検査する。
#[test]
fn null_move_condition_matches_s7_numeric_examples() {
    let coefficients = null_coefficients();
    assert!(coefficients.allows_null_move(4, 151, 0, 100, false, 0));
    assert!(!coefficients.allows_null_move(4, 150, 0, 100, false, 0));
    assert!(coefficients.allows_null_move(4, 128, 0, 100, true, 0));
    assert!(!coefficients.allows_null_move(4, 127, 0, 100, true, 0));
    assert!(coefficients.allows_null_move(4, 103, 0, 100, false, 2));
    assert!(!coefficients.allows_null_move(4, 102, 0, 100, false, 2));
}

// 補正が判定用の評価値を動かすと、補正前の値では成立した枝刈りも抑止する。
#[test]
fn correction_can_prevent_reverse_futility_and_razoring() {
    assert_eq!(test_nnue().pawn_value(), 100, "固定ネットの歩兵価値");
    let position = bare_kings();
    for reverse in [true, false] {
        with_root_searcher(&position, &[], |searcher| {
            searcher.seek_mate = reverse;
            if !reverse {
                searcher.tt.store(
                    search_key(&position),
                    0,
                    evaluate(searcher.nnue, &position),
                    None,
                    Bound::Lower,
                    None,
                    0,
                    true,
                );
            }
            let side = position.side_to_move();
            let key = material_key(&position);
            let bonus = if reverse { -1024 } else { 1024 };
            searcher
                .correction
                .update(side, key, CorrectionTableKind::Material, bonus, 128);
            assert_ne!(position_correction(searcher, side, 0), 0);
            let raw = evaluate(searcher.nnue, &position);
            let alpha = if reverse {
                raw - root_reverse_margin(1, false) - 1
            } else {
                raw + razoring_margin(searcher.nnue.pawn_value(), 1) + 1
            };
            let mut board = position.clone();
            let score = searcher
                .negamax(&mut board, 1, alpha, alpha + 1, 0, false)
                .unwrap();
            assert!(searcher.nodes > 0, "補正により通常探索へ進む");
            let hit = searcher.tt.probe(search_key(&position), 0).unwrap();
            assert_eq!(hit.value.unwrap().depth, 1);
            assert_eq!(hit.value.unwrap().score, score);
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
    let values = new_correction_values();
    let seeded = super::super::correction::CorrectionTable::new(&values);
    seeded.update(
        board.side_to_move(),
        material_key(&board),
        CorrectionTableKind::Material,
        -1024,
        128,
    );
    let correction = correction_from_weighted_sum(
        i64::from(params::correction_material_weight())
            * i64::from(seeded.read(
                board.side_to_move(),
                material_key(&board),
                CorrectionTableKind::Material,
            )),
        100,
    );
    assert!(correction < 0);
    let margin = reverse_futility_margin(100, 1, false, false, true, correction).max(0);
    let beta = evaluate(test_nnue(), &board) + correction - margin + 1;
    for corrected in [false, true] {
        with_root_searcher(&board, &[], |searcher| {
            if corrected {
                searcher.correction.update(
                    board.side_to_move(),
                    material_key(&board),
                    CorrectionTableKind::Material,
                    -1024,
                    128,
                );
            } else {
                zero_position_correction(searcher, &board);
            }
            let score = searcher
                .negamax(&mut board.clone(), 1, beta - 1, beta, 0, false)
                .unwrap();
            if corrected {
                assert!(searcher.nodes > 0);
            } else {
                assert_eq!(
                    score,
                    expected_reverse_futility_value(beta, evaluate(test_nnue(), &board))
                );
                assert_eq!(searcher.nodes, 0);
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
            zero_position_correction(searcher, &board);
            searcher.seek_mate = true;
            let raw = evaluate(searcher.nnue, &board);
            let value = raw + 500;
            // 照合成功の余裕値を使い、下界を採る場合だけβに達する。
            let beta = value - root_reverse_margin(1, true);
            searcher
                .tt
                .store(search_key(&board), 0, value, None, bound, None, 0, false);
            let score = searcher
                .negamax(&mut board.clone(), 1, beta - 1, beta, 0, false)
                .unwrap();
            if bound == Bound::Lower {
                assert_eq!(score, expected_reverse_futility_value(beta, value));
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
    let board = bare_kings();
    for bound in [Bound::Upper, Bound::Lower] {
        with_root_searcher(&board, &[], |searcher| {
            zero_position_correction(searcher, &board);
            let raw = evaluate(searcher.nnue, &board);
            // 静的評価では余裕値に1足りず、低い上界を採れば静止探索へ移る。
            let alpha = raw + razoring_margin(searcher.nnue.pawn_value(), 1) - 1;
            searcher
                .tt
                .store(search_key(&board), 0, raw - 18, None, bound, None, 0, true);
            let (quiet, _) = run_quiesce(&board, alpha, alpha + 1, 0, &small_tt());
            let score = searcher
                .negamax(&mut board.clone(), 1, alpha, alpha + 1, 0, false)
                .unwrap();
            if bound == Bound::Upper && quiet <= alpha {
                assert_eq!(searcher.nodes, 0);
                assert_eq!(score, raw, "捕獲のない静止探索は補正前の評価を返す");
            } else {
                assert!(searcher.nodes > 0);
            }
        });
    }
}

#[cfg(feature = "tuning")]
pub(super) fn null_move_reduction_uses_corrected_static_eval_not_tt_value() {
    assert_eq!(test_nnue().pawn_value(), 100);
    let boundary = (20_000 + params::null_move_eval_scale() - 1) / params::null_move_eval_scale();
    let depth: u32 = 15;
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
            let beta = raw - (boundary - 1);
            let base_child_depth = depth.saturating_sub(
                1 + null_move_reduction(depth, beta, beta, searcher.nnue.pawn_value()),
            );
            assert!(base_child_depth >= 1, "対照ではnull moveの子を1手以上読む");
            zero_position_correction(searcher, &board);
            if corrected {
                searcher.correction.update(
                    board.side_to_move(),
                    material_key(&board),
                    CorrectionTableKind::Material,
                    4,
                    128,
                );
                assert_eq!(position_correction(searcher, board.side_to_move(), 0), 1);
            }
            // 判定用の評価値だけをβ未満にしてreverse futilityを避ける。
            // 減深量は、この値ではなく補正後の静的評価から求める。
            searcher.tt.store(
                search_key(&board),
                0,
                beta - 1,
                None,
                Bound::Upper,
                None,
                0,
                true,
            );
            searcher.follow_pv[0] = true;
            let mut passed = board.clone();
            passed.make_null_move();
            // 補正なしの子には1手だけ深さが足りず、補正で追加減深が1増えた場合だけ打ち切れる上界を置く。
            searcher.tt.store(
                search_key(&passed),
                base_child_depth - 1,
                -beta,
                None,
                Bound::Upper,
                None,
                1,
                false,
            );
            let score = searcher
                .negamax(&mut board.clone(), depth, beta - 1, beta, 0, true)
                .unwrap();
            if corrected {
                assert_eq!(score, beta);
                assert_eq!(searcher.nodes, 0, "追加減深後の深さで置換表が打ち切る");
            } else {
                assert!(searcher.nodes > 0, "基本減深の子では置換表の深さが足りない");
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

// 数値例は係数表の調整と独立に固定する。F1/F4の式に対し、
// f7c0925cの手計算例で使った入力係数を明記する。
pub(super) fn late_move_coefficients() -> super::super::pruning::LateMoveCoefficients {
    super::super::pruning::LateMoveCoefficients {
        lmp_non_improving: 31,
        lmp_base: 1799,
        lmp_slope: 48,
    }
}
pub(super) fn reverse_coefficients() -> super::super::pruning::ReverseFutilityCoefficients {
    super::super::pruning::ReverseFutilityCoefficients {
        reverse_futility_mult_base: 235,
        reverse_futility_mult_slope: 4,
        reverse_futility_no_tt_hit: 0,
        reverse_futility_improving: 144,
        reverse_futility_opponent_worsening: 20,
        reverse_futility_correction: 124,
    }
}
pub(super) fn null_coefficients() -> super::super::pruning::NullMoveCoefficients {
    super::super::pruning::NullMoveCoefficients {
        null_move_base: 7200,
        null_move_slope: 400,
        null_move_eval_scale: 163,
        null_move_eval_margin: 175,
        null_move_eval_depth: 6,
        null_move_eval_improving: 23,
        null_move_fail_high_bonus: 24,
    }
}
pub(super) fn lmr_coefficients() -> super::super::pruning::LmrCoefficients {
    super::super::pruning::LmrCoefficients {
        lmr_non_improving: 3,
        lmr_base: 127,
        lmr_pv_reduction: 6,
        lmr_tt_pv: 70,
        lmr_history_threshold: 35,
        lmr_history_scale: 439,
        lmr_offset: 106,
        lmr_move_count: 0,
        lmr_correction: 101,
        lmr_tt_pv_base: 337,
        lmr_tt_pv_pv: 13,
        lmr_tt_pv_value: 95,
        lmr_tt_pv_depth: 77,
        lmr_tt_pv_depth_cut_node: 90,
        lmr_cut_node: 164,
        lmr_cut_node_no_tt_move: 59,
        lmr_tt_capture: 0,
        lmr_cutoff_count: 48,
        lmr_cutoff_count_many: 232,
        lmr_cutoff_count_all_node: 129,
        lmr_tt_move: 269,
        lmr_alpha_eval: 46,
        lmr_all_node: 142,
    }
}

fn isolated_lmr_coefficients(reduction: i32) -> LmrCoefficients {
    LmrCoefficients {
        lmr_non_improving: 0,
        lmr_base: reduction,
        lmr_pv_reduction: 0,
        lmr_tt_pv: 0,
        lmr_history_threshold: 0,
        lmr_history_scale: 0,
        lmr_offset: 0,
        lmr_move_count: 0,
        lmr_correction: 0,
        lmr_tt_pv_base: 0,
        lmr_tt_pv_pv: 0,
        lmr_tt_pv_value: 0,
        lmr_tt_pv_depth: 0,
        lmr_tt_pv_depth_cut_node: 0,
        lmr_cut_node: 0,
        lmr_cut_node_no_tt_move: 0,
        lmr_tt_capture: 0,
        lmr_cutoff_count: 0,
        lmr_cutoff_count_many: 0,
        lmr_cutoff_count_all_node: 0,
        lmr_tt_move: 0,
        lmr_alpha_eval: 0,
        lmr_all_node: 0,
    }
}

fn example_lmr_reduction(c: LmrContext) -> i32 {
    let table = if c.index == 0 {
        0
    } else {
        super::super::pruning::lmr_base(c.depth as usize, c.index.min(255), 147)
    };
    lmr_coefficients().lmr_reduction(table, c)
}

fn example_lmr_depth(depth: u32, reduction: i32, pv: bool, ply: u32, root: u32) -> u32 {
    super::super::pruning::lmr_depth_with_coefficient(depth, reduction, pv, ply, root, 3)
}
fn example_non_lmr_depth(normal: u32, reduction: i32, has_tt: bool) -> u32 {
    super::super::pruning::non_lmr_depth_with_coefficient(normal, reduction, has_tt, 57)
}
