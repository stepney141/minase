//! 静かな手だけを数えるlate move pruningを検査する。

use super::*;
#[cfg(feature = "tuning")]
use crate::search::REPETITION_BAND_START;
#[cfg(feature = "tuning")]
use crate::search::alphabeta::params;
#[cfg(feature = "tuning")]
use crate::search::alphabeta::pruning::{futility_value, late_move_limit, pruning_lmr_depth};

// 金将を離して捕獲を避け、1枚を10筋11段に置いて全着手の評価増分をfutilityの余裕内に収める。
// 各着手の評価を引き分けより高く保ち、静かな手の上限と探索順序を観測する。
pub(super) fn quiet_board() -> Position {
    position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(1, 9), Color::White, PieceKind::GoldGeneral),
            (sq(10, 9), Color::White, PieceKind::GoldGeneral),
            (sq(3, 2), Color::Black, PieceKind::GoldGeneral),
            (sq(9, 2), Color::Black, PieceKind::GoldGeneral),
            (sq(2, 1), Color::Black, PieceKind::GoldGeneral),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    )
}

// 葉でβ打ち切りが起こらず、親でfutility pruningが発動しない窓を作る。
#[cfg(feature = "tuning")]
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
    let lmr_depth = pruning_lmr_depth(1, 1, false, false, false);
    assert!(alpha < futility_value(nnue.pawn_value(), static_eval, alpha, lmr_depth));
    alpha
}

#[cfg(feature = "tuning")]
pub(super) fn search_quiet_board(board: &Position) -> (i32, u64) {
    let futility = [
        params::futility_base(),
        params::futility_lmr_depth(),
        params::futility_above_alpha(),
    ];
    // late move pruningだけを観測できるよう、先行するfutilityの見積りを十分高くする。
    params::set("FutilityBase", 400).unwrap();
    params::set("FutilityLmrDepth", 0).unwrap();
    params::set("FutilityAboveAlpha", 0).unwrap();
    let alpha = quiet_alpha(board, test_nnue());
    let result = run_negamax(board, 1, alpha, alpha + 1, 0, &small_tt());
    params::set("FutilityBase", futility[0]).unwrap();
    params::set("FutilityLmrDepth", futility[1]).unwrap();
    params::set("FutilityAboveAlpha", futility[2]).unwrap();
    result
}

// 上限は基数と深さ²の項の和を百分率で換算し、improvingでなければ倍率を掛ける。
// 数える手の範囲を選んだ理由は docs/adr/apply-late-move-pruning-at-all-depths-and-pv-nodes-counting-only-quiet-moves.md にある。
#[test]
fn late_move_limits_follow_quadratic_depth_rule() {
    for (depth, improving, expected) in [
        (1, true, 18),
        (1, false, 5),
        (2, true, 19),
        (2, false, 6),
        (3, true, 22),
        (3, false, 6),
        (4, true, 25),
        (4, false, 7),
        (MAX_PLY, true, 31_475),
        (MAX_PLY, false, 9_757),
    ] {
        assert_eq!(
            super::pruning::late_move_coefficients().late_move_limit(depth, improving),
            expected,
            "depth={depth}, improving={improving}"
        );
    }
}

// 調整値は専用の子プロセスで変更し、呼び出し元へ戻る前に変更前の値へ戻す。
#[cfg(feature = "tuning")]
pub(super) fn late_move_non_improving_parameter_scales_limit() {
    let default_scale = params::lmp_non_improving();
    let default_base = params::lmp_base();
    let default_slope = params::lmp_slope();
    params::set("LmpBase", 300).unwrap();
    params::set("LmpSlope", 100).unwrap();
    params::set("LmpNonImproving", 150).unwrap();
    assert_eq!(late_move_limit(1, false), 6);
    assert_eq!(late_move_limit(1, true), 4);
    assert_eq!(search_quiet_board(&quiet_board()).1, 6);
    // 499×150/10000→7。先に499/100を切り捨てると6になってしまう。
    params::set("LmpBase", 399).unwrap();
    assert_eq!(late_move_limit(1, false), 7);
    assert_eq!(late_move_limit(1, true), 4);
    params::set("LmpBase", default_base).unwrap();
    params::set("LmpSlope", default_slope).unwrap();
    params::set("LmpNonImproving", default_scale).unwrap();
}

// Stockfish src/search.cpp:1080, 1199。比較で良化していなくても静的評価がβ以上ならimproving側の上限を使う。
#[cfg(feature = "tuning")]
pub(super) fn late_move_pruning_uses_improving_updated_after_null_move() {
    let nnue = test_nnue();
    assert_eq!(nnue.pawn_value(), 100, "固定ネットの歩兵価値");
    let board = quiet_board();
    let moves = legal_moves(&board);
    let improved_limit = (params::lmp_base() + params::lmp_slope()) / 100;
    let other_limit =
        (params::lmp_base() + params::lmp_slope()) * params::lmp_non_improving() / 10000;

    assert!(!royal_under_attack(&board));
    for (beta_offset, expected_nodes) in [
        (-1, improved_limit as u64),
        (0, improved_limit as u64),
        (1, other_limit as u64),
    ] {
        with_nnue_root_searcher(nnue, &board, &[], |searcher| {
            searcher.seek_mate = true;
            searcher
                .tt
                .store(search_key(&board), 0, 0, Bound::Upper, None, 2, true);
            let side = board.side_to_move();
            let key = material_key(&board);
            searcher.correction.update(side, key, 1000, 8);
            let correction = searcher.correction.read(side, key);
            assert!(correction > 0);
            let static_eval = evaluate(nnue, &board) + correction;
            searcher.accumulators[2] = searcher.accumulators[0].clone();
            searcher.material_keys[2] = key;
            searcher.static_evals[0] = static_eval + 1000;
            searcher.static_evals[1] = -static_eval;
            assert!(!searcher.improving(static_eval, 2));
            let beta = static_eval + beta_offset;
            let alpha = beta - 1;
            let updated_improving = beta_offset <= 0;
            let lmr_depth = pruning_lmr_depth(1, 1, updated_improving, false, true);
            assert!(
                futility_value(nnue.pawn_value(), static_eval, alpha, lmr_depth) > alpha,
                "先行するfutility pruningは発動しない"
            );
            for &mv in &moves {
                assert!(!mv.promote && move_order_key(&board, nnue, mv).is_none());
                let mut child = board.clone();
                child.make_move_unchecked(mv, engine_rules());
                assert_eq!(
                    searcher
                        .correction
                        .read(child.side_to_move(), material_key(&child)),
                    0
                );
                assert!(-evaluate(nnue, &child) < alpha, "子の値は窓を下回る");
                assert!(legal_moves(&child).iter().all(|&reply| {
                    !reply.promote && move_order_key(&child, nnue, reply).is_none()
                }));
            }
            set_test_path_ply(searcher, 2);
            let score = searcher
                .negamax(&mut board.clone(), 1, alpha, beta, 2, false)
                .unwrap();
            assert_eq!(searcher.static_evals[2], static_eval);
            assert!(score < alpha && score.abs() < MATE_THRESHOLD);
            assert_eq!(
                searcher.nodes,
                expected_nodes.max(1).min(moves.len() as u64),
                "beta_offset={beta_offset}"
            );
        });
    }
}

// 深さ4の零窓と、前の反復の主変化を辿るPVノードでも上限後の静かな手を省く。
#[cfg(feature = "tuning")]
pub(super) fn late_move_pruning_applies_at_depth_four_and_follow_pv_nodes() {
    let nnue = test_nnue();
    let board = quiet_board();
    let moves = legal_moves(&board);
    let history = repeated_root_children(&board, &moves);
    let depth = 4;
    let expected_nodes = u64::try_from(late_move_limit(depth, false)).unwrap();
    assert!(!royal_under_attack(&board));
    assert!(
        moves
            .iter()
            .all(|&mv| !mv.promote && move_order_key(&board, nnue, mv).is_none())
    );
    assert!(expected_nodes > 0 && expected_nodes < moves.len() as u64);

    for (case, width, follow_pv) in [("zero-window", 1, false), ("PV", 2, true)] {
        with_nnue_root_searcher(nnue, &board, &history, |searcher| {
            searcher.seek_mate = true;
            searcher.null_move_ply = Some(0);
            searcher.follow_pv[0] = follow_pv;
            let alpha = evaluate(searcher.nnue, &board) + 1;
            assert!(alpha > DRAW_SCORE);
            assert_eq!(
                searcher.negamax(&mut board.clone(), depth, alpha, alpha + width, 0, false),
                Some(DRAW_SCORE)
            );
            assert_eq!(searcher.nodes, expected_nodes, "case={case}");
        });
    }
}

// αまたはβが詰み帯と専用の帯に達する場合は、浅いノードでも深いノードでも静かな手を省かない。
#[cfg(feature = "tuning")]
pub(super) fn late_move_pruning_does_not_apply_when_alpha_or_beta_is_in_reserved_band() {
    let board = quiet_board();
    let moves = legal_moves(&board);
    let history = repeated_root_children(&board, &moves);

    for depth in [1, 4] {
        let limit = u64::try_from(late_move_limit(depth, false)).unwrap();
        assert!(limit < moves.len() as u64);
        for (case, alpha, beta) in [
            ("alpha", -REPETITION_BAND_START, 1),
            ("beta", 0, REPETITION_BAND_START),
        ] {
            assert!(alpha < beta);
            assert_eq!(alpha.abs() >= REPETITION_BAND_START, case == "alpha");
            assert_eq!(beta.abs() >= REPETITION_BAND_START, case == "beta");
            with_root_searcher(&board, &history, |searcher| {
                searcher.seek_mate = true;
                searcher.null_move_ply = Some(0);
                searcher.follow_pv[0] = true;
                assert_eq!(
                    searcher.negamax(&mut board.clone(), depth, alpha, beta, 0, false),
                    Some(DRAW_SCORE)
                );
                assert_eq!(
                    searcher.nodes,
                    moves.len() as u64,
                    "case={case}, depth={depth}"
                );
            });
        }
    }
}

// 手番側の王駒に相手の疑似利きが届く場合は、深さ4でも静かな手を省かない。
#[cfg(feature = "tuning")]
pub(super) fn late_move_pruning_does_not_apply_while_royal_is_attacked() {
    // 金将1枚を8筋8段に置き、飛車の王手を保ったまま静的評価を正にして引き分けで窓を打ち切らせない。
    let board = position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(11, 6), Color::White, PieceKind::Rook),
            (sq(4, 4), Color::Black, PieceKind::GoldGeneral),
            (sq(5, 2), Color::Black, PieceKind::GoldGeneral),
            (sq(8, 2), Color::Black, PieceKind::GoldGeneral),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    );
    let moves = legal_moves(&board);
    let history = repeated_root_children(&board, &moves);
    let depth = 4;
    let limit = usize::try_from(late_move_limit(depth, false)).unwrap();
    let counted_quiets = moves
        .iter()
        .filter(|&&mv| !mv.promote && move_order_key(&board, test_nnue(), mv).is_none())
        .count();
    assert!(royal_under_attack(&board));
    assert!(counted_quiets > limit);

    with_root_searcher(&board, &history, |searcher| {
        searcher.seek_mate = true;
        searcher.null_move_ply = Some(0);
        let alpha = evaluate(searcher.nnue, &board) + 1;
        assert!(alpha > DRAW_SCORE);
        assert_eq!(
            searcher.negamax(&mut board.clone(), depth, alpha, alpha + 1, 0, false),
            Some(DRAW_SCORE)
        );
        assert_eq!(searcher.nodes, moves.len() as u64);
    });
}

// 保護する手は静かな手の枠を消費しない。上限0の調整テストでも同じ保護を確かめる。
#[cfg(feature = "tuning")]
pub(super) fn late_move_pruning_preserves_protected_moves_and_quiet_allowance() {
    check_late_move_protected_moves(late_move_limit(1, true) as u64);
}

#[cfg(feature = "tuning")]
pub(super) fn check_late_move_protected_moves(limit: u64) {
    for (board, promotion) in [
        (
            position(
                Color::Black,
                &[
                    (sq(0, 11), Color::White, PieceKind::King),
                    (sq(4, 4), Color::White, PieceKind::Pawn),
                    (sq(5, 4), Color::White, PieceKind::Pawn),
                    (sq(6, 4), Color::White, PieceKind::Pawn),
                    (sq(5, 5), Color::Black, PieceKind::Lion),
                    (sq(2, 2), Color::Black, PieceKind::GoldGeneral),
                    (sq(5, 2), Color::Black, PieceKind::GoldGeneral),
                    (sq(8, 2), Color::Black, PieceKind::GoldGeneral),
                    (sq(11, 0), Color::Black, PieceKind::King),
                ],
            ),
            false,
        ),
        (
            position(
                Color::Black,
                &[
                    (sq(0, 11), Color::White, PieceKind::King),
                    (sq(5, 7), Color::Black, PieceKind::Kirin),
                    (sq(2, 2), Color::Black, PieceKind::GoldGeneral),
                    (sq(5, 2), Color::Black, PieceKind::GoldGeneral),
                    (sq(8, 2), Color::Black, PieceKind::GoldGeneral),
                    (sq(11, 0), Color::Black, PieceKind::King),
                ],
            ),
            true,
        ),
    ] {
        let moves = legal_moves(&board);
        let history = repeated_root_children(&board, &moves);
        assert!(!royal_under_attack(&board));
        with_root_searcher(&board, &history, |searcher| {
            searcher.seek_mate = true;
            let quiets: Vec<_> = moves
                .iter()
                .copied()
                .filter(|&mv| !mv.promote && move_order_key(&board, searcher.nnue, mv).is_none())
                .collect();
            assert!(quiets.len() > 3);
            let tt_move = quiets[0];
            let killers = [Some(quiets[1]), Some(quiets[2])];
            searcher.killers[0] = killers;
            searcher.tt.store(
                search_key(&board),
                0,
                evaluate(searcher.nnue, &board),
                Bound::Upper,
                Some(tt_move),
                0,
                false,
            );
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
            let mut counted_quiets = 0;
            for &(mv, capture) in &ordered {
                let counted_quiet =
                    mv != tt_move && !capture && !mv.promote && !killers.contains(&Some(mv));
                let late_move_pruned = counted_quiet && counted_quiets >= limit;
                counted_quiets += u64::from(counted_quiet);
                if !late_move_pruned && mv != tt_move && !capture && !mv.promote {
                    assert!(mv.mid.is_none());
                    assert!(
                        !see_prunes(&board, engine_rules(), searcher.nnue, mv, 0),
                        "静かな手のSEEによる枝刈りをこの局面から除外する: {mv:?}"
                    );
                }
            }
            assert!(ordered.iter().any(|&(mv, capture)| if promotion {
                mv.promote
            } else {
                capture
            }));
            let alpha = evaluate(searcher.nnue, &board) - 1;
            assert!(alpha > DRAW_SCORE);
            assert_eq!(
                searcher.negamax(&mut board.clone(), 1, alpha, alpha + 1, 0, false),
                Some(DRAW_SCORE)
            );
            assert_eq!(
                searcher.nodes,
                3 + protected as u64 + limit.min(quiets.len() as u64 - 3),
                "promotion={promotion}, limit={limit}"
            );
        });
    }
}

// 先頭手の評価が負の詰み帯にあれば、静かな手の数が上限に達しても安全な手を探し続ける。
#[test]
pub(super) fn late_move_pruning_searches_safe_quiets_after_losing_tt_move() {
    let board = position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(10, 6), Color::White, PieceKind::Rook),
            (sq(2, 4), Color::Black, PieceKind::GoldGeneral),
            (sq(0, 2), Color::Black, PieceKind::GoldGeneral),
            (sq(3, 2), Color::Black, PieceKind::GoldGeneral),
            (sq(6, 2), Color::Black, PieceKind::GoldGeneral),
            (sq(9, 2), Color::Black, PieceKind::GoldGeneral),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    );
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
    assert!(safe_moves > 0);
    let ply = 5;
    let mut child = board.clone();
    child.make_move_unchecked(losing_move, engine_rules());
    let (opponent_score, _) =
        run_move_pruning(&child, 1, -INFINITY, INFINITY, ply + 1, &small_tt());
    assert!(opponent_score >= MATE_THRESHOLD);
    let table = small_tt();
    // 先頭手の負けを固定し、親が後続の安全な手を読むかを検査する。
    // F5ではβを超える値に要求深さより1深い記録が要る。
    table.store(
        search_key(&child),
        2,
        opponent_score,
        Bound::Exact,
        None,
        ply + 1,
        false,
    );
    table.store(
        search_key(&board),
        0,
        0,
        Bound::Upper,
        Some(losing_move),
        ply,
        false,
    );
    let (score, _) = run_move_pruning(&board, 2, alpha, alpha + 1, ply, &table);
    assert!(score.abs() < MATE_THRESHOLD, "score={score}");
    let hit = table.probe(search_key(&board), ply).unwrap();
    assert_eq!(hit.score, score);
    assert!(hit.score.abs() < MATE_THRESHOLD);
    assert_ne!(hit.best_move, Some(losing_move));
}

// 数だけでなく順序も検査する。対象手以外の子を引き分けに固定し、対象手を読んだかを値で識別する。
#[cfg(feature = "tuning")]
pub(super) fn late_move_pruning_searches_exactly_the_first_counted_quiets_up_to_limit() {
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
    let static_eval = evaluate(test_nnue(), &board);
    let lmr_depth = pruning_lmr_depth(1, 1, false, false, true);
    let futility = futility_value(100, static_eval, static_eval, lmr_depth);
    assert!(futility > static_eval);
    let margin = futility - static_eval;
    let improving = false;
    let limit = late_move_limit(1, improving) as usize;
    let limit = limit.max(1).min(ordered.len());
    for (index, &target) in ordered.iter().enumerate() {
        let other_moves: Vec<_> = ordered.iter().copied().filter(|&mv| mv != target).collect();
        let history = repeated_root_children(&board, &other_moves);
        with_root_searcher(&board, &history, |searcher| {
            let alpha = evaluate(searcher.nnue, &board) + margin - 1;
            searcher.seek_mate = true;
            searcher
                .tt
                .store(search_key(&board), 0, alpha, Bound::Upper, None, 0, true);
            let mut child = board.clone();
            child.make_move_unchecked(target, engine_rules());
            let target_score = -evaluate(searcher.nnue, &child);
            assert!(target_score > DRAW_SCORE && alpha > DRAW_SCORE);
            let score = searcher
                .negamax(&mut board.clone(), 1, alpha, alpha + 1, 0, false)
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
            assert_eq!(
                searcher.nodes,
                if index < limit && target_score > alpha {
                    index as u64 + 1
                } else {
                    limit as u64
                }
            );
        });
    }
}
