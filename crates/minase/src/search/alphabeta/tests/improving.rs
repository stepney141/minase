//! 補正前の静的評価による良化判定を検査する。

use super::*;
use crate::search::alphabeta::params;

// 同じ手番の2手前の静的評価と比べ、上昇だけを真とし、同値と下降は偽とする。
// 適用範囲の理由は docs/adr/use-improving-only-for-futility-margins.md にある。
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
            let bare_acc = searcher.nnue.refresh_accumulator(&bare);
            let richer_acc = searcher.nnue.refresh_accumulator(&richer);
            assert!(evaluate(searcher.nnue, &richer) > evaluate(searcher.nnue, &bare));
            for (previous, current, expected) in [
                (&bare_acc, &richer_acc, true),
                (&bare_acc, &bare_acc, false),
                (&richer_acc, &bare_acc, false),
            ] {
                let ply = 4;
                searcher.accumulators[2] = previous.clone();
                searcher.accumulators[3] = current.clone();
                searcher.accumulators[4] = current.clone();
                let static_eval = searcher.nnue.evaluate_accumulator(current, side);
                assert_eq!(searcher.improving(static_eval, side, ply), expected);
            }
        });
    }
}

// 比較区間が存在しない場合とnull moveを含む場合は追加の枝刈りを行わない。
#[test]
fn improving_preserves_margins_without_two_real_plies() {
    let board = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    with_root_searcher(&board, &[], |searcher| {
        let static_eval = evaluate(searcher.nnue, &board);
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

// 縮めたfutility余裕値の境界では、良化していないノードだけ後続の静かな手を刈る。
#[test]
fn futility_prunes_quiets_only_when_not_improving() {
    let board = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    let worse = minase_core::parse_sfen("k11/12/12/12/12/5r6/12/12/12/12/12/11K b").unwrap();
    let moves = legal_moves(&board);
    for improving in [false, true] {
        with_root_searcher(&board, &[], |searcher| {
            let static_eval = evaluate(searcher.nnue, &board);
            let margin = searcher.nnue.pawn_value() * params::futility_margin1() / 100;
            let reduced = margin * params::non_improving_futility1() / 100;
            assert!(reduced < margin);
            let alpha = static_eval + reduced;
            if improving {
                assert!(evaluate(searcher.nnue, &worse) < static_eval);
                searcher.accumulators[0] = searcher.nnue.refresh_accumulator(&worse);
            }
            assert_eq!(
                searcher.improving(static_eval, board.side_to_move(), 2),
                improving
            );
            assert!(!royal_under_attack(&board));
            for &mv in &moves {
                assert!(!mv.promote && move_order_key(&board, searcher.nnue, mv).is_none());
                let mut child = board.clone();
                child.make_move_unchecked(mv, engine_rules());
                assert!(-evaluate(searcher.nnue, &child) < alpha, "全手が窓を下回る");
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

// 良化判定にはcorrection historyの補正を含めず、補正前の静的評価を比べる。
#[test]
fn improving_uses_raw_evaluation_before_correction() {
    let board = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    with_root_searcher(&board, &[], |searcher| {
        let side = board.side_to_move();
        let key = material_key(&board);
        searcher.correction.update(side, key, 100, 8);
        let correction = searcher.correction.read(side, key);
        assert!(correction > 0);
        let alpha = evaluate(searcher.nnue, &board)
            + correction
            + searcher.nnue.pawn_value() * params::futility_margin1() / 100
                * params::non_improving_futility1()
                / 100;
        let score = searcher
            .negamax(&mut board.clone(), 1, alpha, alpha + 1, 2)
            .unwrap();
        assert!(score < alpha && score.abs() < MATE_THRESHOLD);
        assert_eq!(searcher.nodes, 1);
    });
}
