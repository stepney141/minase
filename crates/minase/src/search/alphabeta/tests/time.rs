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

// 固定時間と時計の併用時の予算は、
// softとhardのそれぞれで両者の小さい方を採る（一括minではない独立比較）。
#[test]
fn movetime_and_clock_combine_per_limit_by_taking_the_smaller() {
    // 時計単独ならsoft=1037、hard=4676。
    let base = clock(60_000, 1_000, 0);
    let with_movetime =
        |milliseconds: u64| SearchLimits::new(None, None, Some(milliseconds), Some(base)).unwrap();

    // (a) 交差例: softは時計側、hardはmovetime側が勝つ。独立比較の固定。
    let budget = time_budget(&with_movetime(2_000)).unwrap();
    assert_eq!(budget.soft, Duration::from_millis(1_037));
    assert_eq!(budget.hard, Duration::from_millis(2_000));

    // (b) movetimeが両方で勝つ。
    let budget = time_budget(&with_movetime(500)).unwrap();
    assert_eq!(budget.soft, Duration::from_millis(500));
    assert_eq!(budget.hard, Duration::from_millis(500));

    // (c) 時計側が両方で勝つ。
    let budget = time_budget(&with_movetime(5_000)).unwrap();
    assert_eq!(budget.soft, Duration::from_millis(1_037));
    assert_eq!(budget.hard, Duration::from_millis(4_676));
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
        movetime_limits(300),
        61,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (_, finished) = event_reports(drain_raw(&handle));
    handle.join().expect("search thread must not panic");

    // 上限ガード: ノード周期チェックの遅延を見込んだ十分な許容幅。
    assert!(finished.elapsed <= Duration::from_millis(1_000));
    // soft=hardのため、先に到達する検査点によって停止理由が変わる。
    assert!(matches!(
        finished.stop_reason,
        StopReason::SoftLimit | StopReason::HardLimit
    ));
    assert!(legal_moves(&initial).contains(&finished.best_move));
}

// 時計による停止は反復境界でのsoft limitか、ノード周期でのhard limitである。
// 実時間の検査では先に到達する検査点が一定でないため、どちらの停止理由も認める。
#[test]
fn clock_driven_searches_stop_with_a_time_limit_reason() {
    let initial = Position::initial();

    // 残り6000 ms、加算100 ms、ply=0ではsoft < hardとなり、反復境界で先に停止し得る。
    let limits = SearchLimits::new(None, None, None, Some(clock(6_000, 100, 0))).unwrap();
    let handle = start(
        snapshot_for(&initial),
        limits,
        85,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (_, finished) = event_reports(drain_raw(&handle));
    handle.join().expect("search thread must not panic");
    assert!(matches!(
        finished.stop_reason,
        StopReason::SoftLimit | StopReason::HardLimit
    ));
    assert!(legal_moves(&initial).contains(&finished.best_move));

    // 時計の安全上限が効いてsoft=hard=70 msとなる設定を検査する。
    let limits = SearchLimits::new(None, None, None, Some(clock(0, 0, 100))).unwrap();
    let budget = time_budget(&limits).expect("byoyomi must produce a time budget");
    assert_eq!(budget.soft, Duration::from_millis(70));
    assert_eq!(budget.hard, Duration::from_millis(70));
}

// 秒読みが0、または深さ・ノード数・固定時間を併用した探索は、通常の時計予算と反復開始判定を使う。
#[test]
fn excluded_clocks_preserve_reference_budgets_and_iteration_gates() {
    let ms = Duration::from_millis;
    for remaining in [0, 1, 30, 60_000, u64::MAX] {
        for byoyomi in [0, 1, 1000, u64::MAX] {
            for margin in [0, 30, 60_000] {
                let clock = clock(remaining, 100, byoyomi).with_byoyomi_margin_ms(margin);
                let reference = super::tuning::reference_clock_budget(clock, 30);
                assert_eq!(clock_budget(clock), reference);
                for (depth, nodes, movetime) in [
                    (None, None, None),
                    (Some(2), None, None),
                    (None, Some(100), None),
                    (None, None, Some(500)),
                    (Some(2), Some(100), Some(500)),
                ] {
                    if byoyomi > 0 && depth.is_none() && nodes.is_none() && movetime.is_none() {
                        continue;
                    }
                    let limits = SearchLimits::new(depth, nodes, movetime, Some(clock)).unwrap();
                    let budget = time_budget(&limits).unwrap();
                    let mut expected = reference;
                    if let Some(fixed) = movetime {
                        expected.soft = expected.soft.min(ms(fixed));
                        expected.hard = expected.hard.min(ms(fixed));
                    }
                    assert_eq!(budget, expected, "{limits:?}");
                    for hit in [Duration::ZERO, ms(1000)] {
                        for elapsed in [
                            Duration::ZERO,
                            ms(1),
                            ms(300),
                            ms(1000),
                            ms(1500),
                            hit + expected.soft,
                            hit + expected.hard,
                        ] {
                            for stable in [false, true] {
                                // 予測完了時刻は予算以下、的中からの経過時間はsoft未満の場合だけ次を始める。
                                let predicted = elapsed.as_nanos()
                                    * crate::search::alphabeta::params::iteration_ratio() as u128;
                                let fits = predicted <= (hit + expected.hard).as_nanos() * 100
                                    && if stable {
                                        predicted <= (hit + expected.soft).as_nanos() * 100
                                    } else {
                                        elapsed.saturating_sub(hit) < expected.soft
                                    };
                                assert_eq!(
                                    should_start_next_iteration(elapsed, hit, budget, stable),
                                    fits
                                );
                            }
                        }
                    }
                }
            }
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

// 残り時間が正の秒読みつき時計では、指定した余裕を安全上限から引き、予算の下限は1 msとする。
#[test]
fn byoyomi_with_remaining_time_changes_only_the_safety_margin() {
    for remaining in [1, 30, 1000, 60_000, u64::MAX] {
        for byoyomi in [1, 30, 1000, u64::MAX] {
            for increment in [0, 1000] {
                for ply in [0, 36, 432, u32::MAX] {
                    for margin in [0, 30, 1000, 60_000] {
                        let clock = clock_at_ply(remaining, increment, byoyomi, ply)
                            .with_byoyomi_margin_ms(margin);
                        let limits = SearchLimits::new(None, None, None, Some(clock)).unwrap();
                        let expected = super::tuning::reference_clock_budget(clock, margin);
                        assert_eq!(time_budget(&limits), Some(expected));
                        if margin == 30 {
                            assert_eq!(time_budget(&limits), Some(clock_budget(clock)));
                        }
                    }
                }
            }
        }
    }
    // 安全上限が他の2項より小さい入力で、上限そのものと1msの下限を固定する。
    for (margin, expected_ms) in [(150, 50), (200, 1), (201, 1)] {
        let clock = clock(100, 1000, 100).with_byoyomi_margin_ms(margin);
        let limits = SearchLimits::new(None, None, None, Some(clock)).unwrap();
        let budget = time_budget(&limits).unwrap();
        assert_eq!(budget.soft, Duration::from_millis(expected_ms));
        assert_eq!(budget.hard, Duration::from_millis(expected_ms));
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
