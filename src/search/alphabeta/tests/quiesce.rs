//! 静止探索を検査する。

use super::*;

// D7-SRCH-08。search.md「静止探索」: 深さ0ではstand-patと捕獲手を探索する。
// 守られた歩兵を飛車で取る損な交換は静止探索で見抜かれ、最善手にならない。
#[test]
fn quiescence_avoids_a_losing_rook_for_pawn_capture() {
    // 先手: 飛車3十、王将6十二。後手: 歩兵3四、飛車3一、玉将6一。
    // 3筋の間は空で、後手飛車は歩兵越しに先手飛車から攻撃されない。
    let position = position(
        Color::Black,
        &[
            (fs(3, 10), Color::Black, PieceKind::Rook),
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(3, 4), Color::White, PieceKind::Pawn),
            (fs(3, 1), Color::White, PieceKind::Rook),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let moves = legal_moves(&position);

    let result = run_search(
        &position,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    assert!(
        result.best_move.from != fs(3, 10) || result.best_move.to != fs(3, 4),
        "守られた歩兵を飛車で取る手は最善手にならない"
    );
    assert!(result.score.abs() < 29_000);
}

// D7-SRCH-09。search.md「静止探索」: 捕獲手のない葉ではstand-patを返すため、
// depth=1の根評価は各合法手の子局面に対する静的評価の最大値と一致する。
#[test]
fn quiescence_without_captures_matches_static_evaluation() {
    let mut position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::GoldGeneral),
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(6, 3), Color::White, PieceKind::GoldGeneral),
        ],
    );
    let moves = legal_moves(&position);
    let expected = moves
        .iter()
        .map(|&mv| {
            let undo = position.make_move_unchecked(mv, engine_rules());
            let score = -crate::eval::evaluate(&crate::eval::weights().unwrap(), &position);
            position.unmake_move(undo);
            score
        })
        .max()
        .expect("root must have a legal move");

    let result = run_search(
        &position,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    assert_eq!(result.score, expected);
    // 各合法手を1回適用し、子の静止探索では捕獲がないので追加の適用はない。
    assert_eq!(result.nodes, moves.len() as u64);
}

// D7-SRCH-10。search.md「静止探索」: stand-patは損な捕獲より優先される
// fail-softの下限であり、捕獲手が存在しても選択を強制されない。
#[test]
fn quiescence_stand_pat_declines_a_losing_capture() {
    let mut position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(9, 9), Color::Black, PieceKind::Pawn),
            (fs(9, 12), Color::Black, PieceKind::Rook),
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(9, 3), Color::White, PieceKind::Rook),
        ],
    );
    let moves = legal_moves(&position);
    let quiet_move = Move {
        from: fs(6, 12),
        mid: None,
        to: fs(6, 11),
        promote: false,
    };
    assert!(moves.contains(&quiet_move));

    position.make_move_unchecked(quiet_move, engine_rules());
    let losing_capture = Move {
        from: fs(9, 3),
        mid: None,
        to: fs(9, 9),
        promote: false,
    };
    assert!(legal_moves(&position).contains(&losing_capture));
    let stand_pat = crate::eval::evaluate(&crate::eval::weights().unwrap(), &position);
    let (score, _) = run_quiesce(&position, -INFINITY, INFINITY, 0, &small_tt());
    assert_eq!(score, stand_pat);
}

// 設計書movegen-speedup-2.md「段階7」: β以上の静的評価は置換表を照合も保存もしない。
#[test]
fn quiescence_stand_pat_cutoff_does_not_probe_or_store() {
    let position = Position::initial();
    let stand_pat = evaluate(&weights().unwrap(), &position);
    let key = search_key(&position);
    for beta in [stand_pat, stand_pat - 1] {
        let table = small_tt();
        let (score, nodes) = run_quiesce(&position, -INFINITY, beta, 0, &table);
        assert_eq!((score, nodes), (stand_pat, 0));
        assert!(table.probe(key, 0).is_none());
        table.store(key, 4, stand_pat + 123, Bound::Exact, None, 0);
        let (score, nodes) = run_quiesce(&position, -INFINITY, beta, 0, &table);
        assert_eq!((score, nodes), (stand_pat, 0));
        let hit = table.probe(key, 0).unwrap();
        assert_eq!(
            (hit.score, hit.depth, hit.bound),
            (stand_pat + 123, 4, Bound::Exact)
        );
    }
}

// D7-SRCH-11。search.md「静止探索」節の2026年8月22日改訂: Exact、
// score >= betaのLower、score <= alphaのUpperは深さ条件なしで返す。
#[test]
fn quiescence_tt_cuts_off_all_three_bounds_at_inclusive_edges() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(3, 10), Color::Black, PieceKind::Rook),
            (fs(3, 4), Color::White, PieceKind::Pawn),
        ],
    );
    let key = search_key(&position);
    let stand_pat = evaluate(&weights().unwrap(), &position);
    let cases = [
        (Bound::Exact, stand_pat + 17, stand_pat - 10, stand_pat + 10),
        (
            Bound::Lower,
            stand_pat + 50,
            stand_pat - 100,
            stand_pat + 50,
        ),
        (
            Bound::Upper,
            stand_pat - 50,
            stand_pat - 50,
            stand_pat + 100,
        ),
    ];

    for (bound, score, alpha, beta) in cases {
        let table = small_tt();
        table.store(key, 0, score, bound, None, 0);
        let (actual, _) = run_quiesce(&position, alpha, beta, 0, &table);
        assert_eq!(actual, score);
    }
}

// D7-SRCH-12。静止探索の通常出口は深さ0で記録され、同じ入口局面を
// 再訪するとExactヒットで捕獲展開を省く。
#[test]
fn quiescence_stores_depth_zero_and_reuses_it_on_revisit() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(3, 10), Color::Black, PieceKind::Rook),
            (fs(3, 4), Color::White, PieceKind::Pawn),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let capture = Move {
        from: fs(3, 10),
        mid: None,
        to: fs(3, 4),
        promote: false,
    };
    assert!(legal_moves(&position).contains(&capture));

    let table = small_tt();
    let (first_score, first_nodes) = run_quiesce(&position, -INFINITY, INFINITY, 0, &table);
    let hit = table.probe(search_key(&position), 0).unwrap();
    assert_eq!(hit.depth, 0);
    assert_eq!(hit.bound, Bound::Exact);
    let stored_move = hit.best_move.expect("捕獲手が最善なら記録手を持つ");
    assert_eq!(stored_move.from, capture.from);
    assert_eq!(stored_move.to, capture.to);
    assert_eq!(hit.score, first_score);

    let (second_score, second_nodes) = run_quiesce(&position, -INFINITY, INFINITY, 0, &table);
    assert_eq!(second_score, first_score);
    assert!(first_nodes > 1);
    assert!(second_nodes < first_nodes);
}

// D7-SRCH-12。通常出口のUpper・Exactを深さ0で記録し、
// stand-patが最善なら手なしとする。
#[test]
fn quiescence_records_bounds_and_no_move_by_exit_path() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(3, 10), Color::Black, PieceKind::Rook),
            (fs(3, 4), Color::White, PieceKind::Pawn),
            (fs(3, 1), Color::White, PieceKind::Rook),
        ],
    );
    let key = search_key(&position);
    let stand_pat = evaluate(&crate::eval::weights().unwrap(), &position);
    let cases = [
        (stand_pat, INFINITY, Bound::Upper),
        (stand_pat - 1, stand_pat + 1, Bound::Exact),
    ];

    for (alpha, beta, expected_bound) in cases {
        let table = small_tt();
        let (score, _) = run_quiesce(&position, alpha, beta, 0, &table);
        let hit = table.probe(key, 0).unwrap();
        assert_eq!(score, stand_pat);
        assert_eq!(hit.depth, 0);
        assert_eq!(hit.bound, expected_bound);
        assert_eq!(hit.best_move, None);
    }
}

// D7-SRCH-13。静止探索で最後の王駒を取ったMATE−(ply+1)は、現在plyを渡した
// 置換表の変換で往復する。捕獲を評価する最大plyでも詰み帯に収まる。
#[test]
fn quiescence_mate_score_round_trips_through_the_tt() {
    let position = position(
        Color::Black,
        &[
            (fs(1, 12), Color::Black, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::Rook),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let mate_move = Move {
        from: fs(6, 10),
        mid: None,
        to: fs(6, 1),
        promote: false,
    };
    assert!(legal_moves(&position).contains(&mate_move));

    for ply in [0, 5, MAX_PLY - 1] {
        let table = small_tt();
        let (score, nodes) = run_quiesce(&position, -INFINITY, INFINITY, ply, &table);
        assert_eq!(nodes, 0);
        let hit = table.probe(search_key(&position), ply).unwrap();
        assert_eq!(score, MATE - (ply + 1) as i32);
        assert!(score >= MATE_THRESHOLD);
        assert_eq!(hit.score, score);
        assert_eq!(hit.depth, 0);
        assert_eq!(hit.best_move, Some(mate_move));
        // 同じ局面を根から評価すると、捕獲までの距離は1手になる。
        assert_eq!(
            table.probe(search_key(&position), 0).unwrap().score,
            MATE - 1
        );
    }
}

// D7-SRCH-13。最大ply葉は置換表照会より先に静的評価を返し、既存の
// エントリを参照も更新もしない。
#[test]
fn quiescence_max_ply_leaf_does_not_probe_or_store() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let key = search_key(&position);
    let table = small_tt();
    table.store(key, 8, 12_345, Bound::Exact, None, MAX_PLY);
    let raw_before = table.raw_entry(key);

    let (score, _) = run_quiesce(&position, -INFINITY, INFINITY, MAX_PLY, &table);
    assert_eq!(score, evaluate(&crate::eval::weights().unwrap(), &position));
    assert_ne!(score, 12_345);
    assert_eq!(table.raw_entry(key), raw_before);
}

// 設計書movegen-speedup-2.md「段階9」: 合法な捕獲がない場合と、入口の閾値で
// 全捕獲が消える場合は、既存の置換表の値にかかわらず静的評価を返す。
#[test]
fn quiescence_empty_candidates_do_not_probe_or_store() {
    let pst = weights().unwrap();
    let capture_position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(3, 10), Color::Black, PieceKind::Rook),
            (fs(3, 4), Color::White, PieceKind::Pawn),
        ],
    );
    let stand_pat = evaluate(&pst, &capture_position);
    for (position, alpha) in [
        (Position::initial(), -INFINITY),
        (capture_position, stand_pat + 4 * pst.pawn_value()),
    ] {
        let key = search_key(&position);
        let stand_pat = evaluate(&pst, &position);
        let table = small_tt();
        let (score, nodes) = run_quiesce(&position, alpha, INFINITY, 0, &table);
        assert_eq!((score, nodes), (stand_pat, 0));
        assert!(table.probe(key, 0).is_none());
        table.store(key, 4, stand_pat + 123, Bound::Exact, None, 0);
        let (score, nodes) = run_quiesce(&position, alpha, INFINITY, 0, &table);
        assert_eq!((score, nodes), (stand_pat, 0));
        let hit = table.probe(key, 0).unwrap();
        assert_eq!((hit.score, hit.depth), (stand_pat + 123, 4));
    }
}

// 設計書movegen-speedup-2.md「段階9」: 候補があり、SEEで全て捨てるノードは保存する。
#[test]
fn quiescence_see_pruned_candidates_still_store() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(3, 10), Color::Black, PieceKind::Rook),
            (fs(3, 4), Color::White, PieceKind::Pawn),
            (fs(3, 1), Color::White, PieceKind::Rook),
        ],
    );
    let pst = weights().unwrap();
    let mut captures = Vec::new();
    MoveGenerator::standard().generate_captures(&position, &mut captures);
    assert!(!captures.is_empty());
    assert!(captures.iter().all(|&mv| capture_is_pruned_by_see(
        &position,
        engine_rules(),
        &pst,
        mv
    )));
    let table = small_tt();
    let stand_pat = evaluate(&pst, &position);
    let (score, nodes) = run_quiesce(&position, -INFINITY, INFINITY, 0, &table);
    assert_eq!((score, nodes), (stand_pat, 0));
    let hit = table.probe(search_key(&position), 0).unwrap();
    assert_eq!(
        (hit.score, hit.bound, hit.best_move),
        (stand_pat, Bound::Exact, None)
    );
}
