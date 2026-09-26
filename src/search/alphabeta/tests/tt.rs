//! 置換表を検査する。

use super::*;

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
    table.store(key, 4, 321, Bound::Exact, Some(stored_move), 0);
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

// ---------------------------------------------------------------------------
// D7-TT　置換表
// ---------------------------------------------------------------------------

// D7-TT-01。search.md「置換表」節: bit 0–7がfrom、8–15がmid、16–23がto、
// 24がpromote。midなしは0xff、升の符号化は`Square::dense_index()`（0–143）。
// 各fieldの符号境界を独立した既知符号値と照合する。
#[test]
fn packed_moves_round_trip_across_field_boundaries() {
    // search.mdのbit配置から手計算した符号値。相異なる升番号でfield混同を検出する。
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
    table.store(key, 0, 321, Bound::Exact, None, 0);
    let hit = table.probe(key, 0).unwrap();
    assert_eq!(hit.best_move, None);
    assert_eq!(hit.score, 321);
}

// D7-TT-02。search.md「置換表」節: 格納時は`score >= MATE−256`なら
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
        table.store(key, 8, score, Bound::Exact, Some(best_move), ply);
        assert_eq!(table.probe(key, ply).unwrap().score, score);
    }

    // 境界: 29744（詰み帯下限）は変換され、別plyの取り出しで根相対値が
    // ずれて観測される。28999（通常帯上限直下）は変換されない。
    table.store(key, 8, 29_744, Bound::Exact, Some(best_move), 5);
    assert_eq!(table.probe(key, 4).unwrap().score, 29_745);
    table.store(key, 8, -29_744, Bound::Exact, Some(best_move), 5);
    assert_eq!(table.probe(key, 4).unwrap().score, -29_745);
    table.store(key, 8, 28_999, Bound::Exact, Some(best_move), 5);
    assert_eq!(table.probe(key, 4).unwrap().score, 28_999);
}

// D7-TT-03。search.md「置換表」節の2026年8月22日改訂: 同一キーは世代を
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
    table.store(key_a, 8, 100, Bound::Exact, Some(best_move), 0);
    let hit = table.probe(key_a, 0).unwrap();
    assert_eq!(hit.score, 100);
    assert_eq!(hit.depth, 8);

    // (2a) 同一キー・同世代の浅い結果は既存値を保持する。
    table.store(key_a, 2, 222, Bound::Upper, Some(best_move), 0);
    let hit = table.probe(key_a, 0).unwrap();
    assert_eq!(hit.score, 100);
    assert_eq!(hit.depth, 8);
    assert_eq!(hit.bound, Bound::Exact);

    // (2b) 同一キー・同深さは最終書き込み優先とし、バウンド種別を問わず
    //      置換する。`>`への変異を検出する境界でもある。
    table.store(key_a, 8, 333, Bound::Lower, Some(best_move), 0);
    let hit = table.probe(key_a, 0).unwrap();
    assert_eq!(hit.score, 333);
    assert_eq!(hit.depth, 8);
    assert_eq!(hit.bound, Bound::Lower);

    // (2c) 同一キーの深い結果は置換する。
    table.store(key_a, 9, 444, Bound::Upper, Some(best_move), 0);
    let hit = table.probe(key_a, 0).unwrap();
    assert_eq!(hit.score, 444);
    assert_eq!(hit.depth, 9);
    assert_eq!(hit.bound, Bound::Upper);

    // (3) 同一キーは過去世代でも浅い結果を保持し、保持時は世代を更新しない。
    table.clear();
    table.new_search();
    table.store(key_a, 8, 100, Bound::Exact, Some(best_move), 0);
    let (_, advisory_before) = table.raw_entry(key_a);
    table.new_search();
    table.store(key_a, 2, 222, Bound::Upper, Some(best_move), 0);
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
    table.store(key_a, 8, 100, Bound::Exact, Some(best_move), 0);
    table.new_search();
    table.store(key_b, 1, 300, Bound::Exact, Some(best_move), 0);
    assert!(table.probe(key_a, 0).is_none());
    assert_eq!(table.probe(key_b, 0).unwrap().score, 300);

    // (5a) 異キー・同世代: 既存depth=3へdepth=5は置換する。
    table.clear();
    table.new_search();
    table.store(key_a, 3, 100, Bound::Exact, Some(best_move), 0);
    table.store(key_b, 5, 400, Bound::Exact, Some(best_move), 0);
    assert!(table.probe(key_a, 0).is_none());
    assert_eq!(table.probe(key_b, 0).unwrap().score, 400);

    // (5b) 異キー・同世代・同深さ: 既存を保持する（`<`と`<=`の変異検出）。
    table.clear();
    table.new_search();
    table.store(key_a, 5, 100, Bound::Exact, Some(best_move), 0);
    table.store(key_b, 5, 500, Bound::Exact, Some(best_move), 0);
    assert_eq!(table.probe(key_a, 0).unwrap().score, 100);
    assert!(table.probe(key_b, 0).is_none());

    // (5c) 異キー・同世代: 既存depth=7へdepth=5は保持する。
    table.clear();
    table.new_search();
    table.store(key_a, 7, 100, Bound::Exact, Some(best_move), 0);
    table.store(key_b, 5, 600, Bound::Exact, Some(best_move), 0);
    assert_eq!(table.probe(key_a, 0).unwrap().score, 100);
    assert!(table.probe(key_b, 0).is_none());

    // 境界: 世代の周回。既存gen=255・現在gen=0のとき年齢は
    // 0 wrapping_sub 255 = 1で「古い」と判定され置換される。
    let table = small_tt();
    for _ in 0..255 {
        table.new_search();
    }
    table.store(key_a, 8, 100, Bound::Exact, Some(best_move), 0);
    table.new_search();
    table.store(key_b, 1, 700, Bound::Exact, Some(best_move), 0);
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
    contaminated_table.store(search_key(&root), 32, MATE, Bound::Exact, Some(illegal), 0);
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

// D7-TT-04。search.md「置換表」節: 反復で終端した評価値は探索経路と対局
// 履歴に依存するため置換表へ保存しない。別経路への引き分けスコアの伝播を
// 防ぐ。親ノードの格納可否は明文がなく、子の不保存だけをassertする。
#[test]
fn repetition_draw_values_are_not_stored_in_the_table() {
    let root = repetition_fixture();
    let moves = legal_moves(&root);
    let draw_move = repetition_move();
    let mut child = root.clone();
    child.make_move_unchecked(draw_move, engine_rules());
    let child_key = search_key(&child);
    let history = [child_key];

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

    // 反復終端が実際に起きたことの確認（D7-SRCH-03と同じ裁定）。
    assert_eq!(result.best_move, draw_move);
    assert_eq!(result.score, 0);
    // 反復で終端した子局面のキーはprobeしてもエントリが存在しない。
    assert!(table.probe(child_key, 0).is_none());
}

// D7-TT-05。search.md「置換表」節: 規則セットの変更時と新規対局の開始時には
// 置換表をクリアする。wire連動はD6の領域であり、ここではクリア契約だけを
// 検証する。クリア後の探索は空の置換表から正常に再構築される。
#[test]
fn tt_clear_is_skipped_until_a_search_uses_the_table() {
    // 対局開始直後の`go`が消去の完了を待たされないよう、作成または消去の後に
    // 探索が始まっていない置換表の消去は何もしない（time-management-efficiency.md）。
    let best_move = Move {
        from: sq(0, 0),
        mid: None,
        to: sq(0, 1),
        promote: false,
    };
    let key = 0x4444_4444_0000_0004_u64;
    let mut table = small_tt();
    table.new_search();
    table.store(key, 4, 100, Bound::Exact, Some(best_move), 0);
    table.clear();
    assert!(table.probe(key, 0).is_none());
    assert_eq!(table.generation(), 0);

    // 消去後に探索を始めずに書いた内容は、次の消去で消えない。
    table.store(key, 4, 100, Bound::Exact, Some(best_move), 0);
    table.clear();
    assert!(table.probe(key, 0).is_some());

    // 探索を始めた後の消去は空にする。
    table.new_search();
    table.clear();
    assert!(table.probe(key, 0).is_none());
}

#[test]
fn tt_clear_empties_all_entries_and_search_restarts() {
    let best_move = Move {
        from: sq(0, 0),
        mid: None,
        to: sq(0, 1),
        promote: false,
    };
    let keys = [
        0x1111_1111_0000_0001_u64,
        0x2222_2222_0000_0002,
        0x3333_3333_0000_0003,
    ];
    let mut table = small_tt();
    table.new_search();
    for &key in &keys {
        table.store(key, 4, 100, Bound::Exact, Some(best_move), 0);
        assert!(table.probe(key, 0).is_some());
    }

    table.clear();

    // 直前までヒットしていたキーのprobeがすべてミスになる。
    for &key in &keys {
        assert!(table.probe(key, 0).is_none());
    }

    // クリア後の探索が正常に完了する（D7-SRCH-07の空置換表前提と接続）。
    let midgame = quiet_midgame();
    let moves = legal_moves(&midgame);
    let result = run_search(
        &midgame,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(2),
        DEFAULT_THREADS,
        &mut table,
    );
    assert!(moves.contains(&result.best_move));
}

// D7-TT-06。search.md「置換表」節: サイズはMB単位で外部から設定でき、
// サイズ変更は探索中でないときに限って適用する。探索中の適用は
// `start_search`が置換表の所有権を奪う設計により静的に排除されるため、
// ここではアイドル時のリサイズと以後の探索の正常完了だけを検証する
// （SPEC_UNCLEAR-06。サイズの内部値はassertしない）。
#[test]
fn idle_resize_applies_and_later_searches_complete() {
    let midgame = quiet_midgame();
    let moves = legal_moves(&midgame);
    let mut table = TranspositionTable::new(4).unwrap();

    let key = search_key(&midgame);
    table.store(key, 2, 100, Bound::Exact, Some(moves[0]), 0);
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

// D7-TT-08。lazy-smp.md「共有置換表」節: 手、評価値、深さ、boundを
// 格納すると、probeから同じ値が得られる。
#[test]
fn atomic_tt_fields_round_trip_through_probe() {
    let best_move = Move {
        from: sq(3, 4),
        mid: Some(sq(4, 5)),
        to: sq(5, 6),
        promote: true,
    };
    let key = 0x89ab_cdef_0000_0042;
    let table = small_tt();
    table.store(key, 23, -1_234, Bound::Upper, Some(best_move), 0);

    let hit = table.probe(key, 0).unwrap();
    assert_eq!(hit.best_move, Some(best_move));
    assert_eq!(hit.score, -1_234);
    assert_eq!(hit.depth, 23);
    assert_eq!(hit.bound, Bound::Upper);
}

// D7-TT-09。lazy-smp.md「共有置換表」節: criticalの予約ビット、バウンド、
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
    table.store(key, 8, 100, Bound::Exact, Some(best_move), 0);
    let (critical, advisory) = table.raw_entry(key);

    table.write_raw(key, critical | CRITICAL_RESERVED_MASK, advisory);
    assert!(table.probe(key, 0).is_none());

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
