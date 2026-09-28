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

    let (score, nodes) = run_negamax(&position, 1, -1, 0, 0, &table);
    assert_ne!(score, 28_000);
    assert!(score.abs() < 29_000);
    assert!(nodes > 0, "深さ1の子を探索しなければならない");
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
    let table = small_tt();
    let (score, nodes) = run_negamax(&position, 4, expected - 1, expected, 0, &table);
    assert_eq!((score, nodes), (expected, 0));
}

// docs/plans/strength-stage6.md「設計判断」のinternal iterative reduction。
// 非PVノードの即時打ち切りは減深前の要求深さで判定し、深さが足りれば減深より先に返す。
#[test]
fn iir_tt_cutoff_uses_requested_depth_before_reduction() {
    let position = crate::parse_sfen("k11/12/12/12/12/12/12/12/12/12/12/11K b").unwrap();
    let key = search_key(&position);
    for stored_depth in [2, 3] {
        let table = small_tt();
        table.store(key, stored_depth, 28_000, Bound::Exact, None, 0);
        let (score, nodes) = run_negamax(&position, 3, -1, 0, 0, &table);
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

// docs/plans/strength-stage12.md「項目8」「検証」。3種類の記録値について、
// 幅1では打ち切り、幅2では探索して最後の王駒の捕獲による詰み値を返す。
#[test]
fn negamax_tt_cutoffs_only_at_non_pv_nodes() {
    let position = position(
        Color::Black,
        &[
            (fs(1, 12), Color::Black, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::Rook),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let stand_pat = evaluate(&weights().unwrap(), &position);
    let key = search_key(&position);
    for width in [1, 2] {
        let beta = stand_pat + 1;
        let alpha = beta - width;
        for (bound, stored_score) in [
            (Bound::Exact, stand_pat + 17),
            (Bound::Lower, beta),
            (Bound::Upper, alpha),
        ] {
            let table = small_tt();
            table.store(key, 1, stored_score, bound, None, 0);
            let (score, nodes) = run_negamax(&position, 1, alpha, beta, 0, &table);
            if width == 1 {
                assert_eq!((score, nodes), (stored_score, 0), "{bound:?}");
            } else {
                assert_eq!(score, MATE - 1, "{bound:?}");
            }
        }
    }
}

// docs/plans/strength-stage12.md「項目8」。PVでも記録手を先に読み、
// 同じ詰み値になる2手のうち記録手で打ち切る。
#[test]
fn negamax_pv_keeps_tt_move_first() {
    let position = position(
        Color::Black,
        &[
            (fs(1, 12), Color::Black, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::Rook),
            (fs(3, 1), Color::Black, PieceKind::Rook),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let stand_pat = evaluate(&weights().unwrap(), &position);
    let key = search_key(&position);
    for from in [fs(6, 10), fs(3, 1)] {
        let tt_move = Move {
            from,
            mid: None,
            to: fs(6, 1),
            promote: false,
        };
        assert!(legal_moves(&position).contains(&tt_move));
        let table = small_tt();
        table.store(key, 1, 0, Bound::Exact, Some(tt_move), 0);
        let (score, _) = run_negamax(&position, 1, stand_pat - 1, stand_pat + 1, 0, &table);
        assert_eq!(score, MATE - 1);
        let hit = table.probe(key, 0).unwrap();
        assert_eq!(hit.score, score);
        assert_eq!(hit.best_move, Some(tt_move));
    }
}
