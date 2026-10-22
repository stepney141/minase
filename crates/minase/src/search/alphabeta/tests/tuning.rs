//! 固定係数での算術と、調整オプションの反映を検査する。

use super::*;
#[cfg(feature = "tuning")]
use crate::search::alphabeta::params;
#[cfg(feature = "tuning")]
use crate::search::alphabeta::{
    correction::{
        CorrectionTable, CorrectionTableKind, correction_from_weighted_sum, correction_update_bonus,
    },
    pruning::{
        capture_futility_value, continuation_history_prunes, futility_value, late_move_limit,
        lmr_base, quiet_see_margin, razoring_margin, reverse_futility_margin, see_margin,
    },
    root::{grow_aspiration_delta, initial_aspiration_delta},
};

/// 時計のdocコメントの式から手計算した例。各項の丸めと上限を個別に検査する。
#[test]
fn tuning_default_clock_budget_matches_numeric_examples() {
    // f7c0925cの時間配分の算術例で使った係数を固定する。
    let coefficients = super::super::time::ClockCoefficients {
        expected_plies: 458,
        min_moves: 40,
        increment_share: 87,
        hard_soft_ratio: 730,
        hard_remaining_share: 39,
        time_advantage: 900,
    };
    // Mの初期値229と下限40の境界、加算87%、hard倍率730%、残り39%を検査する。
    // 相手の残り時間は自分と同じなのでtime advantageは働かず、既存の算術例を保つ。
    // 秒読みの序盤係数は変わらず、各項の除算を先に行ってから合算する。
    for (name, remaining, increment, byoyomi, ply, soft, hard) in [
        ("initial division below", 228, 0, 0, 0, 0, 1),
        ("initial division exact", 229, 0, 0, 0, 1, 7),
        ("initial division above", 230, 0, 0, 0, 1, 7),
        ("increment rounds down", 229, 1, 0, 0, 1, 7),
        ("increment fraction", 229, 9, 0, 0, 8, 58),
        ("increment next integer", 229, 10, 0, 0, 9, 65),
        ("increment above integer", 229, 11, 0, 0, 10, 73),
        ("separate truncations", 230, 1, 12, 0, 1, 7),
        ("opening fraction reaches integer", 229, 0, 13, 0, 2, 14),
        ("opening below cap", 21100, 0, 1000, 35, 880, 6424),
        ("opening at cap", 21100, 0, 1000, 36, 900, 6570),
        ("opening after cap", 21000, 0, 1000, 37, 900, 6570),
        ("moves before floor", 8200, 0, 0, 376, 200, 1460),
        ("moves at floor", 8000, 0, 0, 377, 200, 1460),
        ("moves after floor", 8000, 0, 0, 378, 200, 1460),
        ("moves beyond game", 8000, 0, 0, u32::MAX, 200, 1460),
        ("remaining cap", 1000, 1000, 0, 0, 390, 390),
        ("safety cap", 31, 1000, 0, 0, 1, 1),
        ("safety below zero", 29, 1000, 0, 0, 1, 1),
        ("byoyomi without main time", 0, 100, 1000, 0, 800, 800),
        ("large increment is capped", 1000, u64::MAX, 0, 0, 390, 390),
        (
            "final duration saturates",
            u64::MAX,
            u64::MAX,
            u64::MAX,
            458,
            u64::MAX,
            u64::MAX,
        ),
    ] {
        let actual = coefficients.budget(clock_at_ply(remaining, increment, byoyomi, ply), 30);
        assert_eq!(
            actual,
            TimeBudget {
                byoyomi_period: false,
                soft: Duration::from_millis(soft),
                hard: Duration::from_millis(hard),
            },
            "{name}"
        );
    }
}

/// 全係数の計算への反映を直列に検査する。
/// グローバル係数が既存の並列テストへ漏れないよう、このテストだけを子プロセスで走らせる。
#[cfg(feature = "tuning")]
#[test]
fn tuning_parameters_in_isolated_process() {
    const CHILD: &str = "MINASE_TUNING_TEST_CHILD";
    let scenario = std::env::var(CHILD);
    if scenario.is_err() {
        for scenario in [
            "lmr table",
            "lmr max reduction",
            "m5 root history",
            "m5 root lmr",
            "parameters",
            "time instability integration",
            "disabled capture history",
            "disabled history decay",
            "disabled capture history decay",
            "lmp base",
            "lmp slope",
            "lmp non improving",
            "lmp zero limit",
            "r2 lmp zero limit",
            "r2 lmp small limit",
            "r2 lmp large limit",
            "reverse futility margin",
            "razoring depth margin",
            "null move eval scale",
            "disabled null move eval scale",
            "fixed shallow pruning behavior",
            "fixed history and capture pruning",
        ] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "search::alphabeta::tests::tuning::tuning_parameters_in_isolated_process",
                    "--nocapture",
                ])
                .env(CHILD, scenario)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{scenario}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(
                stdout.lines().any(|line| line
                    .starts_with("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;")),
                "{scenario}: {stdout}"
            );
        }
        return;
    }
    let scenario = scenario.unwrap();
    // 各Tuneオプションの効果を独立に比較するための固定入力。
    // f7c0925cの算術例と同じ係数を子プロセス内だけで設定する。
    let fixture = [
        ("LmrDivisor", 147),
        ("LmrHistoryThreshold", 35),
        ("LmrHistoryScale", 439),
        ("LmrMaxReduction", 3),
        ("LmrBase", 127),
        ("LmrOffset", 106),
        ("LmrNonImproving", 3),
        ("LmrPvReduction", 6),
        ("LmrTtPv", 70),
        ("LmrTtPvBase", 337),
        ("LmrTtPvPv", 13),
        ("LmrTtPvValue", 95),
        ("LmrTtPvDepth", 77),
        ("LmrTtPvDepthCutNode", 90),
        ("LmrMoveCount", 0),
        ("LmrCorrection", 101),
        ("LmrCutNode", 164),
        ("LmrCutNodeNoTtMove", 59),
        ("LmrTtCapture", 0),
        ("LmrCutoffCount", 48),
        ("LmrCutoffCountMany", 232),
        ("LmrCutoffCountAllNode", 129),
        ("LmrTtMove", 269),
        ("LmrAlphaEval", 46),
        ("LmrNoTtMove", 57),
        ("LmrAllNode", 142),
        ("LmrDeeperMargin", 82),
        ("LmrShallowerMargin", 26),
        ("ContinuationPruning", 4000),
        ("FutilityBase", 79),
        ("FutilityLmrDepth", 57),
        ("FutilityAboveAlpha", 43),
        ("QuietSeeMargin", 11),
        ("LmpBase", 1799),
        ("LmpSlope", 48),
        ("LmpNonImproving", 31),
        ("ReverseFutilityMultBase", 235),
        ("ReverseFutilityMultSlope", 4),
        ("ReverseFutilityNoTtHit", 0),
        ("ReverseFutilityImproving", 144),
        ("ReverseFutilityOpponentWorsening", 20),
        ("ReverseFutilityCorrection", 124),
        ("RazoringDepthMargin", 164),
        ("SeeDepthMargin", 85),
        ("SeeCaptureHistory", 34),
        ("CaptureFutilityBase", 113),
        ("CaptureFutilityLmrDepth", 119),
        ("CaptureFutilityHistory", 134),
        ("AspirationDelta", 57),
        ("AspirationScoreWidth", 204),
        ("AspirationGrowth", 168),
        ("NullMoveBase", 7200),
        ("NullMoveSlope", 400),
        ("NullMoveEvalScale", 163),
        ("NullMoveEvalMargin", 175),
        ("NullMoveEvalDepth", 6),
        ("NullMoveEvalImproving", 23),
        ("NullMoveFailHighBonus", 24),
        ("HistoryBonusSlope", 111),
        ("HistoryBonusBase", 70),
        ("HistoryBonusCap", 1400),
        ("HistoryMalusSlope", 900),
        ("HistoryMalusBase", 200),
        ("HistoryMalusCap", 2100),
        ("HistoryMalusDecay", 900),
        ("HistoryTtCutoffBonus", 100),
        ("HistoryTtMoveBonus", 300),
        ("HistoryPriorBonus", 200),
        ("HistoryEvalDiff", 10),
        ("HistoryDecay", 37),
        ("CaptureHistoryDecay", 13),
        ("CaptureHistoryScale", 155),
        ("OrderingCheckBonus", 12000),
        ("OrderingThreatWeight", 3000),
        ("CorrectionMaterialWeight", 277),
        ("CorrectionPawnWeight", 52),
        ("CorrectionMinorWeight", 36),
        ("CorrectionNonPawnWeight", 44),
        ("CorrectionContinuationWeight", 30),
        ("CorrectionContinuation6Weight", 24),
        ("CorrectionBonus", 20),
        ("CorrectionFailLowBonus", 29),
        ("DeltaMargin", 500),
        ("QsearchSeeMargin", 36),
        ("QsearchMoveLimit", 6),
        ("ExpectedPlies", 458),
        ("MinMoves", 40),
        ("IncrementShare", 87),
        ("HardSoftRatio", 730),
        ("HardRemainingShare", 39),
        ("IterationRatio", 277),
        ("StableIterations", 4),
        ("TimeFallingBase", 115),
        ("TimeFallingPrevious", 4784),
        ("TimeFallingIteration", 2288),
        ("TimeReductionIntercept", 254),
        ("TimeReductionSlope", 78),
        ("TimeReductionScale", 2284),
        ("TimeInstabilityBase", 1077),
        ("TimeInstabilitySlope", 2229),
        ("TimeEffortIntercept", 1642),
        ("TimeEffortSlope", -888),
        ("TimeAdvantage", 900),
        ("FmScale", 1024),
    ];
    for (name, value) in fixture {
        params::set(name, value).unwrap();
    }

    if scenario == "time instability integration" {
        // 時間境界を確実に分離できる値へ固定し、SPSA後の既定値には依存させない。
        params::set("HardSoftRatio", 800).unwrap();
        params::set("IterationRatio", 150).unwrap();
        super::time::instability_scaling_starts_an_iteration_that_the_base_soft_rejects();
        super::time::initial_previous_time_reduction_controls_the_first_scaled_budget();
        params::set("TimeFallingBase", 1000).unwrap();
        params::set("TimeFallingPrevious", 0).unwrap();
        params::set("TimeFallingIteration", 0).unwrap();
        params::set("TimeReductionIntercept", 1000).unwrap();
        params::set("TimeReductionSlope", 0).unwrap();
        params::set("TimeReductionScale", 2500).unwrap();
        params::set("TimeInstabilityBase", 1000).unwrap();
        params::set("TimeInstabilitySlope", 0).unwrap();
        params::set("TimeEffortIntercept", 2500).unwrap();
        params::set("TimeEffortSlope", -2000).unwrap();
        super::time::best_move_effort_scaling_ignores_shared_auxiliary_nodes();
        params::set("TimeEffortIntercept", 1000).unwrap();
        params::set("TimeEffortSlope", 0).unwrap();
        super::time::single_root_move_scaling_stops_at_500_millis_boundary();
        return;
    }

    if scenario == "disabled capture history" {
        params::set("CaptureHistoryScale", 0).unwrap();
        super::captures::zero_capture_scale_preserves_reference_order();
        return;
    }
    if scenario == "disabled history decay" {
        params::set("HistoryDecay", 0).unwrap();
        params::set("CaptureHistoryDecay", 0).unwrap();
        super::history::zero_decay_clears_history_between_searches();
        return;
    }

    if scenario == "disabled capture history decay" {
        params::set("CaptureHistoryDecay", 0).unwrap();
        super::capture_history::zero_decay_clears_capture_history_between_searches();
        return;
    }

    if let Some(limit) = match scenario.as_str() {
        "r2 lmp zero limit" => Some(0),
        "r2 lmp small limit" => Some(3),
        "r2 lmp large limit" => Some(20),
        _ => None,
    } {
        params::set("LmpBase", limit * 100).unwrap();
        params::set("LmpSlope", 0).unwrap();
        super::repetition_prohibition::articles_31_r2_and_23_forbidden_tt_killer_and_quiet_moves_do_not_consume_ranks_or_nodes();
        return;
    }

    match scenario.as_str() {
        "lmr max reduction" => {
            super::pruning::lmr_max_reduction_parameter_sets_cap();
            return;
        }
        "m5 root history" => {
            super::root::root_history_updates_best_and_only_searched_non_improving_quiets();
            return;
        }
        "m5 root lmr" => {
            super::root::root_lmr_research_records_only_the_final_score();
            return;
        }
        "lmp base" => {
            late_move_parameter_changes_search("LmpBase");
            return;
        }
        "lmp slope" => {
            late_move_parameter_changes_search("LmpSlope");
            return;
        }
        "lmp non improving" => {
            super::late_move::late_move_non_improving_parameter_scales_limit();
            return;
        }
        "lmp zero limit" => {
            params::set("LmpBase", 0).unwrap();
            params::set("LmpSlope", 0).unwrap();
            super::late_move::check_late_move_protected_moves(0);
            super::late_move::late_move_pruning_searches_safe_quiets_after_losing_tt_move();
            let board = super::late_move::quiet_board();
            let (score, nodes) = super::late_move::search_quiet_board(&board);
            assert_eq!(nodes, 1, "上限0でも最初の安全な手は読む");
            assert!(score.abs() < MATE_THRESHOLD);
            return;
        }
        "reverse futility margin" => {
            reverse_futility_parameter_changes_search();
            return;
        }
        "razoring depth margin" => {
            razoring_parameter_changes_search();
            return;
        }
        "null move eval scale" => {
            null_move_eval_scale_changes_reduction();
            return;
        }
        "disabled null move eval scale" => {
            zero_null_move_eval_scale_matches_base_reduction();
            return;
        }
        "fixed shallow pruning behavior" => {
            // 上の固定係数では、LMPの上限は深さ1のimprovingで18、非improvingで5、
            // 深さ4の非improvingで7となり、各局面の静かな手を一部だけ読む。
            super::history::quiet_history_negamax_updates_only_searched_quiets();
            super::late_move::late_move_pruning_uses_improving_updated_after_null_move();
            super::late_move::late_move_pruning_applies_at_depth_four_and_follow_pv_nodes();
            super::late_move::late_move_pruning_does_not_apply_when_alpha_or_beta_is_in_reserved_band();
            super::late_move::late_move_pruning_does_not_apply_while_royal_is_attacked();
            super::late_move::late_move_pruning_preserves_protected_moves_and_quiet_allowance();
            super::late_move::late_move_pruning_searches_exactly_the_first_counted_quiets_up_to_limit();
            super::pruning::futility_protects_quiet_checks_before_late_move_limit();
            super::pruning::see_pruning_applies_without_depth_or_zero_window_limits();
            super::pruning::quiet_see_pruning_searches_follow_pv_move();
            super::pruning::see_pruning_searches_promoting_quiet_check();
            super::pruning::see_pruning_searches_rule_dependent_capture();
            super::repetition_band::r1_repetition_band_windows_disable_see_pruning();
            super::pruning::null_move_reduction_uses_corrected_static_eval_not_tt_value();
            // 深さ15と16のnull moveの子深さは2と3になり、置換表で値を固定できる。
            #[cfg(feature = "search-stats")]
            super::null_move::verification_starts_at_depth_threshold_and_returns_original_null_score();
            return;
        }
        "fixed history and capture pruning" => {
            super::history_forms::fail_low_and_eval_difference_update_recorded_histories();
            super::pruning::see_pruning_searches_first_capture_without_false_mate();
            return;
        }
        _ => {}
    }

    if scenario == "lmr table" {
        // 表の生成前に除数1.0を設定する。floor(100×ln4×ln3)=152。
        params::set("LmrDivisor", 100).unwrap();
        assert_eq!(lmr_table()[4][3], 152);
        return;
    }
    assert_eq!(scenario, "parameters");
    fn material_correction() -> i64 {
        let values = new_correction_values();
        let table = CorrectionTable::new(&values);
        table.update(Color::Black, 7, CorrectionTableKind::Material, 31, 128);
        i64::from(correction_from_weighted_sum(
            i64::from(params::correction_material_weight())
                * i64::from(table.read(Color::Black, 7, CorrectionTableKind::Material)),
            100,
        ))
    }
    fn pawn_correction() -> i64 {
        i64::from(correction_from_weighted_sum(
            i64::from(params::correction_pawn_weight()) * 1024,
            100,
        ))
    }
    fn minor_correction() -> i64 {
        i64::from(correction_from_weighted_sum(
            i64::from(params::correction_minor_weight()) * 1024,
            100,
        ))
    }
    fn non_pawn_correction() -> i64 {
        i64::from(correction_from_weighted_sum(
            i64::from(params::correction_non_pawn_weight()) * 1024,
            100,
        ))
    }
    fn normal_correction_bonus() -> i64 {
        i64::from(
            correction_update_bonus(
                50,
                3,
                Bound::Exact,
                false,
                params::correction_bonus(),
                params::correction_fail_low_bonus(),
                100,
            )
            .unwrap(),
        )
    }
    fn fail_low_correction_bonus() -> i64 {
        i64::from(
            correction_update_bonus(
                -50,
                3,
                Bound::Upper,
                false,
                params::correction_bonus(),
                params::correction_fail_low_bonus(),
                100,
            )
            .unwrap(),
        )
    }
    fn capture_history_adjustment() -> i64 {
        let board = staged_picker_fixture();
        let mv = Move {
            from: fs(6, 8),
            to: fs(6, 5),
            mid: None,
            promote: false,
        };
        let mut result = 0;
        with_root_searcher(&board, &[], |searcher| {
            searcher.capture_history.update(
                &board,
                searcher.nnue,
                mv,
                super::super::capture_history::CAPTURE_HISTORY_LIMIT,
            );
            result = i64::from(
                searcher
                    .capture_history
                    .adjustment(&board, searcher.nnue, mv),
            );
        });
        result
    }
    fn history_decay() -> i64 {
        let snapshot = snapshot_for(&Position::initial());
        let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
        histories.workers[0][0][60][60] = 100;
        run_search_team(
            test_nnue(),
            &snapshot.position,
            snapshot.rules(),
            snapshot.repetition(),
            DRAW_SCORE,
            &snapshot.root_moves,
            &snapshot.history_keys,
            &depth_limits(1),
            &AtomicBool::new(true),
            DEFAULT_THREADS,
            &small_tt(),
            &mut histories,
            None,
            &ManualClock::new(Duration::ZERO),
            &AtomicU64::new(0),
            false,
        );
        i64::from(histories.workers[0][0][60][60])
    }
    fn qsearch_limit() -> i64 {
        let board = position(
            Color::Black,
            &[
                (sq(11, 0), Color::Black, PieceKind::King),
                (sq(11, 11), Color::White, PieceKind::King),
                (sq(5, 5), Color::Black, PieceKind::Rook),
                (sq(5, 7), Color::White, PieceKind::Pawn),
                (sq(7, 5), Color::White, PieceKind::Pawn),
            ],
        );
        run_quiesce(&board, -INFINITY, INFINITY, MAX_PLY - 1, &small_tt()).1 as i64
    }
    fn delta() -> i64 {
        let mut result = 0;
        with_root_searcher(&Position::initial(), &[], |searcher| {
            result = i64::from(searcher.delta_margin);
        });
        result
    }
    fn prediction() -> i64 {
        i64::from(iteration_prediction_fits(
            Duration::from_millis(50),
            Duration::ZERO,
            TimeBudget {
                byoyomi_period: false,
                soft: Duration::from_millis(100),
                hard: Duration::from_millis(200),
            },
            true,
        ))
    }
    fn stability() -> i64 {
        let best = legal_moves(&Position::initial())[0];
        i64::from(stable_signal(
            &[best; 4],
            params::stable_iterations() as usize,
        ))
    }
    fn falling_eval() -> i64 {
        i64::from(falling_eval_factor(
            Some(20),
            10,
            0,
            100,
            FallingEvalCoefficients::current(),
        ))
    }
    fn best_move_stability() -> i64 {
        let coefficients = TimeReductionCoefficients::current();
        let reduction = time_reduction(5, coefficients);
        i64::from(best_move_stability_factor(
            850,
            reduction,
            coefficients.scale,
        ))
    }
    fn best_move_instability() -> i64 {
        i64::try_from(best_move_instability_factor(
            3072,
            2,
            TimeInstabilityCoefficients::current(),
        ))
        .unwrap()
    }
    fn best_move_effort() -> i64 {
        i64::from(best_move_effort_factor(
            1,
            1,
            TimeEffortCoefficients::current(),
        ))
    }
    fn time_advantage() -> i64 {
        let clock = ClockLimits::new(100_000, 0, 0, Some(200_000), 0).unwrap();
        i64::try_from(clock_budget(clock).soft.as_millis()).unwrap()
    }

    // FmScaleはNNUEには作用しないため、非零の2駒関係を持つ固定重みで係数の反映を検査する。
    fn fm_evaluation() -> i64 {
        use crate::eval::features::FEATURE_COUNT;
        use crate::eval::pst::{self, FM_RANK, FmWeights, Pst};

        static PST: OnceLock<Pst> = OnceLock::new();
        let pst = PST.get_or_init(|| {
            let mut embedding = [0; FM_RANK];
            embedding[0] = 1;
            Pst::from_weights_with_fm(
                &vec![[0; 2]; FEATURE_COUNT],
                std::array::from_fn(|state| test_nnue().piece_value_of_state(state)),
                test_nnue().cp_per_logit(),
                "engine-default",
                FmWeights {
                    embeddings: &vec![embedding; FEATURE_COUNT],
                    signs: &[1; FM_RANK],
                    exponent: 0,
                },
            )
            .unwrap()
        });
        i64::from(pst::evaluate(pst, &Position::initial()))
    }

    // 一度構築した評価関数にも調整値が反映されることを含めて調べる。
    fm_evaluation();
    type Case = (&'static str, i32, fn() -> i64);
    let cases: [Case; 104] = [
        ("LmrDivisor", 400, || {
            i64::from(lmr_base(8, 16, params::lmr_divisor()))
        }),
        ("LmrHistoryThreshold", 512, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                history: Some(256),
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrHistoryScale", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                stat_score: 10_000,
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrMaxReduction", 1, || {
            i64::from(super::super::pruning::lmr_depth(8, 469, false, 0, 8))
        }),
        ("LmrBase", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrOffset", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrNonImproving", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrPvReduction", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                pv_node: true,
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrTtPv", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                tt_pv: true,
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrTtPvBase", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                tt_pv: true,
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrTtPvPv", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                tt_pv: true,
                pv_node: true,
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrTtPvValue", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                tt_pv: true,
                tt_hit: Some((1, 8)),
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrTtPvDepth", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                tt_pv: true,
                tt_hit: Some((0, 8)),
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrTtPvDepthCutNode", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                tt_pv: true,
                tt_hit: Some((0, 8)),
                cut_node: true,
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrMoveCount", 6, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrCorrection", 200, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                correction_weighted_sum: 102_400,
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrCutNode", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                cut_node: true,
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrCutNodeNoTtMove", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                cut_node: true,
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrTtCapture", 100, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                tt_capture: Some(true),
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrCutoffCount", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                cutoff_count: 3,
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrCutoffCountMany", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                cutoff_count: 3,
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrCutoffCountAllNode", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                cutoff_count: 3,
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrTtMove", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                is_tt_move: true,
                tt_capture: Some(false),
                history: None,
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrAlphaEval", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                alpha: 20,
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrNoTtMove", 0, || {
            i64::from(super::super::pruning::non_lmr_depth(5, 500, false))
        }),
        ("LmrAllNode", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrDeeperMargin", 100, || {
            i64::from(super::super::pruning::lmr_research_depth(
                7, 4, 133, 50, 100,
            ))
        }),
        ("LmrShallowerMargin", 0, || {
            i64::from(super::super::pruning::lmr_research_depth(7, 4, 52, 50, 100))
        }),
        ("ContinuationPruning", 12_000, || {
            i64::from(continuation_history_prunes(
                -6000,
                1,
                params::continuation_pruning(),
            ))
        }),
        ("FutilityBase", 100, || {
            i64::from(futility_value(100, 0, 50, 2))
        }),
        ("FutilityLmrDepth", 100, || {
            i64::from(futility_value(100, 0, 50, 2))
        }),
        ("FutilityAboveAlpha", 100, || {
            i64::from(futility_value(100, 60, 50, 0))
        }),
        ("QuietSeeMargin", 100, || {
            i64::from(quiet_see_margin(100, 3))
        }),
        ("LmpBase", 600, || i64::from(late_move_limit(1, false))),
        ("LmpSlope", 400, || i64::from(late_move_limit(1, false))),
        ("LmpNonImproving", 150, || {
            i64::from(late_move_limit(1, false))
        }),
        ("ReverseFutilityMultBase", 0, || {
            i64::from(reverse_futility_margin(101, 1, true, false, false, 0))
        }),
        ("ReverseFutilityMultSlope", 0, || {
            i64::from(reverse_futility_margin(101, 1, true, false, false, 0))
        }),
        ("ReverseFutilityNoTtHit", 20, || {
            i64::from(reverse_futility_margin(101, 1, false, false, false, 0))
        }),
        ("ReverseFutilityImproving", 0, || {
            i64::from(reverse_futility_margin(101, 1, true, true, false, 0))
        }),
        ("ReverseFutilityOpponentWorsening", 0, || {
            i64::from(reverse_futility_margin(101, 1, true, false, true, 0))
        }),
        ("ReverseFutilityCorrection", 0, || {
            i64::from(reverse_futility_margin(101, 1, true, false, false, 100))
        }),
        ("RazoringDepthMargin", 100, || {
            i64::from(razoring_margin(101, 2))
        }),
        ("SeeDepthMargin", 100, || i64::from(see_margin(101, 2, 0))),
        ("SeeCaptureHistory", 100, || {
            i64::from(see_margin(208, 1, 1024))
        }),
        ("CaptureFutilityBase", 0, || {
            i64::from(capture_futility_value(100, 0, 1, 100, 0))
        }),
        ("CaptureFutilityLmrDepth", 0, || {
            i64::from(capture_futility_value(100, 0, 1, 100, 0))
        }),
        ("CaptureFutilityHistory", 100, || {
            i64::from(capture_futility_value(208, 0, 0, 100, 1024))
        }),
        ("AspirationDelta", 100, || {
            i64::from(initial_aspiration_delta(
                101,
                params::aspiration_delta(),
                params::aspiration_score_width(),
                0,
                0,
            ))
        }),
        ("AspirationScoreWidth", 400, || {
            i64::from(initial_aspiration_delta(
                100,
                params::aspiration_delta(),
                params::aspiration_score_width(),
                1_000_000,
                0,
            ))
        }),
        ("AspirationGrowth", 300, || {
            i64::from(grow_aspiration_delta(101))
        }),
        ("NullMoveBase", 1200, || {
            i64::from(null_move_reduction(12, 0, 0, 100))
        }),
        ("NullMoveSlope", 100, || {
            i64::from(null_move_reduction(12, 0, 0, 100))
        }),
        ("NullMoveEvalScale", 0, || {
            i64::from(null_move_reduction(12, 800, 0, 100))
        }),
        ("NullMoveEvalMargin", -1000, || {
            i64::from(allows_null_move(4, 150, 0, 100, false, 0))
        }),
        ("NullMoveEvalDepth", 100, || {
            i64::from(allows_null_move(4, 0, 0, 100, false, 0))
        }),
        ("NullMoveEvalImproving", 400, || {
            i64::from(allows_null_move(4, 0, 0, 100, true, 0))
        }),
        ("NullMoveFailHighBonus", 200, || {
            i64::from(allows_null_move(4, 0, 0, 100, false, 1))
        }),
        ("HistoryBonusSlope", 400, || {
            i64::from(
                super::super::ordering::HistoryUpdateCoefficients::current()
                    .updates(1, false, 0, false, 0)
                    .bonus,
            )
        }),
        ("HistoryBonusBase", -100, || {
            i64::from(
                super::super::ordering::HistoryUpdateCoefficients::current()
                    .updates(1, false, 0, false, 0)
                    .bonus,
            )
        }),
        ("HistoryBonusCap", 200, || {
            i64::from(
                super::super::ordering::HistoryUpdateCoefficients::current()
                    .updates(20, false, 0, false, 0)
                    .bonus,
            )
        }),
        ("HistoryMalusSlope", 0, || {
            i64::from(
                super::super::ordering::HistoryUpdateCoefficients::current()
                    .updates(1, false, 0, false, 0)
                    .malus,
            )
        }),
        ("HistoryMalusBase", -200, || {
            i64::from(
                super::super::ordering::HistoryUpdateCoefficients::current()
                    .updates(1, false, 0, false, 0)
                    .malus,
            )
        }),
        ("HistoryMalusCap", 200, || {
            i64::from(
                super::super::ordering::HistoryUpdateCoefficients::current()
                    .updates(10, false, 0, false, 0)
                    .malus,
            )
        }),
        ("HistoryMalusDecay", 700, || {
            let updates = super::super::ordering::HistoryUpdateCoefficients::current()
                .updates(10, false, 0, false, 0);
            i64::from(updates.decay_quiet_malus(updates.quiet_malus()))
        }),
        ("HistoryTtCutoffBonus", 0, || {
            i64::from(super::super::ordering::tt_cutoff_history_bonus(
                3,
                params::history_tt_cutoff_bonus(),
            ))
        }),
        ("HistoryTtMoveBonus", 0, || {
            i64::from(
                super::super::ordering::HistoryUpdateCoefficients::current()
                    .updates(1, true, 0, false, 0)
                    .bonus,
            )
        }),
        ("HistoryPriorBonus", 0, || {
            i64::from(
                super::super::ordering::fail_low_history_updates(
                    super::super::ordering::FailLowHistoryInput {
                        depth: 4,
                        best_score: 0,
                        static_eval: 208,
                        previous_static_eval: -208,
                        previous_stat_score: 0,
                        previous_move_count: 10,
                        pawn_value: 208,
                    },
                    params::history_prior_bonus(),
                )
                .butterfly,
            )
        }),
        ("HistoryEvalDiff", 0, || {
            i64::from(
                super::super::ordering::eval_difference_history_updates(
                    -208,
                    0,
                    208,
                    params::history_eval_diff(),
                )
                .butterfly,
            )
        }),
        ("HistoryDecay", 50, history_decay),
        (
            "CaptureHistoryDecay",
            50,
            super::capture_history::capture_decay_probe,
        ),
        ("CaptureHistoryScale", 200, capture_history_adjustment),
        ("OrderingCheckBonus", 0, || {
            i64::from(super::super::ordering::QuietOrderingCoefficients::current().check_bonus)
        }),
        ("OrderingThreatWeight", 0, || {
            i64::from(super::super::ordering::QuietOrderingCoefficients::current().threat_weight)
        }),
        ("CorrectionMaterialWeight", 500, material_correction),
        ("CorrectionPawnWeight", 200, pawn_correction),
        ("CorrectionMinorWeight", 200, minor_correction),
        ("CorrectionNonPawnWeight", 200, non_pawn_correction),
        ("CorrectionContinuationWeight", 200, || {
            i64::from(correction_from_weighted_sum(
                i64::from(params::correction_continuation_weight()) * 1024,
                100,
            ))
        }),
        ("CorrectionContinuation6Weight", 200, || {
            i64::from(correction_from_weighted_sum(
                i64::from(params::correction_continuation6_weight()) * 1024,
                100,
            ))
        }),
        ("CorrectionBonus", 60, normal_correction_bonus),
        ("CorrectionFailLowBonus", 90, fail_low_correction_bonus),
        ("DeltaMargin", 300, delta),
        ("QsearchSeeMargin", 200, || {
            let mut result = 0;
            with_root_searcher(&Position::initial(), &[], |searcher| {
                result = i64::from(searcher.qsearch_see_floor);
            });
            result
        }),
        ("QsearchMoveLimit", 1, qsearch_limit),
        ("ExpectedPlies", 250, || {
            clock_budget(clock(100_000, 0, 0)).soft.as_millis() as i64
        }),
        ("MinMoves", 200, || {
            clock_budget(clock_at_ply(100_000, 0, 0, 500))
                .soft
                .as_millis() as i64
        }),
        ("IncrementShare", 100, || {
            clock_budget(clock(100_000, 100, 0)).soft.as_millis() as i64
        }),
        ("HardSoftRatio", 800, || {
            clock_budget(clock(100_000, 0, 0)).hard.as_millis() as i64
        }),
        ("HardRemainingShare", 50, || {
            clock_budget(clock(1000, 1000, 0)).hard.as_millis() as i64
        }),
        ("IterationRatio", 150, prediction),
        ("StableIterations", 8, stability),
        ("TimeFallingBase", 200, falling_eval),
        ("TimeFallingPrevious", 4000, falling_eval),
        ("TimeFallingIteration", 2000, falling_eval),
        ("TimeReductionIntercept", 500, best_move_stability),
        ("TimeReductionSlope", 100, best_move_stability),
        ("TimeReductionScale", 1000, best_move_stability),
        ("TimeInstabilityBase", 1500, best_move_instability),
        ("TimeInstabilitySlope", 4000, best_move_instability),
        ("TimeEffortIntercept", 2000, best_move_effort),
        ("TimeEffortSlope", -1000, best_move_effort),
        ("TimeAdvantage", 0, time_advantage),
        ("FmScale", 0, fm_evaluation),
    ];
    assert_eq!(cases.len(), params::PARAMETERS.len());
    for ((name, value, observe), &(expected_name, _default, min, max)) in
        cases.into_iter().zip(params::PARAMETERS)
    {
        assert_eq!(name, expected_name);
        // multの上限で傾きが隠れない入力を、この係数の検査に限って用意する。
        let mult_base = params::reverse_futility_mult_base();
        if name == "ReverseFutilityMultSlope" {
            params::set("ReverseFutilityMultBase", 0).unwrap();
        }
        let original = fixture.iter().find(|&&(n, _)| n == name).unwrap().1;
        let before = observe();
        params::set(name, value).unwrap();
        assert_ne!(observe(), before, "{name} must affect its calculation");
        assert!(matches!(
            params::set(name, min - 1),
            Err(params::Error::OutOfRange { .. })
        ));
        assert!(matches!(
            params::set(name, max + 1),
            Err(params::Error::OutOfRange { .. })
        ));
        params::set(name, min).unwrap();
        params::set(name, max).unwrap();
        params::set(name, original).unwrap();
        assert_eq!(observe(), before);
        if name == "ReverseFutilityMultSlope" {
            params::set("ReverseFutilityMultBase", mult_base).unwrap();
        }
    }
    assert!(
        matches!(params::set("Unknown", 0), Err(params::Error::UnknownName(name)) if name == "Unknown")
    );
}

// 各係数の変更は枝刈りの境界へ反映される。実際の探索で境界をまたぎ、読み出し忘れを検出する。
#[cfg(feature = "tuning")]
fn reverse_futility_parameter_changes_search() {
    let board = position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 1), Color::Black, PieceKind::Pawn),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    );
    let nnue = test_nnue();
    let value = evaluate(nnue, &board);
    assert_eq!(params::reverse_futility_mult_base(), 235, "数値例の前提");
    assert_eq!(nnue.pawn_value(), 100, "固定ネットの歩兵価値");
    // 既定の余裕値32に1足りない窓。基数0なら余裕値は3となり枝刈りする。
    let beta = value - 31;
    let expected = ((661 * i64::from(beta) + 363 * i64::from(value)) / 1024) as i32;
    assert!(run_negamax(&board, 1, beta - 1, beta, 0, &small_tt()).1 > 0);
    params::set("ReverseFutilityMultBase", 0).unwrap();
    assert_eq!(
        run_negamax(&board, 1, beta - 1, beta, 0, &small_tt()),
        (expected, 0)
    );
    assert_eq!(
        reverse_futility_margin(137, 1, false, false, true, 0),
        4,
        "百分率は乗算してから割る"
    );
}

#[cfg(feature = "tuning")]
fn razoring_parameter_changes_search() {
    let board = bare_kings();
    let nnue = test_nnue();
    let value = evaluate(nnue, &board);
    assert_eq!(nnue.pawn_value(), 100, "固定ネットの歩兵価値");
    let depth = 1;
    let original = params::razoring_depth_margin();
    let changed = original - 1;
    let alpha = value + changed + 1;
    assert!(run_negamax(&board, depth, alpha, alpha + 1, 0, &small_tt()).1 > 0);
    params::set("RazoringDepthMargin", changed).unwrap();
    assert_eq!(
        run_negamax(&board, depth, alpha, alpha + 1, 0, &small_tt()),
        (value, 0)
    );
}

#[cfg(feature = "tuning")]
fn null_move_eval_scale_changes_reduction() {
    let before = null_move_reduction(16, 500, 0, 100);
    params::set("NullMoveEvalScale", 0).unwrap();
    let without_eval_term = null_move_reduction(16, 500, 0, 100);
    assert!(without_eval_term < before);
    params::set("NullMoveEvalScale", 50).unwrap();
    let base = null_move_reduction(6, 0, 0, 37);
    assert_eq!(null_move_reduction(6, 147, 0, 37), base);
    assert_eq!(null_move_reduction(6, 148, 0, 37), base + 1);
    params::set("NullMoveEvalScale", 400).unwrap();
    assert!(null_move_reduction(6, 1000, 0, 37) > base + 3);
}

#[cfg(feature = "tuning")]
fn zero_null_move_eval_scale_matches_base_reduction() {
    params::set("NullMoveEvalScale", 0).unwrap();
    for depth in [0, 1, 4, 16, MAX_PLY] {
        let expected =
            (params::null_move_base() as u32 + depth * params::null_move_slope() as u32) / 1200;
        for value in [i32::MIN, -1000, 0, 1000, i32::MAX] {
            for beta in [-MATE_THRESHOLD + 1, 0, MATE_THRESHOLD - 1] {
                assert_eq!(null_move_reduction(depth, value, beta, 37), expected);
            }
        }
    }
}

#[cfg(feature = "tuning")]
fn late_move_parameter_changes_search(name: &str) {
    let board = super::late_move::quiet_board();
    let base = params::lmp_base();
    let slope = params::lmp_slope();
    let scale = params::lmp_non_improving();
    let limit = (base + slope) * scale / 10000;
    assert_eq!(super::late_move::search_quiet_board(&board).1, limit as u64);
    let numerator = ((limit + 1) * 10000 + scale - 1) / scale;
    let boundary = match name {
        "LmpBase" => numerator - slope,
        "LmpSlope" => numerator - base,
        _ => unreachable!(),
    };
    params::set(name, boundary).unwrap();
    assert_eq!(
        super::late_move::search_quiet_board(&board).1,
        (limit + 1) as u64
    );
    params::set(name, boundary - 1).unwrap();
    assert_eq!(super::late_move::search_quiet_board(&board).1, limit as u64);
}
