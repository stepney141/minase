//! 探索の開始と停止を検査する。

use super::*;

// Threads=1で深さまたはノード数だけを制限した探索は、同じ入力と初期状態で結果が再現する。
// 経過時間の値を除く全観測を比較する。
#[test]
fn search_with_node_limit_is_deterministic() {
    #[allow(clippy::type_complexity)]
    fn run(
        position: &Position,
        limits: SearchLimits,
        id: u64,
    ) -> (
        Vec<(u32, i32, u64, Vec<Move>)>,
        (Move, i32, u32, u64, Vec<Move>, StopReason),
    ) {
        let handle = start(
            snapshot_for(position),
            limits,
            id,
            DEFAULT_THREADS,
            small_tt(),
        );
        let (progress, finished) = event_reports(drain_raw(&handle));
        handle.join().expect("search thread must not panic");
        (
            progress
                .into_iter()
                .map(|(depth, score, nodes, _, pv)| (depth, score, nodes, pv))
                .collect(),
            (
                finished.best_move,
                finished.score,
                finished.depth,
                finished.nodes,
                finished.pv,
                finished.stop_reason,
            ),
        )
    }

    // 中盤フィクスチャで探索結果と全通知の再現性を確認する。
    let midgame = quiet_midgame();
    let first = run(&midgame, nodes_limits(30_000), 73);
    let second = run(&midgame, nodes_limits(30_000), 74);
    assert_eq!(first, second);
}

// 監査「学習NNUE追加後の公開境界」: 探索APIはグローバルな埋め込み重みでは
// なく、呼び出し側が明示した検証済みNNUEで葉を評価する。
#[test]
fn search_apis_use_the_supplied_nnue() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    // 固定ネットの出力バイアスだけを変え、注入した重みを区別する。
    use sha2::{Digest, Sha256};
    let mut bytes = epoch_zero_nnue_bytes();
    let baseline = Nnue::decode(&bytes).unwrap();
    let end = bytes.len();
    bytes[end - 4..].copy_from_slice(&520_192_i32.to_le_bytes());
    let checksum = Sha256::digest(&bytes[192..]);
    bytes[144..176].copy_from_slice(&checksum);
    let supplied = Nnue::decode(&bytes).unwrap();
    let mut child = position.clone();
    let root_move = legal_moves(&position)
        .into_iter()
        .find(|&mv| {
            let undo = child.make_move_unchecked(mv, engine_rules());
            let differs = evaluate(&supplied, &child) != evaluate(&baseline, &child);
            child.unmake_move(undo);
            differs
        })
        .expect("fixture must distinguish the supplied and fixed networks");
    let undo = child.make_move_unchecked(root_move, engine_rules());
    let expected = -evaluate(&supplied, &child);
    child.unmake_move(undo);
    let snapshot = SearchSnapshot::from_parts(
        position,
        engine_rules(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        Vec::new(),
        vec![root_move],
    )
    .unwrap();

    let synchronous = {
        let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
        zero_correction_histories(&mut histories);
        crate::search::search(
            &supplied,
            &snapshot,
            &depth_limits(1),
            DEFAULT_THREADS,
            &mut small_tt(),
            &mut histories,
        )
        .unwrap()
    };
    assert_eq!(synchronous.score, expected);

    let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
    zero_correction_histories(&mut histories);
    let handle = crate::search::start_search(
        Arc::new(supplied),
        snapshot,
        depth_limits(1),
        7,
        DEFAULT_THREADS,
        small_tt(),
        histories,
        false,
    );
    let (_, asynchronous) = event_reports(drain_raw(&handle));
    handle.join().unwrap();

    assert_eq!(asynchronous.score, expected);
}

// ノード予約の合計が上限に達するとNodeLimitで停止し、根の合法手を返す。
// ここでは上限への到達と合法手の返却を検査し、超過しないことはチームのテストで検査する。
#[test]
fn node_limit_stops_the_search_with_a_legal_best_move() {
    let initial = Position::initial();
    let snapshot = snapshot_for(&initial);
    let root_moves = snapshot.root_moves.clone();

    let handle = start(
        snapshot,
        nodes_limits(5_000),
        31,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (_, finished) = event_reports(drain_raw(&handle));
    handle.join().expect("search thread must not panic");

    assert_eq!(finished.stop_reason, StopReason::NodeLimit);
    assert!(finished.nodes >= 5_000);
    assert!(root_moves.contains(&finished.best_move));

    // 境界: nodes=1の極小値でも合法な最善手が返る。
    let moves = legal_moves(&initial);
    let result = run_search(
        &initial,
        engine_rules(),
        &moves,
        &[],
        &nodes_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    );
    assert!(moves.contains(&result.best_move));
}

// 即時停止でも合法手を返し、Finishedを1回だけ送り、共有表を回収する。
#[test]
fn stop_before_depth_one_still_yields_a_legal_best_move() {
    let initial = Position::initial();
    for threads in [worker_count(1), worker_count(4)] {
        let snapshot = snapshot_for(&initial);
        let root_moves = snapshot.root_moves.clone();
        let handle = start(snapshot, infinite_limits(), 41, threads, small_tt());
        handle.request_stop();
        let events = drain_raw(&handle);
        assert!(
            events.iter().all(|event| event.search_id() == 41),
            "threads={threads}"
        );
        let (_, finished) = event_reports(events);
        assert_eq!(
            finished.stop_reason,
            StopReason::ExternalStop,
            "threads={threads}"
        );
        assert!(
            root_moves.contains(&finished.best_move),
            "threads={threads}"
        );
        assert!(
            matches!(
                handle.events().recv_timeout(Duration::from_secs(1)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ),
            "threads={threads}"
        );
        let (table, _) = handle.join().expect("search team must not panic");
        assert_eq!(table.generation(), 1, "threads={threads}");
    }
}

// 無期限ではsoft/hardの両リミットを
// 無効化し、外部停止要求だけで停止する。予算生成、Progress、外部停止、
// joinを検査する。
#[test]
fn infinite_limits_stop_only_on_external_request() {
    let limits = infinite_limits();
    assert!(time_budget(&limits).is_none());
    let initial = Position::initial();
    let snapshot = snapshot_for(&initial);
    let root_moves = snapshot.root_moves.clone();
    let handle = start(snapshot, limits, 51, DEFAULT_THREADS, small_tt());
    assert!(matches!(
        handle.events().recv_timeout(Duration::from_secs(60)),
        Ok(SearchEvent::Progress { search_id: 51, .. })
    ));
    handle.request_stop();
    let (_, finished) = event_reports(drain_raw(&handle));
    handle.join().expect("search thread must not panic");
    assert_eq!(finished.stop_reason, StopReason::ExternalStop);
    assert!(root_moves.contains(&finished.best_move));
}

// 主ワーカーは深さ1から1ずつ深め、窓内で完了した反復ごとにProgressを1回送る。
// ノード数と経過時間は累積値なので減少しない。
#[test]
fn progress_depths_start_at_one_and_increase_by_one() {
    let midgame = quiet_midgame();
    let mut snapshot = snapshot_for(&midgame);
    snapshot.root_moves.truncate(1);
    let handle = start(snapshot, depth_limits(6), 81, DEFAULT_THREADS, small_tt());
    let events = drain_raw(&handle);
    assert!(events.iter().all(|event| event.search_id() == 81));
    let (progress, finished) = event_reports(events);
    handle.join().expect("search thread must not panic");

    let depths: Vec<u32> = progress.iter().map(|entry| entry.0).collect();
    assert_eq!(depths, vec![1, 2, 3, 4, 5, 6]);
    for (_, _, _, _, pv) in &progress {
        assert!(!pv.is_empty());
    }
    let nodes: Vec<u64> = progress.iter().map(|entry| entry.2).collect();
    assert!(nodes.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(finished.depth, 6);
    assert_eq!(finished.stop_reason, StopReason::DepthCompleted);
    assert_eq!(finished.pv.first(), Some(&finished.best_move));
    assert_pv_is_legal(&midgame, &finished.pv);
    let mut elapsed: Vec<_> = progress.iter().map(|entry| entry.3).collect();
    elapsed.push(finished.elapsed);
    assert!(elapsed.windows(2).all(|pair| pair[0] <= pair[1]));

    // 境界: depth=1では深さ列は[1]のみ。
    let handle = start(
        snapshot_for(&midgame),
        depth_limits(1),
        82,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (progress, _) = event_reports(drain_raw(&handle));
    handle.join().expect("search thread must not panic");
    let depths: Vec<u32> = progress.iter().map(|entry| entry.0).collect();
    assert_eq!(depths, vec![1]);
}

// joinは全ワーカーの終了後に置換表を返し、次の探索で再利用できる。
#[test]
fn join_returns_the_transposition_table_for_reuse() {
    let midgame = quiet_midgame();
    let snapshot = snapshot_for(&midgame);

    let handle = start(
        snapshot.clone(),
        depth_limits(3),
        91,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (_, first) = event_reports(drain_raw(&handle));
    let (mut returned_tt, _) = handle.join().expect("search thread must not panic");

    // 根の探索ごとに世代を1回進め、前回の同じ深さの記録も異なるキーで置き換えられるようにする。
    assert_eq!(returned_tt.generation(), 1);

    // 返却された置換表を渡した探索も指定深さまで完了し、合法手を返す。
    let second = run_search(
        &snapshot.position,
        snapshot.rules(),
        &snapshot.root_moves,
        &snapshot.history_keys,
        &depth_limits(3),
        DEFAULT_THREADS,
        &mut returned_tt,
    );
    assert_eq!(first.depth, 3);
    assert_eq!(first.stop_reason, StopReason::DepthCompleted);
    assert_eq!(second.depth, 3);
    assert!(snapshot.root_moves.contains(&first.best_move));
    assert!(snapshot.root_moves.contains(&second.best_move));
    assert_eq!(returned_tt.generation(), 2);
}

// 監査「SearchHandle破棄後の探索スレッド」: 所有者が明示的にjoinしなくても、
// Dropが停止フラグを立て、探索スレッドの終了を待ってから戻る。
#[test]
fn dropping_search_handle_requests_stop_and_joins_the_thread() {
    let (sender, events) = mpsc::channel();
    drop(sender);
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let worker_finished = Arc::new(AtomicBool::new(false));
    let finished = Arc::clone(&worker_finished);
    let thread = thread::spawn(move || {
        while !worker_stop.load(AtomicOrdering::Acquire) {
            thread::yield_now();
        }
        finished.store(true, AtomicOrdering::Release);
        (
            small_tt(),
            crate::search::HistoryTables::new(DEFAULT_THREADS),
        )
    });
    let handle = SearchHandle {
        clock: Arc::new(ManualClock::new(Duration::ZERO)),
        hit_ns: Arc::new(AtomicU64::new(0)),
        events,
        stop,
        thread: Some(thread),
    };

    drop(handle);

    assert!(worker_finished.load(AtomicOrdering::Acquire));
}

// SearchHandleは調整役のパニック結果をjoinへ返し、Finishedを捏造しない。
#[test]
fn search_handle_join_returns_the_coordinator_panic() {
    let (sender, events) = mpsc::channel();
    drop(sender);
    let handle = SearchHandle {
        clock: Arc::new(ManualClock::new(Duration::ZERO)),
        hit_ns: Arc::new(AtomicU64::new(0)),
        events,
        stop: Arc::new(AtomicBool::new(false)),
        thread: Some(thread::spawn(
            || -> (TranspositionTable, crate::search::HistoryTables) {
                panic!("injected coordinator panic")
            },
        )),
    };

    assert!(handle.join().is_err());
}
