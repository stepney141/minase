//! 内部ノードの探索を検査する。

use super::*;

// D7-SRCH-12。深さ0のExact記録は通常探索の深さ1条件を満たさず、
// 通常探索の評価値カットオフには使われない。
#[test]
fn depth_zero_tt_score_does_not_cut_off_depth_one_negamax() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::GoldGeneral),
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(6, 3), Color::White, PieceKind::GoldGeneral),
        ],
    );
    let key = search_key(&position);
    let table = small_tt();
    table.store(key, 0, 28_000, Bound::Exact, None, 0);
    assert_eq!(table.probe(key, 0).unwrap().depth, 0);

    let (score, nodes) = run_negamax(&position, 1, -INFINITY, INFINITY, 0, &table);
    assert_ne!(score, 28_000);
    assert!(score.abs() < 29_000);
    assert!(nodes > 1, "深さ1の子を探索しなければならない");
    assert_eq!(table.probe(key, 0).unwrap().depth, 1);
}

// docs/plans/strength-stage6.md「internal iterative reduction」「検証」。
// 記録手のない深さ3の探索は、深さ2の探索と同じ結果・探索量になり、深さ2で保存する。
#[test]
fn iir_without_tt_entry_searches_and_stores_reduced_depth() {
    let position = crate::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    let expected = run_negamax(&position, 2, -INFINITY, INFINITY, 0, &small_tt());
    let table = small_tt();
    let actual = run_negamax(&position, 3, -INFINITY, INFINITY, 0, &table);

    assert_eq!(actual, expected);
    assert_eq!(table.probe(search_key(&position), 0).unwrap().depth, 2);
}

// docs/plans/strength-stage6.md「internal iterative reduction」「検証」。
// 深さ不足の記録でも記録手があれば、要求された深さ3を保つ。
#[test]
fn iir_preserves_depth_when_tt_has_a_move() {
    let position = crate::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    let key = search_key(&position);
    let table = small_tt();
    run_negamax(&position, 1, -INFINITY, INFINITY, 0, &table);
    let hit = table.probe(key, 0).unwrap();
    assert_eq!(hit.depth, 1);
    assert!(hit.best_move.is_some());

    let (_, nodes) = run_negamax(&position, 3, -INFINITY, INFINITY, 0, &table);
    assert!(nodes > 1);
    assert_eq!(table.probe(key, 0).unwrap().depth, 3);
}

// docs/plans/strength-stage6.md「internal iterative reduction」「検証」。
// 記録手がなくても残り深さ2以下では減深しない。
#[test]
fn iir_preserves_depth_below_three() {
    let position = crate::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    for depth in [1, 2] {
        let table = small_tt();
        run_negamax(&position, depth, -INFINITY, INFINITY, 0, &table);
        assert_eq!(
            u32::from(table.probe(search_key(&position), 0).unwrap().depth),
            depth
        );
    }
}

// docs/plans/strength-stage6.md「internal iterative reduction」「検証」。
// stand-patに相当する記録手なしのエントリも減深の対象になる。
#[test]
fn iir_reduces_depth_when_tt_entry_has_no_move() {
    let position = crate::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    let key = search_key(&position);
    let table = small_tt();
    table.store(key, 0, 28_000, Bound::Exact, None, 0);
    assert!(table.probe(key, 0).unwrap().best_move.is_none());

    let (score, nodes) = run_negamax(&position, 3, -INFINITY, INFINITY, 0, &table);
    assert_ne!(score, 28_000);
    assert!(nodes > 1);
    assert_eq!(table.probe(key, 0).unwrap().depth, 2);
}

// null moveの子が捕獲なしの静止探索で打ち切られる場合、実着手の適用回数は0となる。
#[test]
fn null_move_cutoff_without_captures_counts_no_nodes() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::GoldGeneral),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let mut after_null = position.clone();
    after_null.make_null_move();
    let expected = -evaluate(&weights().unwrap(), &after_null);
    let beta = evaluate(&weights().unwrap(), &position);
    assert!(expected >= beta);
    let table = small_tt();
    let (score, nodes) = run_negamax(&position, 4, beta - 1, beta, 0, &table);
    assert_eq!((score, nodes), (expected, 0));
}

// docs/plans/strength-stage12.md「項目7　null move pruningの前提条件」「検証」。
// 零窓ならnull moveで打ち切れる局面でも、PV窓では実着手を読む。
#[test]
fn null_move_preconditions_skip_pv_nodes() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::GoldGeneral),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let beta = evaluate(&weights().unwrap(), &position);
    for alpha in [beta - 2, -INFINITY] {
        let actual = run_negamax(&position, 4, alpha, beta, 0, &small_tt());
        with_root_searcher(&position, &[], |searcher| {
            searcher.null_move_ply = Some(0);
            let mut board = position.clone();
            let score = searcher.negamax(&mut board, 4, alpha, beta, 0).unwrap();
            assert_eq!(actual, (score, searcher.nodes));
        });
        assert!(actual.1 > 0, "PVノードでは実着手を読む: alpha={alpha}");
    }
}

// 同節。補正後評価がβ-1なら実着手を読み、βとβ+1ならnull moveを試す。
// null move後の局面に深さ1のExact値を置き、試した場合だけ実着手数を0にする。
// 正負の補正と両手番を使い、補正前の評価や相手側の補正では判定できないようにする。
#[test]
fn null_move_preconditions_use_corrected_eval_at_beta_boundary() {
    for side in [Color::Black, Color::White] {
        let position = position(
            side,
            &[
                (fs(6, 12), Color::Black, PieceKind::King),
                (fs(6, 10), side, PieceKind::GoldGeneral),
                (fs(6, 1), Color::White, PieceKind::King),
            ],
        );
        let raw_eval = evaluate(&weights().unwrap(), &position);
        for beta_offset in [-1, 1] {
            let beta = raw_eval + beta_offset;
            for eval_margin in [-1, 0, 1] {
                with_root_searcher(&position, &[], |searcher| {
                    let correction = beta_offset + eval_margin;
                    let key = material_key(&position);
                    searcher.correction.update(side, key, correction * 4, 8);
                    assert_eq!(searcher.correction.read(side, key), correction);
                    let mut after_null = position.clone();
                    after_null.make_null_move();
                    searcher
                        .tt
                        .store(search_key(&after_null), 1, -beta, Bound::Exact, None, 1);
                    // 記録手なしの深さ6は深さ5となり、null moveの子は深さ1になる。
                    let mut board = position.clone();
                    let score = searcher.negamax(&mut board, 6, beta - 1, beta, 0).unwrap();
                    assert_eq!(board, position);
                    if eval_margin < 0 {
                        assert!(searcher.nodes > 0, "補正後評価がβ未満なら実着手を読む");
                    } else {
                        assert_eq!((score, searcher.nodes), (beta, 0));
                    }
                });
            }
        }
    }
}

// docs/plans/strength-stage6.md「設計判断」のinternal iterative reduction。
// 即時打ち切りは減深前の要求深さで判定し、深さが足りる場合は減深より先に返す。
#[test]
fn iir_tt_cutoff_uses_requested_depth_before_reduction() {
    let position = crate::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    let key = search_key(&position);
    for stored_depth in [2, 3] {
        let table = small_tt();
        table.store(key, stored_depth, 28_000, Bound::Exact, None, 0);
        let (score, nodes) = run_negamax(&position, 3, -INFINITY, INFINITY, 0, &table);
        if stored_depth == 3 {
            assert_eq!((score, nodes), (28_000, 0));
            assert!(table.probe(key, 0).unwrap().best_move.is_none());
        } else {
            assert_ne!(score, 28_000);
            assert!(nodes > 1);
            assert!(table.probe(key, 0).unwrap().best_move.is_some());
        }
        assert_eq!(u32::from(table.probe(key, 0).unwrap().depth), stored_depth);
    }
}
