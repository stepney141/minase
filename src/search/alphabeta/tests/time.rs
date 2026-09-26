//! 時間管理を検査する。

use super::*;

// ---------------------------------------------------------------------------
// D7-TIME　時間予算
// ---------------------------------------------------------------------------

// D7-TIME-01。search.md「時間管理」節の予算式。数値例は整数演算による
// 切り捨てを含めて同節から導出する。
#[test]
fn clock_budget_matches_the_normative_formula() {
    // (a) 残り60000・加算1000・秒読み0・ply=0:
    //     moves_to_go=max(88, (432-0)/2)=216、soft_raw=60000/216+760=1037、
    //     safe_hard=59970、hard=min(4676, 16200, 59970)=4676、soft=1037。
    let budget = clock_budget(clock_at_ply(60_000, 1_000, 0, 0));
    assert_eq!(budget.soft, Duration::from_millis(1_037));
    assert_eq!(budget.hard, Duration::from_millis(4_676));

    // (b) 残り10000・加算0・秒読み200・ply=0:
    //     moves_to_go=216、序盤の係数w=4/40、soft_raw=10000/216+200*8*4/400=46+16=62、
    //     safe_hard=10170、hard=min(279, 2860, 10170)=279、soft=62。
    let budget = clock_budget(clock_at_ply(10_000, 0, 200, 0));
    assert_eq!(budget.soft, Duration::from_millis(62));
    assert_eq!(budget.hard, Duration::from_millis(279));

    // (b') 同じ時計でply=36以降は係数が1になり、moves_to_go=198、
    //     soft_raw=10000/198+160=210、hard=min(947, 2860, 10170)=947。
    let budget = clock_budget(clock_at_ply(10_000, 0, 200, 36));
    assert_eq!(budget.soft, Duration::from_millis(210));
    assert_eq!(budget.hard, Duration::from_millis(947));

    // (c) 旧式でhard<softになった入力。残り200・加算100・秒読み0・ply=300:
    //     moves_to_go=88、soft_raw=2+76=78、safe_hard=170、
    //     hard=min(351, 54, 170)=54、soft=min(78, 54)=54。
    let budget = clock_budget(clock_at_ply(200, 100, 0, 300));
    assert_eq!(budget.soft, Duration::from_millis(54));
    assert_eq!(budget.hard, Duration::from_millis(54));

    // (d) 時計合計30ms以下ではsafe_hard=1となり、softもhard以下へ縮む。
    let budget = clock_budget(clock_at_ply(20, 0, 0, 0));
    assert_eq!(budget.hard, Duration::from_millis(1));
    assert!(budget.soft <= budget.hard);

    // 主時間0・秒読み100ではsoft_raw=80、safe_hard=70、hard=soft=70。
    // 残り時間0の手には序盤の係数を掛けない。
    let budget = clock_budget(clock_at_ply(0, 0, 100, 0));
    assert_eq!(budget.soft, Duration::from_millis(70));
    assert_eq!(budget.hard, Duration::from_millis(70));
}

// time-management-efficiency.mdの「予算値」。秒読みの項にだけ序盤の係数を掛け、
// 秒読みのない時計では式が現行と一致する。
#[test]
fn opening_coefficient_scales_only_the_byoyomi_term() {
    for (remaining, increment, byoyomi, ply, soft, hard) in [
        (300_000, 0, 10_000, 0, 2_188, 9_867),
        (300_000, 0, 10_000, 36, 9_515, 42_912),
        (1_800_000, 0, 40_000, 0, 11_533, 52_013),
        (10_000, 100, 0, 0, 122, 550),
        (60_000, 200, 0, 0, 429, 1_934),
        (0, 0, 10_000, 0, 8_000, 8_000),
        (0, 100, 10_000, 0, 8_000, 8_000),
    ] {
        let budget = clock_budget(clock_at_ply(remaining, increment, byoyomi, ply));
        assert_eq!(
            budget.soft,
            Duration::from_millis(soft),
            "soft clock=({remaining}, {increment}, {byoyomi}, {ply})"
        );
        assert_eq!(
            budget.hard,
            Duration::from_millis(hard),
            "hard clock=({remaining}, {increment}, {byoyomi}, {ply})"
        );
    }
}

// D7-TIME-01。search.md「時間管理」節: 残り手数の見積りは手数について
// 単調非増加で、plyがEXPECTED_PLIES以上ならMIN_MOVESに固定される。
#[test]
fn moves_to_go_decreases_monotonically_to_the_documented_floor() {
    let plys = [0, 1, 100, 254, 255, 256, 431, 432, 1_000, u32::MAX];
    let estimates: Vec<u128> = plys.into_iter().map(moves_to_go).collect();

    assert!(estimates.windows(2).all(|pair| pair[0] >= pair[1]));
    assert_eq!(moves_to_go(432), 88);
    assert_eq!(moves_to_go(1_000), 88);
}

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
    // 時計単独ならsoft=1037、hard=4676（D7-TIME-01(a)）。
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

// D7-TIME-05。search.md「時間管理」節: elapsed < softかつ
// elapsed×2.63 <= hardの場合だけ次の反復を開始する。
#[test]
fn next_iteration_requires_both_time_conditions() {
    let budget = |soft, hard| TimeBudget {
        soft: Duration::from_millis(soft),
        hard: Duration::from_millis(hard),
    };

    // soft境界は未満だけを継続する。
    assert!(!should_start_next_iteration(
        Duration::from_millis(100),
        Duration::ZERO,
        budget(100, 263),
        false
    ));
    assert!(should_start_next_iteration(
        Duration::from_millis(99),
        Duration::ZERO,
        budget(100, 261),
        false
    ));

    // 予測完了時刻のhard境界は等号を含む。
    assert!(should_start_next_iteration(
        Duration::from_millis(100),
        Duration::ZERO,
        budget(101, 263),
        false
    ));
    assert!(!should_start_next_iteration(
        Duration::from_millis(100),
        Duration::ZERO,
        budget(101, 262),
        false
    ));

    // movetime相当のsoft=hardでは、経過時間がhardの100/263以下なら継続できる。
    assert!(should_start_next_iteration(
        Duration::from_millis(38),
        Duration::ZERO,
        budget(100, 100),
        false
    ));
    assert!(!should_start_next_iteration(
        Duration::from_millis(39),
        Duration::ZERO,
        budget(100, 100),
        false
    ));
}

// docs/plans/strength-stage6.md「最善手安定時の早期終了」「検証」。
// 安定時は予測完了時刻がsoft以下のときだけ続け、hardの条件は緩めない。
#[test]
fn next_iteration_stable_requires_the_prediction_within_soft() {
    let budget = |soft, hard| TimeBudget {
        soft: Duration::from_millis(soft),
        hard: Duration::from_millis(hard),
    };
    // soft 100ms、hard 400ms: 通常は99msまで続け、安定時は38msまでしか続けない。
    assert!(should_start_next_iteration(
        Duration::from_millis(99),
        Duration::ZERO,
        budget(100, 400),
        false
    ));
    assert!(should_start_next_iteration(
        Duration::from_millis(38),
        Duration::ZERO,
        budget(100, 400),
        true
    ));
    assert!(!should_start_next_iteration(
        Duration::from_millis(39),
        Duration::ZERO,
        budget(100, 400),
        true
    ));
    assert!(!should_start_next_iteration(
        Duration::from_millis(99),
        Duration::ZERO,
        budget(100, 400),
        true
    ));
    // hardの予測が先に拘束する場合は安定の有無で変わらない。
    assert!(should_start_next_iteration(
        Duration::from_millis(38),
        Duration::ZERO,
        budget(400, 100),
        true
    ));
    assert!(!should_start_next_iteration(
        Duration::from_millis(39),
        Duration::ZERO,
        budget(400, 100),
        true
    ));
    assert!(!should_start_next_iteration(
        Duration::from_millis(39),
        Duration::ZERO,
        budget(400, 100),
        false
    ));
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

    // (4) 時計の安全上限が効いてsoft=hard=70msとなる設定（D7-TIME-01(d)）。
    let limits = SearchLimits::new(None, None, None, Some(clock(0, 0, 100))).unwrap();
    let budget = time_budget(&limits).expect("byoyomi must produce a time budget");
    assert_eq!(budget.soft, Duration::from_millis(70));
    assert_eq!(budget.hard, Duration::from_millis(70));
}
