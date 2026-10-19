//! M3aで加えるcontinuation、pawn、およびlow ply historyとstatScoreの数値例。

use super::*;
use crate::search::alphabeta::history::{
    CONTINUATION_HISTORY_LIMIT, LOW_PLY_HISTORY_INITIAL, LOW_PLY_HISTORY_LIMIT, PAWN_HISTORY_LIMIT,
};
use crate::search::alphabeta::ordering::{
    FailLowHistoryInput, HistoryUpdateCoefficients, capture_stat_score, continuation_index,
    eval_difference_history_updates, fail_low_history_updates, quiet_stat_score,
    refutation_continuation_update,
};

fn record(
    color: Color,
    kind: PieceKind,
    from: Square,
    to: Square,
    captured: Option<PieceKind>,
) -> MoveRecord {
    MoveRecord {
        mv: Move {
            from,
            mid: None,
            to,
            promote: false,
        },
        piece: PieceCode::new(color, kind).unwrap(),
        captured,
    }
}

fn set_contexts(
    searcher: &mut Searcher<'_>,
    records: impl IntoIterator<Item = Option<MoveRecord>>,
) {
    searcher.path_moves = records
        .into_iter()
        .map(|record| PathMove {
            record,
            attacking: None,
        })
        .collect();
    searcher.path_keys = vec![0; searcher.path_moves.len() + 1];
}

fn rotated(square: Square) -> Square {
    Square::from_dense(BOARD_SQUARE_COUNT - 1 - square.dense_index()).unwrap()
}

#[test]
fn continuation_history_index_uses_rotation_parity_capture_and_pre_move_kind() {
    let context = record(Color::Black, PieceKind::Rook, sq(1, 1), sq(2, 3), None);
    let current_piece = PieceCode::new(Color::Black, PieceKind::Pawn).unwrap();
    let current = Move {
        from: sq(4, 4),
        mid: None,
        to: sq(5, 6),
        promote: true,
    };
    let black = continuation_index(context, current_piece, current, 1);

    let white_context = MoveRecord {
        mv: Move {
            from: rotated(context.mv.from),
            to: rotated(context.mv.to),
            ..context.mv
        },
        piece: PieceCode::new(Color::White, PieceKind::Rook).unwrap(),
        ..context
    };
    let white_current = Move {
        from: rotated(current.from),
        to: rotated(current.to),
        ..current
    };
    let white = continuation_index(
        white_context,
        PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
        white_current,
        1,
    );
    assert_eq!(black, white, "後手の両方の到達升を180度回転する");
    assert_ne!(
        black,
        continuation_index(context, current_piece, current, 2),
        "奇数手前と偶数手前は別の区画を引く"
    );
    assert_ne!(
        black,
        continuation_index(
            MoveRecord {
                captured: Some(PieceKind::Pawn),
                ..context
            },
            current_piece,
            current,
            1,
        ),
        "捕獲した文脈の手は別の区画を引く"
    );
    assert_eq!(
        black,
        continuation_index(
            context,
            current_piece,
            Move {
                promote: false,
                ..current
            },
            1,
        ),
        "成る手も着手前の駒種で引く"
    );
    assert_ne!(
        black,
        continuation_index(
            context,
            PieceCode::new(Color::Black, PieceKind::GoldGeneral).unwrap(),
            current,
            1,
        )
    );

    with_root_searcher(&bare_kings(), &[], |searcher| {
        searcher.continuation_history.store(black, 321);
        searcher
            .continuation_history
            .store(continuation_index(context, current_piece, current, 2), -654);
        assert_eq!(searcher.continuation_history.read(black), 321);
        assert_eq!(
            searcher.continuation_history.read(continuation_index(
                context,
                current_piece,
                current,
                2
            )),
            -654
        );
    });
}

#[test]
fn continuation_history_skips_missing_contexts_and_null_moves_independently() {
    let current_piece = PieceCode::new(Color::Black, PieceKind::Pawn).unwrap();
    let current = step(sq(4, 4), sq(4, 5));
    let recent = record(
        Color::White,
        PieceKind::CopperGeneral,
        sq(3, 3),
        sq(3, 4),
        None,
    );
    let older = record(
        Color::Black,
        PieceKind::Rook,
        sq(2, 2),
        sq(2, 3),
        Some(PieceKind::Pawn),
    );
    let absent = record(Color::White, PieceKind::Bishop, sq(7, 7), sq(7, 8), None);

    with_root_searcher(&bare_kings(), &[], |searcher| {
        let root_probe = continuation_index(recent, current_piece, current, 1);
        searcher.continuation_history.store(root_probe, 417);
        searcher.update_continuation(0, current_piece, current, 1000);
        assert_eq!(searcher.continuation_history.read(root_probe), 417);

        set_contexts(searcher, [Some(recent)]);
        let one = continuation_index(recent, current_piece, current, 1);
        let missing_two = continuation_index(absent, current_piece, current, 2);
        searcher.continuation_history.store(one, 0);
        searcher.continuation_history.store(missing_two, 283);
        searcher.update_continuation(1, current_piece, current, 1000);
        assert_ne!(searcher.continuation_history.read(one), 0);
        assert_eq!(searcher.continuation_history.read(missing_two), 283);

        set_contexts(searcher, [Some(older), None]);
        let null_one = continuation_index(absent, current_piece, current, 1);
        let present_two = continuation_index(older, current_piece, current, 2);
        searcher.continuation_history.store(null_one, 359);
        searcher.continuation_history.store(present_two, 0);
        searcher.update_continuation(2, current_piece, current, 1000);
        assert_eq!(searcher.continuation_history.read(null_one), 359);
        assert_ne!(searcher.continuation_history.read(present_two), 0);
    });
}

#[test]
fn continuation_and_quiet_history_updates_match_numeric_examples() {
    let current_piece = PieceCode::new(Color::Black, PieceKind::Pawn).unwrap();
    let current = step(sq(4, 4), sq(4, 5));
    let contexts = [
        record(Color::Black, PieceKind::Lance, sq(0, 0), sq(0, 1), None),
        record(
            Color::White,
            PieceKind::SilverGeneral,
            sq(1, 1),
            sq(1, 2),
            None,
        ),
        record(Color::Black, PieceKind::Bishop, sq(2, 2), sq(2, 3), None),
        record(
            Color::White,
            PieceKind::CopperGeneral,
            sq(3, 3),
            sq(3, 4),
            None,
        ),
        record(Color::Black, PieceKind::Rook, sq(5, 5), sq(5, 6), None),
        record(
            Color::White,
            PieceKind::GoldGeneral,
            sq(6, 6),
            sq(6, 7),
            None,
        ),
    ];

    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (current.from, Color::Black, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    with_root_searcher(&board, &[], |searcher| {
        assert!(
            searcher
                .low_ply_history
                .iter()
                .flatten()
                .flatten()
                .all(|&value| value == LOW_PLY_HISTORY_INITIAL)
        );
        set_contexts(searcher, contexts.map(Some));
        let indices = [1, 2, 3, 4, 6].map(|distance| {
            continuation_index(contexts[6 - distance], current_piece, current, distance)
        });
        for (&index, value) in indices.iter().zip([1, -1, 2, -2, 3]) {
            searcher.continuation_history.store(index, value);
        }
        searcher.update_continuation(6, current_piece, current, 1000);
        let actual = indices.map(|index| searcher.continuation_history.read(index));
        assert_eq!(actual, [891, 611, 245, 419, 341]);

        searcher.continuation_history.store(indices[0], 0);
        searcher.update_continuation(6, current_piece, current, 0);
        assert_eq!(searcher.continuation_history.read(indices[0]), 73);

        set_contexts(searcher, contexts[..4].iter().copied().map(Some));
        searcher.history[Color::Black.index()][current.from.dense_index()]
            [current.to.dense_index()] = 0;
        searcher.low_ply_history[4][current.from.dense_index()][current.to.dense_index()] = 0;
        let continuation = continuation_index(contexts[3], current_piece, current, 1);
        searcher.continuation_history.store(continuation, 0);
        searcher.pawn_keys[4] = 0x1234;
        searcher
            .pawn_history
            .store(searcher.pawn_keys[4], current_piece, current.to, 0);
        searcher.update_quiet_histories(4, current_piece, current, 1000);
        assert_eq!(
            searcher.history[Color::Black.index()][current.from.dense_index()]
                [current.to.dense_index()],
            1000
        );
        assert_eq!(
            searcher.low_ply_history[4][current.from.dense_index()][current.to.dense_index()],
            695
        );
        assert_eq!(searcher.continuation_history.read(continuation), 618);
        assert_eq!(
            searcher
                .pawn_history
                .read(searcher.pawn_keys[4], current_piece, current.to),
            1078
        );

        let negative = step(sq(7, 7), sq(7, 8));
        searcher.history[Color::Black.index()][negative.from.dense_index()]
            [negative.to.dense_index()] = 0;
        searcher.low_ply_history[4][negative.from.dense_index()][negative.to.dense_index()] = 0;
        let negative_continuation = continuation_index(contexts[3], current_piece, negative, 1);
        searcher
            .continuation_history
            .store(negative_continuation, 0);
        searcher
            .pawn_history
            .store(searcher.pawn_keys[4], current_piece, negative.to, 0);
        searcher.update_quiet_histories(4, current_piece, negative, -1000);
        assert_eq!(
            searcher.low_ply_history[4][negative.from.dense_index()][negative.to.dense_index()],
            -695
        );
        assert_eq!(
            searcher.continuation_history.read(negative_continuation),
            -472
        );
        assert_eq!(
            searcher
                .pawn_history
                .read(searcher.pawn_keys[4], current_piece, negative.to),
            -448
        );

        set_contexts(searcher, contexts[..5].iter().copied().map(Some));
        let deep = step(sq(8, 8), sq(8, 9));
        searcher.low_ply_history[4][deep.from.dense_index()][deep.to.dense_index()] = 271;
        searcher.pawn_keys[5] = 0x5678;
        searcher.update_quiet_histories(5, current_piece, deep, 1000);
        assert_eq!(
            searcher.low_ply_history[4][deep.from.dense_index()][deep.to.dense_index()],
            271
        );

        set_contexts(searcher, [Some(contexts[3])]);
        searcher.history[Color::Black.index()][current.from.dense_index()]
            [current.to.dense_index()] = 0;
        searcher.low_ply_history[1][current.from.dense_index()][current.to.dense_index()] = 0;
        let cutoff_continuation = continuation_index(contexts[3], current_piece, current, 1);
        searcher.continuation_history.store(cutoff_continuation, 0);
        searcher.pawn_keys[1] = 0xabcd;
        searcher
            .pawn_history
            .store(searcher.pawn_keys[1], current_piece, current.to, 0);
        let updates = HistoryUpdateCoefficients {
            bonus_slope: 1024,
            bonus_base: 0,
            bonus_cap: 1024,
            malus_slope: 0,
            malus_base: 0,
            malus_cap: 0,
            malus_decay: 1024,
            tt_move_bonus: 0,
        }
        .updates(1, false, 0, false, 0);
        searcher.update_best_move_histories(&board, current, false, &[], &[], updates);
        assert_eq!(
            searcher.history[Color::Black.index()][current.from.dense_index()]
                [current.to.dense_index()],
            899
        );
        assert_eq!(
            searcher.low_ply_history[1][current.from.dense_index()][current.to.dense_index()],
            625
        );
        assert_eq!(searcher.continuation_history.read(cutoff_continuation), 563);
        assert_eq!(
            searcher
                .pawn_history
                .read(searcher.pawn_keys[1], current_piece, current.to),
            969
        );
    });
}

#[test]
fn negamax_and_root_record_stat_score_before_searching_each_move() {
    let board = Position::initial();
    with_root_searcher(&board, &[], |searcher| {
        let chosen = legal_moves(&board)
            .into_iter()
            .find(|&mv| board.captured_squares(mv).iter().all(Option::is_none))
            .unwrap();
        searcher.history[board.side_to_move().index()][chosen.from.dense_index()]
            [chosen.to.dense_index()] = 4000;
        searcher.tt.store(
            search_key(&board),
            0,
            0,
            Bound::Upper,
            Some(chosen),
            0,
            false,
        );
        let expected = searcher.move_stat_score(&board, chosen, false, 0);
        let beta = -crate::search::REPETITION_BAND_START;
        assert!(
            searcher
                .negamax(&mut board.clone(), 1, beta - 1, beta, 0, false)
                .is_some()
        );
        assert_eq!(searcher.stat_scores[0], expected);

        let expected = searcher.move_stat_score(&board, chosen, false, 0);
        assert!(
            searcher
                .search_root(&board, &[chosen], 1, -INFINITY, INFINITY)
                .is_some()
        );
        assert_eq!(searcher.stat_scores[0], expected);
    });
}

#[test]
fn history_form_limits_and_stat_score_numeric_examples() {
    assert_eq!(quiet_stat_score(100, -50, 25), 191);
    assert_eq!(capture_stat_score(24 + 24, 100, 17), 692);

    let context = record(Color::White, PieceKind::Rook, sq(3, 3), sq(3, 4), None);
    let piece = PieceCode::new(Color::Black, PieceKind::Pawn).unwrap();
    let positive = step(sq(4, 4), sq(4, 5));
    let negative = step(sq(5, 5), sq(5, 6));
    with_root_searcher(&bare_kings(), &[], |searcher| {
        set_contexts(searcher, [Some(context)]);
        searcher.pawn_keys[1] = 0x9876;
        for _ in 0..4 {
            searcher.update_quiet_histories(1, piece, positive, 100_000);
            searcher.update_quiet_histories(1, piece, negative, -100_000);
        }
        let positive_continuation = continuation_index(context, piece, positive, 1);
        let negative_continuation = continuation_index(context, piece, negative, 1);
        for value in [
            searcher.continuation_history.read(positive_continuation),
            searcher.continuation_history.read(negative_continuation),
        ] {
            assert!((-CONTINUATION_HISTORY_LIMIT..=CONTINUATION_HISTORY_LIMIT).contains(&value));
        }
        for value in [
            searcher
                .pawn_history
                .read(searcher.pawn_keys[1], piece, positive.to),
            searcher
                .pawn_history
                .read(searcher.pawn_keys[1], piece, negative.to),
        ] {
            assert!((-PAWN_HISTORY_LIMIT..=PAWN_HISTORY_LIMIT).contains(&value));
        }
        for value in [
            searcher.low_ply_history[1][positive.from.dense_index()][positive.to.dense_index()],
            searcher.low_ply_history[1][negative.from.dense_index()][negative.to.dense_index()],
        ] {
            assert!((-LOW_PLY_HISTORY_LIMIT..=LOW_PLY_HISTORY_LIMIT).contains(&i32::from(value)));
        }
    });
}

#[test]
fn fail_low_history_updates_match_numeric_examples() {
    let input = FailLowHistoryInput {
        depth: 4,
        best_score: 0,
        static_eval: 200,
        previous_static_eval: -100,
        previous_stat_score: 196,
        previous_move_count: 9,
        pawn_value: 208,
    };
    let nine = fail_low_history_updates(input, 215);
    assert_eq!(
        nine,
        super::super::ordering::FailLowHistoryUpdates {
            bonus_scale: 294,
            scaled_bonus: 151_410,
            continuation: 2430,
            butterfly: 993,
            pawn: 5988,
        }
    );

    let ten = fail_low_history_updates(
        FailLowHistoryInput {
            previous_move_count: 10,
            ..input
        },
        215,
    );
    assert_eq!((ten.bonus_scale, ten.scaled_bonus), (480, 247_200));

    let zero = fail_low_history_updates(
        FailLowHistoryInput {
            depth: 1,
            best_score: 1000,
            static_eval: 0,
            previous_static_eval: 0,
            previous_stat_score: 0,
            previous_move_count: 0,
            pawn_value: 208,
        },
        215,
    );
    assert_eq!(
        zero,
        super::super::ordering::FailLowHistoryUpdates {
            bonus_scale: 0,
            scaled_bonus: 0,
            continuation: 0,
            butterfly: 0,
            pawn: 0,
        }
    );
}

#[test]
fn eval_difference_history_updates_match_clamp_examples() {
    let lower = eval_difference_history_updates(300, 200, 208, 11);
    assert_eq!(
        (lower.delta, lower.butterfly, lower.pawn),
        (-129, -1419, -1677)
    );

    let upper = eval_difference_history_updates(-300, -200, 208, 11);
    assert_eq!(
        (upper.delta, upper.butterfly, upper.pawn),
        (254, 2794, 3302)
    );

    let middle = eval_difference_history_updates(4, 6, 208, 11);
    assert_eq!(
        (middle.delta, middle.butterfly, middle.pawn),
        (50, 550, 650)
    );
    assert_eq!(refutation_continuation_update(2244), -1562);
}

#[test]
fn tt_cutoff_history_updates_obey_move_and_parent_conditions() {
    let board = staged_picker_fixture();
    let quiet = legal_moves(&board)
        .into_iter()
        .find(|&mv| board.captured_squares(mv).iter().all(Option::is_none))
        .unwrap();
    let capture = legal_moves(&board)
        .into_iter()
        .find(|&mv| board.captured_squares(mv).iter().any(Option::is_some))
        .unwrap();
    let illegal = step(sq(1, 1), sq(1, 2));
    assert!(!legal_moves(&board).contains(&illegal));

    for (alpha, beta, bound, score) in
        [(99, 100, Bound::Lower, 150), (-100, 100, Bound::Exact, 150)]
    {
        with_root_searcher(&board, &[], |searcher| {
            searcher
                .tt
                .store(search_key(&board), 4, score, bound, Some(quiet), 0, false);
            assert_eq!(
                searcher.negamax(&mut board.clone(), 3, alpha, beta, 0, alpha == 99),
                Some(score)
            );
            assert!(
                searcher.history[board.side_to_move().index()][quiet.from.dense_index()]
                    [quiet.to.dense_index()]
                    > 0
            );
        });
    }

    with_root_searcher(&board, &[], |searcher| {
        searcher.tt.store(
            search_key(&board),
            4,
            150,
            Bound::Lower,
            Some(illegal),
            0,
            false,
        );
        assert_eq!(
            searcher.negamax(&mut board.clone(), 3, 99, 100, 0, true),
            Some(150)
        );
        assert!(
            searcher
                .history
                .iter()
                .flatten()
                .flatten()
                .all(|&value| value == 0)
        );
    });

    let context = record(
        Color::Black,
        PieceKind::GoldGeneral,
        sq(2, 2),
        sq(2, 3),
        None,
    );
    let previous = record(
        Color::White,
        PieceKind::SilverGeneral,
        sq(3, 3),
        sq(3, 4),
        None,
    );
    for (score, bound, cut_node, move_count, changed) in [
        (150, Bound::Lower, true, 4, true),
        (99, Bound::Upper, false, 4, false),
        (150, Bound::Lower, true, 5, false),
    ] {
        with_root_searcher(&board, &[], |searcher| {
            set_contexts(searcher, [Some(context), Some(previous)]);
            searcher.accumulators[2] = searcher.nnue.refresh_accumulator(&board);
            searcher.move_counts[1] = move_count;
            let continuation = continuation_index(context, previous.piece, previous.mv, 1);
            searcher.continuation_history.store(continuation, 0);
            searcher.tt.store(
                search_key(&board),
                if score >= 100 { 4 } else { 3 },
                score,
                bound,
                Some(capture),
                2,
                false,
            );
            assert_eq!(
                searcher.negamax(&mut board.clone(), 3, 99, 100, 2, cut_node),
                Some(score)
            );
            let actual = searcher.continuation_history.read(continuation);
            assert_eq!(
                actual < 0,
                changed,
                "score={score}, move_count={move_count}"
            );
        });
    }
}

#[cfg(feature = "tuning")]
pub(super) fn fail_low_and_eval_difference_update_recorded_histories() {
    let board = bare_kings();
    let context = record(Color::Black, PieceKind::Rook, sq(2, 2), sq(2, 3), None);
    let quiet = record(
        Color::White,
        PieceKind::GoldGeneral,
        sq(3, 3),
        sq(3, 4),
        None,
    );
    with_root_searcher(&board, &[], |searcher| {
        set_contexts(searcher, [Some(context), Some(quiet)]);
        let pawn = searcher.nnue.pawn_value();
        searcher.static_evals[1] = -pawn;
        searcher.stat_scores[1] = 0;
        searcher.move_counts[1] = 10;
        searcher.pawn_keys[2] = 0x1234;
        let continuation = continuation_index(context, quiet.piece, quiet.mv, 1);
        searcher.continuation_history.store(continuation, 0);
        searcher.history[Color::White.index()][quiet.mv.from.dense_index()]
            [quiet.mv.to.dense_index()] = 0;
        searcher
            .pawn_history
            .store(searcher.pawn_keys[2], quiet.piece, quiet.mv.to, 0);

        searcher.update_fail_low_histories(2, 4, 0, pawn);
        assert!(searcher.continuation_history.read(continuation) > 0);
        assert!(
            searcher.history[Color::White.index()][quiet.mv.from.dense_index()]
                [quiet.mv.to.dense_index()]
                > 0
        );
        assert!(
            searcher
                .pawn_history
                .read(searcher.pawn_keys[2], quiet.piece, quiet.mv.to)
                > 0
        );

        let eval_quiet = record(
            Color::White,
            PieceKind::CopperGeneral,
            sq(4, 4),
            sq(4, 5),
            None,
        );
        set_contexts(searcher, [Some(context), Some(eval_quiet)]);
        searcher.static_evals[1] = -pawn;
        searcher.pawn_keys[2] = 0x5678;
        searcher.history[Color::White.index()][eval_quiet.mv.from.dense_index()]
            [eval_quiet.mv.to.dense_index()] = 0;
        searcher
            .pawn_history
            .store(searcher.pawn_keys[2], eval_quiet.piece, eval_quiet.mv.to, 0);
        searcher.update_eval_difference_histories(2, 0, false);
        assert!(
            searcher.history[Color::White.index()][eval_quiet.mv.from.dense_index()]
                [eval_quiet.mv.to.dense_index()]
                > 0
        );
        assert!(
            searcher
                .pawn_history
                .read(searcher.pawn_keys[2], eval_quiet.piece, eval_quiet.mv.to)
                > 0
        );

        let tt_hit = record(
            Color::White,
            PieceKind::CopperGeneral,
            sq(5, 5),
            sq(5, 6),
            None,
        );
        set_contexts(searcher, [Some(context), Some(tt_hit)]);
        searcher.static_evals[1] = -pawn;
        searcher.pawn_keys[2] = 0x9abc;
        searcher
            .pawn_history
            .store(searcher.pawn_keys[2], tt_hit.piece, tt_hit.mv.to, 0);
        searcher.update_eval_difference_histories(2, 0, true);
        assert_eq!(
            searcher
                .pawn_history
                .read(searcher.pawn_keys[2], tt_hit.piece, tt_hit.mv.to),
            0
        );
    });
}

#[test]
fn fail_low_updates_recorded_capture_history() {
    let context = record(Color::Black, PieceKind::Rook, sq(2, 2), sq(2, 3), None);
    let capture_board = staged_picker_fixture();
    let capture = legal_moves(&capture_board)
        .into_iter()
        .find(|&mv| {
            capture_board
                .captured_squares(mv)
                .iter()
                .any(Option::is_some)
        })
        .unwrap();
    let mut after_capture = capture_board.clone();
    let undo = after_capture.make_move_unchecked(capture, engine_rules());
    let captured = MoveRecord::from_undo(test_nnue(), &undo);
    with_root_searcher(&capture_board, &[], |searcher| {
        set_contexts(searcher, [Some(context), Some(captured)]);
        assert_eq!(searcher.capture_history.read_record(captured), 0);
        searcher.update_fail_low_histories(2, 4, 0, 0);
        assert_eq!(searcher.capture_history.read_record(captured), 892);
        assert_eq!(
            searcher
                .capture_history
                .read(&capture_board, searcher.nnue, capture),
            892
        );
    });
}

#[test]
fn previous_quiet_updates_skip_root_null_and_capture() {
    let board = staged_picker_fixture();
    let tt_capture = legal_moves(&board)
        .into_iter()
        .find(|&mv| board.captured_squares(mv).iter().any(Option::is_some))
        .unwrap();
    let older = record(Color::Black, PieceKind::Rook, sq(2, 2), sq(2, 3), None);
    let quiet = record(
        Color::White,
        PieceKind::GoldGeneral,
        sq(3, 3),
        sq(3, 4),
        None,
    );
    let captured = MoveRecord {
        captured: Some(PieceKind::Pawn),
        ..quiet
    };

    for (records, probe, ply) in [
        (vec![], quiet, 0),
        (vec![Some(older), Some(quiet), None], quiet, 3),
        (vec![Some(older), Some(captured)], captured, 2),
    ] {
        with_root_searcher(&board, &[], |searcher| {
            set_contexts(searcher, records);
            let context = continuation_index(older, probe.piece, probe.mv, 1);
            searcher.continuation_history.store(context, 0);
            searcher.history[Color::White.index()][probe.mv.from.dense_index()]
                [probe.mv.to.dense_index()] = 0;
            searcher.pawn_keys[ply as usize] = 0x2468;
            searcher.pawn_history.store(
                searcher.pawn_keys[ply as usize],
                probe.piece,
                probe.mv.to,
                0,
            );
            if ply > 0 {
                searcher.move_counts[(ply - 1) as usize] = 1;
                searcher.tt_hits[(ply - 1) as usize] = false;
                searcher.static_evals[(ply - 1) as usize] = 0;
            }

            searcher.update_tt_cutoff_histories(&board, ply, 3, tt_capture, Some(true));
            searcher.update_eval_difference_histories(ply, 0, false);
            searcher.update_refutation_history(ply, 1000);
            searcher.update_fail_low_histories(ply, 4, 0, 0);

            assert_eq!(searcher.continuation_history.read(context), 0);
            assert_eq!(
                searcher.history[Color::White.index()][probe.mv.from.dense_index()]
                    [probe.mv.to.dense_index()],
                0
            );
            assert_eq!(
                searcher.pawn_history.read(
                    searcher.pawn_keys[ply as usize],
                    probe.piece,
                    probe.mv.to
                ),
                0
            );
        });
    }
}

#[test]
fn refutation_penalty_requires_previous_node_move_count() {
    let board = bare_kings();
    let context = record(Color::Black, PieceKind::Rook, sq(2, 2), sq(2, 3), None);
    let previous = record(
        Color::White,
        PieceKind::GoldGeneral,
        sq(3, 3),
        sq(3, 4),
        None,
    );
    for (move_count, tt_hit, changed) in [
        (1, false, true),
        (2, true, true),
        (2, false, false),
        (1, true, false),
    ] {
        with_root_searcher(&board, &[], |searcher| {
            set_contexts(searcher, [Some(context), Some(previous)]);
            searcher.move_counts[1] = move_count;
            searcher.tt_hits[1] = tt_hit;
            let index = continuation_index(context, previous.piece, previous.mv, 1);
            searcher.continuation_history.store(index, 0);
            searcher.update_refutation_history(2, 2244);
            assert_eq!(
                searcher.continuation_history.read(index) < 0,
                changed,
                "move_count={move_count}, tt_hit={tt_hit}"
            );
        });
    }
}

#[test]
fn pv_alpha_improvement_updates_history_without_killer() {
    let board = bare_kings();
    let moves = legal_moves(&board);
    let best = moves[0];
    with_root_searcher(&board, &[], |searcher| {
        searcher
            .tt
            .store(search_key(&board), 0, 0, Bound::Upper, Some(best), 0, false);
        for &mv in &moves {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            searcher
                .tt
                .store(search_key(&child), 4, 0, Bound::Exact, None, 1, false);
        }
        assert_eq!(
            searcher.negamax(&mut board.clone(), 2, -10, 10, 0, false),
            Some(0)
        );
        assert!(
            searcher.history[Color::Black.index()][best.from.dense_index()][best.to.dense_index()]
                > 0
        );
        assert_eq!(searcher.killers[0], [None, None]);
    });
}

#[test]
fn post_lmr_continuation_bonus_requires_alpha_improvement() {
    let root = bare_kings();
    let context_move = legal_moves(&root)[0];
    for (value, changed) in [(1, true), (0, false)] {
        with_root_searcher(&root, &[], |searcher| {
            let mut current = root.clone();
            push_path_move(searcher, &mut current, Some(context_move));
            let context = searcher.path_moves[0].record.unwrap();
            let mv = legal_moves(&current)[0];
            let piece = current.piece_at(mv.from).unwrap();
            let continuation = continuation_index(context, piece, mv, 1);
            searcher.continuation_history.store(continuation, 0);

            let mut child = current.clone();
            child.make_move_unchecked(mv, engine_rules());
            searcher
                .tt
                .store(search_key(&child), 6, -value, Bound::Exact, None, 2, false);
            assert_eq!(
                searcher.search_move(&mut current, mv, 7, 0, 10, 1, false, 5, Some(0), false),
                Some(value)
            );
            assert_eq!(
                searcher.continuation_history.read(continuation) > 0,
                changed,
                "value={value}"
            );
        });
    }
}
