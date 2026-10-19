//! 捕獲履歴の添字、重力式による更新、駒価値への換算、整列、対局内の持ち越しを検査する。
//! 換算値の絶対値は歩兵価値×尺度%以下に保ち、通常探索では捕獲価値へ加えるが根の整列には加えない。
//! 判断の理由は`docs/adr/bound-capture-history-and-use-it-in-qsearch.md`にある。

use super::*;
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
    let nnue = test_nnue();
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
        let mut history = empty_capture_history();
        history.update(&board, nnue, mv, 9);
        for (victim, expected) in [(PieceKind::Pawn, 0), (PieceKind::Rook, 9)] {
            let single = position(
                Color::Black,
                &[
                    (sq(5, 5), Color::Black, PieceKind::Lion),
                    (sq(6, 6), Color::White, victim),
                ],
            );
            assert_eq!(
                history.read(&single, nnue, capture(mv.from, mv.to)),
                expected
            );
        }
        assert_eq!(history.read(&board, nnue, capture(mv.from, sq(5, 6))), 0);
    }
}

/// 同値の2枚取りでは到達升の駒種を選ぶ。王将と太子の価値は等しい。
#[test]
fn capture_history_equal_victims_prefer_destination() {
    let nnue = test_nnue();
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
    let mut history = empty_capture_history();
    history.update(&board, nnue, mv, 4);
    assert_eq!(history.read(&board, nnue, capture(mv.from, mv.to)), 4);
    let king = position(
        Color::Black,
        &[
            (mv.from, Color::Black, PieceKind::Lion),
            (mv.to, Color::White, PieceKind::King),
        ],
    );
    assert_eq!(history.read(&king, nnue, capture(mv.from, mv.to)), 0);
}

/// 成る手は着手前の駒種を使い、成駒と先後を区別する。
#[test]
fn capture_history_promotion_uses_pre_move_piece_and_color() {
    let nnue = test_nnue();
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
    let mut history = empty_capture_history();
    history.update(&board, nnue, mv, 16);
    assert_eq!(
        history.read(
            &board,
            nnue,
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
        assert_eq!(history.read(&other, nnue, mv), expected);
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
    assert_eq!(test_nnue().pawn_value(), 100, "固定ネットの歩兵価値");
    // 歩兵価値100での上限の換算値は尺度そのものになる。
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
        searcher.capture_history.update(
            &board,
            searcher.nnue,
            winner,
            super::super::capture_history::CAPTURE_HISTORY_LIMIT,
        );
        searcher.capture_history.update(
            &board,
            searcher.nnue,
            captures[0],
            -super::super::capture_history::CAPTURE_HISTORY_LIMIT,
        );
        assert_eq!(
            searcher.capture_history.read(&board, searcher.nnue, winner),
            super::super::capture_history::CAPTURE_HISTORY_LIMIT
        );
        assert_eq!(
            searcher
                .capture_history
                .read(&board, searcher.nnue, captures[0]),
            -super::super::capture_history::CAPTURE_HISTORY_LIMIT
        );
        assert_eq!(
            searcher
                .capture_history
                .adjustment(&board, searcher.nnue, winner),
            params::capture_history_scale()
        );
        assert_eq!(
            searcher
                .capture_history
                .adjustment(&board, searcher.nnue, captures[0]),
            -params::capture_history_scale()
        );
        let after = picked(searcher, &board, None);
        if params::capture_history_scale() == 0 {
            assert_eq!(after, before);
        } else {
            assert_eq!(after[0], (winner, true));
            assert_eq!(
                after[1..4],
                [
                    (captures[1], true),
                    (captures[2], true),
                    (captures[0], true)
                ]
            );
        }
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
        // F5の浅いcutNodeでfail-lowの打ち切りが制限されるため、子の要求深さを5より深く保つ。
        searcher.follow_pv[0] = true;
        searcher.null_move_ply = Some(0);
        let moves = picked(searcher, &board, None);
        let winner_index = if quiet_cutoff { 4 } else { 2 };
        assert_eq!(moves[winner_index].1, !quiet_cutoff);
        // 深さ10の履歴更新だけを検査し、子の値は十分深い記録で固定する。
        for (index, &(mv, _)) in moves.iter().enumerate() {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            let score = if index == winner_index { 20 } else { 0 };
            searcher
                .tt
                .store(search_key(&child), 16, -score, Bound::Exact, None, 1, false);
        }
        assert_eq!(
            searcher.negamax(&mut board.clone(), 10, -10, 10, 0, false),
            Some(20)
        );
        assert_eq!(searcher.nodes, winner_index as u64 + 1);
        for (index, &(mv, is_capture)) in moves.iter().enumerate() {
            if is_capture {
                let expected = if index > winner_index {
                    0
                } else if index == winner_index {
                    1740
                } else if index == 0 {
                    // 最初の手はαを−10から0へ上げたため、探索済みの手の列には入らない。
                    0
                } else {
                    -3263
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

/// 捕獲履歴の更新はbutterfly historyと同じ重力式を使う。
#[test]
fn capture_history_update_uses_shared_gravity() {
    let board = capture_fixture();
    let nnue = test_nnue();
    let mv = capture(sq(5, 5), sq(5, 7));
    let limit = super::super::capture_history::CAPTURE_HISTORY_LIMIT;
    for old in [-limit, -10, 0, 10, limit] {
        for bonus in [-(limit + 1), -9, 0, 9, limit + 1] {
            let mut history = empty_capture_history();
            history.values_mut().fill(old);
            history.update(&board, nnue, mv, bonus);
            assert_eq!(
                history.read(&board, nnue, mv),
                super::super::history::history_gravity(old, bonus, limit)
            );
        }
    }
}

/// 評価値1未満の補正は、正負とも0方向へ丸める。
#[test]
fn capture_history_adjustment_truncates_toward_zero() {
    let board = capture_fixture();
    let nnue = test_nnue();
    let winner = capture(sq(5, 5), sq(5, 7));
    let loser = capture(sq(5, 5), sq(3, 5));
    let mut history = empty_capture_history();
    history.update(&board, nnue, winner, 1);
    history.update(&board, nnue, loser, -1);
    assert!(
        nnue.pawn_value() * params::capture_history_scale()
            < 100 * super::super::capture_history::CAPTURE_HISTORY_LIMIT
    );
    assert_eq!(history.adjustment(&board, nnue, winner), 0);
    assert_eq!(history.adjustment(&board, nnue, loser), 0);
}

/// S3は整列用の駒価値への換算値ではなく、capture historyの生の値を余裕値に使う。
#[test]
fn see_pruning_uses_raw_capture_history_in_margin() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::GoBetween),
            (sq(5, 4), Color::White, PieceKind::Pawn),
            (sq(5, 5), Color::White, PieceKind::Pawn),
            (sq(0, 11), Color::Black, PieceKind::Pawn),
            (sq(0, 5), Color::White, PieceKind::Rook),
        ],
    );
    let target = capture(sq(5, 3), sq(5, 4));
    let tt_move = capture(sq(11, 0), sq(10, 0));
    let nnue = test_nnue();
    let moves = legal_moves(&board);
    assert!(moves.contains(&target));
    assert!(moves.contains(&tt_move));
    assert!(!royal_under_attack(&board));

    let mut penalized = empty_capture_history();
    penalized.update(
        &board,
        nnue,
        target,
        -super::super::capture_history::CAPTURE_HISTORY_LIMIT,
    );
    let raw = penalized.read(&board, nnue, target);
    let adjustment = penalized.adjustment(&board, nnue, target);
    let base_margin = super::super::pruning::see_margin(nnue.pawn_value(), 1, 0);
    let raw_margin = super::super::pruning::see_margin(nnue.pawn_value(), 1, raw);
    let adjusted_margin = super::super::pruning::see_margin(nnue.pawn_value(), 1, adjustment);
    assert!(!see_prunes(
        &board,
        engine_rules(),
        nnue,
        target,
        base_margin
    ));
    assert!(see_prunes(&board, engine_rules(), nnue, target, raw_margin));
    assert!(!see_prunes(
        &board,
        engine_rules(),
        nnue,
        target,
        adjusted_margin
    ));

    let minimum_child_value = moves
        .iter()
        .map(|&mv| {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            -evaluate(nnue, &child)
        })
        .min()
        .unwrap();
    let alpha = minimum_child_value - 1;
    let beta = alpha + 2;
    let run = |penalize: bool| {
        let table = TranspositionTable::new(16).unwrap();
        for &mv in &moves {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            let parent_score = if mv == target { beta } else { alpha - 1 };
            table.store(
                search_key(&child),
                MAX_PLY,
                -parent_score,
                Bound::Exact,
                None,
                1,
                false,
            );
        }
        table.store(
            search_key(&board),
            0,
            0,
            Bound::Upper,
            Some(tt_move),
            0,
            false,
        );
        run_configured_negamax(&board, 1, alpha, beta, 0, &table, |searcher| {
            searcher.follow_pv[0] = true;
            if penalize {
                searcher.capture_history.update(
                    &board,
                    searcher.nnue,
                    target,
                    -super::super::capture_history::CAPTURE_HISTORY_LIMIT,
                );
            }
        });
        table.probe(search_key(&board), 0).unwrap()
    };

    let hit = run(false);
    assert_eq!(hit.best_move, Some(target));
    let hit = run(true);
    assert_ne!(hit.best_move, Some(target));
}

/// SEEで飛ばした捕獲手と先行する静かなTT手は、捕獲打ち切り時にも減点しない。
#[test]
fn capture_history_excludes_see_pruned_captures_and_quiet_tt_move() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(3, 8), Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::FreeKing),
            (sq(5, 4), Color::White, PieceKind::GoldGeneral),
            (sq(5, 5), Color::White, PieceKind::Pawn),
            (sq(3, 3), Color::Black, PieceKind::Rook),
            (sq(3, 4), Color::White, PieceKind::Pawn),
        ],
    );
    let quiet = capture(sq(11, 0), sq(10, 0));
    let pruned = capture(sq(5, 3), sq(5, 4));
    let winner = capture(sq(3, 3), sq(3, 4));
    let ply = MAX_PLY - 1;
    let leaf_value = |mv| {
        let mut child = board.clone();
        child.make_move_unchecked(mv, engine_rules());
        -evaluate(test_nnue(), &child)
    };
    let beta = leaf_value(winner);
    assert!(leaf_value(quiet) < beta);
    with_root_searcher(&board, &[], |searcher| {
        let moves = picked(searcher, &board, Some(quiet));
        assert_eq!(
            &moves[..3],
            &[(quiet, false), (pruned, true), (winner, true)]
        );
        assert!(!royal_under_attack(&board));
        assert!(board.gives_check(&MoveGenerator::standard(), winner));
        let history = searcher.capture_history.read(&board, searcher.nnue, pruned);
        let margin =
            crate::search::alphabeta::pruning::see_margin(searcher.nnue.pawn_value(), 3, history);
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
        searcher.tt.store(
            search_key(&board),
            0,
            0,
            Bound::Upper,
            Some(quiet),
            ply,
            false,
        );
        // seek_mateで入口のrazoringを除外し、子は最大手数の静的評価で固定する。
        searcher.seek_mate = true;
        searcher.null_move_ply = Some(ply);
        set_test_path_ply(searcher, ply);
        assert_eq!(
            searcher.negamax(&mut board.clone(), 3, beta - 1, beta, ply, false),
            Some(beta)
        );
        assert_eq!(searcher.nodes, 2);
        assert_eq!(
            searcher.capture_history.read(&board, searcher.nnue, pruned),
            0
        );
        assert!(searcher.capture_history.read(&board, searcher.nnue, winner) > 0);
    });
}

/// 捕獲手のfutility pruningで飛ばした手は、後続の静かな手の打ち切りでも減点しない。
#[test]
fn capture_history_excludes_capture_futility_pruned_capture() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::Rook),
            (sq(5, 4), Color::White, PieceKind::Pawn),
            (sq(6, 7), Color::Black, PieceKind::Pawn),
        ],
    );
    let tt_move = capture(sq(11, 0), sq(10, 0));
    let pruned = capture(sq(5, 3), sq(5, 4));
    let winner = Move {
        promote: true,
        ..capture(sq(6, 7), sq(6, 8))
    };
    let moves = legal_moves(&board);
    assert!(moves.contains(&tt_move));
    assert!(moves.contains(&pruned));
    assert!(moves.contains(&winner));
    assert!(!royal_under_attack(&board));
    assert!(!board.gives_check(&MoveGenerator::standard(), pruned));

    with_root_searcher(&board, &[], |searcher| {
        searcher.killers[0][0] = Some(winner);
        let ordered = picked(searcher, &board, Some(tt_move));
        assert_eq!(ordered[0], (tt_move, false));
        let pruned_index = ordered.iter().position(|&(mv, _)| mv == pruned).unwrap();
        let winner_index = ordered.iter().position(|&(mv, _)| mv == winner).unwrap();
        assert!(pruned_index < winner_index);

        let depth = 7;
        let static_eval = evaluate(searcher.nnue, &board);
        let lmr_depth =
            super::super::pruning::pruning_lmr_depth(depth, pruned_index, false, false, true);
        assert!(lmr_depth < super::super::pruning::CAPTURE_FUTILITY_LMR_DEPTH_LIMIT);
        let captured_value = board
            .captured_squares(pruned)
            .into_iter()
            .flatten()
            .map(|square| searcher.nnue.piece_value(board.piece_at(square).unwrap()))
            .sum();
        let alpha = super::super::pruning::capture_futility_value(
            searcher.nnue.pawn_value(),
            static_eval,
            lmr_depth,
            captured_value,
            0,
        );
        let beta = alpha + 1;
        assert!(alpha > static_eval);
        assert!(!see_prunes(
            &board,
            engine_rules(),
            searcher.nnue,
            pruned,
            super::super::pruning::see_margin(searcher.nnue.pawn_value(), depth, 0),
        ));

        for &mv in &moves {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            let parent_score = if mv == winner { beta } else { alpha - 1 };
            searcher.tt.store(
                search_key(&child),
                MAX_PLY,
                -parent_score,
                Bound::Exact,
                None,
                1,
                false,
            );
        }
        searcher.tt.store(
            search_key(&board),
            0,
            0,
            Bound::Upper,
            Some(tt_move),
            0,
            true,
        );
        searcher.seek_mate = true;
        searcher.null_move_ply = Some(0);
        assert_eq!(
            searcher.negamax(&mut board.clone(), depth, alpha, beta, 0, false),
            Some(beta)
        );
        assert_eq!(
            searcher.tt.probe(search_key(&board), 0).unwrap().best_move,
            Some(winner)
        );
        assert_eq!(
            searcher.capture_history.read(&board, searcher.nnue, pruned),
            0
        );
    });
}

/// 候補4: 停止済みの探索でも、根の開始時に全ワーカーの履歴を1回だけ減衰する。
fn stopped_search(histories: &mut crate::search::HistoryTables, threads: NonZeroUsize) {
    let snapshot = snapshot_for(&Position::initial());
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
        threads,
        &small_tt(),
        histories,
        None,
        &ManualClock::new(Duration::ZERO),
        &AtomicU64::new(0),
        false,
    );
    assert_eq!(outcome.result.nodes, 0);
}

/// newとclearは全ワーカーの履歴をStockfishと同じ初期値にする。
#[test]
fn history_tables_new_and_clear_use_stockfish_initial_values() {
    let mut histories = crate::search::HistoryTables::new(worker_count(4));
    assert!(
        histories.workers.iter().all(|history| history
            .iter()
            .flatten()
            .flatten()
            .all(|&value| value == -5))
    );
    assert!(
        histories
            .capture
            .iter()
            .all(|history| history.values().iter().all(|&value| value == -742))
    );
    for index in [
        0,
        histories.continuation.values().len() / 2,
        histories.continuation.values().len() - 1,
    ] {
        assert_eq!(
            histories.continuation.values()[index].load(AtomicOrdering::Relaxed),
            -586
        );
    }
    for index in [
        0,
        histories.pawn.values().len() / 2,
        histories.pawn.values().len() - 1,
    ] {
        assert_eq!(
            histories.pawn.values()[index].load(AtomicOrdering::Relaxed),
            -1338
        );
    }
    for history in &mut histories.workers {
        for value in history.iter_mut().flatten().flatten() {
            *value = -123;
        }
    }
    for history in &mut histories.capture {
        history.values_mut().fill(456);
    }
    let pawn_piece = PieceCode::new(Color::Black, PieceKind::Pawn).unwrap();
    let pawn_square = sq(4, 4);
    histories.continuation.store(0, 789);
    histories.pawn.store(0x1234, pawn_piece, pawn_square, 987);
    histories.clear();
    assert_eq!(histories.workers.len(), 4);
    assert_eq!(histories.capture.len(), 4);
    assert!(
        histories.workers.iter().all(|history| history
            .iter()
            .flatten()
            .flatten()
            .all(|&value| value == -5))
    );
    assert!(
        histories
            .capture
            .iter()
            .all(|history| history.values().iter().all(|&value| value == -742))
    );
    assert_eq!(histories.continuation.read(0), -586);
    assert_eq!(histories.pawn.read(0x1234, pawn_piece, pawn_square), -1338);

    for (count, pawn_entries) in [(1, 1024), (2, 2048), (3, 4096), (4, 4096)] {
        let histories = crate::search::HistoryTables::new(worker_count(count));
        assert_eq!(histories.workers.len(), count);
        assert_eq!(histories.capture.len(), count);
        assert_eq!(histories.pawn.entry_count(), pawn_entries);
        assert!(
            histories.workers.iter().all(|history| history
                .iter()
                .flatten()
                .flatten()
                .all(|&value| value == -5))
        );
        assert!(
            histories
                .capture
                .iter()
                .all(|history| history.values().iter().all(|&value| value == -742))
        );
    }
}

/// 探索開始時にワーカー数が変わると、新品の表を作ってから初回の減衰を適用する。
#[test]
fn changed_worker_count_uses_decayed_initial_history() {
    let mut histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
    for count in [4, 2, 1] {
        for history in &mut histories.capture {
            history.values_mut().fill(1000);
        }
        stopped_search(&mut histories, worker_count(count));
        assert_eq!(histories.workers.len(), count);
        assert_eq!(histories.capture.len(), count);
        assert!(
            histories.workers.iter().all(|history| history
                .iter()
                .flatten()
                .flatten()
                .all(|&value| value == -3))
        );
        assert!(
            histories
                .capture
                .iter()
                .all(|history| history.values().iter().all(|&value| value == -452))
        );
    }
}

/// 候補4: 0は全消去、100は全保持し、負値の端数も0方向へ丸める。
#[test]
fn capture_history_decay_boundaries() {
    let mut history = empty_capture_history();
    let values = [7, -7, 100, -100, -5, 1000];
    // 23%を掛けると1.61,−1.61,23,−23,−1.15,230。整数除算は0方向へ丸める。
    for (percent, expected) in [(0, [0; 6]), (23, [1, -1, 23, -23, -1, 230]), (100, values)] {
        for (index, value) in history.values_mut().iter_mut().enumerate() {
            *value = values[index % values.len()];
        }
        history.decay(percent);
        for (index, value) in history.values().iter().enumerate() {
            assert_eq!(*value, expected[index % expected.len()]);
        }
    }
}

/// tuningの隔離プロセスから呼び、他の並列テストへ係数変更を漏らさない。
#[cfg(feature = "tuning")]
pub(super) fn zero_decay_clears_capture_history_between_searches() {
    assert_eq!(params::capture_history_decay(), 0);
    assert_eq!(params::history_decay(), 37, "捕獲履歴だけを消去する");
    for threads in [DEFAULT_THREADS, worker_count(4)] {
        let mut histories = crate::search::HistoryTables::new(threads);
        crate::search::search(
            test_nnue(),
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
                .all(|history| history[0][60][60] == 37)
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

/// 捕獲履歴の加減点は正負の上限を保ち、上限を超える更新量でも端点を越えない。
#[test]
fn capture_history_gravity_stays_bounded_at_limit_extremes() {
    let board = capture_fixture();
    let nnue = test_nnue();
    let winner = capture(sq(5, 5), sq(5, 7));
    let earlier = capture(sq(5, 5), sq(3, 5));
    let limit = super::super::capture_history::CAPTURE_HISTORY_LIMIT;
    let mut history = empty_capture_history();
    for old in [-limit, -limit + 1, -10, 0, 10, limit - 1, limit] {
        for bonus in [0, 1, 9, limit, limit + 1] {
            history.values_mut().fill(old);
            history.update(&board, nnue, winner, bonus);
            history.update(&board, nnue, earlier, -bonus);
            let rewarded = history.read(&board, nnue, winner);
            let penalized = history.read(&board, nnue, earlier);
            assert!((-limit..=limit).contains(&rewarded));
            assert!((-limit..=limit).contains(&penalized));
            if bonus >= limit {
                assert_eq!(rewarded, limit);
                assert_eq!(penalized, -limit);
            }
            if old == -10 && bonus == 9 {
                // |−10×9/limit|<1なので重力項は0。−10+9=−1、−10−9=−19。
                assert_eq!((rewarded, penalized), (-1, -19));
            }
        }
    }
}
