//! 時間管理を検査する。

use super::*;

// 時計の予算はsoft ≤ hardかつhard ≥ 1 msを満たす。
// 残り時間と秒読みの合計が30 msを超えるとき、hardはその合計から30 msを引いた値以下になる。
#[test]
fn clock_budget_preserves_bounds_over_a_deterministic_grid() {
    let remaining_values = [0, 20, 31, 200, 10_000, 60_000, u64::MAX];
    let increment_values = [0, 100, 1_000, u64::MAX];
    let byoyomi_values = [0, 100, 200, u64::MAX];
    let ply_values = [0, 100, 300, 431, 432, 1_000, u32::MAX];

    for remaining in remaining_values {
        for increment in increment_values {
            for byoyomi in byoyomi_values {
                if remaining == 0 && increment == 0 && byoyomi == 0 {
                    continue;
                }
                for ply in ply_values {
                    let budget = clock_budget(clock_at_ply(remaining, increment, byoyomi, ply));
                    assert!(budget.soft <= budget.hard);
                    assert!(budget.hard >= Duration::from_millis(1));

                    let clock_total = u128::from(remaining) + u128::from(byoyomi);
                    if clock_total > 30 {
                        assert!(budget.hard.as_millis() <= clock_total - 30);
                    }
                }
            }
        }
    }
}

// 時計なしの固定時間単独では、併用則
// （softとhardのそれぞれで小さい方）の帰結として両リミットとも指定値になる。
#[test]
fn movetime_alone_sets_both_soft_and_hard_to_the_given_value() {
    let budget = time_budget(&movetime_limits(500)).expect("movetime must produce a budget");
    assert_eq!(budget.soft, Duration::from_millis(500));
    assert_eq!(budget.hard, Duration::from_millis(500));
}

// 直近4反復の最善手が同じときだけ安定とする。
// 採用理由はdocs/adr/stop-early-on-stable-best-move-without-extensions.md。
#[test]
fn stable_signal_requires_four_identical_recent_best_moves() {
    let moves = legal_moves(&Position::initial());
    let [a, b] = [moves[0], moves[1]];
    assert!(stable_signal(&[a, a, a, a]));
    assert!(!stable_signal(&[a, a, a]));
    assert!(stable_signal(&[b, a, a, a, a]));
    assert!(!stable_signal(&[a, a, a, b]));
    assert!(!stable_signal(&[a, b, a, a, a]));
    assert!(!stable_signal(&[]));
}

// 継続条件を満たさない主ワーカーは
// 次の深さへ入らず、softリミットとして最後の完了反復の合法手を返す。
#[test]
fn next_iteration_gate_stops_main_worker_with_a_legal_best_move() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let root_moves = legal_moves(&position);
    let history_keys = [search_key(&position)];
    let external_stop = AtomicBool::new(false);
    let budget = TimeBudget {
        byoyomi_period: false,
        soft: Duration::from_secs(1),
        hard: Duration::from_secs(1),
    };
    let shared = SharedSearch {
        external_stop: &external_stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        node_limit: None,
        started: Instant::now() - Duration::from_millis(500),
        hard_limit: Some(HardLimit {
            duration: budget.hard,
            hit_ns: &AtomicU64::new(0),
        }),
    };
    let nnue = test_nnue();
    let tt = small_tt();

    let outcome = run_main_worker(
        nnue,
        &position,
        engine_rules(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        DRAW_SCORE,
        &root_moves,
        &history_keys,
        2,
        Some(budget),
        &shared,
        &tt,
        &mut Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]),
        &mut crate::search::alphabeta::capture_history::CaptureHistory::new(),
        None,
        false,
        false,
    );

    assert_eq!(shared.reason(), StopReason::SoftLimit);
    assert_eq!(outcome.result.depth, 1);
    assert!(root_moves.contains(&outcome.result.best_move));
}

// hardはノード周期の時計チェックで
// 適用される。上限の許容幅はテスト環境定数であり規範値ではない。
// 次の反復を開始するかどうかの判断との先着は実時間に依存するため停止理由は2値を許容する。
#[test]
fn movetime_search_respects_the_hard_limit_and_returns_a_legal_move() {
    let initial = Position::initial();
    let handle = start(
        snapshot_for(&initial),
        movetime_limits(30),
        61,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (_, finished) = event_reports(drain_raw(&handle));
    handle.join().expect("search thread must not panic");

    // 上限ガード: ノード周期チェックの遅延を見込んだ十分な許容幅。
    assert!(finished.elapsed <= Duration::from_millis(530));
    // soft=hardのため、先に到達する検査点によって停止理由が変わる。
    assert!(matches!(
        finished.stop_reason,
        StopReason::SoftLimit | StopReason::HardLimit
    ));
    assert!(legal_moves(&initial).contains(&finished.best_move));
}

// 深さ・ノード数・固定時間の併用では通常の時計予算を使う。
#[test]
fn excluded_clocks_preserve_numeric_budgets_and_iteration_gates() {
    let ms = Duration::from_millis;
    for (name, remaining, increment, byoyomi, depth, nodes, movetime, soft, hard) in [
        ("clock only", 60000, 1000, 0, None, None, None, 1037, 4676),
        ("depth", 60000, 1000, 0, Some(2), None, None, 1037, 4676),
        ("nodes", 60000, 1000, 0, None, Some(100), None, 1037, 4676),
        (
            "movetime crosses clock",
            60000,
            1000,
            0,
            None,
            None,
            Some(2000),
            1037,
            2000,
        ),
        (
            "movetime wins both",
            60000,
            1000,
            0,
            None,
            None,
            Some(500),
            500,
            500,
        ),
        (
            "clock wins both",
            60000,
            1000,
            0,
            None,
            None,
            Some(5000),
            1037,
            4676,
        ),
        (
            "all limits",
            60000,
            1000,
            0,
            Some(2),
            Some(100),
            Some(500),
            500,
            500,
        ),
        (
            "byoyomi with depth",
            0,
            100,
            1000,
            Some(2),
            None,
            None,
            800,
            800,
        ),
        (
            "byoyomi with nodes",
            0,
            100,
            1000,
            None,
            Some(100),
            None,
            800,
            800,
        ),
        (
            "byoyomi with movetime",
            0,
            100,
            1000,
            None,
            None,
            Some(500),
            500,
            500,
        ),
    ] {
        for margin in [0, 30, 60000] {
            let clock = clock(remaining, increment, byoyomi).with_byoyomi_margin_ms(margin);
            let limits = SearchLimits::new(depth, nodes, movetime, Some(clock)).unwrap();
            let budget = time_budget(&limits).unwrap();
            assert_eq!(
                budget,
                TimeBudget {
                    byoyomi_period: false,
                    soft: ms(soft),
                    hard: ms(hard)
                },
                "{name}, margin={margin}"
            );
        }
    }
    // 予測比263%で、100msの予測は263ms。等号と1ns超過を固定する。
    let budget = TimeBudget {
        byoyomi_period: false,
        soft: ms(263),
        hard: ms(526),
    };
    for (elapsed, hit, stable, expected) in [
        (ms(100), ms(0), true, true),
        (ms(100) + Duration::from_nanos(1), ms(0), true, false),
        (ms(200), ms(0), false, true),
        (ms(200) + Duration::from_nanos(1), ms(0), false, false),
        (ms(150), Duration::from_micros(131500), true, true),
        (
            ms(150) + Duration::from_nanos(1),
            Duration::from_micros(131500),
            true,
            false,
        ),
        (ms(299), ms(300), false, true),
    ] {
        assert_eq!(
            should_start_next_iteration(elapsed, hit, budget, stable),
            expected,
            "elapsed={elapsed:?}, hit={hit:?}, stable={stable}"
        );
    }
    // 不安定時は経過時間がsoft未満であることも必要。
    let budget = TimeBudget {
        byoyomi_period: false,
        soft: ms(100),
        hard: ms(1000),
    };
    for hit in [ms(0), ms(100)] {
        for (since_hit, expected) in [(ms(99), true), (ms(100), false), (ms(101), false)] {
            assert_eq!(
                should_start_next_iteration(hit + since_hit, hit, budget, false),
                expected,
                "hit={hit:?}, since_hit={since_hit:?}"
            );
        }
    }
}

// 秒読み期は加算、手数、最善手の安定性によらず、経過時間が締切max(1 ms, 秒読み−余裕)未満なら次を始める。
#[test]
fn byoyomi_period_budget_and_gate_use_only_the_deadline() {
    let ms = Duration::from_millis;
    for byoyomi in [1_u64, 29, 30, 31, 1000, 60_000, u64::MAX] {
        for margin in [0, 30, 60_000] {
            for increment in [0, 1000] {
                for ply in [0, 36, u32::MAX] {
                    let clock =
                        clock_at_ply(0, increment, byoyomi, ply).with_byoyomi_margin_ms(margin);
                    let limits = SearchLimits::new(None, None, None, Some(clock)).unwrap();
                    let budget = time_budget(&limits).unwrap();
                    let deadline = ms(byoyomi.saturating_sub(margin).max(1));
                    assert_eq!(budget.soft, deadline);
                    assert_eq!(budget.hard, deadline);
                    for hit in [Duration::ZERO, ms(10_000)] {
                        for stable in [false, true] {
                            for (since_hit, expected) in [
                                (deadline - Duration::from_nanos(1), true),
                                (deadline, false),
                                (deadline + Duration::from_nanos(1), false),
                            ] {
                                assert_eq!(
                                    should_start_next_iteration(
                                        hit + since_hit,
                                        hit,
                                        budget,
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
    }
}

// 秒読みつき時計では、指定余裕を安全上限から引き、予算の下限は1msとする。
#[test]
fn byoyomi_with_remaining_time_changes_only_the_safety_margin() {
    for (remaining, increment, byoyomi, ply, margin, soft, hard) in [
        (60000, 1000, 1000, 0, 0, 1117, 5037),
        (60000, 1000, 1000, 0, 30, 1117, 5037),
        (60000, 1000, 1000, 36, 1000, 1863, 8402),
        (60000, 1000, 1000, 0, 60000, 1000, 1000),
        (100, 1000, 100, 0, 0, 107, 107),
        (100, 1000, 100, 0, 150, 50, 50),
        (100, 1000, 100, 0, 200, 1, 1),
        (100, 1000, 100, 0, 201, 1, 1),
        (1, 0, 1, 0, 0, 0, 1),
        (
            u64::MAX,
            u64::MAX,
            u64::MAX,
            u32::MAX,
            60000,
            u64::MAX,
            u64::MAX,
        ),
    ] {
        let clock = clock_at_ply(remaining, increment, byoyomi, ply).with_byoyomi_margin_ms(margin);
        let limits = SearchLimits::new(None, None, None, Some(clock)).unwrap();
        assert_eq!(
            time_budget(&limits),
            Some(TimeBudget {
                byoyomi_period: false,
                soft: Duration::from_millis(soft),
                hard: Duration::from_millis(hard),
            }),
            "{clock:?}"
        );
        if margin == 30 {
            assert_eq!(time_budget(&limits), Some(clock_budget(clock)));
        }
    }
}

// 秒読み期は反復の開始前と完了後に締切を検査し、先読み中は的中まで時計による判定を行わない。
#[test]
fn byoyomi_period_starts_iterations_before_deadline_in_normal_and_ponder_search() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let root_moves = legal_moves(&position);
    let nnue = test_nnue();
    let limits = SearchLimits::new(None, None, None, Some(clock(0, 0, 10_030))).unwrap();
    let budget = time_budget(&limits).unwrap();
    for ponder in [false, true] {
        let stop = AtomicBool::new(false);
        let hit_ns = AtomicU64::new(if ponder { 100_000_000_000 } else { 0 });
        let shared = SharedSearch {
            external_stop: &stop,
            team_stop: AtomicBool::new(false),
            stop_reason: AtomicU8::new(0),
            total_nodes: AtomicU64::new(0),
            node_limit: None,
            started: Instant::now() - Duration::from_secs(if ponder { 105 } else { 5 }),
            hard_limit: Some(HardLimit {
                duration: budget.hard,
                hit_ns: &hit_ns,
            }),
        };
        let outcome = run_main_worker(
            nnue,
            &position,
            engine_rules(),
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            DRAW_SCORE,
            &root_moves,
            &[],
            2,
            Some(budget),
            &shared,
            &small_tt(),
            &mut Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]),
            &mut crate::search::alphabeta::capture_history::CaptureHistory::new(),
            None,
            ponder,
            true,
        );
        assert_eq!(outcome.result.depth, 2);
        assert_eq!(shared.reason(), StopReason::DepthCompleted);
        assert!(root_moves.contains(&outcome.result.best_move));
    }
}
