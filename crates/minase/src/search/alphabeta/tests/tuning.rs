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

/// `byoyomi-time-usage.md`「設計判断」の予算式を独立に計算する。
pub(super) fn reference_clock_budget(clock: ClockLimits, margin_ms: u64) -> TimeBudget {
    let remaining = u128::from(clock.remaining_ms);
    let increment = u128::from(clock.increment_ms);
    let byoyomi = u128::from(clock.byoyomi_ms);
    let moves = u128::from(
        (params::min_moves() as u32)
            .max((params::expected_plies() as u32).saturating_sub(clock.ply) / 2),
    );
    let opening = if remaining > 0 {
        u128::from(clock.ply.saturating_add(4).min(40))
    } else {
        40
    };
    let soft = remaining / moves
        + increment * params::increment_share() as u128 / 100
        + byoyomi * 8 * opening / 400;
    let hard = (soft * params::hard_soft_ratio() as u128 / 100)
        .min(remaining * params::hard_remaining_share() as u128 / 100 + byoyomi * 8 / 10)
        .min(
            (remaining + byoyomi)
                .saturating_sub(u128::from(margin_ms))
                .max(1),
        )
        .max(1);
    TimeBudget {
        byoyomi_period: false,
        soft: Duration::from_millis(soft.min(hard).min(u128::from(u64::MAX)) as u64),
        hard: Duration::from_millis(hard.min(u128::from(u64::MAX)) as u64),
    }
}

/// D7-TIME-01。docs/plans/search.md「時間管理」、time-management-efficiency.md
/// 「予算値」、spsa.md「整数表現」の時間予算式と丸めを係数から照合する。
#[test]
fn tuning_default_clock_budget_matches_reference_grid() {
    let expected_plies = params::expected_plies() as u32;
    let min_moves = params::min_moves() as u32;
    let initial_moves = u64::from(min_moves.max(expected_plies / 2));
    let floor_ply = expected_plies.saturating_sub(2 * min_moves);
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
        initial_moves - 1,
        initial_moves,
        initial_moves + 1,
        10_000,
        u64::MAX,
    ] {
        for increment in [0, 1, 9, 10, 11, 100, u64::MAX] {
            for byoyomi in [0, 1, 4, 5, 6, 30, 31, 1000, u64::MAX] {
                for ply in [
                    0,
                    1,
                    35,
                    36,
                    37,
                    floor_ply.saturating_sub(1),
                    floor_ply,
                    floor_ply + 1,
                    expected_plies - 1,
                    expected_plies,
                    expected_plies + 1,
                    u32::MAX,
                ] {
                    if remaining == 0 && increment == 0 && byoyomi == 0 {
                        continue;
                    }
                    let clock = clock_at_ply(remaining, increment, byoyomi, ply);
                    assert_eq!(
                        clock_budget(clock),
                        reference_clock_budget(clock, 30),
                        "{clock:?}"
                    );
                }
            }
        }
    }
}

/// 全35係数の反映とUSIの入力契約を直列に検査する。
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
            "disabled capture history",
            "disabled history decay",
            "non improving futility 1",
            "non improving futility 2",
            "non improving futility 3",
            "lmp base",
            "lmp slope",
            "lmp zero limit",
            "reverse futility margin",
            "razoring margin 1",
            "razoring margin 2",
            "null move eval scale",
            "disabled null move eval scale",
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
    if scenario == "disabled capture history" {
        params::set("CaptureHistoryScale", 0).unwrap();
        super::captures::zero_capture_scale_preserves_reference_order();
        return;
    }
    if scenario == "disabled history decay" {
        params::set("HistoryDecay", 0).unwrap();
        super::history::zero_decay_clears_history_between_searches();
        return;
    }

    match scenario.as_str() {
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

    use crate::protocol::{Protocol, engine::Engine, usi::UsiProtocol};
    fn run(protocol: &mut UsiProtocol, engine: &mut Engine, input: &str) -> String {
        let mut output = Vec::new();
        protocol
            .run(engine, &mut std::io::Cursor::new(input), &mut output)
            .unwrap();
        String::from_utf8(output).unwrap()
    }
    fn engine() -> Engine {
        Engine::new(minase_core::rules::parse_rule_set("engine-default").unwrap()).unwrap()
    }
    if scenario == "lmr table" {
        // 探索が引く減深量表はプロセス内で1回だけ生成されるので、表の生成より前に
        // 設定した除数が表へ反映されることを、表を経由する`lmr_reduction`で調べる。
        // 除数1.0ならln4 × ln3 ≈ 1.52を切り捨てた1が表へ入る。
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
            // 宣言範囲の上端なら半減せず、既定の上限なら半減する値を置く。
            searcher.history[side][from][to] = 65_535;
            searcher.record_quiet_beta_cutoff(&board, mv, 1, 0);
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
                .record_cutoff(&board, searcher.pst, mv, &[], 100);
            result = i64::from(
                searcher
                    .capture_history
                    .adjustment(&board, searcher.pst, mv),
            );
        });
        result
    }
    fn history_decay() -> i64 {
        let snapshot = snapshot_for(&Position::initial());
        let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
        histories.workers[0][0][60][60] = 100;
        run_search_team(
            &weights().unwrap(),
            &snapshot.position,
            snapshot.rules,
            &snapshot.root_moves,
            &snapshot.history_keys,
            &depth_limits(1),
            &AtomicBool::new(true),
            DEFAULT_THREADS,
            &small_tt(),
            &mut histories,
            None,
            Instant::now(),
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

    // USIの宣言行が係数表の名前、既定値、範囲と一致する。
    let mut engine = engine();
    let mut protocol = UsiProtocol::new(&engine);
    let handshake = run(&mut protocol, &mut engine, "usi\n");
    let declarations: Vec<_> = handshake
        .lines()
        .filter(|line| line.starts_with("option name Tune_"))
        .collect();
    let expected_declarations: Vec<_> = params::PARAMETERS
        .iter()
        .map(|(name, default, min, max)| {
            format!("option name Tune_{name} type spin default {default} min {min} max {max}")
        })
        .collect();
    assert_eq!(declarations, expected_declarations);

    // 既に復号したPSTにも調整値が反映されることを含めて調べる。
    let _pst = weights().unwrap();
    type Case = (&'static str, i32, fn() -> i64);
    let cases: [Case; 35] = [
        ("LmrDivisor", 400, || {
            i64::from(lmr_base(8, 16, params::lmr_divisor()))
        }),
        ("LmrHistoryThreshold", 512, || {
            i64::from(lmr_reduction(4, 8, 256))
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
        ("LmpBase", 600, || i64::from(late_move_limit(1))),
        ("LmpSlope", 400, || i64::from(late_move_limit(1))),
        ("ReverseFutilityMargin", 100, || {
            i64::from(reverse_futility_margin(101))
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
        ("HistoryDecay", 50, history_decay),
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
    ];
    assert_eq!(cases.len(), params::PARAMETERS.len());
    for ((name, value, observe), &(expected_name, default, min, max)) in
        cases.into_iter().zip(params::PARAMETERS)
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
    let default_delta = params::delta_margin();
    assert!(
        run(
            &mut protocol,
            &mut engine,
            "setoption name Tune_DeltaMargin value 300\n"
        )
        .is_empty()
    );
    assert_eq!(params::delta_margin(), 300);
    params::set("DeltaMargin", default_delta).unwrap();
}

// フェーズ1-B指示書。実際の探索で各係数の境界をまたぎ、係数の読み出し忘れを検出する。
#[cfg(feature = "tuning")]
fn reverse_futility_parameter_changes_search() {
    let board = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/5P6/11K b").unwrap();
    let pst = weights().unwrap();
    let value = evaluate(&pst, &board);
    let percent = params::reverse_futility_margin();
    let beta = value - pst.pawn_value() * percent / 100;
    assert_eq!(
        run_negamax(&board, 1, beta - 1, beta, 0, &small_tt()),
        (value, 0)
    );
    params::set("ReverseFutilityMargin", percent + 100).unwrap();
    assert!(run_negamax(&board, 1, beta - 1, beta, 0, &small_tt()).1 > 0);
    assert_eq!(
        reverse_futility_margin(137),
        137 * (percent + 100) / 100,
        "百分率は乗算してから割る"
    );
}

#[cfg(feature = "tuning")]
fn razoring_parameter_changes_search(depth: u32, name: &str) {
    let board = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    let pst = weights().unwrap();
    let value = evaluate(&pst, &board);
    let percent = match depth {
        1 => params::razoring_margin1(),
        2 => params::razoring_margin2(),
        _ => unreachable!(),
    };
    let alpha = value + pst.pawn_value() * percent / 100;
    assert_eq!(
        run_negamax(&board, depth, alpha, alpha + 1, 0, &small_tt()),
        (value, 0)
    );
    params::set(name, percent + 100).unwrap();
    assert!(run_negamax(&board, depth, alpha, alpha + 1, 0, &small_tt()).1 > 0);
    assert_eq!(
        razoring_margin(137, depth),
        137 * (percent + 100) / 100,
        "百分率は乗算してから割る"
    );
}

#[cfg(feature = "tuning")]
fn null_move_eval_scale_changes_search() {
    let board = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/5G6/11K b").unwrap();
    let pst = weights().unwrap();
    let scale = params::null_move_eval_scale();
    assert!(scale > 0);
    let beta = evaluate(&pst, &board) - (2 * pst.pawn_value() * 100 + scale - 1) / scale;
    let (_, default_nodes) = run_negamax(&board, 7, beta - 1, beta, 0, &small_tt());
    params::set("NullMoveEvalScale", 0).unwrap();
    let (score, changed_nodes) = run_negamax(&board, 7, beta - 1, beta, 0, &small_tt());
    assert!(score >= beta);
    assert!(changed_nodes > default_nodes);
    params::set("NullMoveEvalScale", 50).unwrap();
    let base = (params::null_move_base() as u32 + 6 * params::null_move_slope() as u32) / 1200;
    // 2pの境界と上限を、百分率の除算に余りが出る歩兵価値でも検査する。
    assert_eq!(null_move_reduction(6, 147, 0, 37), base);
    assert_eq!(null_move_reduction(6, 148, 0, 37), base + 1);
    params::set("NullMoveEvalScale", 400).unwrap();
    assert_eq!(null_move_reduction(6, 19, 0, 37), base + 1);
    assert_eq!(null_move_reduction(6, i32::MAX, i32::MIN, 37), base + 3);
}

#[cfg(feature = "tuning")]
fn zero_null_move_eval_scale_matches_base_reduction_search() {
    params::set("NullMoveEvalScale", 0).unwrap();
    // 尺度0では、静的評価とβにかかわらず基本の減深量になる。
    for depth in 0..=MAX_PLY {
        for value in [i32::MIN, -1000, 0, 1000, i32::MAX] {
            for beta in [-MATE_THRESHOLD + 1, 0, MATE_THRESHOLD - 1] {
                assert_eq!(
                    null_move_reduction(depth, value, beta, 37),
                    (params::null_move_base() as u32 + depth * params::null_move_slope() as u32)
                        / 1200
                );
            }
        }
    }
    let board = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/5G6/11K b").unwrap();
    let pst = weights().unwrap();
    let beta = evaluate(&pst, &board) - 2 * pst.pawn_value();
    // 記録手なしの深さ7は6となる。基本の減深量から子の深さを求める。
    let reduction = (params::null_move_base() as u32 + 6 * params::null_move_slope() as u32) / 1200;
    let child_depth = 6_u32.saturating_sub(1 + reduction);
    let mut passed = board.clone();
    passed.make_null_move();
    let (reply, nodes) = run_negamax(&passed, child_depth, -beta, -beta + 1, 1, &small_tt());
    assert!(-reply >= beta);
    assert!(nodes > 0);
    let table = small_tt();
    assert_eq!(
        run_negamax(&board, 7, beta - 1, beta, 0, &table),
        (-reply, nodes)
    );
    assert!(table.probe(search_key(&board), 0).is_none());
}

// フェーズ1-C指示書。子を反復引き分けに固定し、親の枝刈りだけを観測する。
#[cfg(feature = "tuning")]
fn non_improving_parameter_changes_search(depth: u32, name: &str) {
    let board = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    let moves = legal_moves(&board);
    let history = repeated_root_children(&board, &moves);
    let pst = weights().unwrap();
    let (percent, scale) = [
        (
            params::futility_margin1(),
            params::non_improving_futility1(),
        ),
        (
            params::futility_margin2(),
            params::non_improving_futility2(),
        ),
        (
            params::futility_margin3(),
            params::non_improving_futility3(),
        ),
    ][depth as usize - 1];
    let alpha = evaluate(&pst, &board) + pst.pawn_value() * percent / 100 * scale / 100;
    let search = || {
        let mut nodes = 0;
        with_root_searcher(&board, &history, |searcher| {
            searcher
                .tt
                .store(search_key(&board), 0, 0, Bound::Upper, Some(moves[0]), 2);
            assert!(!searcher.improving(evaluate(&pst, &board), board.side_to_move(), 2));
            assert_eq!(
                searcher.negamax(&mut board.clone(), depth, alpha, alpha + 1, 2),
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
    for pawn in [1, 37, 100, 137, 999] {
        assert_eq!(futility_margin(pawn, depth, false), pawn * percent / 100);
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
    let limit = (params::lmp_base() + params::lmp_slope()) / 100;
    assert_eq!(super::late_move::search_quiet_board(&board).1, limit as u64);
    let other = match name {
        "LmpBase" => params::lmp_slope(),
        "LmpSlope" => params::lmp_base(),
        _ => unreachable!(),
    };
    let boundary = (limit + 1) * 100 - other;
    params::set(name, boundary).unwrap();
    assert_eq!(
        super::late_move::search_quiet_board(&board).1,
        (limit + 1) as u64
    );
    params::set(name, boundary - 1).unwrap();
    assert_eq!(
        super::late_move::search_quiet_board(&board).1,
        limit as u64,
        "整数除算で端数を切り捨てる"
    );
}
