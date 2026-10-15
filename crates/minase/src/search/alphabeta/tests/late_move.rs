//! 静かな手だけを数えるlate move pruningを検査する。

use super::*;
use crate::search::alphabeta::params;
use crate::search::alphabeta::pruning::{futility_margin, late_move_limit};

// 金将を離して捕獲を避け、1枚を10筋11段に置いて全着手の評価増分をfutilityの余裕内に収める。
// 各着手の評価を引き分けより高く保ち、静かな手の上限と探索順序を観測する。
pub(super) fn quiet_board() -> Position {
    minase_core::parse_sfen("k11/12/1g8g1/12/12/12/12/12/12/3G5G2/2G9/11K b").unwrap()
}

// 葉でβ打ち切りが起こらず、親でfutility pruningが発動しない窓を作る。
fn quiet_alpha(board: &Position, nnue: &Nnue) -> i32 {
    let static_eval = evaluate(nnue, board);
    let alpha = legal_moves(board)
        .into_iter()
        .map(|mv| {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            -evaluate(nnue, &child)
        })
        .max()
        .unwrap()
        .max(static_eval)
        + 1;
    assert!(alpha < static_eval + futility_margin(nnue.pawn_value(), 1, true));
    alpha
}

#[cfg(feature = "tuning")]
pub(super) fn search_quiet_board(board: &Position) -> (i32, u64) {
    let alpha = quiet_alpha(board, test_nnue());
    run_negamax(board, 1, alpha, alpha + 1, 0, &small_tt())
}

// 静かな手の上限は `(LmpBase + LmpSlope × 深さ²) / 100` を整数除算した値である。
// 数える手の範囲を選んだ理由は docs/adr/count-only-quiet-moves-for-late-move-pruning.md にある。
#[test]
fn late_move_limits_follow_quadratic_depth_rule() {
    // 基数382、傾き133では分子が515、914、1579になり、100で整数除算する。
    assert_eq!((params::lmp_base(), params::lmp_slope()), (382, 133));
    for (depth, expected) in [(1, 5), (2, 9), (3, 15)] {
        assert_eq!(late_move_limit(depth), expected, "depth={depth}");
    }
}

// PV、詰み帯、王駒への利きがある場合は静かな手を省かない。
#[test]
fn late_move_pruning_exclusions_search_all_quiets() {
    let nnue = test_nnue();
    for (width, mate_window, attacked) in [(2, false, false), (1, true, false), (1, false, true)] {
        let board = if attacked {
            // 金将1枚を8筋8段に置き、飛車の王手を保ったまま静的評価を正にして引き分けで窓を打ち切らせない。
            minase_core::parse_sfen("k11/12/12/12/12/11r/12/4G7/12/5G2G3/12/11K b").unwrap()
        } else {
            quiet_board()
        };
        assert_eq!(royal_under_attack(&board), attacked);
        let moves = legal_moves(&board);
        let history = repeated_root_children(&board, &moves);
        with_nnue_root_searcher(nnue, &board, &history, |searcher| {
            let alpha = if mate_window {
                MATE_THRESHOLD - 1
            } else {
                evaluate(searcher.nnue, &board) + 1
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
    check_late_move_protected_moves(late_move_limit(1) as u64);
}

pub(super) fn check_late_move_protected_moves(limit: u64) {
    for (sfen, promotion) in [
        ("k11/12/12/12/12/4ppp5/5N6/12/12/2G2G2G3/12/11K b", false),
        ("k11/12/12/12/5O6/12/12/12/12/2G2G2G3/12/11K b", true),
    ] {
        let board = minase_core::parse_sfen(sfen).unwrap();
        let moves = legal_moves(&board);
        let history = repeated_root_children(&board, &moves);
        assert!(!royal_under_attack(&board));
        with_root_searcher(&board, &history, |searcher| {
            let quiets: Vec<_> = moves
                .iter()
                .copied()
                .filter(|&mv| !mv.promote && move_order_key(&board, searcher.nnue, mv).is_none())
                .collect();
            assert!(quiets.len() > 3 + limit as usize);
            let tt_move = quiets[0];
            let killers = [Some(quiets[1]), Some(quiets[2])];
            searcher.killers[0] = killers;
            searcher
                .tt
                .store(search_key(&board), 0, 0, Bound::Upper, Some(tt_move), 0);
            // 捕獲手はSEEでも切られず、子はすべて反復引き分けになる。
            let protected = moves
                .iter()
                .filter(|&&mv| mv.promote || move_order_key(&board, searcher.nnue, mv).is_some())
                .count();
            assert!(protected > 0);
            for &mv in &moves {
                if move_order_key(&board, searcher.nnue, mv).is_some() {
                    assert!(!see_prunes(&board, engine_rules(), searcher.nnue, mv, 2));
                }
            }
            let mut picker = MovePicker::new(Some(tt_move), killers);
            let mut ordered = Vec::new();
            while let Some((mv, capture)) = picker.next(
                &board,
                searcher.nnue,
                &searcher.generator,
                searcher.history,
                searcher.capture_history,
            ) {
                ordered.push((mv, capture));
            }
            assert!(
                ordered
                    .iter()
                    .skip(limit as usize)
                    .any(|&(mv, capture)| if promotion { mv.promote } else { capture })
            );
            let alpha = evaluate(searcher.nnue, &board) + 1;
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

// 先頭手の評価が負の詰み帯にあれば、静かな手の数が上限に達しても安全な手を探し続ける。
#[test]
pub(super) fn late_move_pruning_searches_safe_quiets_after_losing_tt_move() {
    let board =
        minase_core::parse_sfen("k11/12/12/12/12/10r1/12/2G9/12/G2G2G2G2/12/11K b").unwrap();
    let nnue = test_nnue();
    let alpha = evaluate(nnue, &board) + 1;
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
            .all(|&mv| !mv.promote && move_order_key(&board, nnue, mv).is_none())
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
    assert!(safe_moves > late_move_limit(2) as usize);
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
fn late_move_pruning_searches_exactly_the_first_counted_quiets_up_to_limit() {
    let board = quiet_board();
    let mut ordered = Vec::new();
    with_root_searcher(&board, &[], |searcher| {
        let mut picker = MovePicker::new(None, [None; KILLER_COUNT]);
        while let Some((mv, capture)) = picker.next(
            &board,
            searcher.nnue,
            &searcher.generator,
            searcher.history,
            searcher.capture_history,
        ) {
            assert!(!capture && !mv.promote);
            ordered.push(mv);
        }
    });
    let limit = late_move_limit(1) as usize;
    assert!(ordered.len() > limit);
    for (index, &target) in ordered.iter().enumerate() {
        let other_moves: Vec<_> = ordered.iter().copied().filter(|&mv| mv != target).collect();
        let history = repeated_root_children(&board, &other_moves);
        with_root_searcher(&board, &history, |searcher| {
            let alpha = quiet_alpha(&board, searcher.nnue);
            let mut child = board.clone();
            child.make_move_unchecked(target, engine_rules());
            let target_score = -evaluate(searcher.nnue, &child);
            assert!(target_score > DRAW_SCORE && target_score < alpha);
            let score = searcher
                .negamax(&mut board.clone(), 1, alpha, alpha + 1, 0)
                .unwrap();
            assert_eq!(
                score,
                if index < limit {
                    target_score
                } else {
                    DRAW_SCORE
                },
                "index={index}"
            );
            assert_eq!(searcher.nodes, limit as u64);
        });
    }
}
