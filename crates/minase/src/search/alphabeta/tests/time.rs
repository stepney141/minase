//! 時間管理を検査する。

use super::*;

// ---------------------------------------------------------------------------
// D7-TIME　時間予算
// ---------------------------------------------------------------------------

// D7-TIME-01。search.md「時間管理」節が規定する予算の不変条件を、代表値の
// 直積で検査する。期待値は公開された式から導出し、探索実装の状態には依存しない。
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

// D7-TIME-02。search.md「時間管理」節: 時計なしの固定時間単独では、併用則
// （softとhardのそれぞれで小さい方）の帰結として両リミットとも指定値になる。
#[test]
fn movetime_alone_sets_both_soft_and_hard_to_the_given_value() {
    let budget = time_budget(&movetime_limits(500)).expect("movetime must produce a budget");
    assert_eq!(budget.soft, Duration::from_millis(500));
    assert_eq!(budget.hard, Duration::from_millis(500));
}

// D7-TIME-03。search.md「時間管理」節: 固定時間と時計の併用時の予算は、
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

// 同「最善手安定時の早期終了」。直近4反復の最善手が同じときだけ安定とする。
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

// D7-TIME-05。search.md「時間管理」節: 継続条件を満たさない主ワーカーは
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
    let pst = crate::eval::weights().unwrap();
    let tt = small_tt();

    let outcome = run_main_worker(
        &pst,
        &position,
        engine_rules(),
        &root_moves,
        &history_keys,
        2,
        Some(budget),
        &shared,
        &tt,
        &mut Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]),
        None,
        false,
    );

    assert_eq!(shared.reason(), StopReason::SoftLimit);
    assert_eq!(outcome.result.depth, 1);
    assert!(root_moves.contains(&outcome.result.best_move)); // INV-1
}

// D7-TIME-04。search.md「時間管理」節: hardはノード周期の時計チェックで
// 適用される。上限の許容幅はテスト環境定数であり規範値ではない。
// D7-TIME-05の継続判断との先着は実時間に依存するため停止理由は2値を許容する。
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
    // soft=hardのため停止理由は機構上の先着が不定（SPEC_UNCLEAR-04）。
    assert!(matches!(
        finished.stop_reason,
        StopReason::SoftLimit | StopReason::HardLimit
    ));
    assert!(legal_moves(&initial).contains(&finished.best_move)); // INV-1
}

// D7-API-03(3)(4)。search.md「スレッド構成」節の停止理由のうちsoftリミットと
// hardリミット。イテレーション境界の先着に依存するため、いずれも2値で
// assertする（SPEC_UNCLEAR-04・09。複数制限の同着は扱わない）。
#[test]
fn clock_driven_searches_stop_with_a_time_limit_reason() {
    let initial = Position::initial();

    // (3) soft≪hardの時計設定（残り6000ms・加算100ms・ply=0 → soft=96ms、
    //     hard=384ms）。原則はsoftリミットで停止する。
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
    assert!(legal_moves(&initial).contains(&finished.best_move)); // INV-1

    // (4) 時計の安全上限が効いてsoft=hard=70msとなる設定。
    let limits = SearchLimits::new(None, None, None, Some(clock(0, 0, 100))).unwrap();
    let budget = time_budget(&limits).expect("byoyomi must produce a time budget");
    assert_eq!(budget.soft, Duration::from_millis(70));
    assert_eq!(budget.hard, Duration::from_millis(70));
}

// byoyomi-time-usage.md「設計判断」の対象外の定義と、変更を秒読みが正の時計に限る理由。
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
                                // strength-stage6.mdとponder.mdの予測式と未満境界。
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

// byoyomi-time-usage.md「秒読み期の締切」。加算、手数、安定性によらずb未満だけで始める。
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

// byoyomi-time-usage.md「締切の余裕を絶対値のUSIオプションにする理由」。
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

// byoyomi-time-usage.md「秒読み期の締切」。反復前と完了後の両方の配線を検査する。
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
    let pst = weights().unwrap();
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
            &pst,
            &position,
            engine_rules(),
            &root_moves,
            &[],
            2,
            Some(budget),
            &shared,
            &small_tt(),
            &mut Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]),
            None,
            ponder,
        );
        assert_eq!(outcome.result.depth, 2);
        assert_eq!(shared.reason(), StopReason::DepthCompleted);
        assert!(root_moves.contains(&outcome.result.best_move));
    }
}
