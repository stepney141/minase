//! 根の探索と探索窓を検査する。

use super::*;

// docs/plans/strength-stage6.md「窓の適用条件と拡大」「検証」。
#[test]
fn aspiration_initial_window_uses_only_eligible_previous_scores() {
    for depth in 1..=4 {
        let window = AspirationWindow::initial(depth, Some(100), 50);
        assert_eq!((window.alpha, window.beta), (-INFINITY, INFINITY));
    }
    for prev in [
        None,
        Some(MATE_THRESHOLD),
        Some(-MATE_THRESHOLD),
        Some(MATE),
        Some(-MATE),
    ] {
        let window = AspirationWindow::initial(5, prev, 50);
        assert_eq!((window.alpha, window.beta), (-INFINITY, INFINITY));
    }
    for depth in [5, 6, 8] {
        let window = AspirationWindow::initial(depth, Some(100), 50);
        assert_eq!((window.alpha, window.beta, window.delta), (50, 150, 50));
    }
}

#[test]
fn aspiration_widens_only_the_failed_side() {
    let mut low = AspirationWindow::initial(5, Some(100), 50);
    low.widen_low();
    assert_eq!((low.alpha, low.beta, low.delta), (0, 150, 100));
    low.widen_low();
    assert_eq!((low.alpha, low.beta, low.delta), (-100, 150, 201));

    let mut high = AspirationWindow::initial(5, Some(100), 50);
    high.widen_high();
    assert_eq!((high.alpha, high.beta, high.delta), (50, 200, 100));
    high.widen_high();
    assert_eq!((high.alpha, high.beta, high.delta), (50, 300, 201));

    low.widen_high();
    assert_eq!((low.alpha, low.beta, low.delta), (-100, 351, 404));
    low.widen_low();
    assert_eq!((low.alpha, low.beta, low.delta), (-504, 351, 812));
    low.widen_high();
    assert_eq!((low.alpha, low.beta, low.delta), (-504, 1163, 1632));
    for _ in 0..4 {
        low.widen_low();
        low.widen_high();
    }
    assert_eq!((low.alpha, low.beta), (-INFINITY, INFINITY));
}

#[test]
fn aspiration_normalizes_initial_and_widened_mate_edges() {
    for distance in [49, 50, 51, 100] {
        let mut high = AspirationWindow::initial(5, Some(MATE_THRESHOLD - distance), 50);
        let mut low = AspirationWindow::initial(5, Some(-MATE_THRESHOLD + distance), 50);
        if distance <= 50 {
            assert_eq!(high.beta, INFINITY);
            assert_eq!(low.alpha, -INFINITY);
        } else {
            assert_eq!(high.beta, MATE_THRESHOLD - distance + 50);
            assert_eq!(low.alpha, -MATE_THRESHOLD + distance - 50);
        }
        let unchanged_alpha = high.alpha;
        let unchanged_beta = low.beta;
        for _ in 0..3 {
            high.widen_high();
            low.widen_low();
            assert_eq!((high.alpha, high.beta), (unchanged_alpha, INFINITY));
            assert_eq!((low.alpha, low.beta), (-INFINITY, unchanged_beta));
        }
    }
}

// 同「根の探索の窓化」。引き分け値0を境界に置き、等号とβ打ち切りを検査する。
// 反復までに適用する着手は、全手探索なら合法手数、β打ち切りなら最初の1手となる。
#[test]
fn root_window_classifies_bounds_and_cuts_off_remaining_moves() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    assert!(moves.len() > 1);
    let history = repeated_root_children(&position, &moves);
    for (alpha, beta, bound, expected_nodes) in [
        (0, 1, Bound::Upper, moves.len() as u64),
        (-1, 0, Bound::Lower, 1),
        (-1, 1, Bound::Exact, moves.len() as u64),
    ] {
        with_root_searcher(&position, &history, |searcher| {
            let (best_move, score) = searcher
                .search_root(&position, &moves, 1, alpha, beta)
                .unwrap();
            assert_eq!(score, DRAW_SCORE);
            assert_eq!(searcher.nodes, expected_nodes);
            let hit = searcher.tt.probe(search_key(&position), 0).unwrap();
            assert_eq!(hit.bound, bound);
            assert_eq!(hit.score, score);
            assert_eq!(hit.best_move, Some(best_move));
        });
    }
}

// 同「aspiration windows」。s == α、s == βのどちらも再探索を要する。
// 再探索では全合法手を適用するため、初回に適用した着手数へ合法手数を加える。
#[test]
fn aspiration_researches_scores_equal_to_either_edge() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    let history = repeated_root_children(&position, &moves);
    let delta = aspiration_delta(weights().unwrap().pawn_value());
    for is_main_worker in [true, false] {
        for (prev, first_nodes) in [(delta, moves.len() as u64), (-delta, 1)] {
            with_root_searcher(&position, &history, |searcher| {
                let result = searcher
                    .search_iteration(&position, &moves, 5, Some(prev), is_main_worker)
                    .unwrap();
                let reduced = is_main_worker && prev == -delta;
                assert_eq!(result.score, DRAW_SCORE);
                assert_eq!(result.depth, if reduced { 4 } else { 5 });
                assert_eq!(searcher.nodes, first_nodes + moves.len() as u64);
                // 減深後の正確な値は、先に保存した深さ5の下界を置き換えない。
                let hit = searcher.tt.probe(search_key(&position), 0).unwrap();
                assert_eq!(hit.depth, 5);
                assert_eq!(hit.bound, if reduced { Bound::Lower } else { Bound::Exact });
            });
        }
    }
}

// 同「根の探索の窓化」。fail-lowでもαの更新と独立に最善手を保存し、
// 読み直しではその記録手を先頭に使う。王駒捕獲は適用しないので0ノードとなる。
#[test]
fn root_fail_low_keeps_the_best_bound_move_for_research() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::Rook),
            (fs(6, 12), Color::Black, PieceKind::King),
        ],
    );
    let moves = legal_moves(&position);
    let quiet = *moves
        .iter()
        .find(|&&mv| !captures_last_royal(&position, mv))
        .unwrap();
    let history = repeated_root_children(&position, &moves);
    with_root_searcher(&position, &history, |searcher| {
        let key = search_key(&position);
        searcher
            .tt
            .store(key, 0, DRAW_SCORE, Bound::Exact, Some(quiet), 0);
        let (best_move, score) = searcher
            .search_root(&position, &moves, 1, MATE, INFINITY)
            .unwrap();
        assert_eq!(score, MATE - 1);
        assert!(captures_last_royal(&position, best_move));
        let hit = searcher.tt.probe(key, 0).unwrap();
        assert_eq!(hit.bound, Bound::Upper);
        assert_eq!(hit.best_move, Some(best_move));
        let before = searcher.nodes;
        assert_eq!(
            searcher.search_root(&position, &moves, 1, -1, 0),
            Some((best_move, MATE - 1))
        );
        assert_eq!(searcher.nodes - before, 0);
        assert_eq!(searcher.tt.probe(key, 0).unwrap().bound, Bound::Lower);
    });
}

// 同「窓外れの報告」「検証」。深さ5の最初の窓外れを実測してノード予算を
// 決め、読み直しで1手を適用した後に中断させる。経過時間やスケジューリングに依存しない。
#[test]
fn aspiration_interruption_preserves_the_last_completed_iteration() {
    let position = quiet_midgame();
    let moves = legal_moves(&position);
    let history = [search_key(&position)];
    let mut node_limit = 0;
    let mut completed = None;
    let mut completed_pv = Vec::new();
    with_root_searcher(&position, &history, |searcher| {
        for depth in 1..=4 {
            completed = searcher.search_iteration(
                &position,
                &moves,
                depth,
                completed
                    .as_ref()
                    .map(|result: &IterationResult| result.score),
                true,
            );
            assert!(completed.is_some());
        }
        completed_pv.clone_from(&searcher.pv[0]);
        let prev = completed.as_ref().unwrap().score;
        let delta = searcher.pst.pawn_value() / 2;
        let (_, score) = searcher
            .search_root(&position, &moves, 5, prev - delta, prev + delta)
            .unwrap();
        assert!(
            score >= prev + delta,
            "fixture must fail high before the interruption"
        );
        assert_eq!(
            searcher.tt.probe(search_key(&position), 0).unwrap().bound,
            Bound::Lower
        );
        node_limit = searcher.nodes + 1;
    });

    let handle = start(
        snapshot_for(&position),
        nodes_limits(node_limit),
        85,
        worker_count(1),
        small_tt(),
    );
    let (progress, finished) = event_reports(drain_raw(&handle));
    handle.join().expect("search thread must not panic");
    assert_eq!(
        progress.iter().map(|entry| entry.0).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(finished.depth, 4);
    let completed = completed.unwrap();
    assert_eq!(
        (finished.best_move, finished.score),
        (completed.best_move, completed.score)
    );
    assert_eq!(finished.pv, completed_pv);
    assert_eq!(finished.nodes, node_limit);
    assert_eq!(finished.stop_reason, StopReason::NodeLimit);
    let last = progress.last().unwrap();
    assert_eq!(last.1, finished.score);
    assert_eq!(last.4, finished.pv);
    assert!(last.2 < finished.nodes);
}

// docs/plans/strength-stage12.md「項目1」「検証」。境界値で窓外れを与え、
// 初回の深さ、連続fail-high、fail-lowによるリセット、深さ1の下限を固定する。
#[test]
fn aspiration_retry_depth_tracks_consecutive_fail_highs_only_for_main_worker() {
    let mv = legal_moves(&Position::initial())[0];
    for (is_main_worker, expected) in [
        (true, vec![5, 4, 3, 5, 4, 3, 2, 1, 1, 1]),
        (false, vec![5; 10]),
    ] {
        let mut depths = Vec::new();
        let result = search_with_aspiration(
            5,
            AspirationWindow::initial(5, Some(0), 1),
            is_main_worker,
            |depth, alpha, beta| {
                depths.push(depth);
                let score = match depths.len() {
                    3 => alpha,
                    10 => alpha + 1,
                    _ => beta,
                };
                Some((mv, score))
            },
        )
        .unwrap();
        assert_eq!(depths, expected);
        assert_eq!(result.depth, *expected.last().unwrap());
        assert_eq!(result.best_move, mv);
    }
}

// 読み直し中の中断は、減深の有無によらず反復の完了として返さない。
#[test]
fn aspiration_interrupted_retry_has_no_completed_depth() {
    let mv = legal_moves(&Position::initial())[0];
    for is_main_worker in [true, false] {
        let mut depths = Vec::new();
        let result = search_with_aspiration(
            5,
            AspirationWindow::initial(5, Some(0), 50),
            is_main_worker,
            |depth, _, beta| {
                depths.push(depth);
                (depths.len() == 1).then_some((mv, beta))
            },
        );
        assert!(result.is_none());
        assert_eq!(
            depths,
            if is_main_worker {
                vec![5, 4]
            } else {
                vec![5, 5]
            }
        );
    }
}

// docs/plans/strength-stage12.md「項目1」「検証」。探索値だけを制御し、
// 反復、窓の読み直し、進捗、時間管理、およびワーカー選択は本番の経路を通す。
// 目標5で1回、目標6で3回fail-highとし、確定深さを4→4→3にする。
#[test]
fn aspiration_completed_depth_drives_reports_and_selection_but_not_iteration_count() {
    let position = quiet_midgame();
    let roots = legal_moves(&position);
    let pst = weights().unwrap();
    for (depth_limit, timed, expected_depths) in [
        (6, false, vec![1, 2, 3, 4, 4, 3]),
        (7, false, vec![1, 2, 3, 4, 4, 3, 7]),
        (8, true, vec![1, 2, 3, 4, 4, 3, 7]),
    ] {
        let stop = AtomicBool::new(false);
        let hit_ns = AtomicU64::new(u64::MAX);
        let budget = TimeBudget {
            soft: Duration::from_secs(20),
            hard: Duration::from_secs(100),
        };
        let shared = SharedSearch {
            external_stop: &stop,
            team_stop: AtomicBool::new(false),
            stop_reason: AtomicU8::new(0),
            total_nodes: AtomicU64::new(0),
            node_limit: None,
            started: Instant::now() - Duration::from_secs(10),
            hard_limit: timed.then_some(HardLimit {
                duration: budget.hard,
                hit_ns: &hit_ns,
            }),
        };
        let table = small_tt();
        let searcher = new_searcher(&pst, &position, engine_rules(), &[], &shared, &table);
        let (sender, receiver) = mpsc::channel();
        let mut targets = Vec::new();
        let mut attempts = Vec::new();
        let mut previous_score = None;
        let outcome = run_main_iterations(
            searcher,
            &position,
            &roots,
            depth_limit,
            timed.then_some(budget),
            Some((&sender, 912)),
            timed,
            |searcher, target, prev| {
                targets.push(target);
                assert_eq!(prev, previous_score);
                assert!(
                    target <= 7,
                    "stable reduced iterations must stop before target 8"
                );
                if timed {
                    // 深さ4〜6の3反復だけでは、まだ4連続の安定には達しない。
                    assert!(!searcher.ponder_iteration.as_ref().unwrap().stable);
                }
                let best_move = roots[(target.min(4) - 1) as usize];
                searcher.pv[0] = vec![best_move];
                let fail_highs = match target {
                    5 => 1,
                    6 => 3,
                    _ => 0,
                };
                let mut calls = 0;
                let result = search_with_aspiration(
                    target,
                    AspirationWindow::initial(target, prev, 50),
                    true,
                    |depth, alpha, beta| {
                        attempts.push(depth);
                        calls += 1;
                        let score = if calls <= fail_highs {
                            beta
                        } else {
                            (alpha + beta) / 2
                        };
                        Some((best_move, score))
                    },
                );
                previous_score = result.as_ref().map(|result| result.score);
                if timed && target == 7 {
                    // 経過10秒では通常は継続できるが、安定時の予測26.3秒はsoftを超える。
                    hit_ns.store(0, AtomicOrdering::Relaxed);
                }
                result
            },
        );
        assert_eq!(
            targets,
            (1..=expected_depths.len() as u32).collect::<Vec<_>>()
        );
        let mut expected_attempts = vec![1, 2, 3, 4, 5, 4, 6, 5, 4, 3];
        if expected_depths.len() == 7 {
            expected_attempts.push(7);
        }
        assert_eq!(attempts, expected_attempts);
        let progress: Vec<_> = receiver
            .try_iter()
            .map(|event| {
                let SearchEvent::Progress {
                    search_id,
                    depth,
                    score,
                    pv,
                    ..
                } = event
                else {
                    panic!("main worker must send progress only");
                };
                assert_eq!(search_id, 912);
                (depth, score, pv)
            })
            .collect();
        assert_eq!(
            progress.iter().map(|event| event.0).collect::<Vec<_>>(),
            expected_depths
        );
        assert_eq!(outcome.result.depth, *expected_depths.last().unwrap());
        assert_eq!(outcome.result.score, previous_score.unwrap());
        assert_eq!(outcome.pv, vec![outcome.result.best_move]);
        assert_eq!(progress.last().unwrap().2, outcome.pv);
        assert_eq!(progress.last().unwrap().1, outcome.result.score);
        assert_eq!(
            shared.reason(),
            if timed {
                StopReason::SoftLimit
            } else {
                StopReason::DepthCompleted
            }
        );
        assert!(shared.team_stop.load(AtomicOrdering::Relaxed));
        if depth_limit == 6 {
            // 目標6を確定深さ3で終えた主より、確定深さ4の補助を採る。
            let auxiliary = WorkerOutcome {
                worker_index: 1,
                result: SearchResult {
                    best_move: roots[0],
                    score: 0,
                    depth: 4,
                    nodes: 0,
                },
                pv: vec![roots[0]],
                nodes: 0,
            };
            let outcomes = [outcome, auxiliary];
            assert_eq!(select_worker_outcome(&outcomes).worker_index, 1);
        }
    }
}

// Threads=1の実探索でも、目標5のfail-high後に確定した深さを外部へ返す。
#[test]
fn aspiration_single_worker_reports_the_final_root_search_depth() {
    let position = quiet_midgame();
    let snapshot = snapshot_for(&position);
    let stop = AtomicBool::new(false);
    let table = small_tt();
    let (sender, receiver) = mpsc::channel();
    let outcome = run_search_team(
        &weights().unwrap(),
        &position,
        snapshot.rules,
        &snapshot.root_moves,
        &snapshot.history_keys,
        &depth_limits(5),
        &stop,
        worker_count(1),
        &table,
        Some((&sender, 913)),
        Instant::now(),
        &AtomicU64::new(0),
        false,
    );
    let depths: Vec<_> = receiver
        .try_iter()
        .map(|event| {
            let SearchEvent::Progress { depth, .. } = event else {
                panic!("expected progress")
            };
            depth
        })
        .collect();
    // 上の窓外れ中断テストと同じ局面で、深さ5のfail-high後に深さ4で確定する。
    assert_eq!(depths, vec![1, 2, 3, 4, 4]);
    assert_eq!(outcome.result.depth, 4);
    assert_eq!(outcome.stop_reason, StopReason::DepthCompleted);
    assert_pv_is_legal(&position, &outcome.pv);
}
