//! correction historyはfutilityの判定だけに加え、詰み帯の外で最善手が非捕獲のときに更新する。
//! 採用理由はdocs/adr/key-correction-history-by-material.md。

use super::*;
use crate::eval::evaluate;
use crate::search::alphabeta::params;
use minase_core::Color;

fn shared(stop: &AtomicBool) -> SharedSearch<'_> {
    SharedSearch {
        external_stop: stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        node_limit: None,
        started: Instant::now(),
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
    assert_eq!(params::correction_weight(), 37, "数値例の重み");
    let nnue = test_nnue();
    let stop = AtomicBool::new(false);
    let shared = shared(&stop);
    for (mut board, delta, bound, expected, capture, depth) in [
        (bare_kings(), 320, Bound::Exact, 40, false, 2),
        (bare_kings(), -320, Bound::Exact, -5, false, 2),
        (bare_kings(), -320, Bound::Upper, -5, false, 2),
        (bare_kings(), 320, Bound::Lower, 40, false, 2),
        (bare_kings(), 320, Bound::Upper, 18, false, 2),
        (bare_kings(), -320, Bound::Lower, 18, false, 2),
        (bare_kings(), 0, Bound::Upper, 18, false, 2),
        (bare_kings(), 0, Bound::Lower, 18, false, 2),
        (capture_board(), 320, Bound::Exact, 18, true, 2),
        (capture_board(), -320, Bound::Upper, 18, true, 2),
        (capture_board(), 320, Bound::Lower, 18, true, 2),
        (attacked_board(), 320, Bound::Exact, 18, false, 2),
        (bare_kings(), 320, Bound::Exact, 40, false, 3),
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
            table.store(search_key(&board), 0, 0, Bound::Upper, Some(first), 0);
        }
        for mv in moves {
            let undo = board.make_move_unchecked(mv, rules);
            table.store(search_key(&board), 8, -score, Bound::Exact, None, 1);
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
        // 更新しないケースも非ゼロの初期値を保持することを確認する。
        searcher.correction.update(Color::Black, key, 64, 8);
        assert_eq!(searcher.correction.read(Color::Black, key), 18);
        assert_eq!(
            searcher.negamax(&mut board, depth, alpha, beta, 0),
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
        // 初期の固定小数点値18944に深さ2の重み74/1024を適用する。
        // 差+320なら41255、差−320なら−6105となり、1024で0方向へ丸める。
        assert_eq!(
            searcher.correction.read(Color::Black, key),
            expected,
            "{board:?}, {delta}, {bound:?}, {depth}"
        );
    }
}

#[test]
fn correction_negamax_score_boundaries_tt_cutoffs_and_interruption() {
    assert_eq!(params::correction_weight(), 37, "数値例の前提");
    assert_eq!(params::correction_cap(), 220, "数値例の前提");
    assert_eq!(test_nnue().pawn_value(), 100, "固定ネットの歩兵価値");
    let nnue = test_nnue();
    for (score, expected) in [
        (MATE_THRESHOLD, 18),
        (-MATE_THRESHOLD, 18),
        (MATE - 1, 18),
        (-MATE + 1, 18),
        (29_486, 220),
        (-29_486, -220),
    ] {
        let stop = AtomicBool::new(false);
        let shared = shared(&stop);
        let mut board = bare_kings();
        let table = TranspositionTable::new(1).unwrap();
        let rules = MoveRules::standard();
        let mut moves = Vec::new();
        MoveGenerator::new(rules).generate_moves(&board, &mut moves);
        for mv in moves {
            let undo = board.make_move_unchecked(mv, rules);
            table.store(search_key(&board), 2, -score, Bound::Exact, None, 1);
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
        assert_eq!(
            searcher.negamax(&mut board, 2, -INFINITY, INFINITY, 0),
            Some(score),
            "article_31_r1 score={score}"
        );
        // 初期値64×8×37=18944、読み出しは18944/1024→18。
        // |静的評価|≤1000より、差の絶対値は28486以上。
        // D=28486ならF=2125539、D=−28486ならF=−2090389となる。
        // 上限220×1024=225280を越えるので、読み出しは±220に飽和する。
        assert!(evaluate(nnue, &board).abs() <= 1000, "飽和する数値例の前提");
        assert_eq!(
            searcher.correction.read(Color::Black, key),
            expected,
            "score={score}"
        );
    }
    for interrupted in [false, true] {
        let stop = AtomicBool::new(interrupted);
        let shared = shared(&stop);
        let mut board = bare_kings();
        let table = TranspositionTable::new(1).unwrap();
        if !interrupted {
            table.store(search_key(&board), 2, 320, Bound::Exact, None, 0);
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
        assert_eq!(
            searcher.negamax(&mut board, 2, -INFINITY, INFINITY, 0),
            if interrupted { None } else { Some(320) }
        );
        assert_eq!(searcher.correction.read(Color::Black, key), 18);
    }
}

#[test]
fn correction_changes_futility_expansion_at_boundary() {
    assert_eq!(params::futility_margin1(), 165, "数値例の前提");
    assert_eq!(params::correction_weight(), 37, "数値例の前提");
    assert_eq!(test_nnue().pawn_value(), 100, "固定ネットの歩兵価値");
    // 100×165/100=165。差4なら4×8×37/1024=1.15625→1。
    let nnue = test_nnue();
    let stop = AtomicBool::new(false);
    let shared = shared(&stop);
    let mut nodes = Vec::new();
    for correction in [0, 1] {
        let mut board = bare_kings();
        let table = TranspositionTable::new(1).unwrap();
        let alpha = evaluate(nnue, &board) + 165;
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
        searcher
            .correction
            .update(Color::Black, material_key(&board), correction * 4, 8);
        assert_eq!(
            searcher.correction.read(Color::Black, material_key(&board)),
            correction
        );
        let score = searcher
            .negamax(&mut board, 1, alpha, alpha + 1, 0)
            .unwrap();
        assert!(score < alpha);
        assert!(score.abs() < MATE_THRESHOLD);
        nodes.push(searcher.nodes);
    }
    assert_eq!(nodes[0], 1);
    assert!(nodes[1] > nodes[0]);
}

#[test]
fn correction_search_move_and_null_move_propagate_material_keys() {
    let nnue = test_nnue();
    let stop = AtomicBool::new(false);
    let shared = shared(&stop);
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
                    .search_move(&mut board, mv, 1, 10_000, 10_001, 0, true, 0)
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
    let score = searcher
        .negamax(&mut board, 4, -INFINITY, -10_000, 0)
        .unwrap();
    assert!(score >= -10_000);
    assert_eq!(searcher.material_keys[1], root_key);
    assert_eq!(material_key(&board), root_key);
    assert_eq!(searcher.nodes, 0, "null moveで打ち切る");
}
