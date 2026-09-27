//! 探索チームを検査する。

use super::*;

// 監査「探索ワーカーのパニック伝播」: 調整役はパニックしたワーカーから
// チーム停止を通知し、残るワーカーをすべて回収してからパニックを伝播する。
#[test]
fn panicking_worker_stops_and_joins_the_remaining_team_before_propagation() {
    let fallback = legal_moves(&Position::initial())[0];

    for panicking_worker in [0, 1] {
        let external_stop = AtomicBool::new(false);
        let shared = SharedSearch {
            external_stop: &external_stop,
            team_stop: AtomicBool::new(false),
            stop_reason: AtomicU8::new(0),
            total_nodes: AtomicU64::new(0),
            node_limit: None,
            started: Instant::now(),
            hard_limit: None,
        };
        let completed = AtomicU64::new(0);
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            run_worker_team(worker_count(3), &shared, |worker_index| {
                if worker_index == panicking_worker {
                    panic!("injected worker {worker_index} panic");
                }
                while !shared.team_stop.load(AtomicOrdering::Acquire) {
                    thread::yield_now();
                }
                completed.fetch_add(1, AtomicOrdering::Release);
                WorkerOutcome {
                    worker_index,
                    result: SearchResult {
                        best_move: fallback,
                        score: 0,
                        depth: 0,
                        nodes: 0,
                    },
                    pv: vec![fallback],
                    nodes: 0,
                }
            })
        }));

        assert!(outcome.is_err());
        assert!(shared.team_stop.load(AtomicOrdering::Acquire));
        assert_eq!(completed.load(AtomicOrdering::Acquire), 2);
    }
}

// ---------------------------------------------------------------------------
// D7-SMP　探索チーム
// ---------------------------------------------------------------------------

// D7-SMP-01・07。時間と外部停止でFinishedを1回だけ送り、全join後に
// 世代が1回だけ進んだ置換表を返す。深さとノードの停止は専用テストで検査する。
#[test]
fn multi_worker_teams_finish_once_and_return_the_shared_table() {
    let initial = Position::initial();
    let root_moves = legal_moves(&initial);
    for (limits, request_stop, search_id) in [
        (movetime_limits(10), false, 300),
        (infinite_limits(), true, 301),
    ] {
        let handle = start(
            snapshot_for(&initial),
            limits,
            search_id,
            worker_count(4),
            small_tt(),
        );
        if request_stop {
            handle.request_stop();
        }
        let (_, finished) = event_reports(drain_raw(&handle));
        assert!(root_moves.contains(&finished.best_move));
        if request_stop {
            assert_eq!(finished.stop_reason, StopReason::ExternalStop);
        } else {
            assert!(matches!(
                finished.stop_reason,
                StopReason::SoftLimit | StopReason::HardLimit
            ));
        }
        assert!(matches!(
            handle.events().recv_timeout(Duration::from_secs(1)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
        let table = handle.join().expect("search team must not panic");
        assert_eq!(table.generation(), 1);
    }
}

// D7-SMP-02。lazy-smp.md「停止と探索予算」: ノード予約は探索チーム全体の
// AtomicU64に対して行い、ProgressとFinishedのノード数をN以下に保つ。
#[test]
fn four_worker_node_limit_never_exceeds_the_team_budget() {
    let initial = Position::initial();
    let root_moves = legal_moves(&initial);
    let limit = 500;
    let handle = start(
        snapshot_for(&initial),
        nodes_limits(limit),
        320,
        worker_count(4),
        small_tt(),
    );
    let (progress, finished) = event_reports(drain_raw(&handle));
    assert!(matches!(
        handle.events().recv_timeout(Duration::from_secs(1)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    ));
    let table = handle.join().expect("search team must not panic");
    assert_eq!(table.generation(), 1);

    assert_eq!(finished.stop_reason, StopReason::NodeLimit);
    assert!(progress.iter().all(|entry| entry.2 <= limit));
    assert!(finished.nodes <= limit);
    if let Some(last) = progress.last() {
        assert!(last.2 <= finished.nodes);
    }
    assert!(root_moves.contains(&finished.best_move));
}

// D7-SMP-04。lazy-smp.md「再現性」: Threads=1の固定ノード探索は、経過
// 時間を除く結果、PV、Progress列、ノード数、停止理由が完全に一致する。
// D7-SRCH-07のsearch_with_node_limit_is_deterministicが同じ観測を初期局面と
// 中盤局面の双方で固定する。

// D7-SMP-05。lazy-smp.md「停止と探索予算」: ExternalStopはNodeLimitより
// 優先する。開始時点で両方が成立し得る構成を直接チーム経路へ与える。
#[test]
fn external_stop_takes_priority_over_the_node_limit() {
    let initial = Position::initial();
    let snapshot = snapshot_for(&initial);
    let limits = nodes_limits(1);
    let external_stop = AtomicBool::new(true);
    let table = small_tt();
    let outcome = run_search_team(
        &crate::eval::weights().unwrap(),
        &snapshot.position,
        snapshot.rules,
        &snapshot.root_moves,
        &snapshot.history_keys,
        &limits,
        &external_stop,
        worker_count(4),
        &table,
        None,
        Instant::now(),
        &AtomicU64::new(0),
        false,
    );

    assert_eq!(outcome.stop_reason, StopReason::ExternalStop);
    assert!(outcome.result.nodes <= 1);
}

// D7-SMP-06。lazy-smp.md「探索チーム」第2版: 補助ワーカーkは周期
// 2 + ((k - 1) % 4)に従う深さと深さ上限を昇順に探索する。
#[test]
fn auxiliary_depth_sequences_follow_the_worker_period_and_include_the_limit() {
    let cases: &[(usize, u32, &[u32])] = &[
        (1, 1, &[1]),
        (1, 2, &[1, 2]),
        (1, 6, &[1, 3, 5, 6]),
        (1, 7, &[1, 3, 5, 7]),
        (2, 7, &[1, 4, 7]),
        (3, 7, &[1, 5, 7]),
        (4, 7, &[1, 6, 7]),
        (5, 6, &[1, 3, 5, 6]),
    ];
    for &(worker_index, depth_limit, expected) in cases {
        assert_eq!(
            auxiliary_depths(worker_index, depth_limit).collect::<Vec<_>>(),
            expected,
            "worker={worker_index}, limit={depth_limit}"
        );
    }
}

// D7-SMP-08。lazy-smp.md「探索チーム」第2版: 最大完了深さを採用し、
// 同じ深さなら番号最小を選ぶ。深さ0は除き、全員0なら主の既定値を返す。
#[test]
fn worker_outcome_selection_uses_depth_then_worker_index_and_excludes_zero() {
    let moves = legal_moves(&Position::initial());
    let outcome = |worker_index: usize, depth: u32, move_index: usize| WorkerOutcome {
        worker_index,
        result: SearchResult {
            best_move: moves[move_index],
            score: worker_index as i32 * 10,
            depth,
            nodes: 0,
        },
        pv: vec![moves[move_index]],
        nodes: worker_index as u64,
    };

    let different_depths = vec![outcome(0, 2, 0), outcome(1, 4, 1), outcome(2, 3, 2)];
    assert_eq!(
        select_worker_outcome(&different_depths),
        &different_depths[1]
    );

    let tied_depths = vec![outcome(2, 4, 2), outcome(0, 3, 0), outcome(1, 4, 1)];
    assert_eq!(select_worker_outcome(&tied_depths), &tied_depths[2]);

    let main_tied = vec![outcome(1, 4, 1), outcome(0, 4, 0), outcome(2, 3, 2)];
    assert_eq!(select_worker_outcome(&main_tied), &main_tied[1]);

    let with_zero = vec![outcome(0, 1, 0), outcome(1, 0, 1), outcome(2, 0, 2)];
    assert_eq!(select_worker_outcome(&with_zero), &with_zero[0]);

    let all_zero = vec![outcome(2, 0, 2), outcome(0, 0, 0), outcome(1, 0, 1)];
    assert_eq!(select_worker_outcome(&all_zero), &all_zero[1]);
}

// D7-SMP-09・10。lazy-smp.md「探索チーム」第2版: いずれかのワーカーが
// 深さ上限を完了するとチームを停止し、最深結果をFinishedへ載せる。
#[test]
fn four_worker_fixed_depth_finishes_at_the_limit_with_a_legal_move() {
    let initial = Position::initial();
    let snapshot = snapshot_for(&initial);
    let root_moves = snapshot.root_moves.clone();
    let depth_limit = 4;
    let handle = start(
        snapshot,
        depth_limits(depth_limit),
        322,
        worker_count(4),
        small_tt(),
    );
    let (progress, finished) = event_reports(drain_raw(&handle));
    assert!(matches!(
        handle.events().recv_timeout(Duration::from_secs(1)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    ));
    let table = handle.join().expect("search team must not panic");
    assert_eq!(table.generation(), 1);

    let depths: Vec<_> = progress.iter().map(|entry| entry.0).collect();
    assert_eq!(depths, (1..=depths.len() as u32).collect::<Vec<_>>());
    assert!(
        progress
            .windows(2)
            .all(|pair| pair[0].2 <= pair[1].2 && pair[0].3 <= pair[1].3)
    );
    for (_, _, _, _, pv) in &progress {
        assert!(!pv.is_empty());
        assert_pv_is_legal(&initial, pv);
    }
    if let Some(last) = progress.last() {
        assert!(last.2 <= finished.nodes);
        assert!(last.3 <= finished.elapsed);
    }
    assert!(finished.nodes > 0);
    assert_eq!(finished.pv.first(), Some(&finished.best_move));
    assert_pv_is_legal(&initial, &finished.pv);
    assert_eq!(finished.depth, depth_limit);
    let last_progress_depth = progress.last().map_or(0, |entry| entry.0);
    assert!(finished.depth >= last_progress_depth);
    assert_eq!(finished.stop_reason, StopReason::DepthCompleted);
    assert!(root_moves.contains(&finished.best_move));
}
