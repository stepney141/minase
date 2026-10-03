//! 補正前の静的評価による良化判定を検査する。

use super::*;
use crate::search::alphabeta::pruning::futility_margin;

// フェーズ1-C指示書。現行余裕値101%、196%、207%へ25%、50%、50%を順に掛ける。
#[test]
fn futility_margins_follow_improving_thresholds() {
    for (improving, expected) in [(false, [25, 98, 103]), (true, [101, 196, 207])] {
        for (index, margin) in expected.into_iter().enumerate() {
            assert_eq!(futility_margin(100, index as u32 + 1, improving), margin);
        }
    }
    assert_eq!(futility_margin(137, 1, false), 34);
    assert_eq!(futility_margin(137, 2, false), 134);
    assert_eq!(futility_margin(137, 3, false), 141);
}

// docs/plans/strength-stage8.md「improvingの定義」。両手番から見て上昇だけを真とし、同値と下降は偽とする。
#[test]
fn improving_compares_static_evaluations_from_the_current_side() {
    for side in [Color::Black, Color::White] {
        let bare = position(
            side,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(11, 11), Color::White, PieceKind::King),
            ],
        );
        let richer = position(
            side,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(11, 11), Color::White, PieceKind::King),
                (sq(5, 5), side, PieceKind::Rook),
            ],
        );
        with_root_searcher(&bare, &[], |searcher| {
            let bare_acc = searcher.pst.refresh_accumulator(&bare);
            let richer_acc = searcher.pst.refresh_accumulator(&richer);
            assert!(evaluate(searcher.pst, &richer) > evaluate(searcher.pst, &bare));
            for (previous, current, expected) in [
                (bare_acc, richer_acc, true),
                (bare_acc, bare_acc, false),
                (richer_acc, bare_acc, false),
            ] {
                let ply = 4;
                searcher.accumulators[2] = previous;
                searcher.accumulators[3] = current;
                searcher.accumulators[4] = current;
                let static_eval = searcher.pst.evaluate_accumulator(current, side);
                assert_eq!(searcher.improving(static_eval, side, ply), expected);
            }
        });
    }
}

// 比較区間が存在しない場合とnull moveを含む場合は追加の枝刈りを行わない。
#[test]
fn improving_preserves_margins_without_two_real_plies() {
    let board = crate::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    with_root_searcher(&board, &[], |searcher| {
        let static_eval = evaluate(searcher.pst, &board);
        for ply in [0, 1] {
            assert!(searcher.improving(static_eval, board.side_to_move(), ply));
        }
        let ply = 4;
        for (null_move_ply, expected) in [
            (None, false),
            (Some(ply), true),
            (Some(ply - 1), true),
            (Some(ply - 2), false),
        ] {
            searcher.null_move_ply = null_move_ply;
            assert_eq!(
                searcher.improving(static_eval, board.side_to_move(), ply),
                expected,
                "null_move_ply={null_move_ply:?}"
            );
        }
    });
}

// 同「improvingフラグ」。縮めた余裕値の境界で、上昇していないノードだけ後続手を刈る。
#[test]
fn futility_prunes_quiets_only_when_not_improving() {
    let board = crate::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    let worse = crate::parse_sfen("k11/12/12/12/12/5r6/12/12/12/12/12/11K b").unwrap();
    let moves = legal_moves(&board);
    for improving in [false, true] {
        with_root_searcher(&board, &[], |searcher| {
            let static_eval = evaluate(searcher.pst, &board);
            assert_eq!(searcher.pst.pawn_value(), 100);
            let alpha = static_eval + 25;
            if improving {
                assert!(evaluate(searcher.pst, &worse) < static_eval);
                searcher.accumulators[0] = searcher.pst.refresh_accumulator(&worse);
            }
            assert_eq!(
                searcher.improving(static_eval, board.side_to_move(), 2),
                improving
            );
            assert!(!royal_under_attack(&board));
            for &mv in &moves {
                assert!(!mv.promote && move_order_key(&board, searcher.pst, mv).is_none());
                let mut child = board.clone();
                child.make_move_unchecked(mv, engine_rules());
                assert!(-evaluate(searcher.pst, &child) < alpha, "全手が窓を下回る");
            }
            let score = searcher
                .negamax(&mut board.clone(), 1, alpha, alpha + 1, 2)
                .unwrap();
            assert!(score < alpha && score.abs() < MATE_THRESHOLD);
            assert_eq!(
                searcher.nodes,
                if improving { moves.len() as u64 } else { 1 }
            );
        });
    }
}

// docs/plans/strength-stage8.md「improvingの定義」。履歴補正は良化判定に含めない。
#[test]
fn improving_uses_raw_evaluation_before_correction() {
    let board = crate::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    with_root_searcher(&board, &[], |searcher| {
        let side = board.side_to_move();
        let key = material_key(&board);
        searcher.correction.update(side, key, 100, 8);
        let correction = searcher.correction.read(side, key);
        assert!(correction > 0);
        let alpha = evaluate(searcher.pst, &board) + correction + 25;
        let score = searcher
            .negamax(&mut board.clone(), 1, alpha, alpha + 1, 2)
            .unwrap();
        assert!(score < alpha && score.abs() < MATE_THRESHOLD);
        assert_eq!(searcher.nodes, 1);
    });
}
