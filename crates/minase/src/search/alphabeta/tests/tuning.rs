//! 探索係数の既定値と設定を検査する。

use super::*;
use crate::search::alphabeta::params;
#[cfg(feature = "tuning")]
use crate::search::alphabeta::{
    correction::CorrectionTable,
    pruning::{
        futility_margin, late_move_limit, lmr_base, razoring_margin, reverse_futility_margin,
        see_margin,
    },
    root::grow_aspiration_delta,
};

/// 時計のdocコメントの式から手計算した例。各項の丸めと上限を個別に検査する。
#[test]
fn tuning_default_clock_budget_matches_numeric_examples() {
    assert_eq!(params::expected_plies(), 432, "数値例の前提");
    assert_eq!(params::min_moves(), 88, "数値例の前提");
    assert_eq!(params::increment_share(), 76, "数値例の前提");
    assert_eq!(params::hard_soft_ratio(), 451, "数値例の前提");
    assert_eq!(params::hard_remaining_share(), 27, "数値例の前提");
    // R/M、加算76%、秒読みの序盤係数を別々に丸め、hardは451%、残り27%、安全余裕で抑える。
    for (name, remaining, increment, byoyomi, ply, soft, hard) in [
        // M=432/2=216。215/216→0、hardは下限1 ms。216/216=217/216→1、1×451/100→4 ms。
        ("initial division below", 215, 0, 0, 0, 0, 1),
        ("initial division exact", 216, 0, 0, 0, 1, 4),
        ("initial division above", 217, 0, 0, 0, 1, 4),
        // 加算1,9,10,11の76%は0,6,7,8。基礎1との和は1,7,8,9、451%は4,31,36,40。
        ("increment rounds down", 216, 1, 0, 0, 1, 4),
        ("increment fraction", 216, 9, 0, 0, 7, 31),
        ("increment next integer", 216, 10, 0, 0, 8, 36),
        ("increment above integer", 216, 11, 0, 0, 9, 40),
        // 217/216→1、1×76/100→0、12×8×4/400→0。和1、hard4 ms。
        ("separate truncations", 217, 1, 12, 0, 1, 4),
        // 216/216=1、13×8×4/400→1。和2、2×451/100→9 ms。
        ("opening fraction reaches integer", 216, 0, 13, 0, 2, 9),
        // ply35ではM198、19800/198=100、1000×8×39/400=780。和880、451%→3968。
        ("opening below cap", 19800, 0, 1000, 35, 880, 3968),
        // ply36ではM198、ply37ではM197。いずれもR/M=100、秒読み800。900×451/100=4059。
        ("opening at cap", 19800, 0, 1000, 36, 900, 4059),
        ("opening after cap", 19700, 0, 1000, 37, 900, 4059),
        // ply254はM89、ply255以後は下限88。R/M=200、200×451/100=902。
        ("moves before floor", 17800, 0, 0, 254, 200, 902),
        ("moves at floor", 17600, 0, 0, 255, 200, 902),
        ("moves after floor", 17600, 0, 0, 256, 200, 902),
        ("moves beyond game", 17600, 0, 0, u32::MAX, 200, 902),
        // 1000×27/100=270がhardを制限し、softも270 msになる。
        ("remaining cap", 1000, 1000, 0, 0, 270, 270),
        // 残り31−余裕30=1 ms。残り29では減算を0に飽和させ、hard下限1 ms。
        ("safety cap", 31, 1000, 0, 0, 1, 1),
        ("safety below zero", 29, 1000, 0, 0, 1, 1),
        // R=0では秒読みの80%=800 ms。加算76 msを含むsoft_raw876もhard800へ収める。
        ("byoyomi without main time", 0, 100, 1000, 0, 800, 800),
        // 加算が最大でも残り1000の27%=270 msで両予算を抑える。
        ("large increment is capped", 1000, u64::MAX, 0, 0, 270, 270),
        // 各項をu128で合算しても、最後のDuration変換ではu64::MAX msに飽和する。
        (
            "final duration saturates",
            u64::MAX,
            u64::MAX,
            u64::MAX,
            432,
            u64::MAX,
            u64::MAX,
        ),
    ] {
        let actual = clock_budget(clock_at_ply(remaining, increment, byoyomi, ply));
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
            "parameters",
            "capture history limits",
            "disabled capture history",
            "quiet history scales",
            "disabled history decay",
            "disabled capture history decay",
            "non improving futility 1",
            "non improving futility 2",
            "non improving futility 3",
            "lmp base",
            "lmp slope",
            "lmp non improving",
            "lmp zero limit",
            "r2 lmp zero limit",
            "r2 lmp small limit",
            "r2 lmp large limit",
            "reverse futility margin",
            "razoring margin 1",
            "razoring margin 2",
            "null move eval scale",
            "disabled null move eval scale",
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
    if scenario == "capture history limits" {
        for limit in [4096, 65536] {
            params::set("CaptureHistoryLimit", limit).unwrap();
            super::capture_history::capture_history_gravity_stays_bounded_at_limit_extremes();
        }
        return;
    }
    if scenario == "quiet history scales" {
        super::history::check_quiet_history_scales();
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
        "non improving futility 1" => {
            non_improving_parameter_changes_search(1, "NonImprovingFutility1");
            return;
        }
        "non improving futility 2" => {
            non_improving_parameter_changes_search(2, "NonImprovingFutility2");
            return;
        }
        "non improving futility 3" => {
            non_improving_parameter_changes_search(3, "NonImprovingFutility3");
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
        "razoring margin 1" => {
            razoring_parameter_changes_search(1, "RazoringMargin1");
            return;
        }
        "razoring margin 2" => {
            razoring_parameter_changes_search(2, "RazoringMargin2");
            return;
        }
        "null move eval scale" => {
            null_move_eval_scale_changes_search();
            return;
        }
        "disabled null move eval scale" => {
            zero_null_move_eval_scale_matches_base_reduction_search();
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
    fn correction(difference: i32) -> i64 {
        let mut table = CorrectionTable::new(100);
        table.update(Color::Black, 7, difference, 8);
        i64::from(table.read(Color::Black, 7))
    }
    fn history() -> i64 {
        let board = Position::initial();
        let mv = legal_moves(&board)[0];
        let mut result = 0;
        with_root_searcher(&board, &[], |searcher| {
            let (side, from, to) = (
                board.side_to_move().index(),
                mv.from.dense_index(),
                mv.to.dense_index(),
            );
            // どちらの上限でも有効な値から、上限に応じた重力項の差を観測する。
            searcher.history[side][from][to] = 4096;
            searcher.record_quiet_beta_cutoff(&board, mv, &[], 8, 0);
            result = i64::from(searcher.history[side][from][to]);
        });
        result
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
            searcher
                .capture_history
                .record_cutoff(&board, searcher.nnue, mv, &[], 100);
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
    let cases: [Case; 69] = [
        ("LmrDivisor", 400, || {
            i64::from(lmr_base(8, 16, params::lmr_divisor()))
        }),
        ("LmrHistoryThreshold", 512, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                history: Some(256),
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
        ("LmrMoveCount", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
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
        ("LmrTtCapture", 0, || {
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
            i64::from(super::super::pruning::non_lmr_depth(5, 450, false))
        }),
        ("LmrAllNode", 0, || {
            i64::from(lmr_reduction(super::super::pruning::LmrContext {
                ..super::pruning::lmr_context()
            }))
        }),
        ("LmrDeeperMargin", 100, || {
            i64::from(super::super::pruning::lmr_research_depth(7, 4, 80, 50, 100))
        }),
        ("LmrShallowerMargin", 0, || {
            i64::from(super::super::pruning::lmr_research_depth(7, 4, 52, 50, 100))
        }),
        ("FutilityMargin1", 100, || {
            i64::from(futility_margin(101, 1, true))
        }),
        ("FutilityMargin2", 100, || {
            i64::from(futility_margin(101, 2, true))
        }),
        ("FutilityMargin3", 100, || {
            i64::from(futility_margin(101, 3, true))
        }),
        ("NonImprovingFutility1", 100, || {
            i64::from(futility_margin(101, 1, false))
        }),
        ("NonImprovingFutility2", 100, || {
            i64::from(futility_margin(101, 2, false))
        }),
        ("NonImprovingFutility3", 100, || {
            i64::from(futility_margin(101, 3, false))
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
        ("ReverseFutilityNoTtHit", 0, || {
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
        ("RazoringMargin1", 100, || {
            i64::from(razoring_margin(101, 1))
        }),
        ("RazoringMargin2", 100, || {
            i64::from(razoring_margin(101, 2))
        }),
        ("SeeMargin1", 100, || i64::from(see_margin(101, 1))),
        ("SeeMargin2", 100, || i64::from(see_margin(101, 2))),
        ("SeeMargin3", 100, || i64::from(see_margin(101, 3))),
        ("AspirationDelta", 100, || i64::from(aspiration_delta(101))),
        ("AspirationGrowth", 300, || {
            i64::from(grow_aspiration_delta(101))
        }),
        ("NullMoveBase", 4800, || {
            i64::from(null_move_reduction(12, 0, 0, 100))
        }),
        ("NullMoveSlope", 400, || {
            i64::from(null_move_reduction(12, 0, 0, 100))
        }),
        ("NullMoveEvalScale", 0, || {
            i64::from(null_move_reduction(12, 800, 0, 100))
        }),
        ("HistoryLimit", 65536, history),
        ("HistoryBonusScale", 200, history),
        ("HistoryMalusScale", 200, || {
            let board = Position::initial();
            let moves = legal_moves(&board);
            let winner = moves[0];
            let earlier = *moves.iter().find(|mv| mv.from != winner.from).unwrap();
            let mut result = 0;
            with_root_searcher(&board, &[], |searcher| {
                searcher.record_quiet_beta_cutoff(&board, winner, &[earlier], 3, 0);
                result = i64::from(
                    searcher.history[0][earlier.from.dense_index()][earlier.to.dense_index()],
                );
            });
            result
        }),
        ("HistoryDecay", 50, history_decay),
        (
            "CaptureHistoryDecay",
            50,
            super::capture_history::capture_decay_probe,
        ),
        ("CaptureHistoryLimit", 65536, capture_history_adjustment),
        ("CaptureHistoryScale", 200, capture_history_adjustment),
        ("CorrectionCap", 400, || correction(100_000)),
        ("CorrectionWeight", 64, || correction(100)),
        ("DeltaMargin", 300, delta),
        ("QsearchMoveLimit", 1, qsearch_limit),
        ("ExpectedPlies", 250, || {
            clock_budget(clock(100_000, 0, 0)).soft.as_millis() as i64
        }),
        ("MinMoves", 40, || {
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
        ("FmScale", 0, fm_evaluation),
    ];
    assert_eq!(cases.len(), params::PARAMETERS.len());
    for ((name, value, observe), &(expected_name, default, min, max)) in
        cases.into_iter().zip(params::PARAMETERS)
    {
        assert_eq!(name, expected_name);
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
        params::set(name, default).unwrap();
        assert_eq!(observe(), before);
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
    assert_eq!(params::reverse_futility_mult_base(), 22, "数値例の前提");
    assert_eq!(nnue.pawn_value(), 100, "固定ネットの歩兵価値");
    // 置換表なし、opponentWorseningあり。余裕値は100×14×67/10000→9。
    // 基数32へ変更後は100×24×67/10000→16。
    let beta = value - 9;
    assert_eq!(
        run_negamax(&board, 1, beta - 1, beta, 0, &small_tt()),
        (value, 0)
    );
    params::set("ReverseFutilityMultBase", 32).unwrap();
    assert!(run_negamax(&board, 1, beta - 1, beta, 0, &small_tt()).1 > 0);
    assert_eq!(
        reverse_futility_margin(137, 1, false, false, true, 0),
        22,
        "百分率は乗算してから割る"
    );
}

#[cfg(feature = "tuning")]
fn razoring_parameter_changes_search(depth: u32, name: &str) {
    let board = bare_kings();
    let nnue = test_nnue();
    let value = evaluate(nnue, &board);
    assert_eq!(params::razoring_margin1(), 1606, "数値例の前提");
    assert_eq!(params::razoring_margin2(), 2015, "数値例の前提");
    assert_eq!(nnue.pawn_value(), 100, "固定ネットの歩兵価値");
    // 100×1606/100=1606、100×2015/100=2015。
    // 変更後の137×1706/100→2337、137×2115/100→2897は、先に割ると1706と2115になる。
    let (margin, changed, expected) = match depth {
        1 => (1606, 1706, 2337),
        2 => (2015, 2115, 2897),
        _ => unreachable!(),
    };
    let alpha = value + margin;
    assert_eq!(
        run_negamax(&board, depth, alpha, alpha + 1, 0, &small_tt()),
        (value, 0)
    );
    params::set(name, changed).unwrap();
    assert!(run_negamax(&board, depth, alpha, alpha + 1, 0, &small_tt()).1 > 0);
    assert_eq!(
        razoring_margin(137, depth),
        expected,
        "百分率は乗算してから割る"
    );
}

#[cfg(feature = "tuning")]
fn null_move_eval_scale_changes_search() {
    let board = position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 1), Color::Black, PieceKind::GoldGeneral),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    );
    let nnue = test_nnue();
    assert_eq!(params::null_move_eval_scale(), 56, "数値例の前提");
    assert_eq!(params::null_move_base(), 3617, "数値例の前提");
    assert_eq!(params::null_move_slope(), 257, "数値例の前提");
    assert_eq!(nnue.pawn_value(), 100, "固定ネットの歩兵価値");
    // 追加減深1の境界はceil(2×100×100/56)=358。
    let beta = evaluate(nnue, &board) - 358;
    // 浅い上界でreverse futilityを避け、null moveの減深量だけを比較する。
    let run = || {
        let table = small_tt();
        table.store(
            search_key(&board),
            0,
            beta - 1,
            Bound::Upper,
            None,
            0,
            false,
        );
        run_negamax(&board, 7, beta - 1, beta, 0, &table)
    };
    let (_, default_nodes) = run();
    params::set("NullMoveEvalScale", 0).unwrap();
    let (score, changed_nodes) = run();
    assert!(score >= beta);
    assert!(changed_nodes > default_nodes);
    params::set("NullMoveEvalScale", 50).unwrap();
    // 深さ6の基本量は(3617+6×257)/1200=5159/1200→4。
    // 147×50/100→73、148×50/100=74。歩兵37の2倍74で割ると追加0,1。
    // 尺度400では19×400/100=76で追加1、最大の差は追加上限3。
    assert_eq!(null_move_reduction(6, 147, 0, 37), 4);
    assert_eq!(null_move_reduction(6, 148, 0, 37), 5);
    params::set("NullMoveEvalScale", 400).unwrap();
    assert_eq!(null_move_reduction(6, 19, 0, 37), 5);
    assert_eq!(null_move_reduction(6, i32::MAX, i32::MIN, 37), 7);
}

#[cfg(feature = "tuning")]
fn zero_null_move_eval_scale_matches_base_reduction_search() {
    params::set("NullMoveEvalScale", 0).unwrap();
    assert_eq!(params::null_move_base(), 3617, "数値例の前提");
    assert_eq!(params::null_move_slope(), 257, "数値例の前提");
    assert_eq!(MAX_PLY, 256, "数値例の前提");
    // 尺度0なら追加0。基本量(3617+d×257)/1200はd=0,4で3、5,9で4、10,12で5、256で57。
    for (depth, expected) in [
        (0, 3),
        (1, 3),
        (4, 3),
        (5, 4),
        (9, 4),
        (10, 5),
        (12, 5),
        (256, 57),
    ] {
        for value in [i32::MIN, -1000, 0, 1000, i32::MAX] {
            for beta in [-MATE_THRESHOLD + 1, 0, MATE_THRESHOLD - 1] {
                assert_eq!(null_move_reduction(depth, value, beta, 37), expected);
            }
        }
    }
    let board = position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 1), Color::Black, PieceKind::GoldGeneral),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    );
    let nnue = test_nnue();
    assert_eq!(nnue.pawn_value(), 100, "固定ネットの歩兵価値");
    let beta = evaluate(nnue, &board) - 200;
    // 記録手なしの深さ7は6となる。基本量5159/1200→4、子の深さ6−1−4=1。
    let child_depth = 1;
    let mut passed = board.clone();
    passed.make_null_move();
    let (reply, nodes) = run_negamax(&passed, child_depth, -beta, -beta + 1, 1, &small_tt());
    assert!(-reply >= beta);
    assert!(nodes > 0);
    let table = small_tt();
    // 判定用の評価値をβ未満にして、reverse futilityによる早期終了を避ける。
    table.store(
        search_key(&board),
        0,
        beta - 1,
        Bound::Upper,
        None,
        0,
        false,
    );
    assert_eq!(
        run_negamax(&board, 7, beta - 1, beta, 0, &table),
        (-reply, nodes)
    );
    let hit = table.probe(search_key(&board), 0).unwrap();
    assert_eq!(
        (hit.depth, hit.score),
        (0, beta - 1),
        "null moveの打ち切りは保存しない"
    );
}

// 子が反復で引き分けになる局面では、親の枝刈りの条件だけで展開数が決まる。
#[cfg(feature = "tuning")]
fn non_improving_parameter_changes_search(depth: u32, name: &str) {
    let board = bare_kings();
    let moves = legal_moves(&board);
    let history = repeated_root_children(&board, &moves);
    let nnue = test_nnue();
    assert_eq!(nnue.pawn_value(), 100, "固定ネットの歩兵価値");
    assert_eq!(params::futility_margin1(), 165, "数値例の前提");
    assert_eq!(params::futility_margin2(), 239, "数値例の前提");
    assert_eq!(params::futility_margin3(), 283, "数値例の前提");
    assert_eq!(params::non_improving_futility1(), 71, "数値例の前提");
    assert_eq!(params::non_improving_futility2(), 72, "数値例の前提");
    assert_eq!(params::non_improving_futility3(), 86, "数値例の前提");
    // 100×百分率/100の後に倍率を掛ける。165×71/100→117、239×72/100→172、283×86/100→243。
    let reduced = [117, 172, 243][depth as usize - 1];
    let alpha = evaluate(nnue, &board) + reduced;
    let search = || {
        let mut nodes = 0;
        with_root_searcher(&board, &history, |searcher| {
            searcher.tt.store(
                search_key(&board),
                0,
                0,
                Bound::Upper,
                Some(moves[0]),
                2,
                false,
            );
            searcher.static_evals[0] = evaluate(nnue, &board);
            assert!(!searcher.improving(evaluate(nnue, &board), 2));
            assert_eq!(
                searcher.negamax(&mut board.clone(), depth, alpha, alpha + 1, 2, false),
                Some(DRAW_SCORE)
            );
            nodes = searcher.nodes;
        });
        nodes
    };
    assert_eq!(search(), 1);
    params::set(name, 100).unwrap();
    assert_eq!(search(), moves.len() as u64);
    // 尺度100では、端数を含めて現行の余裕値と完全に一致する。
    // 歩兵価値×165,239,283%を0方向へ丸める。例: 137×239/100=327.43→327。
    for (pawn, expected) in [
        (1, [1, 2, 2]),
        (37, [61, 88, 104]),
        (100, [165, 239, 283]),
        (137, [226, 327, 387]),
        (999, [1648, 2387, 2827]),
    ] {
        assert_eq!(
            futility_margin(pawn, depth, false),
            expected[depth as usize - 1]
        );
        assert_eq!(
            futility_margin(pawn, depth, false),
            futility_margin(pawn, depth, true)
        );
    }
    params::set(name, 0).unwrap();
    assert_eq!(futility_margin(137, depth, false), 0);
    assert_eq!(search(), 1);
}

#[cfg(feature = "tuning")]
fn late_move_parameter_changes_search(name: &str) {
    let board = super::late_move::quiet_board();
    assert_eq!(params::lmp_base(), 300, "数値例の前提");
    assert_eq!(params::lmp_slope(), 100, "数値例の前提");
    // 深さ1でimprovingは偽。400×50/10000=2、分子の基数が600なら上限3。
    assert_eq!(super::late_move::search_quiet_board(&board).1, 2);
    let boundary = match name {
        "LmpBase" => 500,
        "LmpSlope" => 300,
        _ => unreachable!(),
    };
    params::set(name, boundary).unwrap();
    assert_eq!(super::late_move::search_quiet_board(&board).1, 3);
    // 境界より1小さい599では599×50/10000→2。
    params::set(name, boundary - 1).unwrap();
    assert_eq!(super::late_move::search_quiet_board(&board).1, 2);
}
