//! correction historyは静的評価に加える（Stockfish src/search.cpp:832, 846-870）。
//! 詰み帯の外で最善手が非捕獲のときに、補正前の評価との差で更新する。

use super::*;
use crate::eval::evaluate;
use crate::search::alphabeta::params;
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

/// 子の置換表に既知の値を置き、親の更新契約だけを検査する。
#[test]
fn correction_negamax_update_conditions() {
    let nnue = test_nnue();
    let stop = AtomicBool::new(false);
    let clock = ManualClock::new(Duration::ZERO);
    let shared = shared(&stop, &clock);
    for (mut board, delta, bound, updates, capture, depth) in [
        (bare_kings(), 320, Bound::Exact, true, false, 2),
        (bare_kings(), -320, Bound::Exact, true, false, 2),
        (bare_kings(), -320, Bound::Upper, true, false, 2),
        (bare_kings(), 320, Bound::Lower, true, false, 2),
        (bare_kings(), 320, Bound::Upper, false, false, 2),
        (bare_kings(), -320, Bound::Lower, false, false, 2),
        (bare_kings(), 0, Bound::Upper, false, false, 2),
        (bare_kings(), 0, Bound::Lower, false, false, 2),
        (capture_board(), 320, Bound::Exact, false, true, 2),
        (capture_board(), -320, Bound::Upper, false, true, 2),
        (capture_board(), 320, Bound::Lower, false, true, 2),
        (attacked_board(), 320, Bound::Exact, false, false, 2),
        (bare_kings(), 320, Bound::Exact, true, false, 3),
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
                Bound::Upper,
                Some(first),
                0,
                false,
            );
        }
        for mv in moves {
            let undo = board.make_move_unchecked(mv, rules);
            table.store(search_key(&board), 8, -score, Bound::Exact, None, 1, false);
            board.unmake_move(undo);
        }
        let mut butterfly_history =
            Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
        let mut capture_history = crate::search::alphabeta::capture_history::CaptureHistory::new();
        let mut searcher = new_searcher(
            nnue,
            &board,
            rules,
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            DRAW_SCORE,
            &[],
            &shared,
            &table,
            &mut butterfly_history,
            &mut capture_history,
        );
        // 深さ2ではfollowPVのPVノードとしてS1を除き、補正表の更新条件だけを検査する。
        if depth == 2 {
            assert!(beta - alpha > 1);
            searcher.follow_pv[0] = true;
        }
        // 更新しないケースも非ゼロの初期値を保持することを確認する。
        searcher.correction.update(Color::Black, key, 64, 8);
        let before = searcher.correction.read(Color::Black, key);
        assert!(before > 0);
        assert_eq!(
            searcher.negamax(&mut board, depth, alpha, beta, 0, false),
            Some(score)
        );
        let hit = table.probe(search_key(&board), 0).unwrap();
        assert_eq!(hit.bound, bound);
        assert_eq!(hit.depth, 2);
        assert_eq!(hit.score, score);
        assert_eq!(
            board
                .captured_squares(hit.best_move.unwrap())
                .iter()
                .any(Option::is_some),
            capture
        );
        assert_eq!(royal_under_attack(&board), board == attacked_board());
        let after = searcher.correction.read(Color::Black, key);
        assert!(after.abs() <= params::correction_cap());
        if updates && before != delta.signum() * params::correction_cap() {
            assert_eq!((after - before).signum(), delta.signum());
        } else {
            assert_eq!(after, before);
        }
    }
}

#[test]
fn correction_negamax_score_boundaries_tt_cutoffs_and_interruption() {
    assert_eq!(test_nnue().pawn_value(), 100, "固定ネットの歩兵価値");
    let nnue = test_nnue();
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
            table.store(search_key(&board), 2, -score, Bound::Exact, None, 1, false);
            board.unmake_move(undo);
        }
        let key = material_key(&board);
        let mut butterfly_history =
            Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
        let mut capture_history = crate::search::alphabeta::capture_history::CaptureHistory::new();
        let mut searcher = new_searcher(
            nnue,
            &board,
            rules,
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            DRAW_SCORE,
            &[],
            &shared,
            &table,
            &mut butterfly_history,
            &mut capture_history,
        );
        searcher.correction.update(Color::Black, key, 64, 8);
        let before = searcher.correction.read(Color::Black, key);
        assert_eq!(
            searcher.negamax(&mut board, 2, -INFINITY, INFINITY, 0, false),
            Some(score),
            "article_31_r1 score={score}"
        );
        let after = searcher.correction.read(Color::Black, key);
        if score.abs() >= MATE_THRESHOLD {
            assert_eq!(after, before);
        } else {
            assert_eq!(after, score.signum() * params::correction_cap());
        }
    }
    for interrupted in [false, true] {
        let stop = AtomicBool::new(interrupted);
        let clock = ManualClock::new(Duration::ZERO);
        let shared = shared(&stop, &clock);
        let mut board = bare_kings();
        let table = TranspositionTable::new(1).unwrap();
        if !interrupted {
            table.store(search_key(&board), 2, 320, Bound::Exact, None, 0, false);
        }
        let key = material_key(&board);
        let mut butterfly_history =
            Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
        let mut capture_history = crate::search::alphabeta::capture_history::CaptureHistory::new();
        let mut searcher = new_searcher(
            nnue,
            &board,
            MoveRules::standard(),
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            DRAW_SCORE,
            &[],
            &shared,
            &table,
            &mut butterfly_history,
            &mut capture_history,
        );
        searcher.correction.update(Color::Black, key, 64, 8);
        let before = searcher.correction.read(Color::Black, key);
        assert_eq!(
            searcher.negamax(&mut board, 2, -INFINITY, INFINITY, 0, false),
            if interrupted { None } else { Some(320) }
        );
        assert_eq!(searcher.correction.read(Color::Black, key), before);
    }
}

#[test]
fn correction_changes_futility_estimate() {
    let nnue = test_nnue();
    let stop = AtomicBool::new(false);
    let clock = ManualClock::new(Duration::ZERO);
    let shared = shared(&stop, &clock);
    let board = bare_kings();
    let quiets = legal_moves(&board);
    let history = repeated_root_children(&board, &quiets);
    let raw = evaluate(nnue, &board);
    let alpha = raw + 10 * nnue.pawn_value();
    let mut scores = Vec::new();
    let mut nodes = Vec::new();
    for correction in [0, 1] {
        let mut board = board.clone();
        let table = TranspositionTable::new(1).unwrap();
        let mut butterfly_history =
            Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
        let mut capture_history = crate::search::alphabeta::capture_history::CaptureHistory::new();
        let mut searcher = new_searcher(
            nnue,
            &board,
            MoveRules::standard(),
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            -1000,
            &history,
            &shared,
            &table,
            &mut butterfly_history,
            &mut capture_history,
        );
        table.store(
            search_key(&board),
            0,
            0,
            Bound::Upper,
            Some(quiets[0]),
            0,
            true,
        );
        searcher.correction.update(
            Color::Black,
            material_key(&board),
            correction * (1024 + 8 * params::correction_weight() - 1)
                / (8 * params::correction_weight()),
            8,
        );
        assert_eq!(
            searcher.correction.read(Color::Black, material_key(&board)),
            correction
        );
        let score = searcher
            .negamax(&mut board, 1, alpha, alpha + 2, 0, false)
            .unwrap();
        assert!(score < alpha);
        assert!(score.abs() < MATE_THRESHOLD);
        assert_eq!(searcher.static_evals[0], raw + correction);
        scores.push(score);
        nodes.push(searcher.nodes);
    }
    assert_eq!(nodes, [1, 1]);
    assert_eq!(scores[1], scores[0] + 1);
}

#[test]
fn correction_search_move_and_null_move_propagate_material_keys() {
    let nnue = test_nnue();
    let stop = AtomicBool::new(false);
    let clock = ManualClock::new(Duration::ZERO);
    let shared = shared(&stop, &clock);
    let rules = MoveRules::standard();
    let table = TranspositionTable::new(1).unwrap();
    let mut changed = 0;
    for mut board in minase_core::test_util::bench_positions() {
        let root_key = material_key(&board);
        let mut butterfly_history =
            Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
        let mut capture_history = crate::search::alphabeta::capture_history::CaptureHistory::new();
        let mut searcher = new_searcher(
            nnue,
            &board,
            rules,
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            DRAW_SCORE,
            &[],
            &shared,
            &table,
            &mut butterfly_history,
            &mut capture_history,
        );
        assert_eq!(searcher.material_keys[0], root_key);
        let mut moves = Vec::new();
        MoveGenerator::new(rules).generate_moves(&board, &mut moves);
        for mv in moves {
            if captures_last_royal(&board, mv) {
                continue;
            }
            let undo = board.make_move_unchecked(mv, rules);
            let expected = material_key(&board);
            board.unmake_move(undo);
            searcher.material_keys[1] = !expected;
            assert!(
                searcher
                    .search_move(&mut board, mv, 1, 10_000, 10_001, 0, true, 0, None, false)
                    .is_some()
            );
            assert_eq!(searcher.material_keys[1], expected);
            assert_eq!(searcher.material_keys[0], root_key);
            assert_eq!(material_key(&board), root_key);
            changed += usize::from(expected != root_key);
        }
    }
    assert!(changed > 0);
    let mut board = capture_board();
    let root_key = material_key(&board);
    let mut butterfly_history =
        Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
    let mut capture_history = crate::search::alphabeta::capture_history::CaptureHistory::new();
    let mut searcher = new_searcher(
        nnue,
        &board,
        rules,
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        DRAW_SCORE,
        &[],
        &shared,
        &table,
        &mut butterfly_history,
        &mut capture_history,
    );
    searcher.material_keys[1] = !root_key;
    assert!(
        searcher
            .search_null_move(&mut board, 0, INFINITY, 0)
            .is_some()
    );
    assert_eq!(searcher.material_keys[1], root_key);
    assert_eq!(material_key(&board), root_key);
}
