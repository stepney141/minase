//! docs/plans/archived/multipv.mdの検証6〜10と、Stockfish 5c4cef0b:624の停止条件を検査する。

use super::*;

fn events(snapshot: SearchSnapshot, limits: SearchLimits) -> Vec<SearchEvent> {
    let handle = start_search_with_clock(
        crate::search::alphabeta::Engine {
            eval_file: String::new(),
            network: None,
            transposition_table: Some(small_tt()),
            histories: crate::search::HistoryTables::new(worker_count(1)),
            threads: worker_count(1),
        },
        Arc::clone(shared_test_nnue()),
        snapshot,
        limits,
        301,
        false,
        Arc::new(ManualClock::new(Duration::ZERO)),
    );
    let events = drain_raw(&handle);
    handle.join().unwrap();
    events
}

/// 本数、根の候補、順位、深さ、および最終通知を同じ契約で検査する。
pub(super) fn assert_ranked_search(snapshot: SearchSnapshot, count: usize) {
    let n = count.min(snapshot.root_moves().len());
    let output = events(
        snapshot.clone().with_multi_pv(count).unwrap(),
        depth_limits(4),
    );
    let mut last = None;
    for event in output {
        match event {
            SearchEvent::Progress { lines, .. } => {
                assert_eq!(lines.len(), n);
                let depth = lines[0].depth;
                for (index, line) in lines.iter().enumerate() {
                    assert_eq!(line.depth, depth);
                    assert!(snapshot.root_moves().contains(&line.moves[0]));
                    assert!(
                        !lines[..index]
                            .iter()
                            .any(|prior| prior.moves[0] == line.moves[0])
                    );
                    assert_pv_is_legal(&snapshot.position, &line.moves);
                }
                assert!(
                    lines
                        .windows(2)
                        .all(|pair| pair[0].score.unwrap() >= pair[1].score.unwrap())
                );
                last = Some(lines);
            }
            SearchEvent::Finished {
                lines,
                best_move,
                depth,
                partial_score,
                ..
            } => {
                assert_eq!(depth, 4);
                assert_eq!(partial_score, None);
                assert_eq!(lines[0].moves[0], best_move);
                assert_eq!(Some(lines), last);
            }
        }
    }
}

#[test]
fn multipv_progress_contains_distinct_ranked_candidates_at_one_depth() {
    assert_ranked_search(snapshot_for(&quiet_midgame()), 3);
    // 隅の王には3手しかなく、要求本数より候補数が少ない。
    let snapshot = snapshot_for(&bare_kings());
    assert_eq!(snapshot.root_moves().len(), 3);
    assert_ranked_search(snapshot, 8);
}

#[test]
fn multipv_single_candidate_uses_one_line() {
    let board = bare_kings();
    let roots = legal_moves(&board);
    let snapshot = SearchSnapshot::from_parts(
        board,
        engine_rules(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        vec![],
        vec![roots[0]],
    )
    .unwrap();
    assert_ranked_search(snapshot, 3);
}

#[test]
fn multipv_two_royal_captures_precede_an_ordinary_line() {
    // 成りのない金2枚だけが中央の王を取れる。自王は遠く離す。
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 5), Color::White, PieceKind::King),
            (sq(4, 5), Color::Black, PieceKind::GoldGeneral),
            (sq(6, 5), Color::Black, PieceKind::GoldGeneral),
        ],
    );
    let captures = [step(sq(4, 5), sq(5, 5)), step(sq(6, 5), sq(5, 5))];
    let output = events(
        snapshot_for(&board).with_multi_pv(3).unwrap(),
        depth_limits(1),
    );
    let SearchEvent::Finished { lines, .. } = output.last().unwrap() else {
        panic!()
    };
    assert_eq!(lines.len(), 3);
    assert_ne!(lines[0].moves[0], lines[1].moves[0]);
    for line in &lines[..2] {
        assert!(captures.contains(&line.moves[0]));
        assert_eq!(line.score, Some(MATE - 1));
    }
    assert!(lines[2].score.unwrap().abs() < MATE_THRESHOLD);
}

#[test]
fn multipv_secondary_search_preserves_root_records_and_change_count() {
    // 深さ1の子はcorrectionを更新しないので、根の更新だけを比較できる。
    let board = quiet_midgame();
    let roots = legal_moves(&board);
    let mut records = Vec::new();
    for count in [1, 2] {
        with_root_searcher(&board, &[], |searcher| {
            searcher
                .search_iteration(&board, &roots, 1, &[], count)
                .unwrap();
            let hit = searcher.tt.probe(search_key(&board), 0).unwrap();
            records.push((
                hit.best_move,
                hit.static_eval,
                hit.value,
                hit.is_pv,
                searcher.correction_weighted_sum_at(board.side_to_move(), 0),
                searcher.shared.take_best_move_changes(),
            ));
        });
    }
    assert_eq!(records[0], records[1]);
}

#[test]
fn multipv_interruption_discards_the_entire_incomplete_iteration() {
    let snapshot = snapshot_for(&quiet_midgame()).with_multi_pv(2).unwrap();
    let output = events(snapshot.clone(), depth_limits(3));
    let progress: Vec<_> = output
        .iter()
        .filter_map(|event| match event {
            SearchEvent::Progress { nodes, lines, .. } => Some((*nodes, lines.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(progress.len(), 3);
    for index in [0, 2] {
        let limit = progress[index].0 - 1;
        let stopped = events(
            snapshot.clone(),
            SearchLimits::new(Some(3), Some(limit), None, None).unwrap(),
        );
        let SearchEvent::Finished {
            depth,
            lines,
            best_move,
            stop_reason,
            partial_score,
            ..
        } = stopped.last().unwrap()
        else {
            panic!()
        };
        assert_eq!(*stop_reason, StopReason::NodeLimit);
        assert_eq!(*partial_score, None);
        assert_eq!(*depth, index as u32);
        if index == 0 {
            assert_eq!(stopped.len(), 1);
            assert_eq!(lines.len(), 1);
            assert_eq!(*best_move, snapshot.root_moves()[0]);
            assert_eq!(lines[0].moves, [snapshot.root_moves()[0]]);
        } else {
            assert_eq!(*lines, progress[index - 1].1);
        }
    }
}

#[test]
fn multipv_worker_selection_keeps_all_lines_from_the_selected_worker() {
    let roots = legal_moves(&bare_kings());
    let outcome = |worker_index, depth, first: usize| WorkerOutcome {
        partial_score: None,
        returned_move_average: Some(0),
        time_reduction: (worker_index == 0).then_some(1000),
        search_again_counter: 0,
        worker_index,
        result: SearchResult {
            #[cfg(feature = "search-stats")]
            stats: crate::search::SearchStats::default(),
            best_move: roots[first],
            score: 0,
            depth,
            nodes: 0,
        },
        lines: vec![
            RootLine {
                mv: roots[first],
                score: 0,
                pv: vec![roots[first]],
            },
            RootLine {
                mv: roots[(first + 1) % roots.len()],
                score: -1,
                pv: vec![roots[(first + 1) % roots.len()]],
            },
        ],
        nodes: 0,
    };
    for (aux_depth, selected) in [(4, 1), (3, 0)] {
        let workers = [outcome(0, 3, 0), outcome(1, aux_depth, 1)];
        assert_eq!(
            select_worker_outcome(&workers).lines,
            workers[selected].lines
        );
    }
}

#[test]
fn multipv_mate_early_stop_uses_last_win_and_first_loss() {
    for (first, last, expected) in [
        (MATE - 1, MATE - 1, true),
        (MATE - 1, 0, false),
        (MATE - 1, MATE - 3, true),
        (-(MATE - 2), 0, true),
        (0, -(MATE - 2), false),
    ] {
        assert_eq!(mate_early_stop(first, last), expected);
    }
}

#[test]
fn multipv_secondary_root_does_not_count_a_later_winning_move() {
    // 基準の静かな手の後で王取りが見つかっても、k≥2の変化は時間管理へ加算しない。
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 5), Color::White, PieceKind::King),
            (sq(4, 5), Color::Black, PieceKind::GoldGeneral),
        ],
    );
    let roots = legal_moves(&board);
    let reference = step(sq(0, 0), sq(0, 1));
    let capture = step(sq(4, 5), sq(5, 5));
    with_root_searcher(&board, &[], |searcher| {
        let (best, score) = searcher
            .search_root(&board, &roots, 1, 1, -INFINITY, INFINITY, Some(reference))
            .unwrap();
        assert_eq!((best, score), (capture, MATE - 1));
        assert_eq!(searcher.shared.take_best_move_changes(), 0);
        // 探索値は保存せず、根の補正前の静的評価だけを残す。
        let hit = searcher.tt.probe(search_key(&board), 0).unwrap();
        assert_eq!(hit.value, None);
        assert_eq!(hit.best_move, None);
        assert_eq!(hit.static_eval, Some(evaluate(test_nnue(), &board)));
    });
}
