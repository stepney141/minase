//! 時間管理を検査する。

use super::*;

/// 呼び出し順に指定した経過時間を返し、列の末尾に達した後は最後の時刻を返す時計。
struct SequenceClock {
    elapsed: Vec<Duration>,
    calls: std::sync::atomic::AtomicUsize,
}

impl SequenceClock {
    fn new(elapsed: Vec<Duration>) -> Self {
        assert!(!elapsed.is_empty());
        assert!(elapsed.windows(2).all(|pair| pair[0] <= pair[1]));
        Self {
            elapsed,
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl SearchClock for SequenceClock {
    fn elapsed(&self) -> Duration {
        let call = self.calls.fetch_add(1, AtomicOrdering::Relaxed);
        self.elapsed[call.min(self.elapsed.len() - 1)]
    }
}

/// 進捗通知後の反復境界では、的中時刻を読んだ後の新しい時刻で継続を判定する。
#[test]
fn iteration_boundary_rereads_the_clock_after_progress() {
    let position = bare_kings();
    let root_moves = legal_moves(&position);
    let budget = TimeBudget {
        deadline: false,
        soft: Duration::from_millis(500),
        hard: Duration::from_secs(5),
    };
    let clock = SequenceClock::new(vec![
        Duration::ZERO,
        Duration::from_millis(100),
        Duration::from_millis(600),
    ]);
    let hit_ns = AtomicU64::new(0);
    let stop = AtomicBool::new(false);
    let shared = SharedSearch {
        external_stop: &stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        best_move_changes: AtomicU64::new(0),
        increase_depth: AtomicBool::new(true),
        node_limit: None,
        clock: &clock,
        hard_limit: Some(HardLimit {
            duration: budget.hard,
            hit_ns: &hit_ns,
        }),
    };
    let (sender, receiver) = mpsc::channel();
    let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
    let outcome = run_main_worker(
        test_nnue(),
        &position,
        engine_rules(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        DRAW_SCORE,
        &root_moves,
        &[search_key(&position)],
        2,
        Some(budget),
        None,
        &shared,
        &small_tt(),
        &mut Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]),
        &mut empty_capture_history(),
        &new_correction_values(),
        &mut histories.continuation_correction[0],
        &histories.continuation,
        &histories.pawn,
        Some((&sender, 1)),
        false,
        false,
    );
    drop(sender);

    assert_eq!(outcome.result.depth, 1);
    assert_eq!(shared.reason(), StopReason::SoftLimit);
    let progress = receiver.try_iter().collect::<Vec<_>>();
    assert!(matches!(
        progress.as_slice(),
        [SearchEvent::Progress { elapsed, .. }] if *elapsed == Duration::from_millis(100)
    ));
}

/// increaseDepthは進捗通知の時刻ではなく、同じ反復境界で読み直した時刻を使う。
#[test]
fn increase_depth_uses_the_refreshed_iteration_boundary_time() {
    let position = bare_kings();
    let root_moves = vec![legal_moves(&position)[0]];
    let budget = TimeBudget {
        deadline: false,
        soft: Duration::from_secs(4),
        hard: Duration::from_secs(5),
    };
    let clock = SequenceClock::new(vec![
        Duration::ZERO,
        Duration::from_millis(100),
        Duration::from_millis(300),
    ]);
    let hit_ns = AtomicU64::new(0);
    let stop = AtomicBool::new(false);
    let shared = SharedSearch {
        external_stop: &stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        best_move_changes: AtomicU64::new(0),
        increase_depth: AtomicBool::new(true),
        node_limit: None,
        clock: &clock,
        hard_limit: Some(HardLimit {
            duration: budget.hard,
            hit_ns: &hit_ns,
        }),
    };
    let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
    let outcome = run_main_worker(
        test_nnue(),
        &position,
        engine_rules(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        DRAW_SCORE,
        &root_moves,
        &[search_key(&position)],
        2,
        Some(budget),
        Some(InstabilityScaling::new(
            PreviousSearch::default(),
            DEFAULT_THREADS,
        )),
        &shared,
        &small_tt(),
        &mut Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]),
        &mut empty_capture_history(),
        &new_correction_values(),
        &mut histories.continuation_correction[0],
        &histories.continuation,
        &histories.pawn,
        None,
        false,
        false,
    );

    assert_eq!(outcome.result.depth, 2);
    assert!(!shared.increase_depth.load(AtomicOrdering::Relaxed));
    assert_eq!(outcome.search_again_counter, 1);
}

/// 詰みによる早期停止は3手以内の勝ちとちょうど2手の負けだけを対象にする。
#[test]
fn mate_early_stop_accepts_only_the_configured_mate_distances() {
    assert!(mate_early_stop(MATE - 3));
    assert!(!mate_early_stop(MATE - 4));
    assert!(mate_early_stop(-(MATE - 2)));
    assert!(!mate_early_stop(-(MATE - 1)));
    assert!(!mate_early_stop(-(MATE - 3)));

    for value in [
        REPETITION_BAND_START,
        REPETITION_WIN,
        -REPETITION_BAND_START,
        -REPETITION_WIN,
        0,
        123,
    ] {
        assert!(!mate_early_stop(value), "value={value}");
    }
}

/// 時計予算の内側でも、完了反復が詰みの条件を満たせばその境界で停止する。
#[test]
fn mate_early_stop_stops_a_timed_search_inside_the_budget() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::Rook),
            (fs(6, 12), Color::Black, PieceKind::King),
        ],
    );
    let root_move = legal_moves(&position)
        .into_iter()
        .find(|mv| mv.from == fs(6, 10) && mv.to == fs(6, 1))
        .expect("the rook must be able to capture the last royal");
    let limits = SearchLimits::new(Some(2), None, None, Some(clock(1_000_000, 0, 0)))
        .expect("the test clock must be valid");
    let budget = time_budget(&limits).expect("the timed search must have a budget");
    assert!(Duration::ZERO < budget.soft);

    let outcome = run_search_team(
        test_nnue(),
        &position,
        engine_rules(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        DRAW_SCORE,
        &[root_move],
        &[search_key(&position)],
        &limits,
        &AtomicBool::new(false),
        DEFAULT_THREADS,
        &small_tt(),
        &mut crate::search::HistoryTables::new(DEFAULT_THREADS),
        None,
        &ManualClock::new(Duration::ZERO),
        &AtomicU64::new(0),
        false,
    );

    assert_eq!(outcome.result.depth, 1);
    assert_eq!(outcome.result.score, MATE - 1);
    assert_eq!(outcome.stop_reason, StopReason::SoftLimit);
}

/// 評価値の下落の因子の式は2つの項を加えてから1回だけ歩兵価値で割り、固定範囲へ切り詰める。
#[test]
fn falling_eval_factor_uses_one_truncating_division_and_fixed_bounds() {
    let coefficients = FallingEvalCoefficients {
        base: 1000,
        previous: 200,
        iteration: 100,
    };
    assert_eq!(
        falling_eval_factor(Some(50), 20, 10, 100, coefficients),
        1090
    );
    assert_eq!(
        falling_eval_factor(Some(500), 500, 0, 100, coefficients),
        1728
    );
    assert_eq!(
        falling_eval_factor(Some(-500), -500, 0, 100, coefficients),
        576
    );

    // 2項を別々に割れば0 + 0になるが、和4を3で割るので1になる。
    let one_division = FallingEvalCoefficients {
        base: 1000,
        previous: 1,
        iteration: 1,
    };
    assert_eq!(falling_eval_factor(Some(3), 3, 1, 3, one_division), 1001);
    // 分子−1の0方向への丸めは0であり、床関数の−1とは異なる。
    assert_eq!(falling_eval_factor(Some(0), 1, 1, 3, one_division), 1000);
    assert_eq!(
        falling_eval_factor(None, i32::MIN, i32::MAX, 1, coefficients),
        1728
    );
}

/// 最善手の安定性から求める短縮率は直線を固定範囲へ切り詰め、時間因子は床方向へ丸める。
#[test]
fn best_move_stability_factor_uses_fixed_bounds_and_floor_division() {
    let coefficients = TimeReductionCoefficients {
        intercept: 300,
        slope: 100,
        scale: 2000,
    };
    assert_eq!(time_reduction(0, coefficients), 629);
    assert_eq!(time_reduction(4, coefficients), 700);
    assert_eq!(time_reduction(20, coefficients), 1544);

    // 2_318_000_000 / 1_400_000 = 1655余り1_000_000である。
    assert_eq!(best_move_stability_factor(850, 700, 2000), 1655);
}

/// 最善手が同じ末尾の長さから1を引き、直前の反復で変わった場合は0へ戻す。
#[test]
fn best_move_stability_distance_counts_the_equal_suffix() {
    let moves = legal_moves(&Position::initial());
    let [a, b] = [moves[0], moves[1]];
    assert_eq!(best_move_stability_distance(&[a]), 0);
    assert_eq!(best_move_stability_distance(&[a, b, b, b]), 2);
    assert_eq!(best_move_stability_distance(&[a, a, b]), 0);
}

/// 最善手の変化回数の因子は全ワーカーの平均を使い、正の除算を床方向へ丸める。
#[test]
fn best_move_instability_factor_divides_by_workers_and_rounds_down() {
    let coefficients = TimeInstabilityCoefficients {
        base: 1077,
        slope: 2229,
    };
    // 3回相当を2ワーカーで割った傾きの項は3343.5を床方向へ丸めて3343になる。
    assert_eq!(best_move_instability_factor(3072, 2, coefficients), 4420);
    // 同じ累計を3ワーカーで割れば、傾きの項はちょうど2229になる。
    assert_eq!(best_move_instability_factor(3072, 3, coefficients), 3306);
}

/// 反復開始時に累計を半分にしてから、完了時に回収した回数を1024倍して加える。
#[test]
fn best_move_change_accumulator_decays_before_adding_completed_changes() {
    let coefficients = TimeInstabilityCoefficients {
        base: 1000,
        slope: 1000,
    };
    let mut scaling = InstabilityScaling::new(PreviousSearch::default(), worker_count(2));
    assert_eq!(scaling.best_move_change_accumulator, 0);

    scaling.begin_iteration();
    assert_eq!(scaling.update_best_move_instability(3, coefficients), 2500);
    assert_eq!(scaling.best_move_change_accumulator, 3072);

    scaling.begin_iteration();
    assert_eq!(scaling.best_move_change_accumulator, 1536);
    assert_eq!(scaling.update_best_move_instability(2, coefficients), 2750);
    assert_eq!(scaling.best_move_change_accumulator, 3584);
}

/// 最善手のノード占有率は100000倍した整数とし、全ノード数0は分母1として扱う。
#[test]
fn best_move_effort_ratio_scales_nodes_and_uses_one_for_zero_total() {
    assert_eq!(best_move_effort_ratio(3, 8), 37_500);
    assert_eq!(best_move_effort_ratio(0, 0), 0);
    assert_eq!(best_move_effort_ratio(1, 0), 100_000);
}

/// best move effortの直線は固定範囲へ切り詰め、負の傾きの項を0の方向へ丸める。
#[test]
fn best_move_effort_factor_uses_fixed_bounds_and_truncates_toward_zero() {
    assert_eq!(
        best_move_effort_factor(
            1,
            4,
            TimeEffortCoefficients {
                intercept: 800,
                slope: -200,
            },
        ),
        750
    );
    assert_eq!(
        best_move_effort_factor(
            0,
            1,
            TimeEffortCoefficients {
                intercept: 900,
                slope: 0,
            },
        ),
        838
    );
    assert_eq!(
        best_move_effort_factor(
            0,
            1,
            TimeEffortCoefficients {
                intercept: 600,
                slope: 0,
            },
        ),
        693
    );

    // −3 × 33333 / 100000は0方向なら0、床方向なら−1になる。
    assert_eq!(
        best_move_effort_factor(
            1,
            3,
            TimeEffortCoefficients {
                intercept: 800,
                slope: -3,
            },
        ),
        800
    );
}

/// time advantageの因子は相手の残り時間が多いときだけ減り、正の除算を床方向へ丸める。
#[test]
fn time_advantage_factor_uses_remaining_time_difference_and_floor_division() {
    assert_eq!(time_advantage_factor(5, 5, 900), 1000);
    assert_eq!(time_advantage_factor(6, 5, 900), 1000);
    assert_eq!(time_advantage_factor(5, 0, 900), 1000);

    // 900 × 3 / 8は337余り4なので、因子は663になる。
    assert_eq!(time_advantage_factor(2, 5, 900), 663);
    assert_eq!(time_advantage_factor(1, 4, 0), 1000);
    assert_eq!(time_advantage_factor(1, 4, 1000), 500);
}

/// time advantageはsoft_rawとその倍数で決まるhardだけを縮める。
/// 残り時間の割合と安全上限で決まるhardは変えない。
#[test]
fn time_advantage_scales_soft_and_only_the_soft_multiple_hard_candidate() {
    let coefficients = |hard_remaining_share| ClockCoefficients {
        expected_plies: 20,
        min_moves: 10,
        increment_share: 0,
        hard_soft_ratio: 200,
        hard_remaining_share,
        time_advantage: 900,
    };
    let clock = |opponent_remaining_ms| {
        ClockLimits::new(1000, 0, 0, Some(opponent_remaining_ms), 0).unwrap()
    };

    let neutral = coefficients(50).budget(clock(1000), 30);
    let scaled = coefficients(50).budget(clock(3000), 30);
    assert_eq!(neutral.soft, Duration::from_millis(100));
    assert_eq!(neutral.hard, Duration::from_millis(200));
    assert_eq!(scaled.soft, Duration::from_millis(55));
    assert_eq!(scaled.hard, Duration::from_millis(110));

    let remaining_limited = coefficients(10);
    assert_eq!(
        remaining_limited.budget(clock(1000), 30).hard,
        Duration::from_millis(100)
    );
    assert_eq!(
        remaining_limited.budget(clock(3000), 30).hard,
        Duration::from_millis(100)
    );

    let safety_limited = coefficients(50);
    assert_eq!(
        safety_limited.budget(clock(1000), 950).hard,
        Duration::from_millis(50)
    );
    assert_eq!(
        safety_limited.budget(clock(3000), 950).hard,
        Duration::from_millis(50)
    );
}

/// 時間因子の積は基礎softから求め、積があふれた後もu128の時間としてhardまで保持する。
#[test]
fn scaled_soft_time_handles_one_multiple_and_saturating_factors() {
    assert_eq!(
        scaled_soft_millis(Duration::from_millis(2000), &[1500], 2),
        3000
    );
    assert_eq!(
        scaled_soft_millis(Duration::from_millis(2000), &[1500, 800], 2),
        2400
    );

    let maximum = u128::from(u64::MAX);
    let saturated = scaled_soft_millis(Duration::from_millis(u64::MAX), &[maximum, maximum, 2], 2);
    assert_eq!(saturated, u128::MAX / 1_000_000_000);
    assert!(saturated > u128::from(u64::MAX));

    let base = TimeBudget {
        soft: Duration::from_millis(u64::MAX),
        hard: Duration::from_millis(u64::MAX),
        deadline: false,
    };
    let scaled = scaled_time_budget(base, saturated);
    assert_eq!(scaled.soft, base.hard);
    assert_eq!(scaled.hard, base.hard);
    assert_eq!(scaled.deadline, base.deadline);
}

/// 先読み後の基礎softは25%増しをミリ秒単位で床方向へ丸め、hardを超えない。
#[test]
fn ponder_bonus_adds_one_quarter_with_floor_and_hard_cap() {
    let uncapped = TimeBudget {
        soft: Duration::from_millis(11),
        hard: Duration::from_millis(20),
        deadline: false,
    };
    assert_eq!(
        ponder_bonus_time_budget(uncapped),
        TimeBudget {
            soft: Duration::from_millis(13),
            ..uncapped
        }
    );

    let capped = TimeBudget {
        soft: Duration::from_millis(80),
        hard: Duration::from_millis(90),
        deadline: false,
    };
    assert_eq!(
        ponder_bonus_time_budget(capped),
        TimeBudget {
            soft: capped.hard,
            ..capped
        }
    );
}

/// increaseDepthは的中後の経過時間をhardで抑える前の時間と比べ、50%ちょうどを含む。
#[test]
fn increase_depth_uses_half_of_unclamped_scaled_time_without_rounding() {
    let hit = Duration::from_secs(1);
    assert!(should_increase_depth(
        Duration::from_millis(1500),
        Some(hit),
        1000
    ));
    assert!(!should_increase_depth(
        Duration::from_millis(1500) + Duration::from_nanos(1),
        Some(hit),
        1000
    ));
    assert!(should_increase_depth(Duration::from_secs(100), None, 1));

    let hard_capped = scaled_time_budget(
        TimeBudget {
            soft: Duration::from_millis(100),
            hard: Duration::from_millis(400),
            deadline: false,
        },
        1000,
    );
    assert_eq!(hard_capped.soft, Duration::from_millis(400));
    assert!(Duration::from_millis(450) > hard_capped.soft);
    assert!(should_increase_depth(
        Duration::from_millis(450),
        Some(Duration::ZERO),
        1000
    ));
}

/// 根の候補が1手なら因子の積を求めた後の時間を500 msで抑え、2手以上と500 ms未満は変えない。
#[test]
fn single_root_move_caps_scaled_soft_time_after_factor_product() {
    assert_eq!(
        scaled_soft_millis(Duration::from_millis(2000), &[1500], 1),
        500
    );
    assert_eq!(
        scaled_soft_millis(Duration::from_millis(2000), &[1500], 2),
        3000
    );
    assert_eq!(
        scaled_soft_millis(Duration::from_millis(250), &[1500], 1),
        375
    );

    let maximum = u128::from(u64::MAX);
    assert_eq!(
        scaled_soft_millis(Duration::from_millis(u64::MAX), &[maximum, maximum, 2], 1),
        500
    );
}

/// 評価値の下落の因子の環状配列は前の探索の評価値で始まり、4反復後に最初の完了値を再び参照する。
#[test]
fn falling_eval_ring_uses_previous_score_then_the_value_from_four_iterations_ago() {
    let coefficients = FallingEvalCoefficients {
        base: 1000,
        previous: 0,
        iteration: 100,
    };
    let mut scaling = InstabilityScaling::new(
        PreviousSearch {
            score_and_average: Some((40, 0)),
            time_reduction: 900,
            ponder_search_started: false,
        },
        worker_count(1),
    );
    assert_eq!(scaling.iteration_values, [40; 4]);
    assert_eq!(scaling.previous_time_reduction, 900);
    assert_eq!(scaling.time_reduction, 1000);
    assert_eq!(scaling.best_move_change_accumulator, 0);
    let factors: Vec<_> = [10, 20, 30, 40, 50]
        .into_iter()
        .map(|value| scaling.update_falling_eval(value, 100, coefficients))
        .collect();
    assert_eq!(factors, [1030, 1020, 1010, 1000, 960]);
    assert_eq!(scaling.iteration_values, [50, 20, 30, 40]);
    assert_eq!(scaling.current, 1);

    let mut missing = InstabilityScaling::new(PreviousSearch::default(), worker_count(1));
    assert_eq!(missing.previous_average, None);
    assert_eq!(missing.iteration_values, [0; 4]);
    assert_eq!(missing.update_falling_eval(9, 100, coefficients), 1728);
    assert_eq!(missing.iteration_values, [9, 0, 0, 0]);
}

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

// 固定時間だけを指定した探索は、的中時刻と最善手の安定性によらず、指定時間を締切にする。
#[test]
fn movetime_budget_and_gate_use_only_the_deadline() {
    let ms = Duration::from_millis;
    for movetime in [1, 30, 1000, 60_000, u64::MAX] {
        let budget = time_budget(&movetime_limits(movetime)).unwrap();
        let deadline = ms(movetime);
        assert_eq!(budget.soft, deadline);
        assert_eq!(budget.hard, deadline);
        assert!(budget.deadline);
        for hit in [Duration::ZERO, ms(10_000)] {
            for stable in [false, true] {
                for (since_hit, expected) in [
                    (deadline - Duration::from_nanos(1), true),
                    (deadline, false),
                    (deadline + Duration::from_nanos(1), false),
                ] {
                    assert_eq!(
                        should_start_next_iteration(hit + since_hit, hit, budget, stable),
                        expected,
                        "movetime={movetime}, hit={hit:?}, stable={stable}, since_hit={since_hit:?}"
                    );
                }
            }
        }
    }
}

// 深さやノード数を併用する固定時間の予算は指定値のままで、予測による開始判定を使う。
#[test]
fn movetime_with_depth_or_nodes_keeps_prediction_based_budget() {
    for movetime in [1, 30, 1000, 60_000, u64::MAX] {
        for (depth, nodes) in [(Some(2), None), (None, Some(100)), (Some(2), Some(100))] {
            let limits = SearchLimits::new(depth, nodes, Some(movetime), None).unwrap();
            assert_eq!(
                time_budget(&limits),
                Some(TimeBudget {
                    soft: Duration::from_millis(movetime),
                    hard: Duration::from_millis(movetime),
                    deadline: false,
                })
            );
        }
    }
}

// 時間を指定しない深さ制限、ノード数制限、無限探索は、時間予算を持たない。
#[test]
fn limits_without_time_constraints_have_no_budget() {
    for limits in [depth_limits(2), nodes_limits(100), infinite_limits()] {
        assert_eq!(time_budget(&limits), None);
    }
}

// 時計だけの探索では最初の完了反復後に時間伸縮の因子を適用する。
// 同じ基礎予算でもmovetimeを併用すれば因子を使わず、基礎softで止まる。
#[cfg(feature = "tuning")]
pub(super) fn instability_scaling_starts_an_iteration_that_the_base_soft_rejects() {
    let position = bare_kings();
    let root_moves = vec![legal_moves(&position)[0]];
    let history_keys = [search_key(&position)];
    let clock_limits = clock(100_000, 0, 0);
    let scaled_limits = SearchLimits::new(Some(2), None, None, Some(clock_limits)).unwrap();
    let base = time_budget(&scaled_limits).unwrap();
    let coefficients = TimeReductionCoefficients::current();
    let reduction = time_reduction(0, coefficients);
    let stability = best_move_stability_factor(850, reduction, coefficients.scale);
    let instability = best_move_instability_factor(
        0,
        DEFAULT_THREADS.get(),
        TimeInstabilityCoefficients::current(),
    );
    let effort = best_move_effort_factor(1, 1, TimeEffortCoefficients::current());
    let scaled = scaled_time_budget(
        base,
        scaled_soft_millis(
            base.soft,
            &[1728, stability as u128, instability, effort as u128],
            root_moves.len(),
        ),
    );
    assert!(scaled.soft > base.soft);
    assert!(iteration_prediction_fits(
        base.soft,
        Duration::ZERO,
        base,
        false
    ));

    let fixed_ms = u64::try_from(base.hard.as_millis()).unwrap();
    let excluded_limits =
        SearchLimits::new(Some(2), None, Some(fixed_ms), Some(clock_limits)).unwrap();
    assert_eq!(time_budget(&excluded_limits), Some(base));

    let run = |limits: &SearchLimits| {
        let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
        run_search_team(
            test_nnue(),
            &position,
            engine_rules(),
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            DRAW_SCORE,
            &root_moves,
            &history_keys,
            limits,
            &AtomicBool::new(false),
            DEFAULT_THREADS,
            &small_tt(),
            &mut histories,
            None,
            &ManualClock::new(base.soft),
            &AtomicU64::new(0),
            false,
        )
    };
    let scaled_outcome = run(&scaled_limits);
    assert_eq!(scaled_outcome.result.depth, 2);
    assert_eq!(scaled_outcome.stop_reason, StopReason::DepthCompleted);

    let excluded_outcome = run(&excluded_limits);
    assert_eq!(excluded_outcome.result.depth, 1);
    assert_eq!(excluded_outcome.stop_reason, StopReason::SoftLimit);
}

// 初期化直後の850を使った予算境界では停止し、1000に置き換えた場合だけ次の反復を完了する。
#[cfg(feature = "tuning")]
pub(super) fn initial_previous_time_reduction_controls_the_first_scaled_budget() {
    let position = bare_kings();
    let root_moves = vec![legal_moves(&position)[0]];
    let history_keys = [search_key(&position)];
    let limits = SearchLimits::new(Some(2), None, None, Some(clock(20_000, 0, 0))).unwrap();
    let base = time_budget(&limits).unwrap();
    let coefficients = TimeReductionCoefficients::current();
    let reduction = time_reduction(0, coefficients);
    let initialized_factor = best_move_stability_factor(850, reduction, coefficients.scale);
    let neutral_factor = best_move_stability_factor(1000, reduction, coefficients.scale);
    let instability = best_move_instability_factor(
        0,
        DEFAULT_THREADS.get(),
        TimeInstabilityCoefficients::current(),
    );
    let effort = best_move_effort_factor(1, 1, TimeEffortCoefficients::current()) as u128;
    let initialized_ms = scaled_soft_millis(
        base.soft,
        &[1728, initialized_factor as u128, instability, effort],
        root_moves.len(),
    );
    let neutral_ms = scaled_soft_millis(
        base.soft,
        &[1728, neutral_factor as u128, instability, effort],
        root_moves.len(),
    );
    assert!(initialized_ms < neutral_ms);
    assert!(neutral_ms < 500);
    assert!(initialized_ms < base.hard.as_millis());
    let elapsed = Duration::from_millis(u64::try_from(initialized_ms).unwrap());
    assert!(iteration_prediction_fits(
        elapsed,
        Duration::ZERO,
        base,
        false
    ));

    let run = |histories: &mut crate::search::HistoryTables| {
        run_search_team(
            test_nnue(),
            &position,
            engine_rules(),
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            DRAW_SCORE,
            &root_moves,
            &history_keys,
            &limits,
            &AtomicBool::new(false),
            DEFAULT_THREADS,
            &small_tt(),
            histories,
            None,
            &ManualClock::new(elapsed),
            &AtomicU64::new(0),
            false,
        )
    };

    let mut initialized = crate::search::HistoryTables::new(DEFAULT_THREADS);
    assert_eq!(initialized.previous_search.time_reduction, 850);
    let initialized_outcome = run(&mut initialized);
    assert_eq!(initialized_outcome.result.depth, 1);
    assert_eq!(initialized_outcome.stop_reason, StopReason::SoftLimit);

    let mut neutral = crate::search::HistoryTables::new(DEFAULT_THREADS);
    neutral.previous_search.time_reduction = 1000;
    let neutral_outcome = run(&mut neutral);
    assert_eq!(neutral_outcome.result.depth, 2);
}

#[cfg(feature = "tuning")]
fn run_ponder_bonus_test_search(
    position: &Position,
    root_moves: &[Move],
    limits: &SearchLimits,
    elapsed: Duration,
    hit_ns: u64,
    ponder: bool,
    histories: &mut crate::search::HistoryTables,
) -> (u32, StopReason) {
    let outcome = run_search_team(
        test_nnue(),
        position,
        engine_rules(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        DRAW_SCORE,
        root_moves,
        &[search_key(position)],
        limits,
        &AtomicBool::new(false),
        DEFAULT_THREADS,
        &small_tt(),
        histories,
        None,
        &ManualClock::new(elapsed),
        &AtomicU64::new(hit_ns),
        ponder,
    );
    (outcome.result.depth, outcome.stop_reason)
}

/// 先読み後の通常探索でも、25%増しは秒読みが0の時計だけに適用する。
#[cfg(feature = "tuning")]
pub(super) fn ponder_bonus_requires_a_marked_normal_zero_byoyomi_clock() {
    let position = bare_kings();
    let root_moves = legal_moves(&position);
    assert!(root_moves.len() > 1);
    let zero_byoyomi = SearchLimits::new(Some(2), None, None, Some(clock(20_000, 0, 0))).unwrap();
    let positive_byoyomi =
        SearchLimits::new(Some(2), None, None, Some(clock(20_000, 0, 1))).unwrap();
    assert_eq!(
        time_budget(&zero_byoyomi),
        Some(TimeBudget {
            soft: Duration::from_millis(100),
            hard: Duration::from_millis(200),
            deadline: false,
        })
    );
    assert_eq!(time_budget(&positive_byoyomi), time_budget(&zero_byoyomi));
    let new_histories = || {
        let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
        histories.previous_search.score_and_average = Some((0, 0));
        histories.previous_search.time_reduction = 1000;
        histories
    };
    let elapsed = Duration::from_millis(101);

    let mut unmarked = new_histories();
    assert_eq!(
        run_ponder_bonus_test_search(
            &position,
            &root_moves,
            &zero_byoyomi,
            elapsed,
            0,
            false,
            &mut unmarked,
        ),
        (1, StopReason::SoftLimit)
    );

    let mark_limits = SearchLimits::new(Some(1), None, Some(1_000), None).unwrap();
    let mark_ponder_started = |histories: &mut crate::search::HistoryTables| {
        assert_eq!(
            run_ponder_bonus_test_search(
                &position,
                &root_moves,
                &mark_limits,
                Duration::ZERO,
                u64::MAX,
                true,
                histories,
            ),
            (1, StopReason::DepthCompleted)
        );
        assert!(histories.previous_search.ponder_search_started);
    };

    let mut marked = new_histories();
    mark_ponder_started(&mut marked);
    assert_eq!(
        run_ponder_bonus_test_search(
            &position,
            &root_moves,
            &zero_byoyomi,
            elapsed,
            0,
            false,
            &mut marked,
        ),
        (2, StopReason::DepthCompleted)
    );

    let mut marked_byoyomi = new_histories();
    mark_ponder_started(&mut marked_byoyomi);
    assert_eq!(
        run_ponder_bonus_test_search(
            &position,
            &root_moves,
            &positive_byoyomi,
            elapsed,
            0,
            false,
            &mut marked_byoyomi,
        ),
        (1, StopReason::SoftLimit)
    );
}

/// 先読みとして始めた探索には的中後も増分を使わず、movetime併用と秒読み期も対象外とする。
#[cfg(feature = "tuning")]
pub(super) fn ponder_bonus_excludes_hit_ponder_movetime_and_byoyomi_period() {
    let position = bare_kings();
    let root_moves = legal_moves(&position);
    let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
    histories.previous_search.score_and_average = Some((0, 0));
    histories.previous_search.time_reduction = 1000;
    histories.previous_search.ponder_search_started = true;

    let clock_limits = clock(20_000, 0, 0);
    let timed = SearchLimits::new(Some(2), None, None, Some(clock_limits)).unwrap();
    assert_eq!(
        run_ponder_bonus_test_search(
            &position,
            &root_moves,
            &timed,
            Duration::from_millis(110),
            0,
            true,
            &mut histories,
        ),
        (0, StopReason::SoftLimit)
    );

    let with_movetime = SearchLimits::new(Some(2), None, Some(1_000), Some(clock_limits)).unwrap();
    assert_eq!(
        run_ponder_bonus_test_search(
            &position,
            &root_moves,
            &with_movetime,
            Duration::from_millis(110),
            0,
            false,
            &mut histories,
        ),
        (1, StopReason::SoftLimit)
    );

    let byoyomi = SearchLimits::new(None, None, None, Some(clock(0, 0, 31))).unwrap();
    let _ = run_ponder_bonus_test_search(
        &position,
        &root_moves,
        &byoyomi,
        Duration::from_millis(1),
        0,
        false,
        &mut histories,
    );
    assert!(histories.previous_search.ponder_search_started);
}

// 共有ノード数に補助ワーカー相当の値があっても、最善手の占有率は主ワーカー自身の
// 根の手ごとの記録と全ノード数から求める。
#[cfg(feature = "tuning")]
pub(super) fn best_move_effort_scaling_ignores_shared_auxiliary_nodes() {
    let position = bare_kings();
    let root_moves = vec![legal_moves(&position)[0]];
    let history_keys = [search_key(&position)];
    let external_stop = AtomicBool::new(false);
    let budget = TimeBudget {
        soft: Duration::from_millis(500),
        hard: Duration::from_millis(10_000),
        deadline: false,
    };
    let shared = SharedSearch {
        external_stop: &external_stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(1_000_000),
        best_move_changes: AtomicU64::new(0),
        increase_depth: AtomicBool::new(true),
        node_limit: None,
        clock: &ManualClock::new(Duration::from_millis(346)),
        hard_limit: Some(HardLimit {
            duration: budget.hard,
            hit_ns: &AtomicU64::new(0),
        }),
    };
    let scaling = InstabilityScaling::new(
        PreviousSearch {
            score_and_average: Some((0, 0)),
            time_reduction: 1032,
            ponder_search_started: false,
        },
        worker_count(2),
    );
    let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);

    let outcome = run_main_worker(
        test_nnue(),
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
        Some(scaling),
        &shared,
        &small_tt(),
        &mut Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]),
        &mut empty_capture_history(),
        &new_correction_values(),
        &mut histories.continuation_correction[0],
        &histories.continuation,
        &histories.pawn,
        None,
        false,
        false,
    );

    assert!(outcome.nodes > 0);
    assert!(shared.nodes() > outcome.nodes);
    assert_eq!(outcome.result.depth, 1);
    assert_eq!(shared.reason(), StopReason::SoftLimit);
}

// 基礎softでは次の反復を始められる500 msの境界で、根の候補が1手なら
// 完了反復後の伸縮によって停止する。
#[cfg(feature = "tuning")]
pub(super) fn single_root_move_scaling_stops_at_500_millis_boundary() {
    let position = bare_kings();
    let root_moves = vec![legal_moves(&position)[0]];
    let history_keys = [search_key(&position)];
    let limits = SearchLimits::new(Some(2), None, None, Some(clock(1_000_000, 0, 0))).unwrap();
    let budget = time_budget(&limits).unwrap();
    let clock = ManualClock::new(Duration::from_millis(500));
    assert!(should_start_next_iteration(
        clock.elapsed(),
        Duration::ZERO,
        budget,
        false
    ));
    let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
    histories.previous_search = PreviousSearch {
        score_and_average: Some((0, 0)),
        time_reduction: 1032,
        ponder_search_started: false,
    };

    let outcome = run_search_team(
        test_nnue(),
        &position,
        engine_rules(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        DRAW_SCORE,
        &root_moves,
        &history_keys,
        &limits,
        &AtomicBool::new(false),
        DEFAULT_THREADS,
        &small_tt(),
        &mut histories,
        None,
        &clock,
        &AtomicU64::new(0),
        false,
    );

    assert_eq!(outcome.result.depth, 1);
    assert_eq!(outcome.stop_reason, StopReason::SoftLimit);
}

/// increaseDepthの共有状態は探索ごとに真から始まり、偽で始めた反復だけを数える。
/// 主ワーカーが窓を浅く読んでも、進捗には反復の番号を1ずつ通知する。
#[test]
fn increase_depth_state_resets_and_preserves_main_progress_depths() {
    let position = quiet_midgame();
    let root_moves = vec![legal_moves(&position)[0]];
    let history_keys = [search_key(&position)];
    let threads = worker_count(1);
    let mut histories = crate::search::HistoryTables::new(threads);

    let run = |depth, elapsed, histories: &mut crate::search::HistoryTables| {
        let limits =
            SearchLimits::new(Some(depth), None, None, Some(clock(10_000_000, 0, 0))).unwrap();
        let (sender, receiver) = mpsc::channel();
        let outcome = run_search_team(
            test_nnue(),
            &position,
            engine_rules(),
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            DRAW_SCORE,
            &root_moves,
            &history_keys,
            &limits,
            &AtomicBool::new(false),
            threads,
            &small_tt(),
            histories,
            Some((&sender, 1)),
            &ManualClock::new(elapsed),
            &AtomicU64::new(0),
            false,
        );
        drop(sender);
        let depths = receiver
            .try_iter()
            .filter_map(|event| match event {
                SearchEvent::Progress { depth, .. } => Some(depth),
                SearchEvent::Finished { .. } => None,
            })
            .collect::<Vec<_>>();
        (outcome, depths)
    };

    // 根の候補が1手なのでhardで抑える前の時間は500 msとなる。
    // 400 msの境界で共有状態が偽になり、深さ2を始めるときだけ回数が増える。
    let (first, depths) = run(2, Duration::from_millis(400), &mut histories);
    assert_eq!(first.worker_search_again_counters, [1]);
    assert_eq!(depths, [1, 2]);

    // 前の探索が偽で終わっても、新しい探索の最初の反復は真から始める。
    let (second, depths) = run(1, Duration::ZERO, &mut histories);
    assert_eq!(second.stop_reason, StopReason::DepthCompleted);
    assert_eq!(second.worker_search_again_counters, [0]);
    assert_eq!(depths, [1]);
}

/// 補助ワーカーは深さを飛ばしても、実際に始める各反復で共有状態を1回だけ読む。
#[test]
fn auxiliary_worker_counts_only_the_iterations_it_starts() {
    let position = quiet_midgame();
    let root_moves = vec![legal_moves(&position)[0]];
    let external_stop = AtomicBool::new(false);
    let shared = SharedSearch {
        external_stop: &external_stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        best_move_changes: AtomicU64::new(0),
        increase_depth: AtomicBool::new(false),
        node_limit: None,
        clock: &ManualClock::new(Duration::ZERO),
        hard_limit: None,
    };
    let mut histories = crate::search::HistoryTables::new(worker_count(2));

    assert_eq!(auxiliary_depths(1, 6).collect::<Vec<_>>(), [1, 3, 5, 6]);

    let outcome = run_auxiliary_worker(
        test_nnue(),
        &position,
        engine_rules(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        DRAW_SCORE,
        &root_moves,
        &[search_key(&position)],
        6,
        1,
        &shared,
        &small_tt(),
        &mut Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]),
        &mut empty_capture_history(),
        &new_correction_values(),
        &mut histories.continuation_correction[1],
        &histories.continuation,
        &histories.pawn,
    );

    assert_eq!(outcome.result.depth, 6);
    assert_eq!(outcome.search_again_counter, 4);
}

// 指定した直近の反復数だけ最善手が同じときに安定とする。
// 採用理由はdocs/adr/scale-the-soft-limit-by-search-instability-and-keep-the-stable-best-move-rule.md。
#[test]
fn stable_signal_uses_requested_number_of_recent_best_moves() {
    let moves = legal_moves(&Position::initial());
    let [a, b] = [moves[0], moves[1]];
    for iterations in [2, 4, 8] {
        let identical = vec![a; iterations];
        assert!(!stable_signal(&identical[..iterations - 1], iterations));
        assert!(stable_signal(&identical, iterations));

        let mut different_recent = identical.clone();
        different_recent[iterations - 1] = b;
        assert!(!stable_signal(&different_recent, iterations));

        let mut different_before = vec![b];
        different_before.extend(identical);
        assert!(stable_signal(&different_before, iterations));
    }
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
        deadline: false,
        soft: Duration::from_secs(1),
        hard: Duration::from_secs(1),
    };
    let hard_limit_hit_ns = AtomicU64::new(0);
    let shared = SharedSearch {
        external_stop: &external_stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        best_move_changes: AtomicU64::new(0),
        increase_depth: AtomicBool::new(true),
        node_limit: None,
        clock: &ManualClock::new(Duration::from_millis(
            100000 / super::super::params::iteration_ratio() as u64 + 1,
        )),
        hard_limit: Some(HardLimit {
            duration: budget.hard,
            hit_ns: &hard_limit_hit_ns,
        }),
    };
    let nnue = test_nnue();
    let tt = small_tt();
    let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);

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
        None,
        &shared,
        &tt,
        &mut Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]),
        &mut empty_capture_history(),
        &new_correction_values(),
        &mut histories.continuation_correction[0],
        &histories.continuation,
        &histories.pawn,
        None,
        false,
        false,
    );

    assert_eq!(shared.reason(), StopReason::SoftLimit);
    assert_eq!(outcome.result.depth, 1);
    assert!(root_moves.contains(&outcome.result.best_move));
}

// 固定時間に達した時計を渡すと、最初のノード検査点でhard limitとして止まる。
#[test]
fn movetime_search_respects_the_hard_limit_and_returns_a_legal_move() {
    let initial = Position::initial();
    let clock = Arc::new(ManualClock::new(Duration::from_millis(30)));
    let handle = start_search_with_clock(
        Arc::clone(shared_test_nnue()),
        snapshot_for(&initial),
        movetime_limits(30),
        61,
        DEFAULT_THREADS,
        small_tt(),
        crate::search::HistoryTables::new(DEFAULT_THREADS),
        false,
        clock,
    );
    let (_, finished) = event_reports(drain_raw(&handle));
    handle.join().expect("search thread must not panic");
    assert_eq!(finished.elapsed, Duration::from_millis(30));
    assert_eq!(finished.stop_reason, StopReason::HardLimit);
    assert_eq!(finished.depth, 0);
    assert_eq!(finished.nodes, 0);
    assert!(legal_moves(&initial).contains(&finished.best_move));
}

// 秒読みが正の時計では、相手の残り時間によって予算を変えない。
#[test]
fn byoyomi_clock_does_not_apply_time_advantage() {
    let coefficients = ClockCoefficients {
        expected_plies: 20,
        min_moves: 10,
        increment_share: 0,
        hard_soft_ratio: 200,
        hard_remaining_share: 50,
        time_advantage: 900,
    };
    let budget = |opponent_remaining_ms| {
        let clock = ClockLimits::new(1000, 0, 100, Some(opponent_remaining_ms), 0).unwrap();
        let limits = SearchLimits::new(None, None, None, Some(clock)).unwrap();
        time_budget_with_coefficients(&limits, coefficients).unwrap()
    };
    assert_eq!(budget(1000), budget(3000));
    assert_eq!(budget(1000).soft, Duration::from_millis(108));
    assert_eq!(budget(1000).hard, Duration::from_millis(216));
    assert!(!budget(1000).deadline);
    assert!(!budget(3000).deadline);
}

// 固定時間を併用する時計では、相手の残り時間によって予算を変えない。
#[test]
fn movetime_clock_does_not_apply_time_advantage() {
    let coefficients = ClockCoefficients {
        expected_plies: 20,
        min_moves: 10,
        increment_share: 0,
        hard_soft_ratio: 200,
        hard_remaining_share: 50,
        time_advantage: 900,
    };
    // 残り1000 ms、残り10手ではsoft=100 ms、hard=200 msとなる。
    // 秒読み100 msは序盤にsoftへ8 msを足し、持ち時間0なら安全上限70 msで抑える。
    for (remaining, byoyomi, movetime, soft, hard) in [
        (1000, 0, 50, 50, 50),
        (1000, 0, 150, 100, 150),
        (1000, 0, 500, 100, 200),
        (1000, 100, 500, 108, 216),
        (0, 100, 500, 70, 70),
        (0, 100, 50, 50, 50),
    ] {
        let budget = |opponent_remaining_ms| {
            let clock =
                ClockLimits::new(remaining, 0, byoyomi, Some(opponent_remaining_ms), 0).unwrap();
            let limits = SearchLimits::new(None, None, Some(movetime), Some(clock)).unwrap();
            time_budget_with_coefficients(&limits, coefficients).unwrap()
        };
        let expected = TimeBudget {
            soft: Duration::from_millis(soft),
            hard: Duration::from_millis(hard),
            deadline: false,
        };
        assert_eq!(budget(1000), expected);
        assert_eq!(budget(3000), expected);
    }
}

// 深さやノード数を併用しても、秒読みと固定時間がなければtime advantageを適用する。
#[test]
fn depth_and_node_limited_clocks_apply_time_advantage() {
    let coefficients = ClockCoefficients {
        expected_plies: 20,
        min_moves: 10,
        increment_share: 0,
        hard_soft_ratio: 200,
        hard_remaining_share: 50,
        time_advantage: 900,
    };
    for (depth, nodes) in [
        (None, None),
        (Some(2), None),
        (None, Some(100)),
        (Some(2), Some(100)),
    ] {
        let budget = |opponent_remaining_ms| {
            let clock = ClockLimits::new(1000, 0, 0, Some(opponent_remaining_ms), 0).unwrap();
            let limits = SearchLimits::new(depth, nodes, None, Some(clock)).unwrap();
            time_budget_with_coefficients(&limits, coefficients).unwrap()
        };
        assert_eq!(budget(1000).soft, Duration::from_millis(100));
        assert_eq!(budget(1000).hard, Duration::from_millis(200));
        assert_eq!(budget(3000).soft, Duration::from_millis(55));
        assert_eq!(budget(3000).hard, Duration::from_millis(110));
        assert!(!budget(1000).deadline);
        assert!(!budget(3000).deadline);
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
                    assert!(budget.deadline);
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
    for (remaining, increment, byoyomi, ply) in [
        (60000, 1000, 1000, 0),
        (60000, 1000, 1000, 36),
        (100, 1000, 100, 0),
        (1, 0, 1, 0),
    ] {
        let clock = clock_at_ply(remaining, increment, byoyomi, ply);
        let budget = |margin| {
            time_budget(
                &SearchLimits::new(None, None, None, Some(clock.with_byoyomi_margin_ms(margin)))
                    .unwrap(),
            )
            .unwrap()
        };
        let uncapped = budget(0);
        assert_eq!(budget(30), clock_budget(clock));
        for margin in [
            0,
            30,
            remaining + byoyomi - 1,
            remaining + byoyomi,
            remaining + byoyomi + 1,
        ] {
            let actual = budget(margin);
            let cap = Duration::from_millis((remaining + byoyomi).saturating_sub(margin).max(1));
            assert_eq!(actual.hard, uncapped.hard.min(cap));
            assert_eq!(actual.soft, uncapped.soft.min(cap));
            assert!(!actual.deadline);
        }
    }
}

// 秒読み期は反復の開始前と完了後に締切を検査し、先読み中は的中まで時計による判定を行わない。
#[test]
fn byoyomi_period_starts_iterations_before_deadline_in_normal_and_ponder_search() {
    let limits = SearchLimits::new(None, None, None, Some(clock(0, 0, 10_030))).unwrap();
    assert_starts_iterations_before_deadline(limits, true);
}

// 固定時間だけを指定した探索は、通常探索でも先読みでも締切前なら次の反復を始める。
#[test]
fn movetime_starts_iterations_before_deadline_in_normal_and_ponder_search() {
    assert_starts_iterations_before_deadline(movetime_limits(10_000), false);
}

fn assert_starts_iterations_before_deadline(limits: SearchLimits, adopt_partial: bool) {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let root_moves = legal_moves(&position);
    let nnue = test_nnue();
    let budget = time_budget(&limits).unwrap();
    let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
    for (ponder, hit, elapsed_secs) in [
        (false, 0, 9),
        (true, u64::MAX, 109),
        (true, 100_000_000_000, 109),
    ] {
        let stop = AtomicBool::new(false);
        let hit_ns = AtomicU64::new(hit);
        let shared = SharedSearch {
            external_stop: &stop,
            team_stop: AtomicBool::new(false),
            stop_reason: AtomicU8::new(0),
            total_nodes: AtomicU64::new(0),
            best_move_changes: AtomicU64::new(0),
            increase_depth: AtomicBool::new(true),
            node_limit: None,
            clock: &ManualClock::new(Duration::from_secs(elapsed_secs)),
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
            None,
            &shared,
            &small_tt(),
            &mut Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]),
            &mut empty_capture_history(),
            &new_correction_values(),
            &mut histories.continuation_correction[0],
            &histories.continuation,
            &histories.pawn,
            None,
            ponder,
            adopt_partial,
        );
        assert_eq!(outcome.result.depth, 2);
        assert_eq!(shared.reason(), StopReason::DepthCompleted);
        assert!(root_moves.contains(&outcome.result.best_move));
    }
}
