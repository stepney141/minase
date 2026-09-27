//! 探索係数の既定値と設定を検査する。

use super::*;
#[cfg(feature = "tuning")]
use crate::search::alphabeta::{
    correction::CorrectionTable,
    params,
    pruning::{futility_margin, lmr_base, see_margin},
};

// docs/plans/spsa.md「対象の係数」の採用値での一致契約。
#[test]
fn tuning_default_null_move_and_aspiration_match_reference() {
    for depth in 0..=256 {
        assert_eq!(null_move_reduction(depth), (3_529 + depth * 238) / 1_200);
    }
    for delta in [
        0,
        1,
        49,
        50,
        51,
        1_068_399_824,
        1_068_399_825,
        1_068_399_826,
        i32::MAX - 1,
        i32::MAX,
    ] {
        assert_eq!(
            grow_aspiration_delta(delta),
            (i64::from(delta) * 201 / 100).min(i64::from(i32::MAX)) as i32
        );
    }
    with_root_searcher(&Position::initial(), &[], |searcher| {
        assert_eq!(searcher.delta_margin, 258 * searcher.pst.pawn_value() / 100);
    });
}

/// 固定深さbenchが通らない時間管理を、採用後の仕様の式と照合する。
#[test]
fn tuning_default_clock_budget_matches_reference_grid() {
    fn reference(clock: ClockLimits) -> TimeBudget {
        let remaining = u128::from(clock.remaining_ms);
        let increment = u128::from(clock.increment_ms);
        let byoyomi = u128::from(clock.byoyomi_ms);
        let moves = u128::from(88_u32.max(432_u32.saturating_sub(clock.ply) / 2));
        let opening = if remaining > 0 {
            u128::from(clock.ply.saturating_add(4).min(40))
        } else {
            40
        };
        let soft = remaining / moves + increment * 76 / 100 + byoyomi * 8 * opening / 400;
        let hard = (soft * 451 / 100)
            .min(remaining * 27 / 100 + byoyomi * 8 / 10)
            .min((remaining + byoyomi).saturating_sub(30).max(1))
            .max(1);
        TimeBudget {
            soft: Duration::from_millis(soft.min(hard).min(u128::from(u64::MAX)) as u64),
            hard: Duration::from_millis(hard.min(u128::from(u64::MAX)) as u64),
        }
    }
    for remaining in [
        0,
        1,
        3,
        4,
        29,
        30,
        31,
        99,
        100,
        101,
        215,
        216,
        217,
        10_000,
        u64::MAX,
    ] {
        for increment in [0, 1, 9, 10, 11, 100, u64::MAX] {
            for byoyomi in [0, 1, 4, 5, 6, 30, 31, 1000, u64::MAX] {
                for ply in [0, 1, 35, 36, 37, 254, 255, 256, 431, 432, 433, u32::MAX] {
                    if remaining == 0 && increment == 0 && byoyomi == 0 {
                        continue;
                    }
                    let clock = clock_at_ply(remaining, increment, byoyomi, ply);
                    assert_eq!(clock_budget(clock), reference(clock), "{clock:?}");
                }
            }
        }
    }
}

/// 予測の交差積はナノ秒単位の等号境界と最大Durationでも一致する。
#[test]
fn tuning_default_iteration_prediction_matches_reference_grid() {
    for started in [
        Duration::ZERO,
        Duration::from_nanos(1),
        Duration::from_nanos(37),
        Duration::from_nanos(38),
        Duration::from_nanos(39),
        Duration::from_nanos(99),
        Duration::from_nanos(100),
        Duration::from_nanos(101),
        Duration::MAX,
    ] {
        for hit in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_nanos(10),
            Duration::MAX,
        ] {
            for soft in [
                Duration::ZERO,
                Duration::from_nanos(100),
                Duration::from_nanos(263),
                Duration::MAX,
            ] {
                for hard in [
                    soft,
                    soft.saturating_add(Duration::from_nanos(150)),
                    Duration::MAX,
                ] {
                    for stable in [false, true] {
                        let expected = started.as_nanos() * 263
                            <= (hit.as_nanos() + hard.as_nanos()) * 100
                            && (!stable
                                || started.as_nanos() * 263
                                    <= (hit.as_nanos() + soft.as_nanos()) * 100);
                        assert_eq!(
                            iteration_prediction_fits(
                                started,
                                hit,
                                TimeBudget { soft, hard },
                                stable
                            ),
                            expected
                        );
                    }
                }
            }
        }
    }
}

/// 全22係数の反映とUSIの入力契約を直列に検査する。
/// グローバル係数が既存の並列テストへ漏れないよう、このテストだけを子プロセスで走らせる。
#[cfg(feature = "tuning")]
#[test]
fn tuning_parameters_and_usi_contract_in_isolated_process() {
    const CHILD: &str = "MINASE_TUNING_TEST_CHILD";
    let scenario = std::env::var(CHILD);
    if scenario.is_err() {
        for scenario in [
            "lmr table",
            "parameters",
            "go depth 1",
            "go ponder depth 1",
            "go depth nope",
            "go mate 1",
            "go",
        ] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "search::alphabeta::tests::tuning::tuning_parameters_and_usi_contract_in_isolated_process",
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
        }
        return;
    }
    let scenario = scenario.unwrap();

    use crate::protocol::{Protocol, engine::Engine, usi::UsiProtocol};
    fn run(protocol: &mut UsiProtocol, engine: &mut Engine, input: &str) -> String {
        let mut output = Vec::new();
        protocol
            .run(engine, &mut std::io::Cursor::new(input), &mut output)
            .unwrap();
        String::from_utf8(output).unwrap()
    }
    fn engine() -> Engine {
        Engine::new(crate::core::rules::parse_rule_set("engine-default").unwrap()).unwrap()
    }
    if scenario == "lmr table" {
        // 探索が引く減深量表はプロセス内で1回だけ生成されるので、表の生成より前に
        // 設定した除数が表へ反映されることを、表を経由する`lmr_reduction`で調べる。
        // ln4 × ln3 ≈ 1.52は、既定の除数1.66では0、除数1.0では1に切り捨てられる。
        let mut engine = engine();
        let mut protocol = UsiProtocol::new(&engine);
        let output = run(
            &mut protocol,
            &mut engine,
            "setoption name Tune_LmrDivisor value 100\n",
        );
        assert_eq!(output, "");
        assert_eq!(lmr_reduction(4, 3, 0), 1);
        return;
    }
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
            searcher.history[side][from][to] = 20_755;
            searcher.record_quiet_beta_cutoff(&board, mv, 1, 0);
            result = i64::from(searcher.history[side][from][to]);
        });
        result
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
                soft: Duration::from_millis(100),
                hard: Duration::from_millis(200),
            },
            true,
        ))
    }

    // goの各種入力をそれぞれ新しいプロセスで検査し、先行するgoの固定状態に依存させない。
    if scenario != "parameters" {
        let mut engine = engine();
        let mut protocol = UsiProtocol::new(&engine);
        assert!(
            run(
                &mut protocol,
                &mut engine,
                "setoption name Tune_DeltaMargin value 200\n"
            )
            .is_empty()
        );
        run(
            &mut protocol,
            &mut engine,
            "setoption name USI_Hash value 1\n",
        );
        if scenario != "go" {
            run(&mut protocol, &mut engine, "position startpos\n");
        }
        run(&mut protocol, &mut engine, &format!("{scenario}\nstop\n"));
        for prefix in ["", "usinewgame\n"] {
            let output = run(
                &mut protocol,
                &mut engine,
                &format!("{prefix}setoption name Tune_DeltaMargin value 300\n"),
            );
            assert!(
                output.starts_with("info string error: "),
                "{scenario}: {output}"
            );
            assert_eq!(params::delta_margin(), 200);
        }
        // USIアダプターを作り直しても、プロセス内の固定状態を解除しない。
        let mut protocol = UsiProtocol::new(&engine);
        assert!(
            run(
                &mut protocol,
                &mut engine,
                "setoption name Tune_DeltaMargin value 300\n"
            )
            .starts_with("info string error: ")
        );
        assert_eq!(params::delta_margin(), 200);
        return;
    }

    // 指示書の22行を、宣言順・既定値・範囲の独立した参照値とする。
    let expected = [
        ("LmrDivisor", 166, 100, 400),
        ("LmrHistoryThreshold", 111, 0, 512),
        ("FutilityMargin1", 101, 0, 400),
        ("FutilityMargin2", 196, 0, 400),
        ("FutilityMargin3", 207, 0, 400),
        ("SeeMargin1", 2, 0, 400),
        ("SeeMargin2", 210, 0, 400),
        ("SeeMargin3", 7, 0, 400),
        ("AspirationDelta", 46, 10, 200),
        ("AspirationGrowth", 201, 125, 400),
        ("NullMoveBase", 3529, 1200, 4800),
        ("NullMoveSlope", 238, 100, 400),
        ("HistoryLimit", 20755, 4096, 65536),
        ("CorrectionCap", 193, 50, 400),
        ("CorrectionWeight", 33, 8, 128),
        ("DeltaMargin", 258, 50, 500),
        ("ExpectedPlies", 432, 250, 700),
        ("MinMoves", 88, 40, 200),
        ("IncrementShare", 76, 30, 100),
        ("HardSoftRatio", 451, 150, 800),
        ("HardRemainingShare", 27, 10, 50),
        ("IterationRatio", 263, 150, 400),
    ];
    assert_eq!(params::PARAMETERS, expected);
    let mut engine = engine();
    let mut protocol = UsiProtocol::new(&engine);
    let handshake = run(&mut protocol, &mut engine, "usi\n");
    let declarations: Vec<_> = handshake
        .lines()
        .filter(|line| line.starts_with("option name Tune_"))
        .collect();
    let expected_declarations: Vec<_> = expected
        .iter()
        .map(|(name, default, min, max)| {
            format!("option name Tune_{name} type spin default {default} min {min} max {max}")
        })
        .collect();
    assert_eq!(declarations, expected_declarations);

    // 既に復号したPSTにも調整値が反映されることを含めて調べる。
    let _pst = weights().unwrap();
    type Case = (&'static str, i32, fn() -> i64);
    let cases: [Case; 22] = [
        ("LmrDivisor", 400, || {
            i64::from(lmr_base(8, 16, params::lmr_divisor()))
        }),
        ("LmrHistoryThreshold", 512, || {
            i64::from(lmr_reduction(4, 8, 128))
        }),
        ("FutilityMargin1", 100, || {
            i64::from(futility_margin(101, 1))
        }),
        ("FutilityMargin2", 100, || {
            i64::from(futility_margin(101, 2))
        }),
        ("FutilityMargin3", 100, || {
            i64::from(futility_margin(101, 3))
        }),
        ("SeeMargin1", 100, || i64::from(see_margin(101, 1))),
        ("SeeMargin2", 100, || i64::from(see_margin(101, 2))),
        ("SeeMargin3", 100, || i64::from(see_margin(101, 3))),
        ("AspirationDelta", 100, || i64::from(aspiration_delta(101))),
        ("AspirationGrowth", 300, || {
            i64::from(grow_aspiration_delta(101))
        }),
        ("NullMoveBase", 4800, || i64::from(null_move_reduction(12))),
        ("NullMoveSlope", 400, || i64::from(null_move_reduction(12))),
        ("HistoryLimit", 65536, history),
        ("CorrectionCap", 400, || correction(100_000)),
        ("CorrectionWeight", 64, || correction(100)),
        ("DeltaMargin", 300, delta),
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
    ];
    for ((name, value, observe), &(expected_name, default, min, max)) in
        cases.into_iter().zip(&expected)
    {
        assert_eq!(name, expected_name);
        let before = observe();
        let output = run(
            &mut protocol,
            &mut engine,
            &format!("setoption name Tune_{name} value {value}\n"),
        );
        assert!(output.is_empty(), "{output}");
        assert_ne!(observe(), before, "{name} must affect its calculation");
        for invalid in [
            format!("value {}", min - 1),
            format!("value {}", max + 1),
            "value nope".into(),
            "value 2147483648".into(),
            "value".into(),
            String::new(),
        ] {
            let changed = observe();
            let output = run(
                &mut protocol,
                &mut engine,
                &format!("setoption name Tune_{name} {invalid}\n"),
            );
            assert!(
                output.starts_with("info string error: "),
                "{name}: {output}"
            );
            assert_eq!(observe(), changed, "rejected setting changed {name}");
        }
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
    assert!(
        run(
            &mut protocol,
            &mut engine,
            "setoption name Tune_Unknown value 0\n"
        )
        .starts_with("info string error: ")
    );
    assert!(
        run(
            &mut protocol,
            &mut engine,
            "setoption name Tune_DeltaMargin value 300\n"
        )
        .is_empty()
    );
    assert_eq!(params::delta_margin(), 300);
    params::set("DeltaMargin", 258).unwrap();
}
