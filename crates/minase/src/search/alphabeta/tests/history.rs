//! butterfly履歴と捕獲履歴は対局内で持ち越し、根の探索前に減衰させる。
//! 採用理由はdocs/adr/carry-histories-and-correction-histories-within-a-game.md。

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
    let pawn_piece = PieceCode::new(Color::Black, PieceKind::Pawn).unwrap();
    let pawn_square = sq(4, 4);
    histories.continuation.store(0, 1234);
    histories.pawn.store(0x1234, pawn_piece, pawn_square, 2345);
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
        assert_eq!(histories.continuation.read(0), 1234, "go={go}");
        assert_eq!(
            histories.pawn.read(0x1234, pawn_piece, pawn_square),
            2345,
            "go={go}"
        );
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
            history.values_mut().fill(0);
            history.update(&board, nnue, mv, (depth * depth) as i32);
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
                &histories.correction,
                &mut histories.continuation_correction[0],
                &histories.continuation,
                &histories.pawn,
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
    // 単一ワーカーの実探索でも、減衰対象外の表を同じ状態にそろえれば、
    // ワーカー別の2表を再利用した探索と新品の表の結果が一致する。
    let mut histories = HistoryTables::new(DEFAULT_THREADS);
    crate::search::search(
        nnue,
        &snapshot,
        &depth_limits(3),
        DEFAULT_THREADS,
        &mut small_tt(),
        &mut histories,
    )
    .unwrap();
    let fresh_shared = HistoryTables::new(DEFAULT_THREADS);
    histories.continuation = fresh_shared.continuation;
    histories.pawn = fresh_shared.pawn;
    histories.correction = fresh_shared.correction;
    histories.continuation_correction = fresh_shared.continuation_correction;
    let reused = crate::search::search(
        nnue,
        &snapshot,
        &depth_limits(3),
        DEFAULT_THREADS,
        &mut small_tt(),
        &mut histories,
    )
    .unwrap();
    let fresh = crate::search::search(
        nnue,
        &snapshot,
        &depth_limits(3),
        DEFAULT_THREADS,
        &mut small_tt(),
        &mut HistoryTables::new(DEFAULT_THREADS),
    )
    .unwrap();
    assert_eq!(reused, fresh);
}

/// 重力式は更新量を先に切り詰め、正負の値を常に表の範囲内へ保つ。
#[test]
fn quiet_history_gravity_stays_bounded_and_truncates_toward_zero() {
    let limit = super::super::history::HISTORY_LIMIT;
    for old in [-limit, -limit / 2 - 1, -10, 0, 10, limit] {
        for bonus in [-(limit + 1), -limit, -9, 0, 9, limit, limit + 1] {
            let updated = super::super::history::history_gravity(old, bonus, limit);
            assert!((-limit..=limit).contains(&updated));
        }
    }
    assert_eq!(
        super::super::history::history_gravity(0, limit + 1, limit),
        limit
    );
    assert_eq!(
        super::super::history::history_gravity(0, -(limit + 1), limit),
        -limit
    );
}

fn history_moves(searcher: &Searcher<'_>, board: &Position, tt: Option<Move>) -> Vec<(Move, bool)> {
    let mut picker = MovePicker::new(tt, searcher.killers[0]);
    std::iter::from_fn(|| {
        picker.next(
            board,
            searcher.nnue,
            &searcher.generator,
            searcher_move_picker_context(searcher, 0),
        )
    })
    .collect()
}

/// 候補8: TT手・killer手・通常の静かな手の各カットで、探索済みの非捕獲手だけを減点する。
/// 静かな手のSEEで飛ばした手も探索済みの手に含めない。
#[cfg(feature = "tuning")]
pub(super) fn quiet_history_negamax_updates_only_searched_quiets() {
    let check_bonus = params::ordering_check_bonus();
    let threat_weight = params::ordering_threat_weight();
    // この検査では枝刈りと履歴更新だけを分離する。係数が0でもM3bの判定処理は省かれない。
    params::set("OrderingCheckBonus", 0).unwrap();
    params::set("OrderingThreatWeight", 0).unwrap();
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
    for winner_case in 0..3 {
        with_root_searcher(&board, &[], |searcher| {
            // F5の浅いcutNodeでfail-lowの打ち切りが制限されるため、子を十分深く読む。
            searcher.null_move_ply = Some(0);
            let quiets: Vec<_> = legal_moves(&board)
                .into_iter()
                .filter(|&mv| board.captured_squares(mv).iter().all(Option::is_none))
                .collect();
            let tt_move = quiets[0];
            searcher.killers[0] = [Some(quiets[1]), Some(quiets[2])];
            let moves = history_moves(searcher, &board, Some(tt_move));
            assert!(moves.iter().any(|&(_, capture)| capture));
            assert!(!royal_under_attack(&board));
            let static_eval = evaluate(searcher.nnue, &board);
            let quiet_see_pruned: Vec<_> = moves
                .iter()
                .enumerate()
                .map(|(index, &(mv, capture))| {
                    if index == 0 || capture || mv.promote || mv.mid.is_some() {
                        return false;
                    }
                    let lmr_depth =
                        super::super::pruning::pruning_lmr_depth(10, index, false, true, true);
                    let gives_check = board.gives_check(&searcher.generator, mv);
                    if gives_check {
                        assert!(!see_prunes(
                            &board,
                            engine_rules(),
                            searcher.nnue,
                            mv,
                            -super::super::pruning::see_margin(searcher.nnue.pawn_value(), 10, 0),
                        ));
                        return false;
                    }
                    assert_eq!(
                        super::super::pruning::futility_pruning_value(
                            searcher.nnue.pawn_value(),
                            static_eval,
                            -10,
                            lmr_depth,
                        ),
                        None,
                        "SEEより前のfutility pruningをこの局面から除外する: {mv:?}"
                    );
                    see_prunes(
                        &board,
                        engine_rules(),
                        searcher.nnue,
                        mv,
                        -super::super::pruning::quiet_see_margin(
                            searcher.nnue.pawn_value(),
                            lmr_depth,
                        ),
                    )
                })
                .collect();
            let winner = match winner_case {
                0 => tt_move,
                1 => quiets[2],
                2 => {
                    let first_pruned = quiet_see_pruned
                        .iter()
                        .position(|&pruned| pruned)
                        .expect("fixture must contain a SEE-pruned quiet");
                    moves
                        .iter()
                        .enumerate()
                        .skip(first_pruned + 1)
                        .find_map(|(index, &(mv, capture))| {
                            (!capture
                                && mv != tt_move
                                && !searcher.killers[0].contains(&Some(mv))
                                && !quiet_see_pruned[index]
                                && index + 1 < moves.len())
                            .then_some(mv)
                        })
                        .expect("fixture must contain a later searchable ordinary quiet")
                }
                _ => unreachable!("three winner cases"),
            };
            let cutoff_index = moves.iter().position(|&(mv, _)| mv == winner).unwrap();
            assert!(cutoff_index + 1 < moves.len());
            if winner_case == 2 {
                assert!(
                    quiet_see_pruned[..=cutoff_index]
                        .iter()
                        .any(|&pruned| pruned)
                );
            }
            assert!(!quiet_see_pruned[cutoff_index]);
            searcher.tt.store(
                search_key(&board),
                0,
                0,
                None,
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
                searcher.tt.store(
                    search_key(&child),
                    16,
                    -score,
                    None,
                    Bound::Exact,
                    None,
                    1,
                    false,
                );
            }
            assert_eq!(
                searcher.negamax(&mut board.clone(), 10, -10, 10, 0, false),
                Some(20)
            );
            let searched = quiet_see_pruned[..=cutoff_index]
                .iter()
                .filter(|&&pruned| !pruned)
                .count();
            assert_eq!(searcher.nodes, searched as u64);
            assert_eq!(searcher.killers[0][0], Some(winner));
            let mut alpha = -10;
            let mut penalized = Vec::new();
            for (index, &(mv, capture)) in moves[..cutoff_index].iter().enumerate() {
                if quiet_see_pruned[index] {
                    continue;
                }
                if !capture && index + 1 <= 32 && 0 <= alpha {
                    penalized.push(mv);
                }
                alpha = alpha.max(0);
            }
            for &(mv, _) in &moves {
                let actual = searcher.history[board.side_to_move().index()][mv.from.dense_index()]
                    [mv.to.dense_index()];
                if mv == winner {
                    assert!(actual > 0, "winner={winner:?}, move={mv:?}");
                } else if penalized
                    .iter()
                    .any(|other| other.from == mv.from && other.to == mv.to)
                {
                    assert!(actual < 0, "winner={winner:?}, move={mv:?}");
                } else {
                    assert_eq!(actual, 0, "winner={winner:?}, move={mv:?}");
                }
            }
        });
    }
    params::set("OrderingCheckBonus", check_bonus).unwrap();
    params::set("OrderingThreatWeight", threat_weight).unwrap();
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
        let static_eval = evaluate(searcher.nnue, &board);
        let lmr_depth = super::super::pruning::pruning_lmr_depth(3, 1, false, false, false);
        assert!(
            super::super::pruning::futility_value(
                searcher.nnue.pawn_value(),
                static_eval,
                beta - 1,
                lmr_depth,
            ) < beta - 1
        );
        searcher.tt.store(
            search_key(&board),
            0,
            0,
            None,
            Bound::Upper,
            Some(tt_move),
            ply,
            false,
        );
        // seek_mateで入口のrazoringを除外し、子は最大手数の静的評価で固定する。
        // 麒麟の成りはfutilityの見積りより大きく評価を上げる。
        searcher.seek_mate = true;
        searcher.null_move_ply = Some(ply);
        set_test_path_ply(searcher, ply);
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
                    assert!((0..=super::super::history::HISTORY_LIMIT).contains(&actual));
                    assert!(actual > 0);
                }
                Some(_) => {
                    assert!((-super::super::history::HISTORY_LIMIT..=0).contains(&actual));
                    assert!(actual < 0);
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

fn stockfish_history_coefficients() -> super::super::ordering::HistoryUpdateCoefficients {
    super::super::ordering::HistoryUpdateCoefficients {
        bonus_slope: 133,
        bonus_base: 81,
        bonus_cap: 1487,
        malus_slope: 968,
        malus_base: 235,
        malus_cap: 2244,
        malus_decay: 921,
        tt_move_bonus: 364,
    }
}

/// Stockfishの固定値から、深さ、置換表の手、親のstatScore、および零窓の各項を検査する。
#[test]
fn history_bonus_and_malus_match_numeric_examples() {
    let coefficients = stockfish_history_coefficients();
    assert_eq!(coefficients.updates(10, false, 0, false, 0).bonus, 1249);
    assert_eq!(coefficients.updates(10, true, 0, false, 0).bonus, 1613);
    assert_eq!(coefficients.updates(10, false, -100, false, 0).bonus, 1246);
    assert_eq!(coefficients.updates(10, false, 0, true, 3).bonus, 1263);
    assert_eq!(coefficients.updates(10, false, 0, false, 0).malus, 2244);
    let shallow = coefficients.updates(1, false, 0, false, 0);
    assert_eq!((shallow.bonus, shallow.malus), (52, 733));

    let signed = super::super::ordering::HistoryUpdateCoefficients {
        bonus_slope: 0,
        bonus_base: 10,
        bonus_cap: 200,
        malus_slope: 0,
        malus_base: 20,
        malus_cap: 200,
        malus_decay: 1024,
        tt_move_bonus: 0,
    }
    .updates(1, false, 0, false, 0);
    assert_eq!((signed.bonus, signed.malus), (-10, -20));
}

/// 静かな手の減点は1159/1024倍した後、各手の直前に921/1024倍する。
#[test]
fn quiet_history_malus_decays_before_each_update() {
    let updates = stockfish_history_coefficients().updates(10, false, 0, false, 0);
    let initial = updates.quiet_malus();
    let first = updates.decay_quiet_malus(initial);
    let second = updates.decay_quiet_malus(first);
    assert_eq!((initial, -first, -second), (2539, -2283, -2053));
}

/// 探索済みの手の列は32手目までのfail-lowだけを保持する。
#[test]
fn searched_move_boundary_requires_no_alpha_improvement() {
    assert!(super::super::negamax::should_record_searched_move(
        32, 10, 10
    ));
    assert!(!super::super::negamax::should_record_searched_move(
        33, 10, 10
    ));
    assert!(!super::super::negamax::should_record_searched_move(
        32, 11, 10
    ));
}

#[test]
fn quiet_history_gravity_numeric_examples() {
    // 上限100の独立した算術例。−51×9/100は−4へ丸め、±100を超える更新量は先に切り詰める。
    for (old, delta, expected) in [
        (0, 9, 9),
        (100, 9, 100),
        (-100, 9, -82),
        (-51, 9, -38),
        (-51, -9, -56),
        (100, -100, -100),
        (-100, 100, 100),
        (0, 1000, 100),
        (0, -1000, -100),
    ] {
        assert_eq!(
            super::super::history::history_gravity(old, delta, 100),
            expected
        );
    }
}
