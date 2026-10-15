//! 枝刈りと深さの削減を検査する。

use super::*;
use crate::search::alphabeta::params;

// LMR表は `floor(ln(深さ) × ln(手番号) / (LmrDivisor / 100))` とし、0番の行と列は0にする。
// 式の理由は docs/adr/reduce-lmr-by-log-table-with-stepped-history-adjustment.md にある。
#[test]
fn lmr_table_follows_logarithmic_rule() {
    let table = lmr_table();
    assert!(table[0].iter().all(|&value| value == 0));
    for row in table {
        assert_eq!(row[0], 0);
        assert_eq!(row[1], 0);
    }
    for (depth, index) in [(3, 3), (4, 8), (5, 255)] {
        let expected = ((depth as f64).ln() * (index as f64).ln()
            / (f64::from(params::lmr_divisor()) / 100.0))
            .floor() as u8;
        assert_eq!(table[depth][index], expected);
    }
    assert!(table[5][255] > 3);
}

#[test]
fn lmr_history_adjustment_precedes_clamping() {
    let threshold = params::lmr_history_threshold();
    let base = i32::from(lmr_table()[4][8]);
    for (history, adjustment) in [
        (i32::MIN, 1),
        (-threshold - 1, 1),
        (-threshold, 1),
        (-threshold + 1, 0),
        (0, 0),
        (threshold - 1, 0),
        (threshold, -1),
        (threshold + 1, -1),
        (i32::MAX, -1),
    ] {
        assert_eq!(
            lmr_reduction(4, 8, history),
            (base + adjustment).clamp(0, 2) as u32
        );
    }
    assert_eq!(lmr_reduction(5, 255, i32::MIN), 3);
    assert_eq!(lmr_reduction(5, 255, i32::MAX), 3);
    // 表の値が0でも負のhistoryなら減深し、負の補正結果は0で切る。
    assert_eq!(lmr_reduction(4, 1, -threshold), 1);
    assert_eq!(lmr_reduction(4, 1, threshold), 0);
}

#[test]
fn lmr_reduction_preserves_remaining_depth() {
    let threshold = params::lmr_history_threshold();
    for depth in [0, 1, 2, 3, 4, MAX_PLY] {
        for index in [0, 1, 8, 255] {
            for history in [
                i32::MIN,
                -threshold,
                -threshold + 1,
                0,
                threshold - 1,
                threshold,
                i32::MAX,
            ] {
                let reduction = lmr_reduction(depth, index, history);
                assert!(reduction <= 3);
                match depth {
                    0..=2 => assert_eq!(reduction, 0),
                    3 => assert!(reduction <= 1),
                    _ => {}
                }
                if depth >= 2 {
                    assert!(depth - 1 - reduction >= 1);
                }
            }
        }
    }
}

#[test]
fn lmr_large_move_indices_use_last_column() {
    for depth in [0, 1, 2, 3, 4, MAX_PLY] {
        for history in [
            -params::lmr_history_threshold(),
            0,
            params::lmr_history_threshold(),
        ] {
            for index in [256, usize::MAX] {
                assert_eq!(
                    lmr_reduction(depth, index, history),
                    lmr_reduction(depth, 255, history)
                );
            }
        }
    }
}

// futilityは補正後の静的評価と余裕値の和がα以下なら後続の静かな手を飛ばす。
// 最善値が負の詰み帯にある間は少なくとも1手を読み、合法手なしの詰みと誤認しない。
#[test]
fn futility_reduces_quiet_nodes_without_false_mate() {
    let position = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    let nnue = test_nnue();
    let static_eval = evaluate(nnue, &position);
    let alpha = static_eval + 3 * nnue.pawn_value();
    assert!(!royal_under_attack(&position));
    assert!(
        legal_moves(&position)
            .iter()
            .all(|&mv| { !mv.promote && move_order_key(&position, nnue, mv).is_none() })
    );
    // ply=2では2手前と同じ評価なので、良化していないノードの保護も検査する。
    for (depth, ply) in (1..=3).flat_map(|depth| [0, 2].map(|ply| (depth, ply))) {
        let table = small_tt();
        let (score, pruned_nodes) = run_negamax(&position, depth, alpha, alpha + 1, ply, &table);
        let (_, full_nodes) = run_negamax(&position, depth, alpha, alpha + 2, ply, &small_tt());
        assert!(pruned_nodes > 0, "最初の手は探索する: depth={depth}");
        assert!(
            pruned_nodes < full_nodes,
            "depth={depth}: {pruned_nodes} >= {full_nodes}"
        );
        assert!(score.abs() < MATE_THRESHOLD, "depth={depth}: score={score}");
        let hit = table.probe(search_key(&position), ply).unwrap();
        assert_eq!(hit.score, score);
        assert!(hit.score.abs() < MATE_THRESHOLD);
        if depth == 1 {
            let leaf_scores: Vec<_> = legal_moves(&position)
                .into_iter()
                .map(|mv| {
                    let mut child = position.clone();
                    child.make_move_unchecked(mv, engine_rules());
                    -evaluate(nnue, &child)
                })
                .collect();
            assert!(
                (*leaf_scores.iter().min().unwrap()..=*leaf_scores.iter().max().unwrap())
                    .contains(&score)
            );
        }
    }
}

// 先頭の記録手が王駒を失っても、
// 後続の安全な静かな手を読み、返り値にも置換表にも誤った詰み値を残さない。
#[test]
fn futility_searches_safe_quiets_after_losing_tt_move() {
    let position = minase_core::parse_sfen("k11/12/12/12/12/10r1/12/12/12/12/12/11K b").unwrap();
    let nnue = test_nnue();
    let alpha = evaluate(nnue, &position) + 3 * nnue.pawn_value();
    let losing_move = Move {
        from: sq(11, 0),
        mid: None,
        to: sq(10, 0),
        promote: false,
    };
    assert!(legal_moves(&position).contains(&losing_move));
    assert!(!royal_under_attack(&position));
    assert!(
        legal_moves(&position)
            .iter()
            .all(|&mv| { !mv.promote && move_order_key(&position, nnue, mv).is_none() })
    );
    let ply = 5;
    let mut child = position.clone();
    child.make_move_unchecked(losing_move, engine_rules());
    let (opponent_score, _) = run_negamax(&child, 1, -INFINITY, INFINITY, ply + 1, &small_tt());
    assert!(opponent_score >= MATE_THRESHOLD, "先頭手は王駒を失う");

    let table = small_tt();
    // 子の全窓探索で確認した詰み値を固定し、親で後続手を読む条件だけを検査する。
    table.store(
        search_key(&child),
        1,
        opponent_score,
        Bound::Exact,
        None,
        ply + 1,
    );
    table.store(
        search_key(&position),
        0,
        0,
        Bound::Upper,
        Some(losing_move),
        ply,
    );
    let (score, _) = run_negamax(&position, 2, alpha, alpha + 1, ply, &table);
    assert!(score.abs() < MATE_THRESHOLD, "score={score}");
    let hit = table.probe(search_key(&position), ply).unwrap();
    assert!(hit.score.abs() < MATE_THRESHOLD);
    assert_ne!(hit.best_move, Some(losing_move));
}

// 静かな記録手を先に探索して条件を成立させても、
// 後続の捕獲手と非捕獲の成る手は展開し、記録手だけの場合より探索ノードが増える。
#[test]
fn futility_searches_captures_and_promotions_after_quiet_tt_move() {
    for (sfen, promotion) in [
        ("k11/12/12/12/12/12/12/5p6/5R6/12/12/11K b", false),
        ("k11/12/12/12/5P6/12/12/12/12/12/12/11K b", true),
    ] {
        let position = minase_core::parse_sfen(sfen).unwrap();
        let nnue = test_nnue();
        let alpha = evaluate(nnue, &position) + 3 * nnue.pawn_value();
        let tt_move = Move {
            from: sq(11, 0),
            mid: None,
            to: sq(11, 1),
            promote: false,
        };
        assert!(legal_moves(&position).contains(&tt_move));
        assert!(!royal_under_attack(&position));
        let protected: Vec<_> = legal_moves(&position)
            .into_iter()
            .filter(|&mv| {
                if promotion {
                    mv.promote && move_order_key(&position, nnue, mv).is_none()
                } else {
                    move_order_key(&position, nnue, mv).is_some()
                }
            })
            .collect();
        assert!(!protected.is_empty());
        let table = small_tt();
        table.store(search_key(&position), 0, 0, Bound::Upper, Some(tt_move), 0);
        let (score, nodes) = run_negamax(&position, 1, alpha, alpha + 1, 0, &table);
        assert!(score.abs() < MATE_THRESHOLD);
        assert!(nodes > 1, "記録手の後にも探索する");
    }
}

// SEEが負の捕獲しかなくても先頭手は読み、全手の枝刈りによる偽の詰みを防ぐ。
// この局面は王駒と周囲の歩兵が動けず、仲人が歩兵を取って取り返される2手だけが合法である。
// 判断の理由は docs/adr/prune-losing-captures-by-see-at-shallow-nodes.md にある。
#[test]
fn see_pruning_searches_first_capture_without_false_mate() {
    let board = position(
        Color::Black,
        &[
            (sq(0, 11), Color::Black, PieceKind::King),
            (sq(0, 10), Color::Black, PieceKind::Pawn),
            (sq(1, 10), Color::Black, PieceKind::Pawn),
            (sq(1, 11), Color::Black, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::GoBetween),
            (sq(5, 4), Color::White, PieceKind::Pawn),
            (sq(5, 6), Color::White, PieceKind::Pawn),
            (sq(5, 3), Color::White, PieceKind::GoBetween),
            (sq(5, 7), Color::White, PieceKind::Pawn),
        ],
    );
    let nnue = test_nnue();
    let moves = legal_moves(&board);
    assert_eq!(moves.len(), 2);
    assert!(!royal_under_attack(&board));
    assert!(moves.iter().all(|&mv| {
        move_order_key(&board, nnue, mv).is_some()
            && see_prunes(&board, engine_rules(), nnue, mv, 0)
    }));
    let alpha = evaluate(nnue, &board) + 3 * nnue.pawn_value();
    let table = small_tt();
    let ply = 5;
    let (score, nodes) = run_negamax(&board, 1, alpha, alpha + 1, ply, &table);
    assert!(score.abs() < MATE_THRESHOLD, "score={score}");
    let hit = table.probe(search_key(&board), ply).unwrap();
    // 保存された先頭捕獲の取り返しを別に数え、通常探索の展開が1手だけであることを検査する。
    let mut child = board.clone();
    child.make_move_unchecked(hit.best_move.unwrap(), engine_rules());
    let (_, reply_nodes) = run_quiesce(&child, -alpha - 1, -alpha, ply + 1, &small_tt());
    assert_eq!(nodes, 1 + reply_nodes, "後続の捕獲は枝刈りする");
    assert_eq!(hit.score, score);
    assert!(hit.score.abs() < MATE_THRESHOLD);
    assert_eq!(hit.bound, Bound::Upper);
}

// 奔王が飛車の筋を開ける記録手は負けるが、
// 飛車を取る手は交換で損をしても王将を守るので、負の詰み帯の後にも読む。
#[test]
fn see_pruning_searches_safe_capture_after_losing_tt_move() {
    let board = position(
        Color::Black,
        &[
            (sq(5, 0), Color::Black, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::FreeKing),
            (sq(4, 3), Color::White, PieceKind::Pawn),
            (sq(5, 10), Color::White, PieceKind::Rook),
            (sq(4, 10), Color::White, PieceKind::Rook),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    let nnue = test_nnue();
    let losing_move = Move {
        from: sq(5, 3),
        mid: None,
        to: sq(4, 3),
        promote: false,
    };
    let safe_capture = Move {
        to: sq(5, 10),
        ..losing_move
    };
    let moves = legal_moves(&board);
    assert!(moves.contains(&losing_move));
    assert!(moves.contains(&safe_capture));
    assert!(!royal_under_attack(&board));
    assert!(see_prunes(&board, engine_rules(), nnue, safe_capture, 200));
    let alpha = evaluate(nnue, &board) + 3 * nnue.pawn_value();
    let ply = 5;
    let mut child = board.clone();
    child.make_move_unchecked(losing_move, engine_rules());
    let (opponent_score, _) = run_negamax(&child, 1, -INFINITY, INFINITY, ply + 1, &small_tt());
    assert!(opponent_score >= MATE_THRESHOLD);
    let table = small_tt();
    // 子の全窓探索で確認した詰み値を固定し、親で後続手を読む条件だけを検査する。
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
    assert_eq!(hit.best_move, Some(safe_capture));
    assert_eq!(hit.bound, Bound::Upper);
}

// 深さ2以上では、展開した捕獲の子局面に
// 通常探索の値が保存される。先頭の静かな記録手で負の詰み帯を脱した後を検査する。
#[test]
fn see_pruning_skips_losing_capture_only_at_eligible_nodes() {
    let nnue = test_nnue();
    for (depth, width, capture_is_tt, attacked, victim, expanded) in [
        (2, 1, false, false, PieceKind::Pawn, false),
        (3, 1, false, false, PieceKind::Pawn, false),
        (2, 1, true, false, PieceKind::Pawn, true),
        (2, 2, false, false, PieceKind::Pawn, true),
        (4, 1, false, false, PieceKind::Pawn, true),
        (2, 1, false, true, PieceKind::Pawn, true),
        (2, 1, false, false, PieceKind::Bishop, true),
        (3, 1, false, false, PieceKind::Bishop, false),
    ] {
        let mut pieces = vec![
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::Rook),
            (sq(5, 4), Color::White, victim),
            (sq(5, 5), Color::White, PieceKind::Pawn),
        ];
        if attacked {
            pieces.push((sq(11, 6), Color::White, PieceKind::Rook));
        }
        let board = position(Color::Black, &pieces);
        let capture = Move {
            from: sq(5, 3),
            mid: None,
            to: sq(5, 4),
            promote: false,
        };
        let quiet = Move {
            from: sq(11, 0),
            mid: None,
            to: sq(10, 0),
            promote: false,
        };
        assert_eq!(royal_under_attack(&board), attacked);
        assert!(legal_moves(&board).contains(&capture));
        assert!(legal_moves(&board).contains(&quiet));
        assert!(see_prunes(&board, engine_rules(), nnue, capture, 0));
        assert_eq!(
            see_prunes(&board, engine_rules(), nnue, capture, 200),
            victim == PieceKind::Pawn,
        );
        let alpha = evaluate(nnue, &board) + 3 * nnue.pawn_value();
        let table = small_tt();
        let tt_move = if capture_is_tt { capture } else { quiet };
        table.store(search_key(&board), 0, 0, Bound::Upper, Some(tt_move), 0);
        let (score, _) = run_negamax(&board, depth, alpha, alpha + width, 0, &table);
        assert!(score.abs() < MATE_THRESHOLD);
        let mut child = board.clone();
        child.make_move_unchecked(capture, engine_rules());
        assert_eq!(
            table.probe(search_key(&child), 1).is_some(),
            expanded,
            "depth={depth}, width={width}, capture_is_tt={capture_is_tt}, attacked={attacked}, victim={victim:?}"
        );
        assert_eq!(table.probe(search_key(&board), 0).unwrap().score, score);
    }
}

// 経由升を持つ獅子の捕獲は規則依存なので展開する。
#[test]
fn see_pruning_searches_rule_dependent_capture() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::Lion),
            (sq(5, 4), Color::White, PieceKind::Pawn),
        ],
    );
    let capture = Move {
        from: sq(5, 3),
        mid: Some(sq(5, 4)),
        to: sq(5, 3),
        promote: false,
    };
    let quiet = Move {
        from: sq(11, 0),
        mid: None,
        to: sq(10, 0),
        promote: false,
    };
    let nnue = test_nnue();
    assert!(legal_moves(&board).contains(&capture));
    assert!(!see_prunes(&board, engine_rules(), nnue, capture, 200));
    assert!(!royal_under_attack(&board));
    let static_eval = evaluate(nnue, &board);
    let alpha = static_eval + 3 * nnue.pawn_value();
    let table = small_tt();
    // 他の捕獲による先行カットオフを避け、経由升を持つ捕獲まで探索を進める。
    // 対象と同じ盤面になる手は保存せず、実際に探索された結果を観測する。
    let mut target = board.clone();
    target.make_move_unchecked(capture, engine_rules());
    for mv in legal_moves(&board) {
        let mut child = board.clone();
        child.make_move_unchecked(mv, engine_rules());
        if search_key(&child) != search_key(&target) {
            table.store(search_key(&child), 1, -static_eval, Bound::Exact, None, 1);
        }
    }
    table.store(search_key(&board), 0, 0, Bound::Upper, Some(quiet), 0);
    let (score, _) = run_negamax(&board, 2, alpha, alpha + 1, 0, &table);
    assert!(score > static_eval);
    let hit = table.probe(search_key(&board), 0).unwrap();
    assert_eq!(hit.score, score);
    let best = hit.best_move.unwrap();
    assert!(best.mid.is_some(), "規則依存の捕獲を探索した値で更新する");
    let mut child = board.clone();
    child.make_move_unchecked(best, engine_rules());
    assert_eq!(search_key(&child), search_key(&target));
}

// 最後の王駒の捕獲は詰みとして返し、保存する。
#[test]
fn see_pruning_searches_last_royal_capture_after_quiet_tt_move() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(5, 4), Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::Rook),
        ],
    );
    let capture = Move {
        from: sq(5, 3),
        mid: None,
        to: sq(5, 4),
        promote: false,
    };
    let quiet = Move {
        from: sq(11, 0),
        mid: None,
        to: sq(10, 0),
        promote: false,
    };
    assert!(legal_moves(&board).contains(&capture));
    assert!(!royal_under_attack(&board));
    assert!(captures_last_royal(&board, capture));
    let ply = 5;
    let table = small_tt();
    table.store(search_key(&board), 0, 0, Bound::Upper, Some(quiet), ply);
    let alpha = evaluate(test_nnue(), &board) + 3 * test_nnue().pawn_value();
    let (score, _) = run_negamax(&board, 1, alpha, alpha + 1, ply, &table);
    assert_eq!(score, MATE - (ply + 1) as i32);
    let hit = table.probe(search_key(&board), ply).unwrap();
    assert_eq!(hit.score, score);
    assert_eq!(hit.best_move, Some(capture));
}

// 深さ1〜3の零窓で、王駒に利きがなく、静的評価とβの差が余裕値に等しければ静的評価を返し、置換表には保存しない。
// 王駒への利きを除外する理由は docs/adr/skip-static-eval-pruning-when-a-royal-is-attacked.md にある。
#[test]
fn reverse_futility_returns_static_eval_at_margin_without_storing() {
    let position = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/5P6/11K b").unwrap();
    let nnue = test_nnue();
    let static_eval = evaluate(nnue, &position);
    let beta = static_eval - nnue.pawn_value() * params::reverse_futility_margin() / 100;
    for depth in 1..=3 {
        let table = small_tt();
        let tt_move = legal_moves(&position)[0];
        table.store(search_key(&position), 0, 0, Bound::Upper, Some(tt_move), 0);
        let (score, nodes) = run_negamax(&position, depth, beta - 1, beta, 0, &table);
        assert_eq!(score, static_eval, "depth={depth}");
        assert_eq!(nodes, 0, "子を探索せずに返す: depth={depth}");
        let hit = table.probe(search_key(&position), 0).unwrap();
        assert_eq!(hit.depth, 0);
        assert_eq!(hit.score, 0);
        assert_eq!(hit.best_move, Some(tt_move));
    }
}

// 各除外条件と余裕値の境界では子を探索する。
#[test]
fn reverse_futility_exclusions_search_children() {
    let position = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    let nnue = test_nnue();
    let static_eval = evaluate(nnue, &position);
    let beta = static_eval - nnue.pawn_value() * params::reverse_futility_margin() / 100;
    let cases = [
        (1, beta - 2, beta, "幅2のPV窓"),
        (1, -MATE_THRESHOLD - 1, -MATE_THRESHOLD, "βが負の詰み帯"),
        (1, -MATE_THRESHOLD, -MATE_THRESHOLD + 1, "αだけが詰み帯"),
        (1, MATE_THRESHOLD - 1, MATE_THRESHOLD, "βが正の詰み帯"),
        (1, beta, beta + 1, "余裕値に1不足"),
        (4, beta - 1, beta, "深さ4"),
    ];
    for (depth, alpha, beta, reason) in cases {
        let table = small_tt();
        table.store(
            search_key(&position),
            0,
            0,
            Bound::Upper,
            Some(legal_moves(&position)[0]),
            0,
        );
        let (_, nodes) = run_negamax(&position, depth, alpha, beta, 0, &table);
        assert!(nodes > 0, "{reason}");
    }
}

// 置換表の打ち切りをreverse futilityより先に適用する。
#[test]
fn reverse_futility_preserves_tt_cutoff_priority() {
    let position = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/5P6/11K b").unwrap();
    let nnue = test_nnue();
    let static_eval = evaluate(nnue, &position);
    let beta = static_eval - nnue.pawn_value() * params::reverse_futility_margin() / 100;
    let table = small_tt();
    table.store(search_key(&position), 1, beta - 1, Bound::Exact, None, 0);
    let (score, nodes) = run_negamax(&position, 1, beta - 1, beta, 0, &table);
    assert_eq!(score, beta - 1);
    assert_eq!(nodes, 0);
}

// 相手飛車を1筋へ移し、王駒だけへの利きを加えると枝刈りを控える。
#[test]
fn reverse_futility_does_not_prune_when_royal_is_attacked() {
    for (rook_rank, attacked) in [("10r1", false), ("11r", true)] {
        let position = minase_core::parse_sfen(&format!(
            "k11/12/12/12/12/{rook_rank}/12/12/12/12/5P6/11K b"
        ))
        .unwrap();
        let nnue = test_nnue();
        let static_eval = evaluate(nnue, &position);
        let beta = static_eval - nnue.pawn_value() * params::reverse_futility_margin() / 100;
        assert_eq!(royal_under_attack(&position), attacked);
        let (score, nodes) = run_negamax(&position, 1, beta - 1, beta, 0, &small_tt());
        if attacked {
            assert!(nodes > 0);
        } else {
            assert_eq!(score, static_eval);
            assert_eq!(nodes, 0);
        }
    }
}

// razoringは深さ1〜2で「静的評価 + 余裕値 ≤ α」なら静止探索を試し、その値もα以下なら通常探索を省く。
// 捕獲候補のない局面では静的評価を返し、置換表へ保存しない。
#[test]
fn razoring_returns_quiescence_at_the_margin_boundary() {
    let position = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    let nnue = test_nnue();
    let static_eval = evaluate(nnue, &position);
    assert!(!royal_under_attack(&position));
    for (depth, percent) in [
        (1, params::razoring_margin1()),
        (2, params::razoring_margin2()),
    ] {
        for excess in [0, nnue.pawn_value()] {
            let alpha = static_eval + nnue.pawn_value() * percent / 100 + excess;
            let (quiet_score, quiet_nodes) =
                run_quiesce(&position, alpha, alpha + 1, 0, &small_tt());
            assert_eq!(quiet_score, static_eval);
            let table = small_tt();
            let (score, nodes) = run_negamax(&position, depth, alpha, alpha + 1, 0, &table);
            let (_, full_nodes) = run_negamax(&position, depth, alpha, alpha + 2, 0, &small_tt());
            assert_eq!(score, quiet_score);
            assert_eq!(nodes, quiet_nodes, "通常探索の手を展開しない");
            assert!(nodes < full_nodes);
            assert!(
                table.probe(search_key(&position), 0).is_none(),
                "捕獲のない静止探索は置換表に保存しない"
            );
        }
        let alpha = static_eval + nnue.pawn_value() * percent / 100 - 1;
        let (_, quiet_nodes) = run_quiesce(&position, alpha, alpha + 1, 0, &small_tt());
        let (_, nodes) = run_negamax(&position, depth, alpha, alpha + 1, 0, &small_tt());
        assert!(nodes > quiet_nodes, "余裕値に1足りなければ通常探索する");
    }
}

// 静止探索の上界がαと等しい場合も打ち切る。
#[test]
fn razoring_accepts_quiescence_equal_to_alpha() {
    let position = minase_core::parse_sfen("k11/12/12/12/12/12/12/5q6/5R6/12/12/11K b").unwrap();
    let nnue = test_nnue();
    for (depth, percent) in [
        (1, params::razoring_margin1()),
        (2, params::razoring_margin2()),
    ] {
        let alpha = evaluate(nnue, &position) + nnue.pawn_value() * percent / 100;
        let table = small_tt();
        table.store(search_key(&position), 0, alpha, Bound::Upper, None, 0);
        let (quiet_score, quiet_nodes) = run_quiesce(&position, alpha, alpha + 1, 0, &table);
        assert_eq!(quiet_score, alpha);
        let (score, nodes) = run_negamax(&position, depth, alpha, alpha + 1, 0, &table);
        assert_eq!(score, alpha);
        assert_eq!(nodes, quiet_nodes);
    }
}

// PV窓、詰み帯、深さ3では通常探索を行う。
#[test]
fn razoring_excludes_pv_mate_windows_and_depth_three() {
    let position = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    let nnue = test_nnue();
    let alpha = evaluate(nnue, &position)
        + nnue.pawn_value() * params::razoring_margin1().max(params::razoring_margin2()) / 100;
    for (depth, alpha, beta) in [
        (1, alpha, alpha + 2),
        (2, alpha, alpha + 2),
        (1, MATE_THRESHOLD - 1, MATE_THRESHOLD),
        (2, MATE_THRESHOLD - 1, MATE_THRESHOLD),
        (1, MATE_THRESHOLD, MATE_THRESHOLD + 1),
        (2, MATE_THRESHOLD, MATE_THRESHOLD + 1),
        (1, -MATE_THRESHOLD, -MATE_THRESHOLD + 1),
        (2, -MATE_THRESHOLD, -MATE_THRESHOLD + 1),
        (3, alpha, alpha + 1),
    ] {
        let (_, quiet_nodes) = run_quiesce(&position, alpha, beta, 0, &small_tt());
        let table = small_tt();
        table.store(
            search_key(&position),
            0,
            0,
            Bound::Upper,
            Some(legal_moves(&position)[0]),
            0,
        );
        let (_, nodes) = run_negamax(&position, depth, alpha, beta, 0, &table);
        assert!(
            nodes > quiet_nodes,
            "depth={depth}, alpha={alpha}, beta={beta}: {nodes} <= {}",
            quiet_nodes
        );
    }
}

// 王駒への疑似利きがあれば、静的評価の条件が成立しても探索する。
#[test]
fn razoring_excludes_attacked_royals() {
    let nnue = test_nnue();
    for rook_file in [10, 11] {
        let position = position(
            Color::Black,
            &[
                (sq(0, 11), Color::White, PieceKind::King),
                (sq(11, 0), Color::Black, PieceKind::King),
                (sq(rook_file, 6), Color::White, PieceKind::Rook),
            ],
        );
        assert_eq!(royal_under_attack(&position), rook_file == 11);
        for (depth, percent) in [
            (1, params::razoring_margin1()),
            (2, params::razoring_margin2()),
        ] {
            let alpha = evaluate(nnue, &position) + nnue.pawn_value() * percent / 100;
            let (quiet_score, quiet_nodes) =
                run_quiesce(&position, alpha, alpha + 1, 0, &small_tt());
            assert!(quiet_score <= alpha);
            let (_, nodes) = run_negamax(&position, depth, alpha, alpha + 1, 0, &small_tt());
            if rook_file == 11 {
                assert!(nodes > quiet_nodes, "王駒への利きがあれば通常探索する");
            } else {
                assert_eq!(nodes, quiet_nodes);
            }
        }
    }
}

// 捕獲でαを上回れば通常探索へ進み、PV窓の探索と同じ値を返す。
#[test]
fn razoring_continues_normal_search_after_recovering_capture() {
    let position = minase_core::parse_sfen("k11/12/12/12/12/12/12/5q6/5R6/12/12/11K b").unwrap();
    let nnue = test_nnue();
    let alpha = evaluate(nnue, &position) + nnue.pawn_value() * params::razoring_margin1() / 100;
    assert!(!royal_under_attack(&position));
    let (quiet_score, quiet_nodes) = run_quiesce(&position, alpha, alpha + 1, 0, &small_tt());
    assert!(quiet_score > alpha, "無防備な奔王の捕獲で余裕値を取り戻す");
    // 深さ1なら子は静止探索となり、子ノードでの枝刈りの差を比較へ持ち込まない。
    let table = small_tt();
    let (score, nodes) = run_negamax(&position, 1, alpha, alpha + 1, 0, &table);
    let (full_score, _) = run_negamax(&position, 1, alpha, alpha + 2, 0, &small_tt());
    assert_eq!(score, full_score);
    assert!(nodes > quiet_nodes, "静止探索の後にも通常探索を行う");
    let hit = table.probe(search_key(&position), 0).unwrap();
    assert_eq!(hit.depth, 1);
    assert!(hit.best_move.is_some());
}

// null moveの追加減深は `(静的評価 − β) × NullMoveEvalScale / 100 / (2 × 歩兵価値)` を0〜3に制限する。
// 各整数境界の直前と一致で段階が変わり、深さによる基本量へ加算する。
#[test]
fn null_move_reduction_scales_with_eval_surplus_and_depth() {
    let beta = 137;
    for pawn_value in [37, 100] {
        let scale = params::null_move_eval_scale();
        assert!(scale > 0);
        let boundary = |extra| (extra * 2 * pawn_value * 100 + scale - 1) / scale;
        for depth in [3, 5, 6, 11, 12] {
            let base =
                (params::null_move_base() as u32 + depth * params::null_move_slope() as u32) / 1200;
            for (difference, extra) in [
                (-6 * pawn_value, 0),
                (-1, 0),
                (0, 0),
                (boundary(1) - 1, 0),
                (boundary(1), 1),
                (boundary(2) - 1, 1),
                (boundary(2), 2),
                (boundary(3) - 1, 2),
                (boundary(3), 3),
                (boundary(4), 3),
            ] {
                assert_eq!(
                    null_move_reduction(depth, beta + difference, beta, pawn_value),
                    base + extra,
                    "depth={depth}, difference={difference}, pawn_value={pawn_value}"
                );
            }
        }
    }
}

// 記録手なしの要求深さ7は6へ減深され、静的評価の余裕による追加減深でnull move後の探索が静止探索まで縮む。
// 通常探索の実着手を省くためノード数も減る。
#[test]
fn null_move_reduction_reduces_nodes_with_large_eval_surplus() {
    let position = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/5G6/11K b").unwrap();
    let nnue = test_nnue();
    let scale = params::null_move_eval_scale();
    assert!(scale > 0);
    let beta = evaluate(nnue, &position) - (2 * nnue.pawn_value() * 100 + scale - 1) / scale;
    let mut passed = position.clone();
    passed.make_null_move();
    let (reply_score, reply_nodes) = run_negamax(&passed, 1, -beta, -beta + 1, 1, &small_tt());
    assert!(-reply_score >= beta);
    let table = small_tt();
    let (score, nodes) = run_negamax(&position, 7, beta - 1, beta, 0, &table);
    assert!(score >= beta && score.abs() < MATE_THRESHOLD);
    assert!(table.probe(search_key(&position), 0).is_none());
    assert!(
        nodes < reply_nodes,
        "nodes={nodes}, base_nodes={reply_nodes}"
    );
}

// reverse futilityとrazoringはcorrection historyを加えず、補正前の静的評価を使う。
#[test]
fn revived_pruning_uses_uncorrected_static_evaluation() {
    let position = minase_core::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    for reverse in [true, false] {
        with_root_searcher(&position, &[], |searcher| {
            let side = position.side_to_move();
            let key = material_key(&position);
            let difference = if reverse { -1000 } else { 1000 };
            searcher.correction.update(side, key, difference, 8);
            assert_ne!(searcher.correction.read(side, key), 0);
            let raw = evaluate(searcher.nnue, &position);
            let alpha = if reverse {
                raw - searcher.nnue.pawn_value() * params::reverse_futility_margin() / 100 - 1
            } else {
                raw + searcher.nnue.pawn_value() * params::razoring_margin1() / 100
            };
            let mut board = position.clone();
            assert_eq!(
                searcher.negamax(&mut board, 1, alpha, alpha + 1, 0),
                Some(raw)
            );
            assert_eq!(searcher.nodes, 0);
            assert!(searcher.tt.probe(search_key(&position), 0).is_none());
        });
    }
}
