//! 置換表を検査する。

use super::*;

// F1の準備: PVの印は他の欄と独立して保存・復号される。
#[test]
fn tt_pv_flag_round_trips_without_changing_other_fields() {
    let key = 0x1234_5678_0000_0042;
    let table = small_tt();
    for is_pv in [false, true] {
        for bound in [Bound::Exact, Bound::Lower, Bound::Upper] {
            for best_move in [None, Some(step(sq(0, 0), sq(0, 1)))] {
                table.store(key, 255, -1234, bound, best_move, 0, is_pv);
                let hit = table.probe(key, 0).unwrap();
                assert_eq!(hit.is_pv, is_pv);
                assert_eq!(hit.score, -1234);
                assert_eq!(hit.depth, 255);
                assert_eq!(hit.bound, bound);
                assert_eq!(hit.best_move, best_move);
            }
        }
    }
}

// 同じキーでは印によらず、同深さなら置換し、浅ければ元の記録を保つ。
#[test]
fn tt_pv_flag_does_not_affect_same_key_replacement() {
    let key = 0x1234_5678_0000_0042;
    for first_pv in [false, true] {
        for second_depth in [7, 8] {
            let table = small_tt();
            table.store(key, 8, 100, Bound::Exact, None, 0, first_pv);
            table.store(key, second_depth, 100, Bound::Exact, None, 0, !first_pv);
            let hit = table.probe(key, 0).unwrap();
            assert_eq!(
                hit.is_pv,
                if second_depth == 8 {
                    !first_pv
                } else {
                    first_pv
                }
            );
            assert_eq!(hit.depth, 8);
        }
    }
}

// PVノードは、置換表に記録がなくても印を立てて保存する。
#[test]
fn negamax_pv_node_stores_pv_flag() {
    let position = bare_kings();
    with_root_searcher(&position, &[], |searcher| {
        assert!(searcher.tt_pv.iter().all(|&is_pv| !is_pv));
        searcher
            .negamax(&mut position.clone(), 1, -INFINITY, INFINITY, 0, false)
            .unwrap();
        let hit = searcher.tt.probe(search_key(&position), 0).unwrap();
        assert_eq!(hit.depth, 1);
        assert!(hit.is_pv);
        assert!(searcher.tt_pv[0]);
    });
}

// 零窓でα以下なら親との論理和を保存する。詰み帯の窓で浅い枝刈りを除く。
#[test]
fn negamax_fail_low_inherits_parent_pv_flag() {
    let position = bare_kings();
    for parent_pv in [false, true] {
        for stored_pv in [false, true] {
            with_root_searcher(&position, &[], |searcher| {
                searcher.tt_pv[0] = parent_pv;
                searcher.tt_pv[1] = !stored_pv;
                let key = search_key(&position);
                searcher
                    .tt
                    .store(key, 0, 0, Bound::Upper, None, 1, stored_pv);
                set_test_path_ply(searcher, 1);
                let score = searcher
                    .negamax(&mut position.clone(), 1, INFINITY - 1, INFINITY, 1, false)
                    .unwrap();
                assert!(score < INFINITY);
                let hit = searcher.tt.probe(key, 1).unwrap();
                assert_eq!(hit.depth, 1);
                assert_eq!(hit.bound, Bound::Upper);
                assert_eq!(hit.is_pv, parent_pv || stored_pv);
                assert_eq!(searcher.tt_pv[1], parent_pv || stored_pv);
            });
        }
    }
}

// 捕獲のない王将だけの深さ1では、各子の静的評価の最大値が最善値になる。
// αとの等号でも親の印を継ぐことを、この独立した期待値で確かめる。
#[test]
fn negamax_inherits_parent_pv_flag_when_score_equals_alpha() {
    let mut position = bare_kings();
    let alpha = legal_moves(&position)
        .into_iter()
        .map(|mv| {
            let undo = position.make_move_unchecked(mv, engine_rules());
            let score = -evaluate(test_nnue(), &position);
            position.unmake_move(undo);
            score
        })
        .max()
        .unwrap();
    with_root_searcher(&position, &[], |searcher| {
        searcher.tt_pv[0] = true;
        // 浅い上界でreverse futilityを避け、印のないノードが親の印を継ぐ経路を読む。
        searcher.tt.store(
            search_key(&position),
            0,
            alpha,
            Bound::Upper,
            None,
            1,
            false,
        );
        set_test_path_ply(searcher, 1);
        let score = searcher
            .negamax(&mut position.clone(), 1, alpha, alpha + 1, 1, false)
            .unwrap();
        assert_eq!(score, alpha);
        let hit = searcher.tt.probe(search_key(&position), 1).unwrap();
        assert_eq!(hit.bound, Bound::Upper);
        assert!(hit.is_pv);
    });
}

// 手数0には親がいない。β打ち切りでは、親がいてもその印を継がない。
#[test]
fn negamax_pv_flag_respects_missing_parent_and_fail_high() {
    let position = bare_kings();
    for (ply, alpha, beta, bound) in [
        (0, INFINITY - 1, INFINITY, Bound::Upper),
        (1, -INFINITY, -INFINITY + 1, Bound::Lower),
    ] {
        with_root_searcher(&position, &[], |searcher| {
            searcher.tt_pv.fill(true);
            set_test_path_ply(searcher, ply);
            searcher
                .negamax(&mut position.clone(), 1, alpha, beta, ply, false)
                .unwrap();
            let hit = searcher.tt.probe(search_key(&position), ply).unwrap();
            assert_eq!(hit.bound, bound);
            assert!(!hit.is_pv);
            assert!(!searcher.tt_pv[ply as usize]);
        });
    }
}

// null move直後のノードも、直前の手数の印を継ぐ。
#[test]
fn null_move_child_inherits_parent_pv_flag_on_fail_low() {
    let position = bare_kings();
    let mut child = position.clone();
    child.make_null_move();
    for parent_pv in [false, true] {
        with_root_searcher(&position, &[], |searcher| {
            searcher.tt_pv[0] = parent_pv;
            searcher
                .search_null_move(&mut position.clone(), 1, -INFINITY + 1, 0)
                .unwrap();
            let hit = searcher.tt.probe(search_key(&child), 1).unwrap();
            assert_eq!(hit.bound, Bound::Upper);
            assert_eq!(hit.is_pv, parent_pv);
        });
    }
}

// 根では手数0と置換表の両方に印を立てる。
#[test]
fn root_stores_pv_flag() {
    let position = bare_kings();
    with_root_searcher(&position, &[], |searcher| {
        searcher
            .search_root(&position, &legal_moves(&position), 1, -INFINITY, INFINITY)
            .unwrap();
        assert!(searcher.tt_pv[0]);
        assert!(searcher.tt.probe(search_key(&position), 0).unwrap().is_pv);
    });
}

// 静止探索は照合した印だけを保存し、手数ごとの配列には触れない。
#[test]
fn quiescence_preserves_hit_pv_flag_without_changing_ply_flags() {
    let position = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 0), Color::Black, PieceKind::Rook),
            (sq(0, 2), Color::White, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    for stored_pv in [None, Some(false), Some(true)] {
        for ply_pv in [false, true] {
            with_root_searcher(&position, &[], |searcher| {
                let key = search_key(&position);
                if let Some(is_pv) = stored_pv {
                    searcher.tt.store(key, 0, 0, Bound::Lower, None, 0, is_pv);
                }
                searcher.tt_pv.fill(ply_pv);
                let score = searcher
                    .quiesce(&mut position.clone(), -INFINITY, INFINITY, 0)
                    .unwrap();
                let hit = searcher.tt.probe(key, 0).unwrap();
                assert_eq!(hit.depth, 0);
                assert_eq!(hit.bound, Bound::Exact);
                assert_eq!(hit.score, score);
                assert_eq!(hit.is_pv, stored_pv == Some(true));
                assert!(searcher.tt_pv.iter().all(|&value| value == ply_pv));
            });
        }
    }
}

// 監査「置換表サイズのオーバーフロー」: 容量は外部設定値なので、0と
// MiBからbyteへの変換不能をpanicではなく呼び出し側が処理できるエラーにする。
#[test]
fn invalid_table_sizes_are_reported_without_panicking() {
    assert!(matches!(
        TranspositionTable::new(0),
        Err(TranspositionTableError::Empty)
    ));
    assert!(matches!(
        TranspositionTable::new(usize::MAX),
        Err(TranspositionTableError::SizeOverflow)
    ));

    let mut table = TranspositionTable::new(1).unwrap();
    table.new_search();
    let key = 0x1234_5678_9abc_def0;
    let stored_move = Move {
        from: sq(0, 0),
        mid: None,
        to: sq(0, 1),
        promote: false,
    };
    table.store(key, 4, 321, Bound::Exact, Some(stored_move), 0, false);
    assert_eq!(
        table.resize(usize::MAX),
        Err(TranspositionTableError::SizeOverflow)
    );
    assert_eq!(table.generation(), 1);
    let hit = table
        .probe(key, 0)
        .expect("failed resize must retain entries");
    assert_eq!(hit.score, 321);
    assert_eq!(hit.best_move, Some(stored_move));
}

// bit 0–7がfrom、8–15がmid、16–23がto、
// 24がpromote。midなしは0xff、升の符号化は`Square::dense_index()`（0–143）。
// 各fieldの符号境界を独立した既知符号値と照合する。
#[test]
fn packed_moves_round_trip_across_field_boundaries() {
    // 上記のbit配置から手計算した符号値。相異なる升番号でfield混同を検出する。
    let cases = [
        (0, None, 143, false, 0x008f_ff00),
        (143, Some(0), 0, true, 0x0100_008f),
        (16, Some(32), 64, false, 0x0040_2010),
        (32, Some(64), 16, true, 0x0110_4020),
        (64, Some(16), 32, false, 0x0020_1040),
        (0x55, Some(0x2a), 0x55, false, 0x0055_2a55),
        (0x2a, Some(0x55), 0x2a, true, 0x012a_552a),
        (64, Some(143), 32, false, 0x0020_8f40),
        (143, None, 143, false, 0x008f_ff8f),
    ];
    for (from, mid, to, promote, encoded) in cases {
        let mv = Move {
            from: Square::from_dense(from).unwrap(),
            mid: mid.map(|index| Square::from_dense(index).unwrap()),
            to: Square::from_dense(to).unwrap(),
            promote,
        };
        assert_eq!(pack_move(mv), encoded);
        assert_eq!(unpack_move(encoded), Some(Some(mv)));
    }
    assert_eq!(unpack_move(0x01ff_ffff), Some(None));
    let table = small_tt();
    let key = 0x0fed_cba9_0000_0042;
    table.store(key, 0, 321, Bound::Exact, None, 0, false);
    let hit = table.probe(key, 0).unwrap();
    assert_eq!(hit.best_move, None);
    assert_eq!(hit.score, 321);
}

// 格納時は`score >= MATE−256`なら
// `score + ply`、`score <= −(MATE−256)`なら`score − ply`、取り出し時に
// 逆変換する。分岐境界はMATE−256=29744。
#[test]
fn tt_scores_round_trip_between_root_and_node_relative_forms() {
    let best_move = Move {
        from: sq(0, 0),
        mid: None,
        to: sq(0, 1),
        promote: false,
    };
    let key = 0x0123_4567_0000_0042;
    let table = small_tt();
    table.new_search();

    // 最大詰み値と最大plyでも格納値が欠けず、通常値の符号も保たれる。
    for (score, ply) in [
        (30_000, 0),
        (30_000, 256),
        (-30_000, 256),
        (0, 0),
        (-500, 256),
    ] {
        table.store(key, 8, score, Bound::Exact, Some(best_move), ply, false);
        assert_eq!(table.probe(key, ply).unwrap().score, score);
    }

    for bound in [Bound::Exact, Bound::Lower, Bound::Upper] {
        for sign in [-1, 1] {
            for (magnitude, probe_ply, expected) in [
                (29_744, 4, 29_745),
                (28_999, 4, 28_999),
                (29_487, 40, 29_487),
                (29_740, 40, 29_740),
                (29_743, 40, 29_743),
                (29_990, 40, 29_955),
            ] {
                let score = sign * magnitude;
                table.store(key, 8, score, bound, Some(best_move), 5, false);
                assert_eq!(
                    table.probe(key, probe_ply).unwrap().score,
                    sign * expected,
                    "article_31_r1 score={score}, bound={bound:?}, probe_ply={probe_ply}"
                );
            }
        }
    }
}

// 同一キーを深さで保護する理由はdocs/adr/store-quiescence-results-at-depth-zero.md。
// 同一キーは世代を
// 問わず既存以上の深さだけを書き込み、異なるキーは過去世代または既存より
// 深い結果だけを書き込む。
#[test]
fn tt_replacement_follows_same_key_generation_then_depth() {
    let best_move = Move {
        from: sq(0, 0),
        mid: None,
        to: sq(0, 1),
        promote: false,
    };
    // 1MBの表は2の冪スロットで下位ビットが一致するキー対が同一スロットに
    // 落ちる。上位32bitの照合キーは異なる。
    let key_a = 0x1111_1111_0000_0001;
    let key_b = 0x2222_2222_0000_0001;

    // (1) 空きスロットへは書き込まれる。
    let mut table = small_tt();
    table.new_search();
    table.store(key_a, 8, 100, Bound::Exact, Some(best_move), 0, false);
    let hit = table.probe(key_a, 0).unwrap();
    assert_eq!(hit.score, 100);
    assert_eq!(hit.depth, 8);

    // (2a) 同一キー・同世代の浅い結果は既存値を保持する。
    table.store(key_a, 2, 222, Bound::Upper, Some(best_move), 0, false);
    let hit = table.probe(key_a, 0).unwrap();
    assert_eq!(hit.score, 100);
    assert_eq!(hit.depth, 8);
    assert_eq!(hit.bound, Bound::Exact);

    // (2b) 同一キー・同深さは最終書き込み優先とし、バウンド種別を問わず
    //      置換する。`>`への変異を検出する境界でもある。
    table.store(key_a, 8, 333, Bound::Lower, Some(best_move), 0, false);
    let hit = table.probe(key_a, 0).unwrap();
    assert_eq!(hit.score, 333);
    assert_eq!(hit.depth, 8);
    assert_eq!(hit.bound, Bound::Lower);

    // (2c) 同一キーの深い結果は置換する。
    table.store(key_a, 9, 444, Bound::Upper, Some(best_move), 0, false);
    let hit = table.probe(key_a, 0).unwrap();
    assert_eq!(hit.score, 444);
    assert_eq!(hit.depth, 9);
    assert_eq!(hit.bound, Bound::Upper);

    // (3) 同一キーは過去世代でも浅い結果を保持し、保持時は世代を更新しない。
    table.clear();
    table.new_search();
    table.store(key_a, 8, 100, Bound::Exact, Some(best_move), 0, false);
    let (_, advisory_before) = table.raw_entry(key_a);
    table.new_search();
    table.store(key_a, 2, 222, Bound::Upper, Some(best_move), 0, false);
    let hit = table.probe(key_a, 0).unwrap();
    let (_, advisory_after) = table.raw_entry(key_a);
    assert_eq!(hit.score, 100);
    assert_eq!(hit.depth, 8);
    assert_eq!(advisory_after, advisory_before);
    assert_eq!(
        ((advisory_after & ADVISORY_GENERATION_MASK) >> ADVISORY_GENERATION_SHIFT) as u8,
        1
    );

    // (4) 異キーでも既存世代が古ければ深さによらず置換する。
    table.clear();
    table.new_search();
    table.store(key_a, 8, 100, Bound::Exact, Some(best_move), 0, false);
    table.new_search();
    table.store(key_b, 1, 300, Bound::Exact, Some(best_move), 0, false);
    assert!(table.probe(key_a, 0).is_none());
    assert_eq!(table.probe(key_b, 0).unwrap().score, 300);

    // (5a) 異キー・同世代: 既存depth=3へdepth=5は置換する。
    table.clear();
    table.new_search();
    table.store(key_a, 3, 100, Bound::Exact, Some(best_move), 0, false);
    table.store(key_b, 5, 400, Bound::Exact, Some(best_move), 0, false);
    assert!(table.probe(key_a, 0).is_none());
    assert_eq!(table.probe(key_b, 0).unwrap().score, 400);

    // (5b) 異キー・同世代・同深さ: 既存を保持する（`<`と`<=`の変異検出）。
    table.clear();
    table.new_search();
    table.store(key_a, 5, 100, Bound::Exact, Some(best_move), 0, false);
    table.store(key_b, 5, 500, Bound::Exact, Some(best_move), 0, false);
    assert_eq!(table.probe(key_a, 0).unwrap().score, 100);
    assert!(table.probe(key_b, 0).is_none());

    // (5c) 異キー・同世代: 既存depth=7へdepth=5は保持する。
    table.clear();
    table.new_search();
    table.store(key_a, 7, 100, Bound::Exact, Some(best_move), 0, false);
    table.store(key_b, 5, 600, Bound::Exact, Some(best_move), 0, false);
    assert_eq!(table.probe(key_a, 0).unwrap().score, 100);
    assert!(table.probe(key_b, 0).is_none());

    // 境界: 世代の周回。既存gen=255・現在gen=0のとき年齢は
    // 0 wrapping_sub 255 = 1で「古い」と判定され置換される。
    let table = small_tt();
    for _ in 0..255 {
        table.new_search();
    }
    table.store(key_a, 8, 100, Bound::Exact, Some(best_move), 0, false);
    table.new_search();
    table.store(key_b, 1, 700, Bound::Exact, Some(best_move), 0, false);
    assert!(table.probe(key_a, 0).is_none());
    assert_eq!(table.probe(key_b, 0).unwrap().score, 700);
}

// 監査「置換表の最善手と評価値の別原子語」: 助言手は生成済み合法手と
// 一致した場合だけ順序付けへ使い、同じキーにある不合法手は無視する。
#[test]
fn illegal_transposition_table_move_is_ignored_at_the_root() {
    let root = quiet_midgame();
    let moves = legal_moves(&root);
    let illegal = Move {
        from: fs(2, 2),
        mid: None,
        to: fs(2, 3),
        promote: false,
    };
    assert!(!moves.contains(&illegal));

    let mut clean_table = small_tt();
    let expected = run_search(
        &root,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut clean_table,
    );

    let mut contaminated_table = small_tt();
    contaminated_table.store(
        search_key(&root),
        32,
        MATE,
        Bound::Exact,
        Some(illegal),
        0,
        false,
    );
    let actual = run_search(
        &root,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut contaminated_table,
    );

    assert_eq!(actual.best_move, expected.best_move);
    assert!(moves.contains(&actual.best_move));
}

// 反復した局面は探索経路へ積む前に引き分けとして返し、その局面を置換表へ保存しない。
// 親はこの引き分け値を含む探索結果を保存し得る。ここでは反復した子の不保存を検査する。
#[test]
fn repetition_draw_values_are_not_stored_in_the_table() {
    let root = repetition_fixture();
    let moves = legal_moves(&root);
    let draw_move = repetition_move();
    let mut child = root.clone();
    child.make_move_unchecked(draw_move, engine_rules());
    let child_key = search_key(&child);
    let history = [child_key, search_key(&root)];

    let mut table = small_tt();
    let result = run_search(
        &root,
        engine_rules(),
        &moves,
        &history,
        &depth_limits(2),
        DEFAULT_THREADS,
        &mut table,
    );

    // 反復終端が実際に起きたことの確認。
    assert_eq!(result.best_move, draw_move);
    assert_eq!(result.score, 0);
    // 反復で終端した子局面のキーはprobeしてもエントリが存在しない。
    assert!(table.probe(child_key, 0).is_none());
}

// 規則セットの変更時と新規対局の開始時には
// 置換表をクリアする。プロトコルとの連動はプロトコルのテストで検証し、
// ここではクリア契約だけを検証する。探索を開始した表はクリア後に記録を返さない。
#[test]
fn tt_clear_is_skipped_until_a_search_uses_the_table() {
    // 対局開始直後の`go`が消去の完了を待たされないよう、作成または消去の後に
    // 探索が始まっていない置換表の消去は何もしない。
    let best_move = Move {
        from: sq(0, 0),
        mid: None,
        to: sq(0, 1),
        promote: false,
    };
    let key = 0x4444_4444_0000_0004_u64;
    let mut table = small_tt();
    table.new_search();
    table.store(key, 4, 100, Bound::Exact, Some(best_move), 0, false);
    table.clear();
    assert!(table.probe(key, 0).is_none());
    assert_eq!(table.generation(), 0);

    // 消去後に探索を始めずに書いた内容は、次の消去で消えない。
    table.store(key, 4, 100, Bound::Exact, Some(best_move), 0, false);
    table.clear();
    assert!(table.probe(key, 0).is_some());

    // 探索を始めた後の消去は空にする。
    table.new_search();
    table.clear();
    assert!(table.probe(key, 0).is_none());
}

// サイズはMB単位で外部から設定でき、
// サイズ変更は探索中でないときに限って適用する。探索中の適用は
// `start_search`が置換表の所有権を奪う設計により静的に排除されるため、
// ここではアイドル時のリサイズと以後の探索の正常完了だけを検証し、
// サイズの内部値はassertしない。
#[test]
fn idle_resize_applies_and_later_searches_complete() {
    let midgame = quiet_midgame();
    let moves = legal_moves(&midgame);
    let mut table = TranspositionTable::new(4).unwrap();

    let key = search_key(&midgame);
    table.store(key, 2, 100, Bound::Exact, Some(moves[0]), 0, false);
    assert!(table.probe(key, 0).is_some());
    table.resize(1).unwrap();
    assert!(table.probe(key, 0).is_none());

    let after = run_search(
        &midgame,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(2),
        DEFAULT_THREADS,
        &mut table,
    );
    assert!(moves.contains(&after.best_move));
}

// criticalの予約ビット、バウンド、
// 検証キー、または指し手の復号が不正なら、probeはpanicせず未命中にする。
#[test]
fn malformed_atomic_tt_entries_are_probe_misses() {
    let best_move = Move {
        from: sq(0, 0),
        mid: None,
        to: sq(0, 1),
        promote: false,
    };
    let key = 0x1234_5678_0000_0042;
    let table = small_tt();
    table.new_search();
    table.store(key, 8, 100, Bound::Exact, Some(best_move), 0, false);
    let (critical, advisory) = table.raw_entry(key);

    table.write_raw(key, critical | CRITICAL_RESERVED_MASK, advisory);
    assert!(table.probe(key, 0).is_none());

    // F1の準備で予約領域は59〜63ビットとなり、58ビットはPVの印になる。
    for bit in 59..=63 {
        table.write_raw(key, critical | (1_u64 << bit), advisory);
        assert!(table.probe(key, 0).is_none(), "reserved bit {bit}");
    }
    table.write_raw(key, critical | (1_u64 << 58), advisory);
    assert!(table.probe(key, 0).unwrap().is_pv);

    table.write_raw(key, critical & !CRITICAL_BOUND_MASK, advisory);
    assert!(table.probe(key, 0).is_none());

    let mismatched_key = key ^ (1_u64 << 32);
    table.write_raw(key, critical, advisory);
    assert!(table.probe(mismatched_key, 0).is_none());

    let packed = pack_move(best_move);
    let invalid_moves = [
        (packed & !0xff) | 144,
        (packed & !(0xff << 8)) | (144 << 8),
        (packed & !(0xff << 16)) | (144 << 16),
    ];
    for invalid_move in invalid_moves {
        let invalid_advisory = (advisory & !ADVISORY_MOVE_MASK) | u64::from(invalid_move);
        table.write_raw(key, critical, invalid_advisory);
        assert!(table.probe(key, 0).is_none());
    }
}

// F5では要求深さ7から着手後の値の方向を確かめ、専用の帯は値なしとして扱う。
#[test]
fn zero_window_tt_cutoff_checks_child_from_depth_seven() {
    use crate::search::REPETITION_BAND_START;
    let position = bare_kings();
    let mv = legal_moves(&position)[0];
    for depth in [6, 7] {
        for (child_score, consistent) in [
            (None, true),
            (Some(-100), true),
            (Some(-99), false),
            (Some(0), false),
            (Some(REPETITION_BAND_START), true),
            (Some(-REPETITION_BAND_START), true),
        ] {
            with_root_searcher(&position, &[], |searcher| {
                let mut current = position.clone();
                let undo = current.make_move_unchecked(mv, engine_rules());
                let child_key = search_key(&current);
                current.unmake_move(undo);
                if let Some(score) = child_score {
                    searcher
                        .tt
                        .store(child_key, 8, score, Bound::Exact, None, 1, false);
                }
                searcher.tt.store(
                    search_key(&position),
                    8,
                    150,
                    Bound::Lower,
                    Some(mv),
                    0,
                    false,
                );
                let score = searcher
                    .negamax(&mut current, depth, 99, 100, 0, true)
                    .unwrap();
                assert_eq!(current, position);
                assert_eq!(searcher.path_keys, vec![search_key(&position)]);
                if depth == 6 || consistent {
                    assert_eq!((score, searcher.nodes), (150, 0));
                } else {
                    assert!(
                        searcher.nodes > 0,
                        "深さ7で値の方向が不一致なら子を探索する"
                    );
                }
            });
        }
    }
}

#[test]
fn zero_window_tt_cutoff_skips_child_check_without_legal_move_or_with_mate() {
    let position = bare_kings();
    let legal = legal_moves(&position)[0];
    let illegal = step(sq(5, 5), sq(5, 6));
    for (mv, value) in [
        (None, 150),
        (Some(illegal), 150),
        (Some(legal), MATE_THRESHOLD),
    ] {
        with_root_searcher(&position, &[], |searcher| {
            let mut current = position.clone();
            let undo = current.make_move_unchecked(legal, engine_rules());
            searcher
                .tt
                .store(search_key(&current), 8, 0, Bound::Exact, None, 1, false);
            current.unmake_move(undo);
            searcher
                .tt
                .store(search_key(&position), 8, value, Bound::Lower, mv, 0, false);
            assert_eq!(
                searcher.negamax(&mut current, 7, 99, 100, 0, true),
                Some(value)
            );
            assert_eq!(searcher.nodes, 0);
            assert_eq!(current, position);
        });
    }
}

#[test]
fn zero_window_tt_child_probe_uses_child_ply_and_checks_upper_bound() {
    let position = bare_kings();
    let mv = legal_moves(&position)[0];
    // -MATE_THRESHOLDをply=0で保存すると、ply=1では専用の帯へ入り、値なしとして扱う。
    for (child_score, stored_ply, cutoff) in
        [(-99, 1, true), (-100, 1, false), (-MATE_THRESHOLD, 0, true)]
    {
        with_root_searcher(&position, &[], |searcher| {
            let mut current = position.clone();
            let undo = current.make_move_unchecked(mv, engine_rules());
            searcher.tt.store(
                search_key(&current),
                8,
                child_score,
                Bound::Exact,
                None,
                stored_ply,
                false,
            );
            current.unmake_move(undo);
            searcher.tt.store(
                search_key(&position),
                8,
                50,
                Bound::Upper,
                Some(mv),
                0,
                false,
            );
            let score = searcher
                .negamax(&mut current, 7, 99, 100, 0, false)
                .unwrap();
            assert_eq!(current, position);
            if cutoff {
                assert_eq!((score, searcher.nodes), (50, 0));
            } else {
                assert!(searcher.nodes > 0);
            }
        });
    }
}
