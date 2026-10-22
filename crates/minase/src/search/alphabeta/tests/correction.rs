//! correction historyは静的評価に加える（Stockfish src/search.cpp:832, 846-870）。
//! 詰み帯の外で境界と捕獲手の条件を満たすときに、補正後の評価との差で更新する。

use core::cmp::Ordering::{Equal, Greater, Less};

use super::*;
use crate::eval::evaluate;
use crate::eval::nnue::EVALUATION_LIMIT;
use crate::search::alphabeta::correction::correction_update_bonus;
use crate::search::alphabeta::pruning::corrected_static_eval;
use minase_core::Color;

fn shared<'a>(stop: &'a AtomicBool, clock: &'a dyn SearchClock) -> SharedSearch<'a> {
    SharedSearch {
        external_stop: stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        node_limit: None,
        clock,
        hard_limit: None,
    }
}

fn with_history_worker_searcher(
    histories: &mut crate::search::HistoryTables,
    worker: usize,
    board: &Position,
    test: impl FnOnce(&mut Searcher<'_>),
) {
    let stop = AtomicBool::new(false);
    let clock = ManualClock::new(Duration::ZERO);
    let shared = shared(&stop, &clock);
    let table = small_tt();
    let mut butterfly_history =
        Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
    let mut capture_history = empty_capture_history();
    let mut searcher = new_searcher(
        test_nnue(),
        board,
        MoveRules::standard(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        DRAW_SCORE,
        0,
        &[],
        &shared,
        &table,
        &mut butterfly_history,
        &mut capture_history,
        &histories.correction,
        &mut histories.continuation_correction[worker],
        &histories.continuation,
        &histories.pawn,
    );
    test(&mut searcher);
}

fn capture_board() -> Position {
    position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 4), Color::White, PieceKind::Pawn),
            (sq(5, 3), Color::Black, PieceKind::Rook),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    )
}
fn attacked_board() -> Position {
    position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::Rook),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    )
}

fn near_upper_evaluation_limit_board() -> Position {
    let black_king = sq(11, 0);
    let white_king = sq(0, 11);
    let mut pieces = vec![
        (black_king, Color::Black, PieceKind::King),
        (white_king, Color::White, PieceKind::King),
    ];
    let mut squares = Square::all().filter(|&square| square != black_king && square != white_king);
    pieces.extend(
        squares
            .by_ref()
            .take(17)
            .map(|square| (square, Color::Black, PieceKind::Rook)),
    );
    pieces.push((
        squares.next().expect("fixture has a square for the bishop"),
        Color::Black,
        PieceKind::Bishop,
    ));
    position(Color::Black, &pieces)
}

#[test]
fn quiescence_stand_pat_beta_cutoff_uses_corrected_static_eval() {
    let board = bare_kings();
    let v = evaluate(test_nnue(), &board);
    with_root_searcher(&board, &[], |searcher| {
        zero_position_correction(searcher, &board);
        searcher.correction.store(
            board.side_to_move(),
            material_key(&board),
            CorrectionTableKind::Material,
            1024,
        );
        let correction = position_correction(searcher, board.side_to_move(), 0);
        assert_ne!(correction, 0);
        let expected = (v + correction).clamp(-EVALUATION_LIMIT, EVALUATION_LIMIT);

        assert_eq!(
            searcher.quiesce(&mut board.clone(), -INFINITY, expected, 0),
            Some(expected)
        );
    });
}

#[test]
fn quiescence_stand_pat_clamps_at_evaluation_limit() {
    let board = near_upper_evaluation_limit_board();
    let v = evaluate(test_nnue(), &board);
    with_root_searcher(&board, &[], |searcher| {
        zero_position_correction(searcher, &board);
        store_position_correction(searcher, &board, 1024);
        let correction = position_correction(searcher, board.side_to_move(), 0);
        assert!(v < EVALUATION_LIMIT);
        assert!(v + correction > EVALUATION_LIMIT);

        assert_eq!(
            searcher.quiesce(&mut board.clone(), -INFINITY, EVALUATION_LIMIT, 0),
            Some(EVALUATION_LIMIT)
        );
    });
}

#[test]
fn quiescence_max_ply_returns_uncorrected_static_eval() {
    let board = bare_kings();
    let v = evaluate(test_nnue(), &board);
    with_root_searcher(&board, &[], |searcher| {
        zero_position_correction(searcher, &board);
        searcher.correction.store(
            board.side_to_move(),
            material_key(&board),
            CorrectionTableKind::Material,
            1024,
        );
        assert_ne!(position_correction(searcher, board.side_to_move(), 0), 0);
        set_test_path_ply(searcher, MAX_PLY);

        assert_eq!(
            searcher.quiesce(&mut board.clone(), -INFINITY, INFINITY, MAX_PLY),
            Some(v)
        );
    });
}

#[test]
fn quiescence_tt_stores_uncorrected_static_eval() {
    let board = capture_board();
    let key = search_key(&board);
    let v = evaluate(test_nnue(), &board);
    with_root_searcher(&board, &[], |searcher| {
        zero_position_correction(searcher, &board);
        searcher.correction.store(
            board.side_to_move(),
            material_key(&board),
            CorrectionTableKind::Material,
            1024,
        );
        let correction = position_correction(searcher, board.side_to_move(), 0);
        assert_ne!(correction, 0);
        let s = (v + correction).clamp(-EVALUATION_LIMIT, EVALUATION_LIMIT);
        assert_ne!(s, v);

        let mut current = board.clone();
        assert!(
            searcher
                .quiesce(&mut current, -INFINITY, INFINITY, 0)
                .is_some()
        );
        assert_eq!(searcher.tt.probe(key, 0).unwrap().static_eval, Some(v));
    });
}

#[test]
fn correction_update_applies_one_observation_to_all_five_tables() {
    let board = bare_kings();
    with_root_searcher(&board, &[], |searcher| {
        let side = board.side_to_move();
        let bonus = correction_update_bonus(50, 3, Bound::Exact, false, 20, 29, 100).unwrap();
        assert_eq!(bonus, 31);
        searcher.apply_correction_bonus(side, 0, bonus);

        for (key, table, expected) in [
            (searcher.material_keys[0], CorrectionTableKind::Material, 26),
            (searcher.pawn_keys[0], CorrectionTableKind::Pawn, 26),
            (searcher.minor_keys[0], CorrectionTableKind::Minor, 31),
            (
                searcher.non_pawn_keys[0][Color::Black.index()],
                CorrectionTableKind::BlackNonPawn,
                40,
            ),
            (
                searcher.non_pawn_keys[0][Color::White.index()],
                CorrectionTableKind::WhiteNonPawn,
                40,
            ),
        ] {
            assert_eq!(searcher.correction.read(side, key, table), expected);
        }
    });
}

#[test]
fn continuation_correction_updates_shared_sentinel_element_in_context_order() {
    let root = bare_kings();
    with_root_searcher(&root, &[], |searcher| {
        let mut board = root.clone();
        let mv = legal_moves(&board)[0];
        push_path_move(searcher, &mut board, Some(mv));
        let side = board.side_to_move();
        let indices = searcher.continuation_correction_indices(side, 1).unwrap();
        assert_eq!(indices, [indices[0]; 3]);

        for (multiplier, expected) in [(130, 135), (70, 196), (35, 225)] {
            searcher
                .continuation_correction
                .update(indices[0], 128, multiplier);
            assert_eq!(searcher.continuation_correction.read(indices[0]), expected);
        }

        searcher.continuation_correction.store(indices[0], 5);
        searcher.apply_correction_bonus(side, 1, 128);
        assert_eq!(searcher.continuation_correction.read(indices[0]), 225);
    });
}

#[test]
fn continuation_correction_missing_previous_move_uses_constant_and_skips_update() {
    let root = bare_kings();
    with_root_searcher(&root, &[], |searcher| {
        searcher.continuation_correction.values_mut().fill(1024);
        zero_position_correction(searcher, &root);
        assert_eq!(searcher.correction_weighted_sum_at(Color::Black, 0), 303);
        searcher.update_correction(&root, 20, 0, 31, Bound::Exact, false, 0, Some(false));
        assert!(
            searcher
                .continuation_correction
                .values()
                .iter()
                .all(|&value| value == 1024)
        );

        let mut after_null = root.clone();
        push_path_move(searcher, &mut after_null, None);
        zero_position_correction(searcher, &after_null);
        assert_eq!(searcher.correction_weighted_sum_at(Color::White, 1), 303);
        searcher.update_correction(&after_null, 20, 1, 31, Bound::Exact, false, 0, Some(false));
        assert!(
            searcher
                .continuation_correction
                .values()
                .iter()
                .all(|&value| value == 1024)
        );
    });
}

#[test]
fn continuation_correction_initializes_clears_and_is_worker_local() {
    let mut histories = crate::search::HistoryTables::new(worker_count(2));
    assert_eq!(histories.continuation_correction.len(), 2);
    for history in &histories.continuation_correction {
        assert_eq!(history.values().len(), 30 * 144 * 29 * 144);
        assert!(history.values().iter().all(|&value| value == 5));
    }

    histories.continuation_correction[0].store(17, 99);
    assert_eq!(histories.continuation_correction[0].read(17), 99);
    assert_eq!(histories.continuation_correction[1].read(17), 5);

    histories.clear();
    assert!(histories.continuation_correction.iter().all(|history| {
        history.values().len() == 30 * 144 * 29 * 144
            && history.values().iter().all(|&value| value == 5)
    }));
}

#[test]
fn position_correction_is_shared_between_worker_searchers() {
    let board = bare_kings();
    let mut histories = crate::search::HistoryTables::new(worker_count(2));
    let side = board.side_to_move();
    let key = material_key(&board);
    let continuation_index = 17;

    with_history_worker_searcher(&mut histories, 0, &board, |searcher| {
        searcher
            .correction
            .update(side, key, CorrectionTableKind::Material, 128, 128);
        searcher
            .continuation_correction
            .update(continuation_index, 128, 128);
        assert_eq!(
            searcher
                .correction
                .read(side, key, CorrectionTableKind::Material),
            123
        );
        assert_eq!(
            searcher.continuation_correction.read(continuation_index),
            133
        );
    });

    with_history_worker_searcher(&mut histories, 1, &board, |searcher| {
        assert_eq!(
            searcher
                .correction
                .read(side, key, CorrectionTableKind::Material),
            123
        );
        assert_eq!(searcher.continuation_correction.read(continuation_index), 5);
    });
}

/// 子の置換表に既知の値を置き、親の更新契約だけを検査する。
#[test]
fn correction_negamax_update_conditions() {
    let nnue = test_nnue();
    let mut shared_histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
    let stop = AtomicBool::new(false);
    let clock = ManualClock::new(Duration::ZERO);
    let shared = shared(&stop, &clock);
    for (mut board, delta, bound, expected_change, capture, depth, initial_bonus) in [
        (bare_kings(), 320, Bound::Exact, Greater, false, 2, 5),
        (bare_kings(), -320, Bound::Exact, Equal, false, 2, 5),
        (bare_kings(), -320, Bound::Upper, Less, false, 2, 5),
        (bare_kings(), 320, Bound::Lower, Greater, false, 2, 5),
        (bare_kings(), 320, Bound::Upper, Equal, false, 2, 5),
        (bare_kings(), -320, Bound::Lower, Equal, false, 2, 5),
        (capture_board(), 320, Bound::Exact, Equal, true, 2, 5),
        (capture_board(), -320, Bound::Upper, Less, true, 2, 5),
        (capture_board(), 320, Bound::Lower, Equal, true, 2, 5),
        (attacked_board(), 320, Bound::Exact, Equal, false, 2, 5),
        (bare_kings(), 320, Bound::Exact, Greater, false, 3, 5),
        // 補正前の評価との差は0だが、入口の補正後の評価との差は正なので更新する。
        (bare_kings(), 0, Bound::Exact, Greater, false, 2, -1024),
    ] {
        let key = material_key(&board);
        let eval = evaluate(nnue, &board);
        let score = eval + delta;
        let (alpha, beta) = match bound {
            Bound::Exact => (-INFINITY, INFINITY),
            Bound::Upper => (score + 100, score + 200),
            Bound::Lower => (score - 200, score - 100),
        };
        let table = TranspositionTable::new(1).unwrap();
        let rules = MoveRules::standard();
        let mut moves = Vec::new();
        MoveGenerator::new(rules).generate_moves(&board, &mut moves);
        let first = *moves
            .iter()
            .find(|&&mv| board.captured_squares(mv).iter().any(Option::is_some) == capture)
            .unwrap();
        // 深さ3は記録手なしでIIRを通し、保存深さ2の重みになることを検査する。
        if depth == 2 {
            table.store(
                search_key(&board),
                0,
                0,
                None,
                Bound::Upper,
                Some(first),
                0,
                false,
            );
        }
        for mv in moves {
            let undo = board.make_move_unchecked(mv, rules);
            table.store(
                search_key(&board),
                8,
                -score,
                None,
                Bound::Exact,
                None,
                1,
                false,
            );
            board.unmake_move(undo);
        }
        let mut butterfly_history =
            Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
        let mut capture_history = empty_capture_history();
        let correction_values = new_correction_values();
        let mut searcher = new_searcher(
            nnue,
            &board,
            rules,
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            DRAW_SCORE,
            0,
            &[],
            &shared,
            &table,
            &mut butterfly_history,
            &mut capture_history,
            &correction_values,
            &mut shared_histories.continuation_correction[0],
            &shared_histories.continuation,
            &shared_histories.pawn,
        );
        // 深さ2ではfollowPVのPVノードとしてS1を除き、補正表の更新条件だけを検査する。
        if depth == 2 {
            assert!(beta - alpha > 1);
            searcher.follow_pv[0] = true;
        }
        searcher.correction.update(
            Color::Black,
            key,
            CorrectionTableKind::Material,
            initial_bonus,
            128,
        );
        let before = searcher
            .correction
            .read(Color::Black, key, CorrectionTableKind::Material);
        assert_eq!(
            searcher.negamax(&mut board, depth, alpha, beta, 0, false),
            Some(score)
        );
        let hit = table.probe(search_key(&board), 0).unwrap();
        assert_eq!(hit.value.unwrap().bound, bound);
        assert_eq!(hit.value.unwrap().depth, 2);
        assert_eq!(hit.value.unwrap().score, score);
        assert_eq!(
            board
                .captured_squares(hit.best_move.unwrap())
                .iter()
                .any(Option::is_some),
            capture
        );
        assert_eq!(royal_under_attack(&board), board == attacked_board());
        let after = searcher
            .correction
            .read(Color::Black, key, CorrectionTableKind::Material);
        assert_eq!(after.cmp(&before), expected_change);
    }
}

#[test]
fn correction_negamax_score_boundaries_tt_cutoffs_and_interruption() {
    assert_eq!(test_nnue().pawn_value(), 100, "固定ネットの歩兵価値");
    let nnue = test_nnue();
    let mut shared_histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
    for score in [
        MATE_THRESHOLD,
        -MATE_THRESHOLD,
        MATE - 1,
        -MATE + 1,
        29_486,
        -29_486,
    ] {
        let stop = AtomicBool::new(false);
        let clock = ManualClock::new(Duration::ZERO);
        let shared = shared(&stop, &clock);
        let mut board = bare_kings();
        let table = TranspositionTable::new(1).unwrap();
        let rules = MoveRules::standard();
        let mut moves = Vec::new();
        MoveGenerator::new(rules).generate_moves(&board, &mut moves);
        for mv in moves {
            let undo = board.make_move_unchecked(mv, rules);
            table.store(
                search_key(&board),
                2,
                -score,
                None,
                Bound::Exact,
                None,
                1,
                false,
            );
            board.unmake_move(undo);
        }
        let key = material_key(&board);
        let mut butterfly_history =
            Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
        let mut capture_history = empty_capture_history();
        let correction_values = new_correction_values();
        let mut searcher = new_searcher(
            nnue,
            &board,
            rules,
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            DRAW_SCORE,
            0,
            &[],
            &shared,
            &table,
            &mut butterfly_history,
            &mut capture_history,
            &correction_values,
            &mut shared_histories.continuation_correction[0],
            &shared_histories.continuation,
            &shared_histories.pawn,
        );
        searcher
            .correction
            .update(Color::Black, key, CorrectionTableKind::Material, 5, 128);
        let before = searcher
            .correction
            .read(Color::Black, key, CorrectionTableKind::Material);
        assert_eq!(
            searcher.negamax(&mut board, 2, -INFINITY, INFINITY, 0, false),
            Some(score),
            "article_31_r1 score={score}"
        );
        let after = searcher
            .correction
            .read(Color::Black, key, CorrectionTableKind::Material);
        if score == 29_486 {
            assert_eq!(after, 265);
        } else {
            assert_eq!(after, before);
        }
    }
    for interrupted in [false, true] {
        let stop = AtomicBool::new(interrupted);
        let clock = ManualClock::new(Duration::ZERO);
        let shared = shared(&stop, &clock);
        let mut board = bare_kings();
        let table = TranspositionTable::new(1).unwrap();
        if !interrupted {
            table.store(
                search_key(&board),
                2,
                320,
                None,
                Bound::Exact,
                None,
                0,
                false,
            );
        }
        let key = material_key(&board);
        let mut butterfly_history =
            Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
        let mut capture_history = empty_capture_history();
        let correction_values = new_correction_values();
        let mut searcher = new_searcher(
            nnue,
            &board,
            MoveRules::standard(),
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            DRAW_SCORE,
            0,
            &[],
            &shared,
            &table,
            &mut butterfly_history,
            &mut capture_history,
            &correction_values,
            &mut shared_histories.continuation_correction[0],
            &shared_histories.continuation,
            &shared_histories.pawn,
        );
        searcher
            .correction
            .update(Color::Black, key, CorrectionTableKind::Material, 31, 128);
        let before = searcher
            .correction
            .read(Color::Black, key, CorrectionTableKind::Material);
        assert_eq!(
            searcher.negamax(&mut board, 2, -INFINITY, INFINITY, 0, false),
            if interrupted { None } else { Some(320) }
        );
        assert_eq!(
            searcher
                .correction
                .read(Color::Black, key, CorrectionTableKind::Material),
            before
        );
    }
}

#[test]
fn correction_root_updates_only_after_completed_search() {
    let nnue = test_nnue();
    assert_eq!(nnue.pawn_value(), 100, "固定ネットの歩兵価値");
    let mut shared_histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
    let board = bare_kings();
    let score = evaluate(nnue, &board);
    let rules = MoveRules::standard();
    let moves = legal_moves(&board);

    for interrupted in [false, true] {
        let stop = AtomicBool::new(interrupted);
        let clock = ManualClock::new(Duration::ZERO);
        let shared = shared(&stop, &clock);
        let table = TranspositionTable::new(1).unwrap();
        let mut child = board.clone();
        for &mv in &moves {
            let undo = child.make_move_unchecked(mv, rules);
            table.store(
                search_key(&child),
                8,
                -score,
                None,
                Bound::Exact,
                None,
                1,
                false,
            );
            child.unmake_move(undo);
        }
        let mut butterfly_history =
            Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
        let mut capture_history = empty_capture_history();
        let correction_values = new_correction_values();
        let mut searcher = new_searcher(
            nnue,
            &board,
            rules,
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            DRAW_SCORE,
            0,
            &[],
            &shared,
            &table,
            &mut butterfly_history,
            &mut capture_history,
            &correction_values,
            &mut shared_histories.continuation_correction[0],
            &shared_histories.continuation,
            &shared_histories.pawn,
        );
        let key = material_key(&board);
        searcher
            .correction
            .update(Color::Black, key, CorrectionTableKind::Material, -1024, 128);
        let initial = searcher
            .correction
            .read(Color::Black, key, CorrectionTableKind::Material);
        assert_eq!(initial, -1024);

        let result = searcher.search_root(&board, &moves, 2, 2, -INFINITY, INFINITY);
        if interrupted {
            assert_eq!(result, None);
            assert_eq!(
                searcher
                    .correction
                    .read(Color::Black, key, CorrectionTableKind::Material),
                initial
            );
        } else {
            assert_eq!(result.map(|(_, value)| value), Some(score));
            assert!(
                searcher
                    .correction
                    .read(Color::Black, key, CorrectionTableKind::Material)
                    > initial
            );
        }
    }
}

#[test]
fn correction_changes_futility_estimate() {
    let nnue = test_nnue();
    let mut shared_histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
    let stop = AtomicBool::new(false);
    let clock = ManualClock::new(Duration::ZERO);
    let shared = shared(&stop, &clock);
    let board = bare_kings();
    let quiets = legal_moves(&board);
    let history = repeated_root_children(&board, &quiets);
    let raw = evaluate(nnue, &board);
    let alpha = raw + 10 * nnue.pawn_value();
    let mut corrections = Vec::new();
    let mut scores = Vec::new();
    let mut nodes = Vec::new();
    // 更新量ではなく表の値を固定し、重みが変わっても同じ2状態を比較する。
    for table_value in [-512, 512] {
        let mut board = board.clone();
        let table = TranspositionTable::new(1).unwrap();
        let mut butterfly_history =
            Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
        let mut capture_history = empty_capture_history();
        let correction_values = new_correction_values();
        let mut searcher = new_searcher(
            nnue,
            &board,
            MoveRules::standard(),
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            -1000,
            0,
            &history,
            &shared,
            &table,
            &mut butterfly_history,
            &mut capture_history,
            &correction_values,
            &mut shared_histories.continuation_correction[0],
            &shared_histories.continuation,
            &shared_histories.pawn,
        );
        table.store(
            search_key(&board),
            0,
            0,
            None,
            Bound::Upper,
            Some(quiets[0]),
            0,
            true,
        );
        zero_position_correction(&mut searcher, &board);
        store_position_correction(&searcher, &board, table_value);
        let correction = position_correction(&searcher, Color::Black, 0);
        let score = searcher
            .negamax(&mut board, 1, alpha, alpha + 2, 0, false)
            .unwrap();
        assert!(score < alpha);
        assert!(score.abs() < MATE_THRESHOLD);
        assert_eq!(
            searcher.static_evals[0],
            corrected_static_eval(raw, correction)
        );
        corrections.push(correction);
        scores.push(score);
        nodes.push(searcher.nodes);
    }
    assert_eq!(nodes, [1, 1]);
    assert!(corrections[1] > corrections[0]);
    assert_eq!(scores[1] - scores[0], corrections[1] - corrections[0]);
}

#[test]
fn correction_search_move_and_null_move_propagate_position_keys() {
    let nnue = test_nnue();
    let mut shared_histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
    let stop = AtomicBool::new(false);
    let clock = ManualClock::new(Duration::ZERO);
    let shared = shared(&stop, &clock);
    let rules = MoveRules::standard();
    let table = TranspositionTable::new(1).unwrap();
    let mut changed = 0;
    for mut board in minase_core::test_util::bench_positions() {
        let root_material = material_key(&board);
        let root_placement = correction_piece_keys(&board);
        let mut butterfly_history =
            Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
        let mut capture_history = empty_capture_history();
        let correction_values = new_correction_values();
        let mut searcher = new_searcher(
            nnue,
            &board,
            rules,
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            DRAW_SCORE,
            0,
            &[],
            &shared,
            &table,
            &mut butterfly_history,
            &mut capture_history,
            &correction_values,
            &mut shared_histories.continuation_correction[0],
            &shared_histories.continuation,
            &shared_histories.pawn,
        );
        assert_eq!(searcher.material_keys[0], root_material);
        assert_eq!(searcher.minor_keys[0], root_placement.minor);
        assert_eq!(searcher.non_pawn_keys[0], root_placement.non_pawn);
        let mut moves = Vec::new();
        MoveGenerator::new(rules).generate_moves(&board, &mut moves);
        for mv in moves {
            if captures_last_royal(&board, mv) {
                continue;
            }
            let undo = board.make_move_unchecked(mv, rules);
            let expected_material = material_key(&board);
            let expected_placement = correction_piece_keys(&board);
            board.unmake_move(undo);
            searcher.material_keys[1] = !expected_material;
            searcher.minor_keys[1] = !expected_placement.minor;
            searcher.non_pawn_keys[1] = expected_placement.non_pawn.map(|key| !key);
            assert!(
                searcher
                    .search_move(&mut board, mv, 1, 10_000, 10_001, 0, true, 0, None, false)
                    .is_some()
            );
            assert_eq!(searcher.material_keys[1], expected_material);
            assert_eq!(searcher.minor_keys[1], expected_placement.minor);
            assert_eq!(searcher.non_pawn_keys[1], expected_placement.non_pawn);
            assert_eq!(searcher.material_keys[0], root_material);
            assert_eq!(searcher.minor_keys[0], root_placement.minor);
            assert_eq!(searcher.non_pawn_keys[0], root_placement.non_pawn);
            assert_eq!(material_key(&board), root_material);
            assert_eq!(correction_piece_keys(&board), root_placement);
            changed += usize::from(
                expected_material != root_material || expected_placement != root_placement,
            );
        }
    }
    assert!(changed > 0);
    let mut board = capture_board();
    let root_material = material_key(&board);
    let root_placement = correction_piece_keys(&board);
    let mut butterfly_history =
        Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
    let mut capture_history = empty_capture_history();
    let correction_values = new_correction_values();
    let mut searcher = new_searcher(
        nnue,
        &board,
        rules,
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        DRAW_SCORE,
        0,
        &[],
        &shared,
        &table,
        &mut butterfly_history,
        &mut capture_history,
        &correction_values,
        &mut shared_histories.continuation_correction[0],
        &shared_histories.continuation,
        &shared_histories.pawn,
    );
    searcher.material_keys[1] = !root_material;
    searcher.minor_keys[1] = !root_placement.minor;
    searcher.non_pawn_keys[1] = root_placement.non_pawn.map(|key| !key);
    assert!(
        searcher
            .search_null_move(&mut board, 0, INFINITY, 0)
            .is_some()
    );
    assert_eq!(searcher.material_keys[1], root_material);
    assert_eq!(searcher.minor_keys[1], root_placement.minor);
    assert_eq!(searcher.non_pawn_keys[1], root_placement.non_pawn);
    assert_eq!(material_key(&board), root_material);
    assert_eq!(correction_piece_keys(&board), root_placement);
}

#[test]
fn correction_histories_survive_the_next_search_team_start() {
    let threads = DEFAULT_THREADS;
    let snapshot = snapshot_for(&quiet_midgame());
    let table = small_tt();
    let mut histories = crate::search::HistoryTables::new(threads);
    let first = run_search_team(
        test_nnue(),
        &snapshot.position,
        snapshot.rules(),
        snapshot.repetition(),
        DRAW_SCORE,
        &snapshot.root_moves,
        &snapshot.history_keys,
        &depth_limits(3),
        &AtomicBool::new(false),
        threads,
        &table,
        &mut histories,
        None,
        &ManualClock::new(Duration::ZERO),
        &AtomicU64::new(0),
        false,
    );
    assert_eq!(first.stop_reason, StopReason::DepthCompleted);
    let (position_index, position_value) = histories
        .correction
        .values()
        .iter()
        .enumerate()
        .find_map(|(index, value)| {
            let value = value.load(AtomicOrdering::Relaxed);
            (value != -5).then_some((index, value))
        })
        .expect("the first search must update position correction history");
    let (continuation_index, continuation_value) = histories.continuation_correction[0]
        .values()
        .iter()
        .copied()
        .enumerate()
        .find(|&(_, value)| value != 5)
        .expect("the first search must update continuation correction history");
    let second = run_search_team(
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
        &table,
        &mut histories,
        None,
        &ManualClock::new(Duration::ZERO),
        &AtomicU64::new(0),
        false,
    );
    assert_eq!(second.stop_reason, StopReason::ExternalStop);
    assert_eq!(second.result.nodes, 0);
    assert_eq!(
        histories.correction.values()[position_index].load(AtomicOrdering::Relaxed),
        position_value
    );
    assert_eq!(
        histories.continuation_correction[0].read(continuation_index),
        i32::from(continuation_value)
    );
}
