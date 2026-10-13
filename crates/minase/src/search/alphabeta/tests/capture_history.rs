//! 捕獲履歴の添字、重力式による更新、駒価値への換算、整列、対局内の持ち越しを検査する。
//! 換算値の絶対値は歩兵価値×尺度%以下に保ち、通常探索では捕獲価値へ加えるが根の整列には加えない。
//! 判断の理由は`docs/adr/bound-capture-history-and-use-it-in-qsearch.md`にある。

use super::*;
use crate::search::alphabeta::capture_history::CaptureHistory;
use crate::search::alphabeta::params;

fn capture(from: Square, to: Square) -> Move {
    Move {
        from,
        to,
        mid: None,
        promote: false,
    }
}

/// 2枚取りでは価値の大きい駒種を選び、到達升を添字にする。
#[test]
fn capture_history_double_capture_uses_more_valuable_victim() {
    let nnue = weights().unwrap();
    for (mid_kind, to_kind) in [
        (PieceKind::Pawn, PieceKind::Rook),
        (PieceKind::Rook, PieceKind::Pawn),
    ] {
        let board = position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(11, 11), Color::White, PieceKind::King),
                (sq(5, 5), Color::Black, PieceKind::Lion),
                (sq(5, 6), Color::White, mid_kind),
                (sq(6, 6), Color::White, to_kind),
            ],
        );
        let mv = Move {
            mid: Some(sq(5, 6)),
            ..capture(sq(5, 5), sq(6, 6))
        };
        assert!(legal_moves(&board).contains(&mv));
        let mut history = CaptureHistory::new();
        history.record_cutoff(&board, &nnue, mv, &[], 3);
        for (victim, expected) in [(PieceKind::Pawn, 0), (PieceKind::Rook, 9)] {
            let single = position(
                Color::Black,
                &[
                    (sq(5, 5), Color::Black, PieceKind::Lion),
                    (sq(6, 6), Color::White, victim),
                ],
            );
            assert_eq!(
                history.read(&single, &nnue, capture(mv.from, mv.to)),
                expected
            );
        }
        assert_eq!(history.read(&board, &nnue, capture(mv.from, sq(5, 6))), 0);
    }
}

/// 同値の2枚取りでは到達升の駒種を選ぶ。王将と太子の価値は等しい。
#[test]
fn capture_history_equal_victims_prefer_destination() {
    let nnue = weights().unwrap();
    let board = position_with_promoted_pieces(
        Color::Black,
        &[
            (sq(5, 5), Color::Black, PieceKind::Lion, false),
            (sq(5, 6), Color::White, PieceKind::King, false),
            (sq(6, 6), Color::White, PieceKind::CrownPrince, true),
        ],
    );
    assert_eq!(
        nnue.piece_value(board.piece_at(sq(5, 6)).unwrap()),
        nnue.piece_value(board.piece_at(sq(6, 6)).unwrap())
    );
    let mv = Move {
        mid: Some(sq(5, 6)),
        ..capture(sq(5, 5), sq(6, 6))
    };
    let mut history = CaptureHistory::new();
    history.record_cutoff(&board, &nnue, mv, &[], 2);
    assert_eq!(history.read(&board, &nnue, capture(mv.from, mv.to)), 4);
    let king = position(
        Color::Black,
        &[
            (mv.from, Color::Black, PieceKind::Lion),
            (mv.to, Color::White, PieceKind::King),
        ],
    );
    assert_eq!(history.read(&king, &nnue, capture(mv.from, mv.to)), 0);
}

/// 成る手は着手前の駒種を使い、成駒と先後を区別する。
#[test]
fn capture_history_promotion_uses_pre_move_piece_and_color() {
    let nnue = weights().unwrap();
    let mv = Move {
        promote: true,
        ..capture(sq(5, 7), sq(5, 8))
    };
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (mv.from, Color::Black, PieceKind::Rook),
            (mv.to, Color::White, PieceKind::Pawn),
        ],
    );
    assert!(legal_moves(&board).contains(&mv));
    let mut history = CaptureHistory::new();
    history.record_cutoff(&board, &nnue, mv, &[], 4);
    assert_eq!(
        history.read(
            &board,
            &nnue,
            Move {
                promote: false,
                ..mv
            }
        ),
        16
    );
    for (color, kind, promoted, expected) in [
        (Color::Black, PieceKind::Rook, false, 16),
        (Color::White, PieceKind::Rook, false, 0),
        (Color::Black, PieceKind::DragonKing, true, 0),
    ] {
        let other = position_with_promoted_pieces(
            color,
            &[
                (mv.from, color, kind, promoted),
                (mv.to, color.opposite(), PieceKind::Pawn, false),
            ],
        );
        assert_eq!(history.read(&other, &nnue, mv), expected);
    }
}

fn capture_fixture() -> Position {
    position(
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
    )
}

fn picked(searcher: &Searcher<'_>, board: &Position, tt: Option<Move>) -> Vec<(Move, bool)> {
    let mut picker = MovePicker::new(tt, [None; KILLER_COUNT]);
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

/// 非ゼロ履歴は捕獲段だけを変え、TT手を先頭に保ち、合法手の重複を作らない。
#[test]
fn capture_history_changes_picker_but_preserves_root_and_tt_order() {
    let board = capture_fixture();
    with_root_searcher(&board, &[], |searcher| {
        let before = picked(searcher, &board, None);
        let captures: Vec<_> = before
            .iter()
            .filter(|(_, c)| *c)
            .map(|(mv, _)| *mv)
            .collect();
        assert_eq!(captures.len(), 4);
        let winner = captures[3];
        let mut root_before = legal_moves(&board);
        searcher.order_moves(&board, &mut root_before, None, 0);
        searcher
            .capture_history
            .record_cutoff(&board, searcher.nnue, winner, &[captures[0]], 256);
        assert_eq!(
            searcher.capture_history.read(&board, searcher.nnue, winner),
            params::capture_history_limit()
        );
        assert_eq!(
            searcher
                .capture_history
                .read(&board, searcher.nnue, captures[0]),
            -params::capture_history_limit()
        );
        assert_eq!(
            searcher
                .capture_history
                .adjustment(&board, searcher.nnue, winner),
            searcher.nnue.pawn_value() * params::capture_history_scale() / 100
        );
        assert_eq!(
            searcher
                .capture_history
                .adjustment(&board, searcher.nnue, captures[0]),
            -searcher.nnue.pawn_value() * params::capture_history_scale() / 100
        );
        let after = picked(searcher, &board, None);
        assert_eq!(after[0], (winner, true));
        assert_eq!(
            after[1..4],
            [
                (captures[1], true),
                (captures[2], true),
                (captures[0], true)
            ]
        );
        assert_eq!(after[4..], before[4..]);
        let mut root_after = legal_moves(&board);
        searcher.order_moves(&board, &mut root_after, None, 0);
        assert_eq!(root_after, root_before);
        assert_eq!(
            picked(searcher, &board, Some(captures[0]))[0],
            (captures[0], true)
        );
        let unique: std::collections::HashSet<_> = after.iter().map(|&(mv, _)| mv).collect();
        assert_eq!(unique.len(), after.len());
        assert_eq!(unique, legal_moves(&board).into_iter().collect());
    });
}

/// 子局面の既知の値で打ち切り位置を固定し、通常探索による加減点を調べる。
fn assert_capture_history_after_cutoff(quiet_cutoff: bool) {
    let board = capture_fixture();
    with_root_searcher(&board, &[], |searcher| {
        let moves = picked(searcher, &board, None);
        let winner_index = if quiet_cutoff { 4 } else { 2 };
        assert_eq!(moves[winner_index].1, !quiet_cutoff);
        for (index, &(mv, _)) in moves.iter().enumerate() {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            let score = if index == winner_index { 20 } else { 0 };
            searcher
                .tt
                .store(search_key(&child), 1, -score, Bound::Exact, None, 1);
        }
        assert_eq!(
            searcher.negamax(&mut board.clone(), 2, -10, 10, 0),
            Some(20)
        );
        assert_eq!(searcher.nodes, winner_index as u64 + 1);
        for (index, &(mv, is_capture)) in moves.iter().enumerate() {
            if is_capture {
                let expected = if index > winner_index {
                    0
                } else if index == winner_index {
                    4
                } else {
                    -4
                };
                assert_eq!(
                    searcher.capture_history.read(&board, searcher.nnue, mv),
                    expected,
                    "move {index}, quiet={quiet_cutoff}"
                );
            }
        }
    });
}

#[test]
fn capture_history_negamax_updates_only_searched_captures_on_capture_cutoff() {
    assert_capture_history_after_cutoff(false);
}

/// 候補1: 静かな手でβカットしたノードでも、先に探索した捕獲手を減点する。
#[test]
fn capture_history_negamax_penalizes_searched_captures_on_quiet_cutoff() {
    assert_capture_history_after_cutoff(true);
}

/// 候補1: 減点は既存の重力更新式に従い、探索済みの捕獲手だけに適用する。
#[test]
fn capture_history_quiet_cutoff_applies_gravity_only_to_earlier_captures() {
    let board = capture_fixture();
    let nnue = weights().unwrap();
    let earlier = [
        capture(sq(5, 5), sq(5, 7)),
        capture(sq(5, 5), sq(3, 5)),
        capture(sq(5, 5), sq(7, 5)),
    ];
    let untouched = capture(sq(5, 5), sq(5, 3));
    let limit = i64::from(params::capture_history_limit());
    for depth in [0, 3, 64, 256] {
        let mut history = CaptureHistory::new();
        history.record_cutoff(&board, &nnue, earlier[0], &[earlier[1]], 64);
        history.record_cutoff(&board, &nnue, untouched, &[], 3);
        let before = earlier.map(|mv| history.read(&board, &nnue, mv));
        assert_eq!(before, [4096, -4096, 0]);

        history.record_quiet_cutoff(&board, &nnue, &earlier, depth);

        let malus = -i64::from(depth).pow(2).min(limit);
        for (mv, old) in earlier.into_iter().zip(before) {
            let old = i64::from(old);
            let expected = old + malus - old * malus.abs() / limit;
            assert_eq!(
                i64::from(history.read(&board, &nnue, mv)),
                expected,
                "depth={depth}, old={old}"
            );
        }
        assert_eq!(history.read(&board, &nnue, untouched), 9);
    }
}

/// 候補1: 探索済みの捕獲手がなければ、非ゼロの履歴も変えない。
#[test]
fn capture_history_quiet_cutoff_with_no_earlier_captures_preserves_history() {
    let board = capture_fixture();
    let nnue = weights().unwrap();
    let captures: Vec<_> = legal_moves(&board)
        .into_iter()
        .filter(|&mv| board.captured_squares(mv).iter().any(Option::is_some))
        .collect();
    assert_eq!(captures.len(), 4);
    let mut history = CaptureHistory::new();
    history.record_cutoff(&board, &nnue, captures[0], &captures[1..3], 64);
    let before: Vec<_> = captures
        .iter()
        .map(|&mv| history.read(&board, &nnue, mv))
        .collect();

    history.record_quiet_cutoff(&board, &nnue, &[], 256);

    let after: Vec<_> = captures
        .iter()
        .map(|&mv| history.read(&board, &nnue, mv))
        .collect();
    assert_eq!(after, before);
}

/// 別の履歴表を借りる探索ワーカー間では更新が混ざらない。
#[test]
fn capture_history_workers_with_separate_tables_are_independent() {
    let board = capture_fixture();
    let mv = capture(sq(5, 5), sq(5, 7));
    with_root_searcher(&board, &[], |first| {
        first
            .capture_history
            .record_cutoff(&board, first.nnue, mv, &[], 3);
        with_root_searcher(&board, &[], |second| {
            assert_eq!(second.capture_history.read(&board, second.nnue, mv), 0);
        });
        assert_eq!(first.capture_history.read(&board, first.nnue, mv), 9);
    });
}

/// 評価値1未満の補正は、正負とも0方向へ丸める。
#[test]
fn capture_history_adjustment_truncates_toward_zero() {
    let board = capture_fixture();
    let nnue = weights().unwrap();
    let winner = capture(sq(5, 5), sq(5, 7));
    let loser = capture(sq(5, 5), sq(3, 5));
    let mut history = CaptureHistory::new();
    history.record_cutoff(&board, &nnue, winner, &[loser], 1);
    assert!(
        nnue.pawn_value() * params::capture_history_scale() < 100 * params::capture_history_limit()
    );
    assert_eq!(history.adjustment(&board, &nnue, winner), 0);
    assert_eq!(history.adjustment(&board, &nnue, loser), 0);
}

/// SEEで飛ばした捕獲手と先行する静かなTT手は、捕獲打ち切り時にも減点しない。
#[test]
fn capture_history_excludes_see_pruned_captures_and_quiet_tt_move() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::FreeKing),
            (sq(5, 4), Color::White, PieceKind::Rook),
            (sq(5, 5), Color::White, PieceKind::Pawn),
            (sq(3, 3), Color::Black, PieceKind::Pawn),
            (sq(3, 4), Color::White, PieceKind::Pawn),
        ],
    );
    let quiet = capture(sq(11, 0), sq(10, 0));
    let pruned = capture(sq(5, 3), sq(5, 4));
    let winner = capture(sq(3, 3), sq(3, 4));
    with_root_searcher(&board, &[], |searcher| {
        let moves = picked(searcher, &board, Some(quiet));
        assert_eq!(
            &moves[..3],
            &[(quiet, false), (pruned, true), (winner, true)]
        );
        assert!(!royal_under_attack(&board));
        let margin = crate::search::alphabeta::pruning::see_margin(searcher.nnue.pawn_value(), 3);
        assert!(see_prunes(
            &board,
            engine_rules(),
            searcher.nnue,
            pruned,
            margin
        ));
        assert!(!see_prunes(
            &board,
            engine_rules(),
            searcher.nnue,
            winner,
            margin
        ));
        searcher
            .tt
            .store(search_key(&board), 0, 0, Bound::Upper, Some(quiet), 0);
        for (mv, score) in [(quiet, 0), (winner, 10_001)] {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            searcher
                .tt
                .store(search_key(&child), 2, -score, Bound::Exact, None, 1);
        }
        // このテストでは実着手による打ち切りだけを検査する。
        searcher.null_move_ply = Some(0);
        assert_eq!(
            searcher.negamax(&mut board.clone(), 3, 10_000, 10_001, 0),
            Some(10_001)
        );
        assert_eq!(searcher.nodes, 2);
        assert_eq!(
            searcher.capture_history.read(&board, searcher.nnue, pruned),
            0
        );
        assert_eq!(
            searcher.capture_history.read(&board, searcher.nnue, winner),
            9
        );
    });
}

/// 候補4: 停止済みの探索でも、根の開始時に全ワーカーの履歴を1回だけ減衰する。
fn stopped_search(histories: &mut crate::search::HistoryTables, threads: NonZeroUsize) {
    let snapshot = snapshot_for(&Position::initial());
    let outcome = run_search_team(
        &weights().unwrap(),
        &snapshot.position,
        snapshot.rules,
        &snapshot.root_moves,
        &snapshot.history_keys,
        &depth_limits(3),
        &AtomicBool::new(true),
        threads,
        &small_tt(),
        histories,
        None,
        Instant::now(),
        &AtomicU64::new(0),
        false,
    );
    assert_eq!(outcome.result.nodes, 0);
}

#[test]
fn capture_history_carry_ages_every_worker_once_per_go() {
    #[cfg(not(feature = "tuning"))]
    assert_eq!(params::capture_history_decay(), 23);
    let mut histories = crate::search::HistoryTables::new(worker_count(4));
    let mut expected = [-65_536, -100, -7, -4, -1, 0, 1, 4, 7, 100, 65_536];
    for (worker, history) in histories.capture.iter_mut().enumerate() {
        for (index, value) in history.values_mut().iter_mut().enumerate() {
            *value = expected[(index + worker) % expected.len()];
        }
    }
    for _ in 0..2 {
        expected = expected.map(|value| value * params::capture_history_decay() / 100);
        stopped_search(&mut histories, worker_count(4));
        for (worker, history) in histories.capture.iter().enumerate() {
            for (index, value) in history.values().iter().enumerate() {
                assert_eq!(*value, expected[(index + worker) % expected.len()]);
            }
        }
    }
}

/// 初期局面から深さ3では王が到達できない捕獲升を印にし、反復中の再減衰も検出する。
#[test]
fn capture_history_carry_survives_handle_round_trips() {
    let board = position(
        Color::Black,
        &[
            (sq(5, 5), Color::Black, PieceKind::King),
            (sq(5, 6), Color::White, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    let mv = capture(sq(5, 5), sq(5, 6));
    let nnue = weights().unwrap();
    for threads in [DEFAULT_THREADS, worker_count(4)] {
        let mut histories = crate::search::HistoryTables::new(threads);
        let mut expected = Vec::new();
        for (worker, history) in histories.capture.iter_mut().enumerate() {
            let depth = 30 + worker as u32;
            history.record_cutoff(&board, &nnue, mv, &[], depth);
            expected.push((depth * depth) as i32);
        }
        let mut table = small_tt();
        for search_id in [901, 902] {
            for value in &mut expected {
                *value = *value * params::capture_history_decay() / 100;
            }
            let handle = crate::search::start_search(
                weights().unwrap(),
                snapshot_for(&Position::initial()),
                depth_limits(3),
                search_id,
                threads,
                table,
                histories,
                false,
            );
            let (_, result) = event_reports(drain_raw(&handle));
            assert_eq!(result.depth, 3);
            (table, histories) = handle.join().unwrap();
            for (worker, history) in histories.capture.iter().enumerate() {
                assert_eq!(history.read(&board, &nnue, mv), expected[worker]);
            }
        }
    }
}

/// 候補4: clearは両方の履歴を全消去し、ワーカー数を保つ。
#[test]
fn history_tables_clear_resets_butterfly_and_capture_history() {
    let mut histories = crate::search::HistoryTables::new(worker_count(4));
    for history in &mut histories.workers {
        for value in history.iter_mut().flatten().flatten() {
            *value = -123;
        }
    }
    for history in &mut histories.capture {
        history.values_mut().fill(456);
    }
    histories.clear();
    assert_eq!(histories.workers.len(), 4);
    assert_eq!(histories.capture.len(), 4);
    assert!(
        histories.workers.iter().all(|history| history
            .iter()
            .flatten()
            .flatten()
            .all(|&value| value == 0))
    );
    assert!(
        histories
            .capture
            .iter()
            .all(|history| history.values().iter().all(|&value| value == 0))
    );
}

/// 候補4: ワーカー数が増減すると、持ち越し表を新品に置き換える。
#[test]
fn changed_worker_count_resets_capture_history() {
    let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
    for count in [4, 2, 1] {
        for history in &mut histories.capture {
            history.values_mut().fill(1000);
        }
        stopped_search(&mut histories, worker_count(count));
        assert_eq!(histories.capture.len(), count);
        assert!(
            histories
                .capture
                .iter()
                .all(|history| history.values().iter().all(|&value| value == 0))
        );
    }
}

/// 候補4: 0は全消去、100は全保持し、負値の端数も0方向へ丸める。
#[test]
fn capture_history_decay_boundaries() {
    let mut history = CaptureHistory::new();
    let values = [-65_536, -7, -1, 0, 1, 7, 65_536];
    for percent in [0, 23, 100] {
        for (index, value) in history.values_mut().iter_mut().enumerate() {
            *value = values[index % values.len()];
        }
        history.decay(percent);
        for (index, value) in history.values().iter().enumerate() {
            assert_eq!(*value, values[index % values.len()] * percent / 100);
        }
    }
}

/// tuningの隔離プロセスから呼び、他の並列テストへ係数変更を漏らさない。
#[cfg(feature = "tuning")]
pub(super) fn zero_decay_clears_capture_history_between_searches() {
    assert_eq!(params::capture_history_decay(), 0);
    assert_eq!(params::history_decay(), 23, "捕獲履歴だけを消去する");
    for threads in [DEFAULT_THREADS, worker_count(4)] {
        let mut histories = crate::search::HistoryTables::new(threads);
        crate::search::search(
            &weights().unwrap(),
            &snapshot_for(&quiet_midgame()),
            &depth_limits(3),
            threads,
            &mut small_tt(),
            &mut histories,
        )
        .unwrap();
        assert!(
            histories
                .capture
                .iter()
                .any(|history| history.values().iter().any(|&value| value != 0))
        );
        for history in &mut histories.workers {
            history[0][60][60] = 100;
        }
        stopped_search(&mut histories, threads);
        assert!(
            histories
                .capture
                .iter()
                .all(|history| history.values().iter().all(|&value| value == 0))
        );
        assert!(
            histories
                .workers
                .iter()
                .all(|history| history[0][60][60] == 23)
        );
    }
}

#[cfg(feature = "tuning")]
pub(super) fn capture_decay_probe() -> i64 {
    let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
    histories.capture[0].values_mut()[0] = 100;
    stopped_search(&mut histories, DEFAULT_THREADS);
    i64::from(histories.capture[0].values()[0])
}
