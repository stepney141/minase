//! 静かな手だけを数えるlate move pruningを検査する。

use super::*;
use crate::search::alphabeta::pruning::{futility_margin, late_move_limit};

pub(super) fn quiet_board() -> Position {
    crate::parse_sfen("k11/12/12/12/12/12/12/12/12/2G2G2G3/12/11K b").unwrap()
}

pub(super) fn search_quiet_board(board: &Position) -> (i32, u64) {
    let pst = weights().unwrap();
    let alpha = legal_moves(board)
        .into_iter()
        .map(|mv| {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            -evaluate(&pst, &child)
        })
        .max()
        .unwrap();
    assert!(alpha < evaluate(&pst, board) + futility_margin(pst.pawn_value(), 1, true));
    run_negamax(board, 1, alpha, alpha + 1, 0, &small_tt())
}

// docs/plans/search-revival-spsa.md「late move pruningの定義の変更」。
#[test]
fn late_move_limits_follow_quadratic_depth_rule() {
    assert_eq!(
        [late_move_limit(1), late_move_limit(2), late_move_limit(3)],
        [4, 7, 12]
    );
}

// docs/plans/strength-stage4.md「検証」の第1局面。5手目以降を切っても詰みにしない。
#[test]
fn late_move_pruning_reduces_quiet_nodes_without_false_mate() {
    let board = quiet_board();
    let pst = weights().unwrap();
    let moves = legal_moves(&board);
    assert!(moves.len() > 12);
    assert!(!royal_under_attack(&board));
    assert!(
        moves
            .iter()
            .all(|&mv| !mv.promote && move_order_key(&board, &pst, mv).is_none())
    );
    let (score, nodes) = search_quiet_board(&board);
    assert_eq!(nodes, 4);
    let leaf_scores: Vec<_> = moves
        .iter()
        .map(|&mv| {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            -evaluate(&pst, &child)
        })
        .collect();
    assert!(score.abs() < MATE_THRESHOLD);
    assert!(
        (*leaf_scores.iter().min().unwrap()..=*leaf_scores.iter().max().unwrap()).contains(&score)
    );
}

// 同「適用するノード」。PV、詰み帯、王駒への利きがある場合は静かな手を省かない。
#[test]
fn late_move_pruning_exclusions_search_all_quiets() {
    for (width, mate_window, attacked) in [(2, false, false), (1, true, false), (1, false, true)] {
        let board = if attacked {
            crate::parse_sfen("k11/12/12/12/12/11r/12/12/12/2G2G2G3/12/11K b").unwrap()
        } else {
            quiet_board()
        };
        assert_eq!(royal_under_attack(&board), attacked);
        let moves = legal_moves(&board);
        let history = repeated_root_children(&board, &moves);
        with_root_searcher(&board, &history, |searcher| {
            let alpha = if mate_window {
                MATE_THRESHOLD - 1
            } else {
                evaluate(searcher.pst, &board) + 1
            };
            assert!(alpha > DRAW_SCORE);
            assert_eq!(
                searcher.negamax(&mut board.clone(), 1, alpha, alpha + width, 0),
                Some(DRAW_SCORE)
            );
            assert_eq!(searcher.nodes, moves.len() as u64);
        });
    }
}

// 保護する手は静かな手の枠を消費しない。上限0の調整テストでも同じ保護を確かめる。
#[test]
fn late_move_pruning_preserves_protected_moves_and_quiet_allowance() {
    check_late_move_protected_moves(4);
}

pub(super) fn check_late_move_protected_moves(limit: u64) {
    for (sfen, promotion) in [
        ("k11/12/12/12/12/4ppp5/5N6/12/12/2G2G2G3/12/11K b", false),
        ("k11/12/12/12/5O6/12/12/12/12/2G2G2G3/12/11K b", true),
    ] {
        let board = crate::parse_sfen(sfen).unwrap();
        let moves = legal_moves(&board);
        let history = repeated_root_children(&board, &moves);
        assert!(!royal_under_attack(&board));
        with_root_searcher(&board, &history, |searcher| {
            let quiets: Vec<_> = moves
                .iter()
                .copied()
                .filter(|&mv| !mv.promote && move_order_key(&board, searcher.pst, mv).is_none())
                .collect();
            assert!(quiets.len() > 7);
            let tt_move = quiets[0];
            let killers = [Some(quiets[1]), Some(quiets[2])];
            searcher.killers[0] = killers;
            searcher
                .tt
                .store(search_key(&board), 0, 0, Bound::Upper, Some(tt_move), 0);
            // 捕獲手はSEEでも切られず、子はすべて反復引き分けになる。
            let protected = moves
                .iter()
                .filter(|&&mv| mv.promote || move_order_key(&board, searcher.pst, mv).is_some())
                .count();
            assert!(protected > 0);
            for &mv in &moves {
                if move_order_key(&board, searcher.pst, mv).is_some() {
                    assert!(!see_prunes(&board, engine_rules(), searcher.pst, mv, 2));
                }
            }
            let mut picker = MovePicker::new(Some(tt_move), killers);
            let mut ordered = Vec::new();
            while let Some((mv, capture)) = picker.next(
                &board,
                searcher.pst,
                &searcher.generator,
                searcher.history,
                &searcher.capture_history,
            ) {
                ordered.push((mv, capture));
            }
            assert!(ordered.iter().skip(4).any(|&(mv, capture)| if promotion {
                mv.promote
            } else {
                capture
            }));
            let alpha = evaluate(searcher.pst, &board) + 1;
            assert!(alpha > DRAW_SCORE);
            assert_eq!(
                searcher.negamax(&mut board.clone(), 1, alpha, alpha + 1, 0),
                Some(DRAW_SCORE)
            );
            assert_eq!(
                searcher.nodes,
                3 + protected as u64 + limit,
                "promotion={promotion}, limit={limit}"
            );
        });
    }
}

// docs/plans/strength-stage4.md「検証」の第2局面。先頭の王駒を失う手の後も安全な手を探す。
#[test]
pub(super) fn late_move_pruning_searches_safe_quiets_after_losing_tt_move() {
    let board = crate::parse_sfen("k11/12/12/12/12/10r1/12/2G9/12/G2G2G2G2/12/11K b").unwrap();
    let pst = weights().unwrap();
    let alpha = evaluate(&pst, &board) + 1;
    let losing_move = Move {
        from: sq(11, 0),
        mid: None,
        to: sq(10, 0),
        promote: false,
    };
    let moves = legal_moves(&board);
    assert!(moves.contains(&losing_move));
    assert!(!royal_under_attack(&board));
    assert!(
        moves
            .iter()
            .all(|&mv| !mv.promote && move_order_key(&board, &pst, mv).is_none())
    );
    let safe_moves = moves
        .iter()
        .filter(|&&mv| {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            !legal_moves(&child)
                .into_iter()
                .any(|reply| captures_last_royal(&child, reply))
        })
        .count();
    assert!(safe_moves > 7);
    let ply = 5;
    let mut child = board.clone();
    child.make_move_unchecked(losing_move, engine_rules());
    let (opponent_score, _) = run_negamax(&child, 1, -INFINITY, INFINITY, ply + 1, &small_tt());
    assert!(opponent_score >= MATE_THRESHOLD);
    let table = small_tt();
    // 先頭手の負けを固定し、親が後続の安全な手を読むかを検査する。
    table.store(
        search_key(&child),
        1,
        opponent_score,
        Bound::Exact,
        None,
        ply + 1,
    );
    table.store(
        search_key(&board),
        0,
        0,
        Bound::Upper,
        Some(losing_move),
        ply,
    );
    let (score, _) = run_negamax(&board, 2, alpha, alpha + 1, ply, &table);
    assert!(score.abs() < MATE_THRESHOLD, "score={score}");
    let hit = table.probe(search_key(&board), ply).unwrap();
    assert_eq!(hit.score, score);
    assert!(hit.score.abs() < MATE_THRESHOLD);
    assert_ne!(hit.best_move, Some(losing_move));
}

// 数だけでなく順序も検査する。対象手以外の子を引き分けに固定し、対象手を読んだかを値で識別する。
#[test]
fn late_move_pruning_searches_exactly_the_first_four_counted_quiets() {
    let board = quiet_board();
    let mut ordered = Vec::new();
    with_root_searcher(&board, &[], |searcher| {
        let mut picker = MovePicker::new(None, [None; KILLER_COUNT]);
        while let Some((mv, capture)) = picker.next(
            &board,
            searcher.pst,
            &searcher.generator,
            searcher.history,
            &searcher.capture_history,
        ) {
            assert!(!capture && !mv.promote);
            ordered.push(mv);
        }
    });
    assert!(ordered.len() > 4);
    for (index, &target) in ordered.iter().enumerate() {
        let other_moves: Vec<_> = ordered.iter().copied().filter(|&mv| mv != target).collect();
        let history = repeated_root_children(&board, &other_moves);
        with_root_searcher(&board, &history, |searcher| {
            let alpha = evaluate(searcher.pst, &board) + 100;
            let mut child = board.clone();
            child.make_move_unchecked(target, engine_rules());
            let target_score = -evaluate(searcher.pst, &child);
            assert!(target_score > DRAW_SCORE && target_score < alpha);
            let score = searcher
                .negamax(&mut board.clone(), 1, alpha, alpha + 1, 0)
                .unwrap();
            assert_eq!(
                score,
                if index < 4 { target_score } else { DRAW_SCORE },
                "index={index}"
            );
            assert_eq!(searcher.nodes, 4);
        });
    }
}
