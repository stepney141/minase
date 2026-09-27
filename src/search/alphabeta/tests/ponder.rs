//! 先読みを検査する。

use super::*;

// ponder.md設計判断「反復開始の判定」「的中の時点で進行中の反復」
// （D7-TIME-07、D7-TIME-08）。境界値はマトリクスの試算に従う。
#[test]
fn ponder_iteration_predictions_obey_hit_offset_and_exact_boundaries() {
    let ms = Duration::from_millis;
    let budget = TimeBudget {
        soft: ms(100),
        hard: ms(400),
    };
    for (elapsed, hit, stable, expected) in [
        (1040, 1000, true, false),
        (38, 0, true, true),
        (150, 100, false, true),
        (150, 100, true, false),
        (1099, 1000, false, false),
        (100, 0, false, false),
    ] {
        assert_eq!(
            should_start_next_iteration(ms(elapsed), ms(hit), budget, stable),
            expected
        );
    }
    let exact_soft = TimeBudget {
        soft: ms(263),
        hard: ms(400),
    };
    assert!(should_start_next_iteration(
        ms(100),
        Duration::ZERO,
        exact_soft,
        true
    ));
    assert!(!should_start_next_iteration(
        ms(100) + Duration::from_nanos(1),
        Duration::ZERO,
        exact_soft,
        true
    ));
    // h=100ms、hard=426msなら予測上限はT=200ms。softは境界を隠さない値にする。
    let wide_soft = TimeBudget {
        soft: ms(426),
        hard: ms(426),
    };
    assert!(should_start_next_iteration(
        ms(200),
        ms(100),
        wide_soft,
        false
    ));
    assert!(!should_start_next_iteration(
        ms(200) + Duration::from_nanos(1),
        ms(100),
        wide_soft,
        false
    ));
    for (started, stable, expected) in [
        (500, false, true),
        (600, false, false),
        (400, true, true),
        (500, true, false),
    ] {
        assert_eq!(
            iteration_prediction_fits(ms(started), ms(1000), budget, stable),
            expected
        );
    }
}

// ponder.md設計判断「探索中の打ち切り」（D7-TIME-06）。70msのhardを
// 超えても先読みは続き、深さとノード数の上限は的中前から働く。
#[test]
fn ponder_ignores_time_until_hit_but_obeys_depth_nodes_and_stop() {
    let clock = ClockLimits::new(0, 0, 100, 0).unwrap();
    let handle = start_ponder(SearchLimits::new(None, None, None, Some(clock)).unwrap(), 1);
    assert_ponder_running(&handle, Duration::from_millis(350));
    handle.request_stop();
    let (_, result) = event_reports(drain_raw(&handle));
    assert_eq!(result.stop_reason, StopReason::ExternalStop);
    assert!(legal_moves(&Position::initial()).contains(&result.best_move));
    assert!(result.elapsed >= Duration::from_millis(350));
    handle.join().unwrap();
    for (depth, nodes, reason) in [
        (Some(1), None, StopReason::DepthCompleted),
        (None, Some(5000), StopReason::NodeLimit),
    ] {
        let handle = start_ponder(
            SearchLimits::new(depth, nodes, None, Some(clock)).unwrap(),
            1,
        );
        let (_, result) = event_reports(drain_raw(&handle));
        assert_eq!(result.stop_reason, reason);
        handle.join().unwrap();
    }
}

// ponder.md設計判断「時間予算の起点」「検査の手順」（D7-TIME-07、D7-TIME-08）。
// 実時間の統合経路は負荷による検査遅延を許容し、厳密なsoft/hardの区別は下の検査点で固定する。
#[test]
fn ponder_hit_finishes_within_the_post_hit_budget_tolerance() {
    for threads in [1, 2] {
        let clock = ClockLimits::new(0, 0, 100, 0).unwrap();
        let handle = start_ponder(
            SearchLimits::new(None, None, None, Some(clock)).unwrap(),
            threads,
        );
        assert_ponder_running(&handle, Duration::from_millis(350));
        let hit = Instant::now();
        handle.ponderhit();
        let (_, result) = event_reports(drain_raw(&handle));
        assert!(hit.elapsed() < Duration::from_secs(2));
        assert!(matches!(
            result.stop_reason,
            StopReason::SoftLimit | StopReason::HardLimit
        ));
        assert!(result.elapsed >= Duration::from_millis(350));
        assert!(legal_moves(&Position::initial()).contains(&result.best_move));
        handle.join().unwrap();
    }
}

// ponder.md設計判断「起点と共有状態の所有者」（D7-API-06）。
// 生成前の的中は共有起点を持つハンドルをスレッドなしで構成して厳密に検査する。
#[test]
fn ponderhit_records_only_the_first_notification_even_before_worker_start() {
    let (_sender, events) = mpsc::channel();
    let handle = SearchHandle {
        events,
        started: Instant::now() - Duration::from_millis(300),
        hit_ns: Arc::new(AtomicU64::new(u64::MAX)),
        stop: Arc::new(AtomicBool::new(false)),
        thread: None,
    };
    handle.ponderhit();
    let first = handle.hit_ns.load(AtomicOrdering::Relaxed);
    assert!(first >= 300_000_000 && first != u64::MAX);
    handle.ponderhit();
    assert_eq!(handle.hit_ns.load(AtomicOrdering::Relaxed), first);
    for ponder in [false, true] {
        for _ in 0..20 {
            let handle = crate::search::start_search(
                weights().unwrap(),
                snapshot_for(&Position::initial()),
                depth_limits(1),
                701,
                DEFAULT_THREADS,
                small_tt(),
                ponder,
            );
            handle.ponderhit();
            let first = handle.hit_ns.load(AtomicOrdering::Relaxed);
            handle.ponderhit();
            assert_eq!(handle.hit_ns.load(AtomicOrdering::Relaxed), first);
            if !ponder {
                assert_eq!(first, 0);
            }
            let (_, result) = event_reports(drain_raw(&handle));
            assert_eq!(result.stop_reason, StopReason::DepthCompleted);
            assert!(
                handle
                    .events()
                    .try_iter()
                    .all(|event| !matches!(event, SearchEvent::Finished { .. }))
            );
            handle.join().unwrap();
        }
    }
}

// ponder.md設計判断「的中の時点で進行中の反復」（D7-TIME-08）。
// 既存のノード検査点へ時刻を注入し、負荷に依存せず当て直しの停止、継続、
// hard優先、的中後の開始、1回性を検査する。反復の状態は仕様で明示された境界。
#[test]
fn ponder_rechecks_pre_hit_iterations_once_and_checks_hard_first() {
    let ms = Duration::from_millis;
    let position = Position::initial();
    let pst = weights().unwrap();
    let table = small_tt();
    let stop = AtomicBool::new(false);
    for (t, hit, elapsed, stable, expected) in [
        (600, 1000, 1010, false, Some(StopReason::SoftLimit)),
        (500, 1000, 1010, false, None),
        (500, 1000, 1010, true, Some(StopReason::SoftLimit)),
        (400, 1000, 1010, true, None),
        (600, 1000, 1500, false, Some(StopReason::HardLimit)),
        (1001, 1000, 1010, true, None),
        (600, u64::MAX, 1010, true, None),
    ] {
        let hit_ns = AtomicU64::new(if hit == u64::MAX {
            hit
        } else {
            hit * 1_000_000
        });
        let shared = SharedSearch {
            external_stop: &stop,
            team_stop: AtomicBool::new(false),
            stop_reason: AtomicU8::new(0),
            total_nodes: AtomicU64::new(0),
            node_limit: None,
            started: Instant::now() - ms(elapsed),
            hard_limit: Some(HardLimit {
                duration: ms(400),
                hit_ns: &hit_ns,
            }),
        };
        let mut searcher = new_searcher(&pst, &position, engine_rules(), &[], &shared, &table);
        searcher.ponder_iteration = Some(PonderIteration {
            started: ms(t),
            stable,
            checked: false,
            budget: TimeBudget {
                soft: ms(100),
                hard: ms(400),
            },
        });
        assert_eq!(searcher.enter_node(), expected.is_none());
        assert_eq!(searcher.stop_reason, expected);
        if expected.is_none() && hit != u64::MAX {
            // 一度通った反復は、開始時の判定材料を変えても再検査しない。
            searcher.ponder_iteration.as_mut().unwrap().started = ms(900);
            searcher.nodes = STOP_CHECK_INTERVAL;
            assert!(searcher.enter_node());
        }
    }
}

// ponder.md設計判断「的中の時点で進行中の反復」（D7-TIME-08）。
// aspirationの再探索を発生させても反復の開始時刻と安定性を変えない。
#[test]
fn ponder_aspiration_research_preserves_iteration_start_and_stability() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    with_root_searcher(&position, &[], |searcher| {
        let started = Duration::from_millis(123);
        searcher.ponder_iteration = Some(PonderIteration {
            started,
            stable: true,
            checked: false,
            budget: TimeBudget {
                soft: Duration::from_secs(1),
                hard: Duration::from_secs(4),
            },
        });
        let (_, score) = searcher
            .search_iteration(&position, &legal_moves(&position), 5, Some(20_000))
            .unwrap();
        assert!(score < 20_000 - searcher.pst.pawn_value() / 2);
        let iteration = searcher.ponder_iteration.as_ref().unwrap();
        assert_eq!(iteration.started, started);
        assert!(iteration.stable);
    });
}

// ponder.md設計判断「的中の時点で進行中の反復」（D7-TIME-08）。
// 長い反復の開始直後に的中させ、hardまで走り切らずsoftで止まる配線を固定する。
#[test]
fn ponder_long_iteration_stops_on_hit_without_spending_hard_budget() {
    for threads in [1, 2] {
        let handle = start_ponder(
            SearchLimits::new(None, None, Some(70), None).unwrap(),
            threads,
        );
        let mut completed_depth = 0;
        loop {
            let event = handle
                .events()
                .recv_timeout(Duration::from_secs(20))
                .unwrap();
            match event {
                SearchEvent::Progress { depth, elapsed, .. } => {
                    completed_depth = completed_depth.max(depth);
                    if elapsed >= Duration::from_millis(700) {
                        break;
                    }
                }
                event => panic!("ponder ended before hit: {event:?}"),
            }
        }
        // 次の反復が開始済みになるまで進捗を観測する。新しい完了が来たら
        // その反復から同じ間隔を取り直すため、時計の絶対値は仮定しない。
        loop {
            match handle.events().recv_timeout(Duration::from_millis(15)) {
                Ok(SearchEvent::Progress { depth, .. }) => {
                    completed_depth = completed_depth.max(depth)
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                event => panic!("unexpected ponder event: {event:?}"),
            }
        }
        let hit = Instant::now();
        handle.ponderhit();
        let (_, result) = event_reports(drain_raw(&handle));
        assert_eq!(result.stop_reason, StopReason::SoftLimit);
        assert!(result.depth >= completed_depth);
        assert!(
            hit.elapsed() < Duration::from_millis(70),
            "post-hit elapsed: {:?}",
            hit.elapsed()
        );
        handle.join().unwrap();
    }
}

// ponder.md設計判断「的中の時点で進行中の反復」（D7-TIME-08、D7-LIM-04）。
// 深さ1を完了できない制限でも、的中した探索は根の先頭の合法手を返す。
#[test]
fn ponder_hit_before_first_completed_depth_returns_the_root_fallback() {
    let first = snapshot_for(&Position::initial()).root_moves[0];
    for threads in [1, 2] {
        let handle = start_ponder(nodes_limits(1), threads);
        handle.ponderhit();
        let (_, result) = event_reports(drain_raw(&handle));
        assert_eq!(result.depth, 0);
        assert_eq!(result.best_move, first);
        assert_eq!(result.nodes, 1);
        handle.join().unwrap();
    }
}

// ponder.md設計判断「的中の時点で進行中の反復」（D7-TIME-08、D7-SMP-08）。
// 補助ワーカーを完了させた後で主ワーカーの検査点に的中を通知し、実際の
// チーム回収と採用処理が、当て直しで中断した反復を選ばないことを固定する。
#[test]
fn ponder_team_adopts_completed_auxiliary_result_after_main_recheck() {
    let position = Position::initial();
    let roots = legal_moves(&position);
    let pst = weights().unwrap();
    let table = small_tt();
    let stop = AtomicBool::new(false);
    let hit_ns = AtomicU64::new(u64::MAX);
    let shared = SharedSearch {
        external_stop: &stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        node_limit: None,
        started: Instant::now() - Duration::from_secs(1),
        hard_limit: Some(HardLimit {
            duration: Duration::from_millis(400),
            hit_ns: &hit_ns,
        }),
    };
    let completed = std::sync::Barrier::new(2);
    let outcomes = run_worker_team(worker_count(2), &shared, |worker_index| {
        if worker_index == 1 {
            let outcome = run_auxiliary_worker(
                &pst,
                &position,
                engine_rules(),
                &roots,
                &[],
                2,
                1,
                &shared,
                &table,
            );
            completed.wait();
            return outcome;
        }
        let mut searcher = new_searcher(&pst, &position, engine_rules(), &[], &shared, &table);
        searcher.ponder_iteration = Some(PonderIteration {
            started: Duration::from_millis(600),
            stable: false,
            checked: false,
            budget: TimeBudget {
                soft: Duration::from_millis(100),
                hard: Duration::from_millis(400),
            },
        });
        completed.wait();
        hit_ns.store(1_000_000_000, AtomicOrdering::Relaxed);
        assert!(
            searcher
                .search_iteration(&position, &roots, 1, None)
                .is_none()
        );
        assert_eq!(searcher.stop_reason, Some(StopReason::SoftLimit));
        WorkerOutcome {
            worker_index: 0,
            result: SearchResult {
                best_move: roots[0],
                score: 0,
                depth: 0,
                nodes: 0,
            },
            pv: vec![roots[0]],
            nodes: searcher.nodes,
        }
    });
    let adopted = select_worker_outcome(&outcomes);
    assert_eq!(adopted.worker_index, 1);
    assert_eq!(adopted.result.depth, 2);
    assert!(roots.contains(&adopted.result.best_move));
    assert_eq!(shared.reason(), StopReason::SoftLimit);
}

// ponder.md設計判断「的中の時点で進行中の反復」（D7-TIME-08、D7-API-06）。
// スレッド生成前の的中も通常の開始判定を通し、完了済みの反復がなければ根の先頭を返す。
#[test]
fn ponder_hit_before_worker_start_obeys_the_iteration_start_budget() {
    let snapshot = snapshot_for(&Position::initial());
    let table = small_tt();
    let stop = AtomicBool::new(false);
    let hit_ns = AtomicU64::new(0);
    let outcome = run_search_team(
        &weights().unwrap(),
        &snapshot.position,
        snapshot.rules,
        &snapshot.root_moves,
        &snapshot.history_keys,
        &SearchLimits::new(None, None, Some(70), None).unwrap(),
        &stop,
        DEFAULT_THREADS,
        &table,
        None,
        Instant::now() - Duration::from_secs(1),
        &hit_ns,
        true,
    );
    assert_eq!(outcome.stop_reason, StopReason::SoftLimit);
    assert_eq!(outcome.result.depth, 0);
    assert_eq!(outcome.result.nodes, 0);
    assert_eq!(outcome.result.best_move, snapshot.root_moves[0]);
}
