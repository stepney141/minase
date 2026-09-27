//! 探索結果の回帰契約を検査する。

use super::*;

/// 「段階6」（movegen-speedup-2.md）の300手の履歴と反復判定を検証する。
/// 「段階7」以降は探索木が変わるため、結果の再現性と合法性を確かめる。
#[test]
fn stage6_long_history_search_contract() {
    let rules = engine_rules();
    let generator = MoveGenerator::new(rules);
    let mut root = Position::initial();
    let mut history = vec![search_key(&root)];
    let mut rng = crate::rng::XorShift64::new(NonZeroU64::new(0x5354_4147_4536_0001).unwrap());
    for _ in 0..300 {
        let mut moves = Vec::new();
        generator.generate_moves(&root, &mut moves);
        moves.retain(|&mv| !captures_last_royal(&root, mv));
        let mv = moves[rng.next() as usize % moves.len()];
        root.make_move_unchecked(mv, rules);
        history.push(search_key(&root));
    }
    let moves = legal_moves(&root);
    let result = run_search(
        &root,
        rules,
        &moves,
        &history,
        &depth_limits(5),
        DEFAULT_THREADS,
        &mut small_tt(),
    );
    assert!(moves.contains(&result.best_move));
    assert_eq!(result.depth, 5);
    assert!(result.nodes > 0);
    assert_eq!(
        result,
        run_search(
            &root,
            rules,
            &moves,
            &history,
            &depth_limits(5),
            DEFAULT_THREADS,
            &mut small_tt(),
        )
    );
    // null moveは経路にキーを追加しない。パス後の実着手による反復も参照に含める。
    let mut after_null = root.clone();
    after_null.make_null_move();
    let reply = legal_moves(&after_null)
        .into_iter()
        .find(|&mv| !captures_last_royal(&after_null, mv))
        .unwrap();
    let mut repeated = after_null.clone();
    repeated.make_move_unchecked(reply, rules);
    history[1] = search_key(&repeated);
    with_root_searcher(&root, &history, |searcher| {
        searcher.null_move_ply = Some(1);
        searcher.accumulators[1] = searcher.pst.refresh_accumulator(&after_null);
        let path = searcher.path_keys.clone();
        let score =
            searcher.search_move(&mut after_null, reply, 2, -INFINITY, INFINITY, 1, true, 0);
        assert_eq!(score, Some(DRAW_SCORE));
        assert_eq!(searcher.path_keys, path);
        assert_eq!(searcher.nodes, 1);
    });
    // 探索木を変える枝刈りでも、同じ履歴からの零窓探索は再現でき、
    // 局面と経路を復元し、主変化には合法手だけを保持する。
    let mut outcomes = Vec::new();
    for _ in 0..2 {
        with_root_searcher(&root, &history, |searcher| {
            let mut current = root.clone();
            let path = searcher.path_keys.clone();
            let score = searcher
                .negamax(&mut current, 5, -1, 0, 0)
                .expect("unlimited negamax search must complete");
            assert_eq!(current, root);
            assert_eq!(searcher.path_keys, path);
            assert!(searcher.nodes > 0);
            for &mv in &searcher.pv[0] {
                assert!(legal_moves(&current).contains(&mv));
                current.make_move_unchecked(mv, rules);
            }
            outcomes.push((score, searcher.nodes, searcher.pv[0].clone()));
        });
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

/// 「段階6」（movegen-speedup-2.md）のノード上限を境界の両側で検証する。
/// 「段階8」では探索木が変わるため、完了深さの旧値ではなく上限と再現性を固定する。
#[test]
fn stage6_fixed_node_search_contract() {
    let root = Position::initial();
    for nodes in [
        1,
        STOP_CHECK_INTERVAL - 1,
        STOP_CHECK_INTERVAL,
        STOP_CHECK_INTERVAL + 1,
        10_000,
    ] {
        let result = run_search(
            &root,
            engine_rules(),
            &legal_moves(&root),
            &[search_key(&root)],
            &nodes_limits(nodes),
            DEFAULT_THREADS,
            &mut small_tt(),
        );
        assert!(legal_moves(&root).contains(&result.best_move));
        assert_eq!(
            result,
            run_search(
                &root,
                engine_rules(),
                &legal_moves(&root),
                &[search_key(&root)],
                &nodes_limits(nodes),
                DEFAULT_THREADS,
                &mut small_tt(),
            )
        );
        assert_eq!(result.nodes, nodes);
    }
}
