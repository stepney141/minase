//! 内部ノードの探索を検査する。

use super::super::negamax::{ChildSearch, child_cut_node};
use super::*;

// F0の受け渡し表を期待値とし、親の値に依存する行は真と偽の両方を検査する。
#[test]
fn null_move_child_is_not_cut_node() {
    for parent in [false, true] {
        assert!(!child_cut_node(ChildSearch::NullMove, parent));
    }
}

#[test]
fn first_child_of_pv_node_is_not_cut_node() {
    assert!(!child_cut_node(ChildSearch::First { pv_node: true }, false));
}

#[test]
fn first_child_of_zero_window_node_inverts_cut_node() {
    assert!(child_cut_node(ChildSearch::First { pv_node: false }, false));
    assert!(!child_cut_node(ChildSearch::First { pv_node: false }, true));
}

#[test]
fn later_child_at_depth_two_or_more_is_cut_node() {
    for depth in [2, 5] {
        for parent in [false, true] {
            assert!(child_cut_node(ChildSearch::Later { depth }, parent));
        }
    }
}

#[test]
fn later_child_at_depth_one_inverts_cut_node() {
    assert!(child_cut_node(ChildSearch::Later { depth: 1 }, false));
    assert!(!child_cut_node(ChildSearch::Later { depth: 1 }, true));
}

#[test]
fn zero_window_research_inverts_cut_node() {
    assert!(child_cut_node(ChildSearch::ZeroWindowResearch, false));
    assert!(!child_cut_node(ChildSearch::ZeroWindowResearch, true));
}

#[test]
fn full_window_research_is_not_cut_node() {
    for parent in [false, true] {
        assert!(!child_cut_node(ChildSearch::FullWindowResearch, parent));
    }
}

// 深さ0のExact記録は通常探索の深さ1条件を満たさず、
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
    table.store(key, 0, 28_000, Bound::Exact, None, 0, false);
    assert_eq!(table.probe(key, 0).unwrap().depth, 0);

    let (score, nodes) = run_negamax(&position, 1, -INFINITY, INFINITY, 0, &table);
    assert_ne!(score, 28_000);
    assert!(score.abs() < 29_000);
    assert!(nodes > 1, "深さ1の子を探索しなければならない");
    assert_eq!(table.probe(key, 0).unwrap().depth, 1);
}

// 深さ不足の記録でも記録手があれば、内部反復減深を行わず要求深さ3を保つ。
// 判断の理由は docs/adr/reduce-depth-by-one-without-tt-move-except-at-all-nodes-and-on-the-previous-pv.md にある。
#[test]
fn iir_preserves_depth_when_tt_has_a_move() {
    let position = bare_kings();
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

// 内部反復減深は要求深さ3以上に限り、記録手がなくても残り深さ2以下では減深しない。
#[test]
fn iir_preserves_depth_below_three() {
    let position = bare_kings();
    for depth in [1, 2] {
        let table = small_tt();
        run_negamax(&position, depth, -INFINITY, INFINITY, 0, &table);
        assert_eq!(
            u32::from(table.probe(search_key(&position), 0).unwrap().depth),
            depth
        );
    }
}

// stand-patに相当する記録手なしのエントリも内部反復減深の対象となり、要求深さ3を2へ減らす。
#[test]
fn iir_reduces_depth_when_tt_entry_has_no_move() {
    let position = bare_kings();
    let key = search_key(&position);
    let table = small_tt();
    table.store(key, 0, 28_000, Bound::Exact, None, 0, false);
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
    let expected = -evaluate(test_nnue(), &after_null);
    with_root_searcher(&position, &[], |searcher| {
        assert_eq!(
            searcher.search_null_move(&mut position.clone(), 0, expected, 0),
            Some(expected)
        );
        assert_eq!(searcher.nodes, 0);
    });
}

// 置換表の打ち切りは減深前の要求深さで判定し、深さが足りる場合は内部反復減深より先に返す。
#[test]
fn iir_tt_cutoff_uses_requested_depth_before_reduction() {
    let position = bare_kings();
    let key = search_key(&position);
    for stored_depth in [2, 3] {
        let table = small_tt();
        table.store(key, stored_depth, 28_000, Bound::Exact, None, 0, false);
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

// null moveより前との一致は反復とせず、直近のnull move直後以降の一致だけを引き分けにする。
// この局面では獅子が2手で接触せず、先手だけが奔王を持つので、反復でなければ後手のじっとは不利な値を保つ。
// 判定範囲の理由は docs/adr/detect-repetition-only-after-latest-null-move.md にある。
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
    assert!(evaluate(test_nnue(), &root) > DRAW_SCORE);

    with_root_searcher(&root, &history, |searcher| {
        // 深さ0の記録は手順の優先にだけ使い、後手のじっとを最初に読む。
        searcher.tt.store(
            search_key(&after_null),
            0,
            0,
            Bound::Upper,
            Some(reply),
            1,
            false,
        );
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
    // 対称な駒配置で評価差を小さく保ち、静的評価による枝刈りより先に反復を読ませる。
    let root = position(
        Color::Black,
        &[
            (fs(1, 12), Color::Black, PieceKind::King),
            (fs(4, 10), Color::Black, PieceKind::Lion),
            (fs(12, 1), Color::White, PieceKind::King),
            (fs(9, 3), Color::White, PieceKind::Lion),
        ],
    );
    let mut after_null = root.clone();
    after_null.make_null_move();
    let black_jitto = jitto(&root);
    let white_jitto = jitto(&after_null);
    let mut returned = after_null.clone();
    returned.make_move_unchecked(white_jitto, engine_rules());
    returned.make_move_unchecked(black_jitto, engine_rules());
    assert_eq!(search_key(&returned), search_key(&after_null));

    with_root_searcher(&root, &[], |searcher| {
        // じっと以外の応手は後手から見て負と固定し、反復の引き分けを最善にする。
        // 子の値1はβ=0より大きいので、F5の要求深さ1に対して記録深さ2を用いる。
        for mv in legal_moves(&after_null) {
            if mv != white_jitto {
                let mut child = after_null.clone();
                child.make_move_unchecked(mv, engine_rules());
                searcher
                    .tt
                    .store(search_key(&child), 2, 1, Bound::Exact, None, 2, false);
            }
        }

        // 双方のじっとを先に読む。後手のじっと後の先手の窓は[-1, 0]なので、
        // 2手目が反復ならその引き分け値で打ち切り、先手局面に下界0が残る。
        searcher.tt.store(
            search_key(&after_null),
            0,
            -1,
            Bound::Upper,
            Some(white_jitto),
            1,
            false,
        );
        searcher.tt.store(
            search_key(&root),
            0,
            1,
            Bound::Lower,
            Some(black_jitto),
            2,
            false,
        );
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
    for history in [&[key, search_key(&root)][..], &[][..]] {
        let start = if history.is_empty() { &child } else { &root };
        with_root_searcher(start, history, |searcher| {
            if history.is_empty() {
                let mut current = child.clone();
                let reply = jitto(&current);
                push_path_move(searcher, &mut current, Some(reply));
                assert_eq!(search_key(&current), search_key(&root));
            }
            let path = searcher.path_keys.clone();
            let mut current = root.clone();
            assert_eq!(
                searcher.search_move(
                    &mut current,
                    mv,
                    1,
                    -INFINITY,
                    INFINITY,
                    0,
                    true,
                    0,
                    None,
                    false
                ),
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
    push_path_move(searcher, &mut current, None);
    let reply = jitto(&current);
    push_path_move(searcher, &mut current, Some(reply));
    assert_eq!(search_key(&current), search_key(root));
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
            false,
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
                0,
                None,
                false
            ),
            Some(DRAW_SCORE)
        );
        assert_eq!(current, root);
        assert_eq!(searcher.path_keys, path);
    });
}

#[test]
fn article_31_r1_null_moves_restore_repetition_scope_after_completion_or_interruption() {
    let root = null_repetition_fixture();
    let nnue = test_nnue();
    let mut shared_histories = crate::search::HistoryTables::new(DEFAULT_THREADS);
    for nested in [false, true] {
        let external_stop = AtomicBool::new(false);
        let shared = SharedSearch {
            external_stop: &external_stop,
            team_stop: AtomicBool::new(false),
            stop_reason: AtomicU8::new(0),
            total_nodes: AtomicU64::new(0),
            node_limit: Some(1),
            clock: &ManualClock::new(Duration::ZERO),
            hard_limit: None,
        };
        let table = small_tt();
        let mut butterfly_history =
            Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
        let mut capture_history = empty_capture_history();
        let correction_values = new_correction_values();
        let mut searcher = new_searcher(
            nnue,
            &root,
            engine_rules(),
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            DRAW_SCORE,
            &[],
            &shared,
            &table,
            &mut butterfly_history,
            &mut capture_history,
            &correction_values,
            &mut shared_histories.continuation_correction[0],
            &shared_histories.continuation,
            &shared_histories.pawn,
        );
        searcher.seek_mate = true;
        if nested {
            set_outer_null_path(&mut searcher, &root);
        }
        let path = searcher.path_keys.clone();
        let null_ply = searcher.null_move_ply;
        let boundary = searcher.null_move_boundary;
        let mut current = root.clone();
        // 専用帯の窓で入口の枝刈りを避け、実着手を1手積んだ後に中断する。
        let beta = -crate::search::REPETITION_BAND_START;
        let mut passed = root.clone();
        passed.make_null_move();
        table.store(
            search_key(&passed),
            0,
            -beta,
            Bound::Exact,
            None,
            if nested { 3 } else { 1 },
            true,
        );
        assert_eq!(
            searcher.search_null_move(&mut current, 2, beta, if nested { 2 } else { 0 }),
            None,
            "article_31_r1 nested={nested}"
        );
        assert_eq!(
            searcher.stop_reason,
            Some(StopReason::NodeLimit),
            "article_31_r1 nested={nested}"
        );
        assert_eq!(searcher.nodes, 1, "article_31_r1 nested={nested}");
        assert_eq!(current, root, "article_31_r1 nested={nested}");
        assert_eq!(searcher.path_keys, path, "article_31_r1 nested={nested}");
        assert_eq!(
            searcher.null_move_ply, null_ply,
            "article_31_r1 nested={nested}"
        );
        assert_eq!(
            searcher.null_move_boundary, boundary,
            "article_31_r1 nested={nested}"
        );
    }

    let (root, _) = rook_cycle(true, false);
    for interrupt in [None, Some(0), Some(1), Some(2)] {
        with_root_searcher(&root, &[], |searcher| {
            searcher.seek_mate = true;
            let mut current = root.clone();
            searcher.interrupt_at = interrupt.map(|node| (node, StopReason::ExternalStop));
            let beta = -crate::search::REPETITION_BAND_START;
            let mut passed = root.clone();
            passed.make_null_move();
            searcher
                .tt
                .store(search_key(&passed), 0, -beta, Bound::Exact, None, 1, true);
            let score = searcher.search_null_move(&mut current, 2, beta, 0);
            assert_eq!(
                score.is_none(),
                interrupt.is_some(),
                "article_31_r1 null, interrupt={interrupt:?}"
            );
            assert_eq!(current, root, "article_31_r1 null, interrupt={interrupt:?}");
            assert_eq!(
                searcher.path_keys,
                [search_key(&root)],
                "article_31_r1 null, interrupt={interrupt:?}"
            );
            assert!(
                searcher.path_moves.is_empty(),
                "article_31_r1 null, interrupt={interrupt:?}"
            );
            assert_eq!(
                searcher.null_move_boundary, None,
                "article_31_r1 null, interrupt={interrupt:?}"
            );
            assert_eq!(
                searcher.null_move_ply, None,
                "article_31_r1 null, interrupt={interrupt:?}"
            );
        });
    }
}

/// 置換表の手がない零窓の深さ3以下で、先頭捕獲がSEE負ならkiller手を先に読み、そこで打ち切れば捕獲の部分木を読まない。
/// 判断の理由は`docs/adr/try-killers-before-a-losing-first-capture.md`にある。
#[test]
fn early_killer_cutoff_skips_losing_capture_only_in_eligible_nodes() {
    let (board, capture, killers) = super::ordering::losing_capture_fixture();
    let killer = killers[0].unwrap();
    let mut killer_child = board.clone();
    killer_child.make_move_unchecked(killer, engine_rules());
    // 零窓の親をcutNodeにし、先頭の子のfail-lowをF5でも打ち切れるようにする。
    // 深さは要求値。4はIIR後に3となり対象、5は4となり対象外である。
    for (depth, width, tt_move, expected) in [
        (1, 1, None, killer),
        (2, 1, None, killer),
        (3, 1, None, killer),
        (4, 1, None, killer),
        (5, 1, None, capture),
        (2, 2, None, capture),
        (2, 1, Some(capture), capture),
    ] {
        with_root_searcher(&board, &[], |searcher| {
            searcher.killers[0] = killers;
            // null moveを無効にし、実着手による打ち切りだけを検査する。
            searcher.null_move_ply = Some(0);
            let static_eval = evaluate(searcher.nnue, &board);
            // 親ではrazoring、深さ1のkillerの子ではstand-patによる打ち切りを避ける。
            // 子の評価を親の手番へ戻し、子の静止探索が置換表を読む窓にする。
            let alpha = static_eval.min(-evaluate(searcher.nnue, &killer_child)) - 1;
            let beta = alpha + width;
            // 浅い上界で判定用の評価値をαに置き、reverse futilityを避ける。
            // 記録手の有無と窓は保ち、killerを先に読む条件を変えない。
            searcher.tt.store(
                search_key(&board),
                0,
                alpha,
                Bound::Upper,
                tt_move,
                0,
                false,
            );
            for (mv, score) in [(killer, beta), (capture, beta + 100)] {
                let mut child = board.clone();
                child.make_move_unchecked(mv, engine_rules());
                searcher
                    .tt
                    .store(search_key(&child), 5, -score, Bound::Exact, None, 1, false);
            }
            let mut current = board.clone();
            assert_eq!(
                searcher.negamax(&mut current, depth, alpha, beta, 0, width == 1),
                Some(if expected == killer { beta } else { beta + 100 }),
                "depth={depth}, width={width}, tt_move={tt_move:?}",
            );
            assert_eq!(searcher.nodes, 1, "先頭の1手だけで打ち切る");
            assert_eq!(
                searcher.tt.probe(search_key(&board), 0).unwrap().best_move,
                Some(expected)
            );
            assert_eq!(current, board);
        });
    }
}

/// 2手先だけを、静止探索への分岐後かつ置換表の照合前に初期化する（F4）。
#[test]
fn cutoff_counts_reset_two_plies_ahead_before_tt_probe() {
    let board = bare_kings();
    with_root_searcher(&board, &[], |searcher| {
        assert_eq!(searcher.root_depth, 0);
        assert!(searcher.cutoff_counts.iter().all(|&count| count == 0));
        searcher.cutoff_counts.fill(7);
        // allNodeでfail-lowになるExact値を使い、置換表の照合で直ちに戻る。
        searcher
            .tt
            .store(search_key(&board), 3, 0, Bound::Exact, None, 1, false);
        set_test_path_ply(searcher, 1);
        assert_eq!(
            searcher.negamax(&mut board.clone(), 2, 0, 1, 1, false),
            Some(0)
        );
        assert_eq!(&searcher.cutoff_counts[..5], &[7, 7, 7, 0, 7]);
        searcher.cutoff_counts[3] = 9;
        searcher
            .negamax(&mut board.clone(), 0, -INFINITY, INFINITY, 1, false)
            .unwrap();
        assert_eq!(
            searcher.cutoff_counts[3], 9,
            "静止探索へ移る場合は初期化しない"
        );
    });
}

/// β打ち切りを起こした手数の数を増やし、兄弟の探索で参照する子の数を保つ（F4）。
#[test]
fn cutoff_counts_accumulate_at_cutoff_ply() {
    let board = bare_kings();
    with_root_searcher(&board, &[], |searcher| {
        searcher.cutoff_counts[1] = 4;
        searcher.cutoff_counts[2] = 6;
        searcher.cutoff_counts[3] = 8;
        set_test_path_ply(searcher, 1);
        assert!(
            searcher
                .negamax(&mut board.clone(), 1, -INFINITY, -INFINITY + 1, 1, false)
                .unwrap()
                > -INFINITY
        );
        assert_eq!(&searcher.cutoff_counts[1..4], &[5, 6, 0]);
        searcher
            .tt
            .store(search_key(&board), 2, 0, Bound::Upper, None, 1, false);
        searcher
            .negamax(&mut board.clone(), 1, -INFINITY, -INFINITY + 1, 1, false)
            .unwrap();
        assert_eq!(searcher.cutoff_counts[1], 6);
    });
}

#[test]
fn root_resets_cutoff_counts_and_records_iteration_depth() {
    let board = bare_kings();
    with_root_searcher(&board, &[], |searcher| {
        searcher.cutoff_counts[2] = 7;
        searcher
            .search_root(&board, &legal_moves(&board), 1, -INFINITY, -INFINITY + 1)
            .unwrap();
        assert_eq!(searcher.cutoff_counts[2], 0);
        assert_eq!(searcher.cutoff_counts[0], 1);
        assert_eq!(searcher.root_depth, 1);
    });
}

/// 延長後の深さを255以下に制限してから置換表の深さを比較する（Stockfish src/search.cpp:752）。
#[test]
fn extended_depth_is_capped_before_tt_probe() {
    let board = bare_kings();
    for (requested, recorded, expected) in [
        (254, 254, Some(123)),
        (255, 254, None),
        (255, 255, Some(123)),
        (256, 255, Some(123)),
        (258, 255, Some(123)),
    ] {
        with_root_searcher(&board, &[], |searcher| {
            searcher.tt.store(
                search_key(&board),
                recorded,
                123,
                Bound::Exact,
                Some(legal_moves(&board)[0]),
                0,
                false,
            );
            // 深さ不足なら実着手の前で止め、置換表の値を返さないことを検査する。
            searcher.null_move_ply = Some(0);
            searcher.interrupt_at = Some((0, StopReason::ExternalStop));
            assert_eq!(
                searcher.negamax(&mut board.clone(), requested, -INFINITY, INFINITY, 0, false),
                expected,
                "要求深さ{requested}、記録深さ{recorded}"
            );
        });
    }
}

/// LMRの延長が最大手数へ達しても、静止探索の終端評価で止める。
#[test]
fn extended_search_stops_at_max_ply() {
    let board = bare_kings();
    with_root_searcher(&board, &[], |searcher| {
        let value = evaluate(test_nnue(), &board);
        set_test_path_ply(searcher, MAX_PLY);
        assert_eq!(
            searcher.negamax(&mut board.clone(), 2, -INFINITY, INFINITY, MAX_PLY, false),
            Some(value)
        );
        assert_eq!(searcher.nodes, 0);
    });
}

/// F4の数値例で初回の値を固定し、零窓と全窓の再探索が調整後の深さを使うことを確かめる。
#[test]
fn lmr_search_move_researches_at_adjusted_depth() {
    let deeper = 50 + crate::search::alphabeta::params::lmr_deeper_margin();
    let shallower = 50 + crate::search::alphabeta::params::lmr_shallower_margin();
    let board = bare_kings();
    let mv = legal_moves(&board)[0];
    let mut child = board.clone();
    child.make_move_unchecked(mv, engine_rules());
    let reply = legal_moves(&child)[0];
    // F5のcutNodeでfail-lowの値を固定するため、初回深さは4より深くする。
    for (reduced, value, expected) in [
        (5, deeper.max(shallower) + 1, 8),
        (5, deeper.min(shallower) - 1, 6),
        (7, deeper.max(shallower) + 1, 7),
    ] {
        with_root_searcher(&board, &[], |searcher| {
            searcher.tt.store(
                search_key(&child),
                reduced,
                -value,
                Bound::Exact,
                Some(reply),
                1,
                false,
            );
            let score = searcher
                .search_move(
                    &mut board.clone(),
                    mv,
                    8,
                    -INFINITY,
                    INFINITY,
                    0,
                    false,
                    reduced,
                    Some(50),
                    false,
                )
                .unwrap();
            let hit = searcher.tt.probe(search_key(&child), 1).unwrap();
            assert_eq!(hit.depth, expected, "初回深さ{reduced}、値{value}");
            // 全窓の再探索を経た値は正確な値として記録される。
            assert_eq!(hit.bound, Bound::Exact);
            assert_eq!(hit.score, -score);
        });
    }
}

// F5のIIR条件を、探索後に保存される深さで確かめる。
// 詰み帯の窓で静的評価による枝刈りを避け、王将だけの局面でnull moveを避ける。
#[test]
fn iir_respects_node_type_and_follow_pv() {
    let position = bare_kings();
    for (alpha, beta, cut_node, follow_pv, expected_depth) in [
        (INFINITY - 1, INFINITY, false, false, 3),
        (INFINITY - 1, INFINITY, true, false, 2),
        (-INFINITY, INFINITY, false, false, 2),
        (INFINITY - 1, INFINITY, true, true, 3),
        (-INFINITY, INFINITY, false, true, 3),
    ] {
        with_root_searcher(&position, &[], |searcher| {
            searcher.follow_pv[0] = follow_pv;
            searcher
                .negamax(&mut position.clone(), 3, alpha, beta, 0, cut_node)
                .unwrap();
            assert_eq!(
                searcher.tt.probe(search_key(&position), 0).unwrap().depth,
                expected_depth
            );
        });
    }
}

#[test]
fn follow_pv_requires_parent_and_matching_move_at_same_ply() {
    let position = bare_kings();
    let moves = legal_moves(&position);
    for ply in [0, 1, 2] {
        for parent in [false, true] {
            for mv in [moves[0], moves[1]] {
                with_root_searcher(&position, &[], |searcher| {
                    searcher.previous_pv = vec![moves[1], moves[0]];
                    searcher.follow_pv[ply as usize] = parent;
                    // 子の古い値は毎回上書きされなければならない。
                    searcher.follow_pv[(ply + 1) as usize] = true;
                    set_test_path_ply(searcher, ply);
                    searcher
                        .search_move(
                            &mut position.clone(),
                            mv,
                            1,
                            -INFINITY,
                            INFINITY,
                            ply,
                            true,
                            0,
                            None,
                            false,
                        )
                        .unwrap();
                    let expected = parent
                        && match ply {
                            0 => mv == moves[1],
                            1 => mv == moves[0],
                            _ => false,
                        };
                    assert_eq!(searcher.follow_pv[(ply + 1) as usize], expected);
                });
            }
        }
    }
}

#[test]
fn null_move_child_never_follows_pv() {
    let position = bare_kings();
    with_root_searcher(&position, &[], |searcher| {
        searcher.follow_pv[0] = true;
        searcher.follow_pv[1] = true;
        searcher.previous_pv = legal_moves(&position);
        searcher
            .search_null_move(&mut position.clone(), 0, 100, 0)
            .unwrap();
        assert!(!searcher.follow_pv[1]);
        assert!(searcher.follow_pv[0]);
    });
}
