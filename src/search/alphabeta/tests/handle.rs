//! 探索の開始と停止を検査する。

use super::*;

// D7-SRCH-07。search.md「bench」節・「検証」節、docs/guides/sprt.mdの完全再現契約:
// depthまたはnodes制限だけの探索は同一入力に対し完全に決定的である
// （INV-2）。経過時間の値を除く全観測を比較する。
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

    // 初期局面（RULES.md第5条）と中盤フィクスチャの双方で確認する。
    let initial = Position::initial();
    let first = run(&initial, nodes_limits(100_000), 71);
    let second = run(&initial, nodes_limits(100_000), 72);
    assert_eq!(first, second);

    let midgame = quiet_midgame();
    let first = run(&midgame, nodes_limits(30_000), 73);
    let second = run(&midgame, nodes_limits(30_000), 74);
    assert_eq!(first, second);
}

// 監査「学習PST追加後の公開境界」: 探索APIはグローバルな埋め込み重みでは
// なく、呼び出し側が明示した検証済みPSTで葉を評価する。
#[test]
fn search_apis_use_the_supplied_pst() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let supplied = Pst::decode(include_bytes!("../../../../nets/pst-init.bin")).unwrap();
    let embedded = crate::eval::weights().unwrap();
    let mut child = position.clone();
    let root_move = legal_moves(&position)
        .into_iter()
        .find(|&mv| {
            let undo = child.make_move_unchecked(mv, engine_rules());
            let differs = evaluate(&supplied, &child) != evaluate(&embedded, &child);
            child.unmake_move(undo);
            differs
        })
        .expect("fixture must distinguish the supplied and embedded PSTs");
    let undo = child.make_move_unchecked(root_move, engine_rules());
    let expected = -evaluate(&supplied, &child);
    child.unmake_move(undo);
    let snapshot =
        SearchSnapshot::from_parts(position, engine_rules(), Vec::new(), vec![root_move]).unwrap();

    let synchronous = crate::search::search(
        &supplied,
        &snapshot,
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    )
    .unwrap();
    assert_eq!(synchronous.score, expected);

    let handle = crate::search::start_search(
        Arc::new(supplied),
        snapshot,
        depth_limits(1),
        7,
        DEFAULT_THREADS,
        small_tt(),
        false,
    );
    let (_, asynchronous) = event_reports(drain_raw(&handle));
    handle.join().unwrap();

    assert_eq!(asynchronous.score, expected);
}

// D7-LIM-03。search.md「時間管理」節（最も早く満たされた条件で停止）と
// 「スレッド構成」節（停止理由の語彙にノード上限）。超過幅の上限は規範に
// 明文がなく（SPEC_UNCLEAR-05）、ノード数は下限だけをassertする。
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
    assert!(root_moves.contains(&finished.best_move)); // INV-1

    // 境界: nodes=1の極小値でも合法な最善手が返る（D7-LIM-04と同根）。
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

// D7-LIM-04。search.md「時間管理」節・「スレッド構成」節: 探索開始前に
// ルート合法手から1手を確保し、深さ1の完了前に停止しても常に合法な
// `bestmove`を返す。「確保した1手が先頭」は規範節にないため、集合帰属
// だけを検証する。
#[test]
fn stop_or_tiny_budget_before_depth_one_still_yields_a_legal_best_move() {
    let initial = Position::initial();

    // 最速ケース: 開始直後に停止フラグを立てる。無期限指定のため停止理由は
    // 外部停止要求しかあり得ない。
    let snapshot = snapshot_for(&initial);
    let root_moves = snapshot.root_moves.clone();
    let handle = start(snapshot, infinite_limits(), 41, DEFAULT_THREADS, small_tt());
    handle.request_stop();
    let events = drain_raw(&handle);
    assert!(events.iter().all(|event| event.search_id() == 41));
    let (_, finished) = event_reports(events);
    handle.join().expect("search thread must not panic");
    assert_eq!(finished.stop_reason, StopReason::ExternalStop);
    assert!(root_moves.contains(&finished.best_move)); // INV-1

    // 境界: 極小のhard予算（movetime=1ms、D7-TIME-02境界の受理側）でも
    // 同じ保証が成り立つ。soft=hardのため停止理由は2値を許容する
    // （SPEC_UNCLEAR-04）。
    let handle = start(
        snapshot_for(&initial),
        movetime_limits(1),
        42,
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
}

// D7-LIM-05。search.md「時間管理」節: 無期限ではsoft/hardの両リミットを
// 無効化し、外部停止要求だけで停止する。予算生成、Progress、外部停止、
// joinを検査し、D7-API-03(5)の外部停止要求も兼ねる。
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

// ---------------------------------------------------------------------------
// D7-API　探索呼び出し境界
// ---------------------------------------------------------------------------

// D7-API-01。search.md「探索骨格」節（深さ1から1ずつ深める）・「スレッド
// 構成」節・「検証」節（単調な深さ）。ノード数の単調非減少は累積からの
// 導出であり実装契約としてassertする。
#[test]
fn progress_depths_start_at_one_and_increase_by_one() {
    let midgame = quiet_midgame();
    let handle = start(
        snapshot_for(&midgame),
        depth_limits(6),
        81,
        DEFAULT_THREADS,
        small_tt(),
    );
    let events = drain_raw(&handle);
    assert!(events.iter().all(|event| event.search_id() == 81));
    let (progress, finished) = event_reports(events);
    handle.join().expect("search thread must not panic");

    let depths: Vec<u32> = progress.iter().map(|entry| entry.0).collect();
    assert_eq!(depths, vec![1, 2, 3, 4, 5, 6]);
    // strength-stage6.md「窓外れの報告」。深さ5は初期窓を外れるが、
    // 読み直しは通知されず、窓内で完了した反復が1回だけ通知される。
    let delta = weights().unwrap().pawn_value() / 2;
    assert!(progress[4].1 >= progress[3].1 + delta);
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

// D7-API-05。search.md「実施状況」2026年8月12日（`join()`による置換表の
// 返却）とライフサイクル契約。ノード数の非厳密な`≤`は実装契約。
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
    let mut returned_tt = handle.join().expect("search thread must not panic");

    // ルート探索ごとに世代が進む(search.md「置換表」節)。世代が進まないと、
    // 前回探索の同深度エントリが異キーの新規格納を探索をまたいで阻止し続ける。
    // 変異検証(フェーズ4)で検出した配線無検証の補強。
    assert_eq!(returned_tt.generation(), 1);

    // 返却された置換表を渡した探索2は正常に完了し、最善手と最終評価が
    // 探索1と一致する。記録手とカットオフによりノード数は増えない。
    let second = run_search(
        &snapshot.position,
        snapshot.rules,
        &snapshot.root_moves,
        &snapshot.history_keys,
        &depth_limits(3),
        DEFAULT_THREADS,
        &mut returned_tt,
    );
    assert_eq!(second.best_move, first.best_move);
    assert_eq!(second.score, first.score);
    assert!(second.nodes <= first.nodes);
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
        small_tt()
    });
    let handle = SearchHandle {
        started: Instant::now(),
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
        started: Instant::now(),
        hit_ns: Arc::new(AtomicU64::new(0)),
        events,
        stop: Arc::new(AtomicBool::new(false)),
        thread: Some(thread::spawn(|| -> TranspositionTable {
            panic!("injected coordinator panic")
        })),
    };

    assert!(handle.join().is_err());
}
