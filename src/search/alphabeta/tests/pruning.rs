//! 枝刈りと深さの削減を検査する。

use super::*;

// docs/plans/strength-stage5.md「LMRの減深量」「検証」とフェーズ6指示書。
// c = 1.66、H = 111の生成規則と、補正後の切り詰めを個別に固定する。
#[test]
fn lmr_table_follows_logarithmic_rule() {
    let table = lmr_table();
    assert!(table[0].iter().all(|&value| value == 0));
    for row in table {
        assert_eq!(row[0], 0);
        assert_eq!(row[1], 0);
    }
    assert_eq!(table[3][3], 0);
    assert_eq!(table[4][8], 1);
    assert_eq!(table[5][255], 5);
}

#[test]
fn lmr_history_adjustment_precedes_clamping() {
    for (history, expected) in [
        (i32::MIN, 2),
        (-112, 2),
        (-111, 2),
        (-110, 1),
        (0, 1),
        (110, 1),
        (111, 0),
        (112, 0),
        (i32::MAX, 0),
    ] {
        assert_eq!(lmr_reduction(4, 8, history), expected);
    }
    assert_eq!(lmr_reduction(5, 255, i32::MIN), 3);
    assert_eq!(lmr_reduction(5, 255, i32::MAX), 3);
    // 表の値が0でも負のhistoryなら減深し、負の補正結果は0で切る。
    assert_eq!(lmr_reduction(4, 1, -111), 1);
    assert_eq!(lmr_reduction(4, 1, 111), 0);
}

#[test]
fn lmr_reduction_preserves_remaining_depth() {
    for depth in [0, 1, 2, 3, 4, MAX_PLY] {
        for index in [0, 1, 8, 255] {
            for history in [i32::MIN, -111, -110, 0, 110, 111, i32::MAX] {
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
        for history in [-128, 0, 128] {
            for index in [256, usize::MAX] {
                assert_eq!(
                    lmr_reduction(depth, index, history),
                    lmr_reduction(depth, 255, history)
                );
            }
        }
    }
}

// docs/plans/strength-stage4.md「futility pruning」「検証」。
// 静かな合法手だけの局面では零窓の探索量が減り、少なくとも1手を探索して詰みを捏造しない。
#[test]
fn futility_reduces_quiet_nodes_without_false_mate() {
    let position = crate::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    let pst = weights().unwrap();
    let static_eval = evaluate(&pst, &position);
    let alpha = static_eval + 10 * pst.pawn_value();
    assert!(!royal_under_attack(&position));
    assert!(
        legal_moves(&position)
            .iter()
            .all(|&mv| { !mv.promote && move_order_key(&position, &pst, mv).is_none() })
    );
    for depth in 1..=3 {
        let (score, pruned_nodes) = run_negamax(&position, depth, alpha, alpha + 1, 0, &small_tt());
        let (_, full_nodes) = run_negamax(&position, depth, alpha, alpha + 2, 0, &small_tt());
        assert!(pruned_nodes > 0, "最初の手は探索する: depth={depth}");
        assert!(
            pruned_nodes < full_nodes,
            "depth={depth}: {pruned_nodes} >= {full_nodes}"
        );
        assert!(score.abs() < MATE_THRESHOLD, "depth={depth}: score={score}");
        if depth == 1 {
            let leaf_scores: Vec<_> = legal_moves(&position)
                .into_iter()
                .map(|mv| {
                    let mut child = position.clone();
                    child.make_move_unchecked(mv, engine_rules());
                    -evaluate(&pst, &child)
                })
                .collect();
            assert!(
                (*leaf_scores.iter().min().unwrap()..=*leaf_scores.iter().max().unwrap())
                    .contains(&score)
            );
        }
    }
}

// 同「展開しない手の範囲」「検証」。先頭の記録手が王駒を失っても、
// 後続の安全な静かな手を読み、返り値にも置換表にも誤った詰み値を残さない。
#[test]
fn futility_searches_safe_quiets_after_losing_tt_move() {
    let position = crate::parse_sfen("k11/12/12/12/12/10r1/12/12/12/12/12/11K b").unwrap();
    let pst = weights().unwrap();
    let alpha = evaluate(&pst, &position) + 10 * pst.pawn_value();
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
            .all(|&mv| { !mv.promote && move_order_key(&position, &pst, mv).is_none() })
    );
    let ply = 5;
    let mut child = position.clone();
    child.make_move_unchecked(losing_move, engine_rules());
    let (opponent_score, _) = run_negamax(&child, 1, -alpha - 1, -alpha, ply + 1, &small_tt());
    assert!(opponent_score >= MATE_THRESHOLD, "先頭手は王駒を失う");

    let table = small_tt();
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

// 同「展開しない手の範囲」。静かな記録手を先に探索して条件を成立させても、
// 後続の捕獲手と非捕獲の成る手は展開し、記録手だけの場合より探索ノードが増える。
#[test]
fn futility_searches_captures_and_promotions_after_quiet_tt_move() {
    for (sfen, promotion) in [
        ("k11/12/12/12/12/12/12/5p6/5R6/12/12/11K b", false),
        ("k11/12/12/12/5P6/12/12/12/12/12/12/11K b", true),
    ] {
        let position = crate::parse_sfen(sfen).unwrap();
        let pst = weights().unwrap();
        let alpha = evaluate(&pst, &position) + 10 * pst.pawn_value();
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
                    mv.promote && move_order_key(&position, &pst, mv).is_none()
                } else {
                    move_order_key(&position, &pst, mv).is_some()
                }
            })
            .collect();
        assert!(!protected.is_empty());
        let table = small_tt();
        table.store(search_key(&position), 0, 0, Bound::Upper, Some(tt_move), 0);
        let (score, nodes) = run_negamax(&position, 1, alpha, alpha + 1, 0, &table);
        assert!(
            score < alpha,
            "全対象手を調べる窓: score={score}, alpha={alpha}"
        );
        assert!(nodes > 1, "記録手の後にも探索する");
    }
}

// docs/plans/strength-stage8.md「検証」。王駒とそれを囲む歩兵は動けず、
// 仲人の合法手は歩兵を取って取り返される2手だけである。
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
    let pst = weights().unwrap();
    let moves = legal_moves(&board);
    assert_eq!(moves.len(), 2);
    assert!(!royal_under_attack(&board));
    assert!(moves.iter().all(|&mv| {
        move_order_key(&board, &pst, mv).is_some()
            && see_prunes(&board, engine_rules(), &pst, mv, 0)
    }));
    let alpha = evaluate(&pst, &board) + 10 * pst.pawn_value();
    let table = small_tt();
    let ply = 5;
    let (score, nodes) = run_negamax(&board, 1, alpha, alpha + 1, ply, &table);
    assert!(score.abs() < MATE_THRESHOLD, "score={score}");
    assert_eq!(nodes, 1, "先頭の捕獲だけを探索し、後続を枝刈りする");
    let hit = table.probe(search_key(&board), ply).unwrap();
    assert_eq!(hit.score, score);
    assert!(hit.score.abs() < MATE_THRESHOLD);
    assert_eq!(hit.bound, Bound::Upper);
}

// 同「適用するノード」「検証」。奔王が飛車の筋を開ける記録手は負けるが、
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
    let pst = weights().unwrap();
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
    assert!(see_prunes(&board, engine_rules(), &pst, safe_capture, 200));
    let alpha = evaluate(&pst, &board) + 10 * pst.pawn_value();
    let ply = 5;
    let mut child = board.clone();
    child.make_move_unchecked(losing_move, engine_rules());
    let (opponent_score, _) = run_negamax(&child, 1, -alpha - 1, -alpha, ply + 1, &small_tt());
    assert!(opponent_score >= MATE_THRESHOLD);
    let table = small_tt();
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

// 同「SEEによる捕獲手の枝刈り」。深さ2以上では、展開した捕獲の子局面に
// 通常探索の値が保存される。先頭の静かな記録手で負の詰み帯を脱した後を検査する。
#[test]
fn see_pruning_skips_losing_capture_only_at_eligible_nodes() {
    let pst = weights().unwrap();
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
        assert!(see_prunes(&board, engine_rules(), &pst, capture, 0));
        assert_eq!(
            see_prunes(&board, engine_rules(), &pst, capture, 200),
            victim == PieceKind::Pawn,
        );
        let alpha = evaluate(&pst, &board) + 10 * pst.pawn_value();
        let table = small_tt();
        let tt_move = if capture_is_tt { capture } else { quiet };
        table.store(search_key(&board), 0, 0, Bound::Upper, Some(tt_move), 0);
        let (score, _) = run_negamax(&board, depth, alpha, alpha + width, 0, &table);
        assert!(score < alpha, "全対象手を調べる窓");
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

// 同「SEEの余裕値」。経由升を持つ獅子の捕獲は規則依存なので展開する。
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
    let pst = weights().unwrap();
    assert!(legal_moves(&board).contains(&capture));
    assert!(!see_prunes(&board, engine_rules(), &pst, capture, 200));
    assert!(!royal_under_attack(&board));
    // 捕獲後の評価の上がり幅は重みに依存する（先読み教師の重みでは約10歩分）ので、
    // 窓は捕獲の利得より十分に高い位置に置き、ノードが必ず下限で失敗するようにする。
    let alpha = evaluate(&pst, &board) + 30 * pst.pawn_value();
    let table = small_tt();
    table.store(search_key(&board), 0, 0, Bound::Upper, Some(quiet), 0);
    let (score, _) = run_negamax(&board, 2, alpha, alpha + 1, 0, &table);
    assert!(score < alpha);
    let mut child = board.clone();
    child.make_move_unchecked(capture, engine_rules());
    assert!(table.probe(search_key(&child), 1).is_some());
    assert_eq!(table.probe(search_key(&board), 0).unwrap().score, score);
}

// 同「SEEによる捕獲手の枝刈り」。最後の王駒の捕獲は詰みとして返し、保存する。
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
    let (score, _) = run_negamax(
        &board,
        1,
        MATE_THRESHOLD - 2,
        MATE_THRESHOLD - 1,
        ply,
        &table,
    );
    assert_eq!(score, MATE - (ply + 1) as i32);
    let hit = table.probe(search_key(&board), ply).unwrap();
    assert_eq!(hit.score, score);
    assert_eq!(hit.best_move, Some(capture));
}
