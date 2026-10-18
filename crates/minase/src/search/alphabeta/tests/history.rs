//! butterfly履歴と捕獲履歴は対局内で持ち越し、根の探索前に減衰させる。
//! 採用理由はdocs/adr/carry-history-tables-within-a-game.md。

use super::*;
use crate::search::HistoryTables;
use crate::search::alphabeta::params;

// 停止済みでも根の開始は1回。両方の履歴を全要素、各ワーカーで検査する。
#[test]
fn history_carry_ages_every_worker_once_before_the_first_iteration() {
    let snapshot = snapshot_for(&Position::initial());
    let mut histories = HistoryTables::new(worker_count(4));
    let quiet_values = [-7, -4, -3, -1, 0, 1, 3, 4, 7, 100];
    let capture_values = [-65_536, -100, -7, -4, -1, 0, 1, 4, 7, 100, 65_536];
    for (worker, history) in histories.workers.iter_mut().enumerate() {
        for (index, value) in history.iter_mut().flatten().flatten().enumerate() {
            *value = quiet_values[(index + worker) % quiet_values.len()];
        }
    }
    for (worker, history) in histories.capture.iter_mut().enumerate() {
        for (index, value) in history.values_mut().iter_mut().enumerate() {
            *value = capture_values[(index + worker) % capture_values.len()];
        }
    }
    for go in 1..=2 {
        let quiet_before = histories.workers.clone();
        let capture_before: Vec<_> = histories
            .capture
            .iter()
            .map(|h| h.values().to_vec())
            .collect();
        let outcome = run_search_team(
            test_nnue(),
            &snapshot.position,
            snapshot.rules(),
            snapshot.repetition(),
            DRAW_SCORE,
            &snapshot.root_moves,
            &snapshot.history_keys,
            &depth_limits(3),
            &AtomicBool::new(true),
            worker_count(4),
            &small_tt(),
            &mut histories,
            None,
            &ManualClock::new(Duration::ZERO),
            &AtomicU64::new(0),
            false,
        );
        assert_eq!(outcome.result.nodes, 0, "go={go}");
        for (worker, history) in histories.workers.iter().enumerate() {
            for (value, before) in history
                .iter()
                .flatten()
                .flatten()
                .zip(quiet_before[worker].iter().flatten().flatten())
            {
                assert_truncated_ratio(
                    i64::from(*value),
                    i64::from(*before) * i64::from(params::history_decay()),
                    100,
                );
            }
        }
        for (worker, history) in histories.capture.iter().enumerate() {
            for (index, value) in history.values().iter().enumerate() {
                assert_truncated_ratio(
                    i64::from(*value),
                    i64::from(capture_before[worker][index])
                        * i64::from(params::capture_history_decay()),
                    100,
                );
            }
        }
    }
}

// 学習に使われない初期局面の空升→同じ空升を印とし、反復中の再減衰を検出する。
#[test]
fn history_carry_survives_handle_round_trips_and_ages_once_per_go() {
    let board = position(
        Color::Black,
        &[
            (sq(5, 5), Color::Black, PieceKind::King),
            (sq(5, 6), Color::White, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    let mv = Move {
        from: sq(5, 5),
        to: sq(5, 6),
        mid: None,
        promote: false,
    };
    let nnue = test_nnue();
    for threads in [DEFAULT_THREADS, worker_count(4)] {
        let mut histories = HistoryTables::new(threads);
        for (worker, history) in histories.workers.iter_mut().enumerate() {
            history[0][60][60] = 160 + worker as i32 * 16;
        }
        for (worker, history) in histories.capture.iter_mut().enumerate() {
            let depth = 30 + worker as u32;
            history.record_cutoff(&board, nnue, mv, &[], depth);
            // 深さ30,31,32,33の二乗は900,961,1024,1089。
            assert_eq!(
                history.read(&board, nnue, mv),
                [900, 961, 1024, 1089][worker]
            );
        }
        let mut table = small_tt();
        for search_id in [801, 802] {
            let before: Vec<_> = histories.workers.iter().map(|h| h[0][60][60]).collect();
            let capture_before: Vec<_> = histories
                .capture
                .iter()
                .map(|h| h.read(&board, nnue, mv))
                .collect();
            let handle = crate::search::start_search(
                Arc::clone(shared_test_nnue()),
                snapshot_for(&Position::initial()),
                depth_limits(3),
                search_id,
                threads,
                table,
                histories,
                false,
            );
            let (_, result) = event_reports(drain_raw(&handle));
            assert_eq!(result.depth, 3, "threads={threads}, search_id={search_id}");
            (table, histories) = handle.join().unwrap();
            for (worker, history) in histories.capture.iter().enumerate() {
                assert_truncated_ratio(
                    i64::from(history.read(&board, nnue, mv)),
                    i64::from(capture_before[worker]) * i64::from(params::capture_history_decay()),
                    100,
                );
            }
            for (worker, history) in histories.workers.iter().enumerate() {
                assert_truncated_ratio(
                    i64::from(history[0][60][60]),
                    i64::from(before[worker]) * i64::from(params::history_decay()),
                    100,
                );
            }
        }
    }
}

// 持ち越した履歴は根の次回開始時に減衰した後も、静かな手の優先順位を決める。
#[test]
fn history_carry_orders_quiets_with_a_fresh_transposition_table() {
    let board = Position::initial();
    let snapshot = snapshot_for(&board);
    let moves = legal_moves(&board);
    let [first, second] = [moves[0], moves[1]];
    for preferred in [first, second] {
        let mut histories = HistoryTables::new(DEFAULT_THREADS);
        histories.workers[0][board.side_to_move().index()][preferred.from.dense_index()]
            [preferred.to.dense_index()] = 10_000;
        for _ in 1..=2 {
            let table = small_tt();
            run_search_team(
                test_nnue(),
                &board,
                snapshot.rules(),
                snapshot.repetition(),
                DRAW_SCORE,
                snapshot.root_moves(),
                &snapshot.history_keys,
                &depth_limits(1),
                &AtomicBool::new(true),
                DEFAULT_THREADS,
                &table,
                &mut histories,
                None,
                &ManualClock::new(Duration::ZERO),
                &AtomicU64::new(0),
                false,
            );
            let stop = AtomicBool::new(false);
            let shared = SharedSearch {
                external_stop: &stop,
                team_stop: AtomicBool::new(false),
                stop_reason: AtomicU8::new(0),
                total_nodes: AtomicU64::new(0),
                node_limit: None,
                clock: &ManualClock::new(Duration::ZERO),
                hard_limit: None,
            };
            let searcher = new_searcher(
                test_nnue(),
                &board,
                snapshot.rules(),
                snapshot.repetition(),
                DRAW_SCORE,
                &[],
                &shared,
                &table,
                &mut histories.workers[0],
                &mut histories.capture[0],
            );
            let carried = searcher.history[board.side_to_move().index()]
                [preferred.from.dense_index()][preferred.to.dense_index()];
            let mut ordered = [first, second];
            searcher.order_moves(&board, &mut ordered, None, 0);
            if carried > 0 {
                assert_eq!(ordered[0], preferred);
            } else {
                let mut baseline = [first, second];
                with_root_searcher(&board, &[], |fresh| {
                    fresh.order_moves(&board, &mut baseline, None, 0)
                });
                assert_eq!(ordered, baseline);
            }
        }
    }
}

// ワーカーへ貸す表は番号で固定され、書き込みも同じ表へ返る。
#[test]
fn history_carry_workers_update_their_own_table() {
    let stop = AtomicBool::new(false);
    let shared = SharedSearch {
        external_stop: &stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        node_limit: None,
        clock: &ManualClock::new(Duration::ZERO),
        hard_limit: None,
    };
    let mut histories = HistoryTables::new(worker_count(4));
    for (index, history) in histories.workers.iter_mut().enumerate() {
        history[1][50][51] = index as i32 + 10;
    }
    for (index, history) in histories.capture.iter_mut().enumerate() {
        history.values_mut()[0] = -(index as i32 + 20);
    }
    let mv = legal_moves(&Position::initial())[0];
    let mut worker_histories: Vec<_> = histories
        .workers
        .iter_mut()
        .zip(&mut histories.capture)
        .collect();
    run_worker_team(
        &mut worker_histories,
        &shared,
        |index, (history, capture)| {
            assert_eq!(history[1][50][51], index as i32 + 10);
            assert_eq!(capture.values()[0], -(index as i32 + 20));
            history[1][50][51] += 100;
            capture.values_mut()[0] -= 100;
            WorkerOutcome {
                partial_score: None,
                worker_index: index,
                result: SearchResult {
                    #[cfg(feature = "search-stats")]
                    stats: crate::search::SearchStats::default(),
                    best_move: mv,
                    score: 0,
                    depth: 1,
                    nodes: 0,
                },
                pv: vec![mv],
                nodes: 0,
            }
        },
    );
    for (index, history) in histories.workers.iter().enumerate() {
        assert_eq!(history[1][50][51], index as i32 + 110);
    }
    for (index, history) in histories.capture.iter().enumerate() {
        assert_eq!(history.values()[0], -(index as i32 + 120));
    }
}

/// 両方の履歴の残存率を0%にすると、次の根の探索前に全要素が0になる。
#[cfg(feature = "tuning")]
pub(super) fn zero_decay_clears_history_between_searches() {
    let snapshot = snapshot_for(&quiet_midgame());
    let nnue = test_nnue();
    for threads in [DEFAULT_THREADS, worker_count(4)] {
        let mut histories = HistoryTables::new(threads);
        crate::search::search(
            nnue,
            &snapshot,
            &depth_limits(3),
            threads,
            &mut small_tt(),
            &mut histories,
        )
        .unwrap();
        assert!(
            histories
                .workers
                .iter()
                .any(|h| h.iter().flatten().flatten().any(|&v| v != 0))
        );
        // 2回目は新しい履歴を書き込む前に止め、前回の全要素が消えたことを確かめる。
        run_search_team(
            nnue,
            &snapshot.position,
            snapshot.rules(),
            snapshot.repetition(),
            DRAW_SCORE,
            &snapshot.root_moves,
            &snapshot.history_keys,
            &depth_limits(3),
            &AtomicBool::new(true),
            threads,
            &small_tt(),
            &mut histories,
            None,
            &ManualClock::new(Duration::ZERO),
            &AtomicU64::new(0),
            false,
        );
        assert!(
            histories
                .workers
                .iter()
                .all(|h| h.iter().flatten().flatten().all(|&v| v == 0))
        );
    }
    // 単一ワーカーの実探索でも、履歴を再利用した2回目と新品の表の結果が一致する。
    let mut histories = HistoryTables::new(DEFAULT_THREADS);
    let first = crate::search::search(
        nnue,
        &snapshot,
        &depth_limits(3),
        DEFAULT_THREADS,
        &mut small_tt(),
        &mut histories,
    )
    .unwrap();
    let second = crate::search::search(
        nnue,
        &snapshot,
        &depth_limits(3),
        DEFAULT_THREADS,
        &mut small_tt(),
        &mut histories,
    )
    .unwrap();
    assert_eq!(first, second);
}

/// 候補8の更新式は両端で飽和し、負の重力項も0方向へ丸める。
#[test]
fn quiet_history_gravity_stays_bounded_and_truncates_toward_zero() {
    let board = Position::initial();
    let moves = legal_moves(&board);
    let winner = moves[0];
    let earlier = *moves.iter().find(|mv| mv.from != winner.from).unwrap();
    let side = board.side_to_move().index();
    let limit = params::history_limit();
    with_root_searcher(&board, &[], |searcher| {
        for old in [-limit, -limit / 2 - 1, -10, 0, 10, limit] {
            for depth in [0, 3, 256] {
                searcher.history[side][winner.from.dense_index()][winner.to.dense_index()] = old;
                searcher.history[side][earlier.from.dense_index()][earlier.to.dense_index()] = old;
                searcher.history[side ^ 1][0][0] = 123;
                searcher.record_quiet_beta_cutoff(&board, winner, &[earlier], depth, 0);
                let bonus =
                    searcher.history[side][winner.from.dense_index()][winner.to.dense_index()];
                let malus =
                    searcher.history[side][earlier.from.dense_index()][earlier.to.dense_index()];
                assert!((old..=limit).contains(&bonus));
                assert!((-limit..=old).contains(&malus));
                if depth == 0 || params::history_bonus_scale() == 0 {
                    assert_eq!(bonus, old);
                }
                if depth == 0 || params::history_malus_scale() == 0 {
                    assert_eq!(malus, old);
                }
                assert_eq!(searcher.history[side ^ 1][0][0], 123);
            }
        }
    });
}

fn history_moves(searcher: &Searcher<'_>, board: &Position, tt: Option<Move>) -> Vec<(Move, bool)> {
    let mut picker = MovePicker::new(tt, searcher.killers[0]);
    std::iter::from_fn(|| {
        picker.next(
            board,
            searcher.nnue,
            &searcher.generator,
            searcher.history,
            searcher.capture_history,
        )
    })
    .collect()
}

/// 候補8: TT手・killer手・通常の静かな手の各カットで、探索済みの非捕獲手だけを減点する。
#[test]
fn quiet_history_negamax_updates_only_searched_quiets() {
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Rook),
            (sq(5, 7), Color::White, PieceKind::Pawn),
            (sq(3, 5), Color::White, PieceKind::Pawn),
            (sq(7, 5), Color::White, PieceKind::Pawn),
            (sq(5, 3), Color::White, PieceKind::Pawn),
        ],
    );
    for winner_index in [0, 2, 4] {
        with_root_searcher(&board, &[], |searcher| {
            // F5の浅いcutNodeでfail-lowの打ち切りが制限されるため、子を十分深く読む。
            searcher.null_move_ply = Some(0);
            let quiets: Vec<_> = legal_moves(&board)
                .into_iter()
                .filter(|&mv| board.captured_squares(mv).iter().all(Option::is_none))
                .collect();
            let tt_move = quiets[0];
            let winner = quiets[winner_index];
            searcher.killers[0] = [Some(quiets[1]), Some(quiets[2])];
            let moves = history_moves(searcher, &board, Some(tt_move));
            assert!(moves.iter().any(|&(_, capture)| capture));
            let cutoff_index = moves.iter().position(|&(mv, _)| mv == winner).unwrap();
            assert!(cutoff_index + 1 < moves.len());
            searcher.tt.store(
                search_key(&board),
                0,
                0,
                Bound::Upper,
                Some(tt_move),
                0,
                false,
            );
            // 深さ10の履歴更新だけを検査し、子の値は十分深い記録で固定する。
            for &(mv, _) in &moves {
                let mut child = board.clone();
                child.make_move_unchecked(mv, engine_rules());
                let score = if mv == winner { 20 } else { 0 };
                searcher
                    .tt
                    .store(search_key(&child), 16, -score, Bound::Exact, None, 1, false);
            }
            assert_eq!(
                searcher.negamax(&mut board.clone(), 10, -10, 10, 0, false),
                Some(20)
            );
            assert_eq!(searcher.nodes, cutoff_index as u64 + 1);
            assert_eq!(searcher.killers[0][0], Some(winner));
            for (index, &(mv, capture)) in moves.iter().enumerate() {
                let expected = if capture || index > cutoff_index {
                    0
                } else if mv == winner {
                    params::history_bonus_scale()
                } else {
                    -params::history_malus_scale()
                };
                assert_eq!(
                    searcher.history[board.side_to_move().index()][mv.from.dense_index()]
                        [mv.to.dense_index()],
                    expected,
                    "winner={winner:?}, move={mv:?}"
                );
            }
        });
    }
}

/// 候補8: futilityで飛ばした非捕獲手は、後続の成る手がカットしても減点しない。
#[test]
fn quiet_history_excludes_pruned_quiets_on_promotion_cutoff() {
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(5, 7), Color::Black, PieceKind::Kirin),
        ],
    );
    let ply = MAX_PLY - 1;
    with_root_searcher(&board, &[], |searcher| {
        let tt_move = Move {
            from: sq(0, 0),
            to: sq(1, 0),
            mid: None,
            promote: false,
        };
        let pruned_move = Move {
            to: sq(0, 1),
            ..tt_move
        };
        searcher.history[0][pruned_move.from.dense_index()][pruned_move.to.dense_index()] = 100;
        let moves = history_moves(searcher, &board, Some(tt_move));
        assert_eq!(moves[1], (pruned_move, false));
        assert!(moves.iter().all(|&(_, capture)| !capture));
        let cutoff_index = moves.iter().position(|&(mv, _)| mv.promote).unwrap();
        let winner = moves[cutoff_index].0;
        let leaf_value = |mv| {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            -evaluate(test_nnue(), &child)
        };
        let beta = leaf_value(winner);
        assert!(leaf_value(tt_move) < beta);
        assert!(!royal_under_attack(&board));
        assert!(
            evaluate(searcher.nnue, &board)
                + super::super::pruning::futility_margin(searcher.nnue.pawn_value(), 3, false)
                < beta - 1
        );
        searcher.tt.store(
            search_key(&board),
            0,
            0,
            Bound::Upper,
            Some(tt_move),
            ply,
            false,
        );
        // 子は最大手数の静的評価で固定する。麒麟の成りはfutilityの余裕値より大きく評価を上げる。
        searcher.null_move_ply = Some(ply);
        assert_eq!(
            searcher.negamax(&mut board.clone(), 3, beta - 1, beta, ply, false),
            Some(beta)
        );
        let searched: Vec<_> = moves[..=cutoff_index]
            .iter()
            .map(|&(mv, _)| mv)
            .filter(|mv| *mv == tt_move || mv.promote)
            .collect();
        assert_eq!(searcher.nodes, searched.len() as u64);
        let mut pruned = 0;
        for &(mv, _) in &moves[..=cutoff_index] {
            // 成る手と不成の手は同じbutterfly要素を共有する。
            let learned = searched
                .iter()
                .find(|other| other.from == mv.from && other.to == mv.to);
            let actual = searcher.history[0][mv.from.dense_index()][mv.to.dense_index()];
            match learned {
                Some(other) if *other == winner => {
                    assert!((0..=params::history_limit()).contains(&actual));
                    assert_eq!(actual > 0, params::history_bonus_scale() >= 12);
                }
                Some(_) => {
                    assert!((-params::history_limit()..=0).contains(&actual));
                    assert_eq!(actual < 0, params::history_malus_scale() >= 12);
                }
                None => {
                    pruned += 1;
                    assert_eq!(actual, if mv == pruned_move { 100 } else { 0 });
                }
            }
        }
        assert!(pruned > 0);
    });
}

/// 調整係数は子プロセス内だけで変更し、加点と減点の独立性と百分率の丸めを固定する。
#[cfg(feature = "tuning")]
pub(super) fn check_quiet_history_scales() {
    let board = Position::initial();
    let moves = legal_moves(&board);
    let winner = moves[0];
    let earlier = *moves.iter().find(|mv| mv.from != winner.from).unwrap();
    for (bonus_scale, malus_scale, depth, bonus, malus) in [
        (0, 100, 3, 0, 9),
        (100, 0, 3, 9, 0),
        (0, 0, 256, 0, 0),
        (150, 250, 3, 13, 22),
        (
            400,
            400,
            256,
            params::history_limit(),
            params::history_limit(),
        ),
    ] {
        params::set("HistoryBonusScale", bonus_scale).unwrap();
        params::set("HistoryMalusScale", malus_scale).unwrap();
        with_root_searcher(&board, &[], |searcher| {
            searcher.record_quiet_beta_cutoff(&board, winner, &[earlier], depth, 0);
            assert_eq!(
                searcher.history[0][winner.from.dense_index()][winner.to.dense_index()],
                bonus
            );
            assert_eq!(
                searcher.history[0][earlier.from.dense_index()][earlier.to.dense_index()],
                -malus
            );
            assert_eq!(searcher.killers[0][0], Some(winner));
            if bonus_scale == 0 && malus_scale == 0 {
                searcher.history[0][winner.from.dense_index()][winner.to.dense_index()] = -123;
                searcher.history[0][earlier.from.dense_index()][earlier.to.dense_index()] = 456;
                searcher.record_quiet_beta_cutoff(&board, winner, &[earlier], depth, 0);
                assert_eq!(
                    searcher.history[0][winner.from.dense_index()][winner.to.dense_index()],
                    -123
                );
                assert_eq!(
                    searcher.history[0][earlier.from.dense_index()][earlier.to.dense_index()],
                    456
                );
            }
        });
    }
}

#[test]
fn quiet_history_gravity_numeric_examples() {
    // 上限100、更新量±9の独立した算術例。−51×9/100は−4へ丸める。
    for (old, delta, expected) in [
        (0, 9, 9),
        (100, 9, 100),
        (-100, 9, -82),
        (-51, 9, -38),
        (-51, -9, -56),
        (100, -100, -100),
        (-100, 100, 100),
    ] {
        assert_eq!(
            super::super::ordering::history_gravity(old, delta, 100),
            expected
        );
    }
}
