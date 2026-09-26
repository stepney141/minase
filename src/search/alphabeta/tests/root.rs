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
    for (prev, first_nodes) in [(delta, moves.len() as u64), (-delta, 1)] {
        with_root_searcher(&position, &history, |searcher| {
            let (_, score) = searcher
                .search_iteration(&position, &moves, 5, Some(prev))
                .unwrap();
            assert_eq!(score, DRAW_SCORE);
            assert_eq!(searcher.nodes, first_nodes + moves.len() as u64);
            assert_eq!(
                searcher.tt.probe(search_key(&position), 0).unwrap().bound,
                Bound::Exact
            );
        });
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
                completed.map(|(_, score)| score),
            );
            assert!(completed.is_some());
        }
        completed_pv.clone_from(&searcher.pv[0]);
        let (_, prev) = completed.unwrap();
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
    assert_eq!((finished.best_move, finished.score), completed.unwrap());
    assert_eq!(finished.pv, completed_pv);
    assert_eq!(finished.nodes, node_limit);
    assert_eq!(finished.stop_reason, StopReason::NodeLimit);
    let last = progress.last().unwrap();
    assert_eq!(last.1, finished.score);
    assert_eq!(last.4, finished.pv);
    assert!(last.2 < finished.nodes);
}
