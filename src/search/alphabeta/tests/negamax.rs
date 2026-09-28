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
    let table = small_tt();
    let (score, nodes) = run_negamax(&position, 4, expected - 1, expected, 0, &table);
    assert_eq!((score, nodes), (expected, 0));
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

// docs/plans/search-bug-fixes.md「フェーズ1」。双方の獅子は2手では接触せず、
// 先手だけが奔王を持つため、反復がなければ後手のじっとは不利な評価を保つ。
fn null_repetition_fixture() -> Position {
    position(
        Color::Black,
        &[
            (fs(1, 12), Color::Black, PieceKind::King),
            (fs(4, 10), Color::Black, PieceKind::Lion),
            (fs(6, 11), Color::Black, PieceKind::FreeKing),
            (fs(12, 1), Color::White, PieceKind::King),
            (fs(9, 3), Color::White, PieceKind::Lion),
        ],
    )
}

fn jitto(position: &Position) -> Move {
    legal_moves(position)
        .into_iter()
        .find(|mv| mv.from == mv.to)
        .expect("fixture must allow jitto")
}

#[test]
fn null_move_jitto_does_not_repeat_pre_null_position() {
    let root = null_repetition_fixture();
    let history = [search_key(&root)];
    let mut after_null = root.clone();
    after_null.make_null_move();
    let reply = jitto(&after_null);
    let mut returned = after_null.clone();
    returned.make_move_unchecked(reply, engine_rules());
    assert_eq!(search_key(&returned), search_key(&root));
    assert!(evaluate(&weights().unwrap(), &root) > DRAW_SCORE);

    with_root_searcher(&root, &history, |searcher| {
        // 深さ0の記録は手順の優先にだけ使い、後手のじっとを最初に読む。
        searcher
            .tt
            .store(search_key(&after_null), 0, 0, Bound::Upper, Some(reply), 1);
        let mut current = root.clone();
        let path = searcher.path_keys.clone();
        let score = searcher.search_null_move(&mut current, 1, 1, 0).unwrap();
        assert!(
            score > DRAW_SCORE,
            "null move前との一致で引き分けにしてはならない: {score}"
        );
        assert_eq!(current, root);
        assert_eq!(searcher.path_keys, path);
        assert_eq!(searcher.null_move_boundary, None);
    });
}

#[test]
fn null_move_two_jittos_repeat_post_null_position() {
    let root = null_repetition_fixture();
    let mut after_null = root.clone();
    after_null.make_null_move();
    let black_jitto = jitto(&root);
    let white_jitto = jitto(&after_null);
    let mut returned = after_null.clone();
    returned.make_move_unchecked(white_jitto, engine_rules());
    returned.make_move_unchecked(black_jitto, engine_rules());
    assert_eq!(search_key(&returned), search_key(&after_null));

    with_root_searcher(&root, &[], |searcher| {
        // 双方のじっとを先に読む。後手のじっと後の先手の窓は[-1, 0]なので、
        // 2手目が反復ならその引き分け値で打ち切り、先手局面に下界0が残る。
        searcher.tt.store(
            search_key(&after_null),
            0,
            -1,
            Bound::Upper,
            Some(white_jitto),
            1,
        );
        searcher
            .tt
            .store(search_key(&root), 0, 1, Bound::Lower, Some(black_jitto), 2);
        let mut current = root.clone();
        let path = searcher.path_keys.clone();
        let score = searcher.search_null_move(&mut current, 2, 0, 0).unwrap();
        assert_eq!(score, DRAW_SCORE);
        let hit = searcher.tt.probe(search_key(&root), 2).unwrap();
        assert_eq!(
            hit.score, DRAW_SCORE,
            "最初のじっとで打ち切らず、双方のじっとで反復を検出する"
        );
        assert_eq!(hit.bound, Bound::Lower);
        assert_eq!(hit.best_move, Some(black_jitto));
        assert_eq!(current, root);
        assert_eq!(searcher.path_keys, path);
        assert_eq!(searcher.null_move_boundary, None);
    });
}

// null moveの外では、対局履歴と探索経路のどちらとの一致も反復とする。
#[test]
fn jitto_repetition_without_null_move_uses_history_and_path() {
    let root = null_repetition_fixture();
    let mv = jitto(&root);
    let mut child = root.clone();
    child.make_move_unchecked(mv, engine_rules());
    let key = search_key(&child);
    for history in [&[key][..], &[][..]] {
        with_root_searcher(&root, history, |searcher| {
            if history.is_empty() {
                searcher.path_keys.insert(0, key);
            }
            let path = searcher.path_keys.clone();
            let mut current = root.clone();
            assert_eq!(
                searcher.search_move(&mut current, mv, 1, -INFINITY, INFINITY, 0, true, 0),
                Some(DRAW_SCORE)
            );
            assert_eq!(searcher.nodes, 1);
            assert_eq!(current, root);
            assert_eq!(searcher.path_keys, path);
        });
    }
}

/// 外側のnull moveと後手のじっとを経た、先手番の探索経路を作る。
fn set_outer_null_path(searcher: &mut Searcher<'_>, root: &Position) {
    let mut current = root.clone();
    current.make_null_move();
    searcher.path_keys.push(search_key(&current));
    current.make_move_unchecked(jitto(&current), engine_rules());
    assert_eq!(search_key(&current), search_key(root));
    searcher.path_keys.push(search_key(&current));
    searcher.null_move_ply = Some(1);
    searcher.null_move_boundary = Some(1);
}

#[test]
fn nested_null_move_restores_outer_repetition_scope() {
    let root = null_repetition_fixture();
    with_root_searcher(&root, &[], |searcher| {
        set_outer_null_path(searcher, &root);
        let mut after_null = root.clone();
        after_null.make_null_move();
        searcher.tt.store(
            search_key(&after_null),
            0,
            0,
            Bound::Upper,
            Some(jitto(&after_null)),
            3,
        );
        let path = searcher.path_keys.clone();
        let mut current = root.clone();
        // 内側のnull moveでは外側の経路との一致を無視する。
        assert!(searcher.search_null_move(&mut current, 1, 1, 2).unwrap() > DRAW_SCORE);
        assert_eq!(current, root);
        assert_eq!(searcher.path_keys, path);
        assert_eq!(searcher.null_move_ply, Some(1));
        assert_eq!(searcher.null_move_boundary, Some(1));
        // 内側から戻れば、外側のnull move直後との一致は再び反復となる。
        assert_eq!(
            searcher.search_move(
                &mut current,
                jitto(&root),
                1,
                -INFINITY,
                INFINITY,
                2,
                true,
                0
            ),
            Some(DRAW_SCORE)
        );
        assert_eq!(current, root);
        assert_eq!(searcher.path_keys, path);
    });
}

#[test]
fn interrupted_null_move_restores_repetition_scope() {
    let root = null_repetition_fixture();
    let pst = weights().unwrap();
    for nested in [false, true] {
        let external_stop = AtomicBool::new(false);
        let shared = SharedSearch {
            external_stop: &external_stop,
            team_stop: AtomicBool::new(false),
            stop_reason: AtomicU8::new(0),
            total_nodes: AtomicU64::new(0),
            node_limit: Some(1),
            started: Instant::now(),
            hard_limit: None,
        };
        let table = small_tt();
        let mut searcher = new_searcher(&pst, &root, engine_rules(), &[], &shared, &table);
        if nested {
            set_outer_null_path(&mut searcher, &root);
        }
        let path = searcher.path_keys.clone();
        let null_ply = searcher.null_move_ply;
        let boundary = searcher.null_move_boundary;
        let mut current = root.clone();
        // 実着手を1手積んだ後、次の実着手でノード上限に達して中断する。
        assert_eq!(
            searcher.search_null_move(&mut current, 2, 0, if nested { 2 } else { 0 }),
            None
        );
        assert_eq!(searcher.stop_reason, Some(StopReason::NodeLimit));
        assert_eq!(searcher.nodes, 1);
        assert_eq!(current, root);
        assert_eq!(searcher.path_keys, path);
        assert_eq!(searcher.null_move_ply, null_ply);
        assert_eq!(searcher.null_move_boundary, boundary);
    }
}
