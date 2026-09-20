//! 段階8の補正表について、通常探索との接続と更新条件を検査する。

use super::*;
use crate::Color;
use crate::eval::{evaluate, weights};

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

const QUIET: &str = "k11/12/12/12/12/12/12/12/12/12/12/11K b";
const CAPTURE: &str = "k11/12/12/12/12/12/12/5p6/5R6/12/12/11K b";
const ATTACKED: &str = "k10r/12/12/12/12/12/12/12/12/12/12/11K b";

/// 子の置換表に既知の値を置き、親の更新契約だけを検査する。
#[test]
fn correction_negamax_update_conditions() {
    let pst = weights().unwrap();
    let stop = AtomicBool::new(false);
    let shared = shared(&stop);
    for (sfen, delta, bound, update, capture, depth) in [
        (QUIET, 320, Bound::Exact, true, false, 2),
        (QUIET, -320, Bound::Exact, true, false, 2),
        (QUIET, -320, Bound::Upper, true, false, 2),
        (QUIET, 320, Bound::Lower, true, false, 2),
        (QUIET, 320, Bound::Upper, false, false, 2),
        (QUIET, -320, Bound::Lower, false, false, 2),
        (QUIET, 0, Bound::Upper, false, false, 2),
        (QUIET, 0, Bound::Lower, false, false, 2),
        (CAPTURE, 320, Bound::Exact, false, true, 2),
        (CAPTURE, -320, Bound::Upper, false, true, 2),
        (CAPTURE, 320, Bound::Lower, false, true, 2),
        (ATTACKED, 320, Bound::Exact, false, false, 2),
        (QUIET, 320, Bound::Exact, true, false, 3),
    ] {
        let mut board = crate::parse_sfen(sfen).unwrap();
        let key = material_key(&board);
        let eval = evaluate(&pst, &board);
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
        let mut searcher = new_searcher(&pst, &board, rules, &[], &shared, &table);
        // 更新しないケースも非ゼロの初期値を保持することを確認する。
        searcher.correction.update(Color::Black, key, 64, 8);
        assert_eq!(searcher.correction.read(Color::Black, key), 16);
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
        assert_eq!(royal_under_attack(&board), sfen == ATTACKED);
        let expected = if update {
            (16 * 1024 + (delta * 1024 - 16 * 1024) * 2 / 32) / 1024
        } else {
            16
        };
        assert_eq!(
            searcher.correction.read(Color::Black, key),
            expected,
            "{sfen}, {delta}, {bound:?}, {depth}"
        );
    }
}

#[test]
fn correction_negamax_skips_mates_tt_cutoffs_and_interruption() {
    let pst = weights().unwrap();
    for score in [MATE_THRESHOLD, -MATE_THRESHOLD, MATE - 1, -MATE + 1] {
        let stop = AtomicBool::new(false);
        let shared = shared(&stop);
        let mut board = crate::parse_sfen(QUIET).unwrap();
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
        let mut searcher = new_searcher(&pst, &board, rules, &[], &shared, &table);
        searcher.correction.update(Color::Black, key, 64, 8);
        assert_eq!(
            searcher.negamax(&mut board, 2, -INFINITY, INFINITY, 0),
            Some(score)
        );
        assert_eq!(searcher.correction.read(Color::Black, key), 16);
    }
    for interrupted in [false, true] {
        let stop = AtomicBool::new(interrupted);
        let shared = shared(&stop);
        let mut board = crate::parse_sfen(QUIET).unwrap();
        let table = TranspositionTable::new(1).unwrap();
        if !interrupted {
            table.store(search_key(&board), 2, 320, Bound::Exact, None, 0);
        }
        let key = material_key(&board);
        let mut searcher = new_searcher(&pst, &board, MoveRules::standard(), &[], &shared, &table);
        searcher.correction.update(Color::Black, key, 64, 8);
        assert_eq!(
            searcher.negamax(&mut board, 2, -INFINITY, INFINITY, 0),
            if interrupted { None } else { Some(320) }
        );
        assert_eq!(searcher.correction.read(Color::Black, key), 16);
    }
}

#[test]
fn correction_workers_are_independent_and_new_search_resets_table() {
    let pst = weights().unwrap();
    let stop = AtomicBool::new(false);
    let shared = shared(&stop);
    let board = crate::parse_sfen(QUIET).unwrap();
    let table = TranspositionTable::new(1).unwrap();
    let mut a = new_searcher(&pst, &board, MoveRules::standard(), &[], &shared, &table);
    let mut b = new_searcher(&pst, &board, MoveRules::standard(), &[], &shared, &table);
    let key = material_key(&board);
    a.correction.update(Color::Black, key, 100, 8);
    assert_eq!(a.correction.read(Color::Black, key), 25);
    assert_eq!(b.correction.read(Color::Black, key), 0);
    b.correction.update(Color::Black, key, -100, 8);
    assert_eq!(a.correction.read(Color::Black, key), 25);
    assert_eq!(b.correction.read(Color::Black, key), -25);
    let c = new_searcher(&pst, &board, MoveRules::standard(), &[], &shared, &table);
    assert_eq!(c.correction.read(Color::Black, key), 0);
}

#[test]
fn correction_changes_futility_expansion_at_boundary() {
    let pst = weights().unwrap();
    let stop = AtomicBool::new(false);
    let shared = shared(&stop);
    let mut nodes = Vec::new();
    for correction in [0, 1] {
        let mut board = crate::parse_sfen(QUIET).unwrap();
        let table = TranspositionTable::new(1).unwrap();
        let alpha = evaluate(&pst, &board) + pst.pawn_value() / 2;
        let mut searcher = new_searcher(&pst, &board, MoveRules::standard(), &[], &shared, &table);
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
    let pst = weights().unwrap();
    let stop = AtomicBool::new(false);
    let shared = shared(&stop);
    let rules = MoveRules::standard();
    let table = TranspositionTable::new(1).unwrap();
    let mut changed = 0;
    for mut board in crate::test_util::bench_positions() {
        let root_key = material_key(&board);
        let mut searcher = new_searcher(&pst, &board, rules, &[], &shared, &table);
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
    let mut board = crate::parse_sfen(CAPTURE).unwrap();
    let root_key = material_key(&board);
    let mut searcher = new_searcher(&pst, &board, rules, &[], &shared, &table);
    searcher.material_keys[1] = !root_key;
    let score = searcher
        .negamax(&mut board, 4, -10_001, -10_000, 0)
        .unwrap();
    assert!(score >= -10_000);
    assert_eq!(searcher.material_keys[1], root_key);
    assert_eq!(material_key(&board), root_key);
    assert_eq!(searcher.nodes, 0, "null moveで打ち切る");
}
