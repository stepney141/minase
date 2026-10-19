//! USIの調整係数の宣言、設定の受理・拒否、および最初のgo以後の固定を検査する。
#![cfg(feature = "tuning")]

use super::*;
use crate::search::alphabeta::params;

// 係数とgo後の固定状態がプロセス全体に属するので、各会話を別のプロセスで実行する。
#[test]
fn tuning_usi_contract_in_isolated_process() {
    const CHILD: &str = "MINASE_USI_TUNING_TEST_CHILD";
    let scenario = match std::env::var(CHILD) {
        Ok(scenario) => scenario,
        Err(std::env::VarError::NotPresent) => {
            for scenario in [
                "options",
                "go depth 1",
                "go ponder depth 1",
                "go depth nope",
                "go mate 1",
                "go",
            ] {
                let output = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        concat!(module_path!(), "::tuning_usi_contract_in_isolated_process")
                            .strip_prefix("minase::")
                            .unwrap(),
                        "--nocapture",
                    ])
                    .env(CHILD, scenario)
                    .output()
                    .unwrap();
                let stdout = String::from_utf8(output.stdout).unwrap();
                assert!(
                    output.status.success(),
                    "{scenario}: {stdout}\n{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(
                    stdout.lines().any(|line| line.starts_with(
                        "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;"
                    )),
                    "{scenario}: {stdout}"
                );
            }
            return;
        }
        Err(error) => panic!("invalid child scenario: {error}"),
    };
    let mut engine = make_engine(&[RuleCode::R1]);
    let mut protocol = UsiProtocol::new(&engine);
    if scenario == "options" {
        let handshake = run(&mut protocol, &mut engine, "usi\n");
        let declarations: Vec<_> = handshake
            .lines()
            .filter(|line| line.starts_with("option name Tune_"))
            .collect();
        let expected: Vec<_> = params::PARAMETERS
            .iter()
            .map(|(name, default, min, max)| {
                format!("option name Tune_{name} type spin default {default} min {min} max {max}")
            })
            .collect();
        assert_eq!(declarations, expected);
        // 受理した値は係数表の公開アクセサで読み、拒否後にも同じ値が残ることを確かめる。
        let accessors = [
            ("LmrDivisor", params::lmr_divisor as fn() -> i32),
            (
                "LmrHistoryThreshold",
                params::lmr_history_threshold as fn() -> i32,
            ),
            ("LmrMaxReduction", params::lmr_max_reduction as fn() -> i32),
            ("LmrBase", params::lmr_base as fn() -> i32),
            ("LmrOffset", params::lmr_offset as fn() -> i32),
            ("LmrNonImproving", params::lmr_non_improving as fn() -> i32),
            ("LmrPvReduction", params::lmr_pv_reduction as fn() -> i32),
            ("LmrTtPv", params::lmr_tt_pv as fn() -> i32),
            ("LmrTtPvBase", params::lmr_tt_pv_base as fn() -> i32),
            ("LmrTtPvPv", params::lmr_tt_pv_pv as fn() -> i32),
            ("LmrTtPvValue", params::lmr_tt_pv_value as fn() -> i32),
            ("LmrTtPvDepth", params::lmr_tt_pv_depth as fn() -> i32),
            (
                "LmrTtPvDepthCutNode",
                params::lmr_tt_pv_depth_cut_node as fn() -> i32,
            ),
            ("LmrMoveCount", params::lmr_move_count as fn() -> i32),
            ("LmrCutNode", params::lmr_cut_node as fn() -> i32),
            (
                "LmrCutNodeNoTtMove",
                params::lmr_cut_node_no_tt_move as fn() -> i32,
            ),
            ("LmrTtCapture", params::lmr_tt_capture as fn() -> i32),
            ("LmrCutoffCount", params::lmr_cutoff_count as fn() -> i32),
            (
                "LmrCutoffCountMany",
                params::lmr_cutoff_count_many as fn() -> i32,
            ),
            (
                "LmrCutoffCountAllNode",
                params::lmr_cutoff_count_all_node as fn() -> i32,
            ),
            ("LmrTtMove", params::lmr_tt_move as fn() -> i32),
            ("LmrAlphaEval", params::lmr_alpha_eval as fn() -> i32),
            ("LmrNoTtMove", params::lmr_no_tt_move as fn() -> i32),
            ("LmrAllNode", params::lmr_all_node as fn() -> i32),
            ("LmrDeeperMargin", params::lmr_deeper_margin as fn() -> i32),
            (
                "LmrShallowerMargin",
                params::lmr_shallower_margin as fn() -> i32,
            ),
            ("FutilityBase", params::futility_base as fn() -> i32),
            (
                "FutilityLmrDepth",
                params::futility_lmr_depth as fn() -> i32,
            ),
            (
                "FutilityAboveAlpha",
                params::futility_above_alpha as fn() -> i32,
            ),
            ("QuietSeeMargin", params::quiet_see_margin as fn() -> i32),
            ("LmpBase", params::lmp_base as fn() -> i32),
            ("LmpSlope", params::lmp_slope as fn() -> i32),
            ("LmpNonImproving", params::lmp_non_improving as fn() -> i32),
            (
                "ReverseFutilityMultBase",
                params::reverse_futility_mult_base as fn() -> i32,
            ),
            (
                "ReverseFutilityMultSlope",
                params::reverse_futility_mult_slope as fn() -> i32,
            ),
            (
                "ReverseFutilityNoTtHit",
                params::reverse_futility_no_tt_hit as fn() -> i32,
            ),
            (
                "ReverseFutilityImproving",
                params::reverse_futility_improving as fn() -> i32,
            ),
            (
                "ReverseFutilityOpponentWorsening",
                params::reverse_futility_opponent_worsening as fn() -> i32,
            ),
            (
                "ReverseFutilityCorrection",
                params::reverse_futility_correction as fn() -> i32,
            ),
            (
                "RazoringDepthMargin",
                params::razoring_depth_margin as fn() -> i32,
            ),
            ("SeeDepthMargin", params::see_depth_margin as fn() -> i32),
            (
                "SeeCaptureHistory",
                params::see_capture_history as fn() -> i32,
            ),
            (
                "CaptureFutilityBase",
                params::capture_futility_base as fn() -> i32,
            ),
            (
                "CaptureFutilityLmrDepth",
                params::capture_futility_lmr_depth as fn() -> i32,
            ),
            (
                "CaptureFutilityHistory",
                params::capture_futility_history as fn() -> i32,
            ),
            ("AspirationDelta", params::aspiration_delta as fn() -> i32),
            ("AspirationGrowth", params::aspiration_growth as fn() -> i32),
            ("NullMoveBase", params::null_move_base as fn() -> i32),
            ("NullMoveSlope", params::null_move_slope as fn() -> i32),
            (
                "NullMoveEvalScale",
                params::null_move_eval_scale as fn() -> i32,
            ),
            (
                "NullMoveEvalMargin",
                params::null_move_eval_margin as fn() -> i32,
            ),
            (
                "NullMoveEvalDepth",
                params::null_move_eval_depth as fn() -> i32,
            ),
            (
                "NullMoveEvalImproving",
                params::null_move_eval_improving as fn() -> i32,
            ),
            (
                "NullMoveFailHighBonus",
                params::null_move_fail_high_bonus as fn() -> i32,
            ),
            (
                "HistoryBonusSlope",
                params::history_bonus_slope as fn() -> i32,
            ),
            (
                "HistoryBonusBase",
                params::history_bonus_base as fn() -> i32,
            ),
            ("HistoryBonusCap", params::history_bonus_cap as fn() -> i32),
            (
                "HistoryMalusSlope",
                params::history_malus_slope as fn() -> i32,
            ),
            (
                "HistoryMalusBase",
                params::history_malus_base as fn() -> i32,
            ),
            ("HistoryMalusCap", params::history_malus_cap as fn() -> i32),
            (
                "HistoryMalusDecay",
                params::history_malus_decay as fn() -> i32,
            ),
            (
                "HistoryTtCutoffBonus",
                params::history_tt_cutoff_bonus as fn() -> i32,
            ),
            (
                "HistoryTtMoveBonus",
                params::history_tt_move_bonus as fn() -> i32,
            ),
            (
                "HistoryPriorBonus",
                params::history_prior_bonus as fn() -> i32,
            ),
            ("HistoryEvalDiff", params::history_eval_diff as fn() -> i32),
            ("HistoryDecay", params::history_decay as fn() -> i32),
            (
                "CaptureHistoryDecay",
                params::capture_history_decay as fn() -> i32,
            ),
            (
                "CaptureHistoryScale",
                params::capture_history_scale as fn() -> i32,
            ),
            (
                "OrderingCheckBonus",
                params::ordering_check_bonus as fn() -> i32,
            ),
            (
                "OrderingThreatWeight",
                params::ordering_threat_weight as fn() -> i32,
            ),
            ("CorrectionCap", params::correction_cap as fn() -> i32),
            ("CorrectionWeight", params::correction_weight as fn() -> i32),
            ("DeltaMargin", params::delta_margin as fn() -> i32),
            (
                "QsearchMoveLimit",
                params::qsearch_move_limit as fn() -> i32,
            ),
            ("ExpectedPlies", params::expected_plies as fn() -> i32),
            ("MinMoves", params::min_moves as fn() -> i32),
            ("IncrementShare", params::increment_share as fn() -> i32),
            ("HardSoftRatio", params::hard_soft_ratio as fn() -> i32),
            (
                "HardRemainingShare",
                params::hard_remaining_share as fn() -> i32,
            ),
            ("IterationRatio", params::iteration_ratio as fn() -> i32),
            ("FmScale", params::fm_scale as fn() -> i32),
        ];
        assert_eq!(accessors.len(), params::PARAMETERS.len());
        for (&(name, read), &(declared, default, min, max)) in
            accessors.iter().zip(params::PARAMETERS)
        {
            assert_eq!(name, declared);
            for value in [min, max] {
                assert_eq!(
                    run(
                        &mut protocol,
                        &mut engine,
                        &format!("setoption name Tune_{name} value {value}\n")
                    ),
                    ""
                );
                assert_eq!(read(), value);
            }
            for (suffix, error) in [
                (
                    format!("value {}", min - 1),
                    format!("Tune_{name} must be from {min} to {max}, got {}", min - 1),
                ),
                (
                    format!("value {}", max + 1),
                    format!("Tune_{name} must be from {min} to {max}, got {}", max + 1),
                ),
                (
                    "value nope".into(),
                    format!("Tune_{name} must be an integer"),
                ),
                (
                    "value 2147483648".into(),
                    format!("Tune_{name} must be an integer"),
                ),
                ("value".into(), format!("Tune_{name} requires a value")),
                (String::new(), format!("Tune_{name} requires a value")),
            ] {
                assert_eq!(
                    run(
                        &mut protocol,
                        &mut engine,
                        &format!("setoption name Tune_{name} {suffix}\n")
                    ),
                    format!("info string error: {error}\n")
                );
                assert_eq!(read(), max, "rejected setting changed {name}");
            }
            assert_eq!(
                run(
                    &mut protocol,
                    &mut engine,
                    &format!("setoption name Tune_{name} value {default}\n")
                ),
                ""
            );
            assert_eq!(read(), default);
        }
        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "setoption name Tune_Unknown value 0\n"
            ),
            "info string error: unknown tuning parameter 'Unknown'\n"
        );
        return;
    }
    assert_eq!(
        run(
            &mut protocol,
            &mut engine,
            "setoption name Tune_DeltaMargin value 200\nsetoption name USI_Hash value 1\n"
        ),
        ""
    );
    if scenario != "go" {
        run(&mut protocol, &mut engine, "position startpos\n");
    }
    run(&mut protocol, &mut engine, &format!("{scenario}\nstop\n"));
    for prefix in ["", "usinewgame\n"] {
        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("{prefix}setoption name Tune_DeltaMargin value 300\n")
            ),
            "info string error: tuning parameters cannot change after go\n"
        );
        assert_eq!(params::delta_margin(), 200);
    }
    let mut protocol = UsiProtocol::new(&engine);
    assert_eq!(
        run(
            &mut protocol,
            &mut engine,
            "setoption name Tune_DeltaMargin value 300\n"
        ),
        "info string error: tuning parameters cannot change after go\n"
    );
    assert_eq!(params::delta_margin(), 200);
}
