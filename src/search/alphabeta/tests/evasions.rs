//! search-bug-fixes.md「フェーズ3」の発動条件、上限、反復、置換表を検査する。

use super::*;
use crate::search::alphabeta::quiesce::QSEARCH_QUIET_EVASION_LIMIT;
use crate::search::alphabeta::royal::all_royals_under_attack;

/// 遠方の飛車に狙われた王将は、捕獲以外の手だけで脅威を解消できる。
fn escapable_position() -> Position {
    crate::parse_sfen("k11/12/12/12/12/5r6/12/12/12/12/12/5K6 b").unwrap()
}

#[test]
fn quiescence_partial_royal_threat_keeps_stand_pat() {
    // 王将だけ、または太子だけに利きが届く。どちらも安全な王駒が残る。
    for last_rank in ["5K5+E", "5+E5K"] {
        let position =
            crate::parse_sfen(&format!("k11/12/12/12/12/5r6/12/12/12/12/12/{last_rank} b"))
                .unwrap();
        assert!(royal_under_attack(&position));
        assert!(!all_royals_under_attack(&position));
        let stand_pat = evaluate(&weights().unwrap(), &position);
        assert_eq!(
            run_quiesce(&position, stand_pat - 1, stand_pat, 0, &small_tt()),
            (stand_pat, 0)
        );
    }
}

#[test]
fn all_royals_threat_requires_nonempty_royals_and_includes_prince() {
    let no_royals = position(Color::Black, &[(fs(1, 1), Color::White, PieceKind::King)]);
    assert!(!all_royals_under_attack(&no_royals));
    for last_rank in ["5+E6", "5K5+E"] {
        let position = crate::parse_sfen(&format!(
            "k11/12/12/12/12/5r5r/12/12/12/12/12/{last_rank} b"
        ))
        .unwrap();
        assert!(all_royals_under_attack(&position));
    }
}

#[test]
fn quiescence_quiet_escape_avoids_false_mate_and_restores_path() {
    let position = escapable_position();
    assert!(all_royals_under_attack(&position));
    with_root_searcher(&position, &[], |searcher| {
        let path = searcher.path_keys.clone();
        let mut current = position.clone();
        let score = searcher
            .quiesce(&mut current, -INFINITY, INFINITY, 0)
            .unwrap();
        assert!(score.abs() < MATE_THRESHOLD);
        assert!(searcher.nodes > 0, "stand-patではなく逃げる手を読む");
        assert_eq!(searcher.quiet_evasions, 0);
        assert_eq!(searcher.path_keys, path);
        assert_eq!(current, position);
    });
}

#[test]
fn quiescence_attacked_royal_without_legal_moves_loses() {
    // 隅の玉将を自軍の歩兵で囲み、獅子の跳躍で玉将を狙う。
    let position = position(
        Color::White,
        &[
            (fs(1, 12), Color::White, PieceKind::King),
            (fs(1, 11), Color::White, PieceKind::Pawn),
            (fs(2, 11), Color::White, PieceKind::Pawn),
            (fs(2, 12), Color::White, PieceKind::Pawn),
            (fs(3, 12), Color::Black, PieceKind::Lion),
            (fs(8, 5), Color::Black, PieceKind::King),
        ],
    );
    assert!(all_royals_under_attack(&position));
    assert!(legal_moves(&position).is_empty());
    for ply in [0, 7] {
        assert_eq!(
            run_quiesce(&position, -INFINITY, INFINITY, ply, &small_tt()),
            (-MATE + ply as i32, 0)
        );
    }
}

#[test]
fn quiescence_quiet_evasion_limit_restores_stand_pat() {
    let position = escapable_position();
    let stand_pat = evaluate(&weights().unwrap(), &position);
    for count in [QSEARCH_QUIET_EVASION_LIMIT, QSEARCH_QUIET_EVASION_LIMIT + 1] {
        with_root_searcher(&position, &[], |searcher| {
            searcher.quiet_evasions = count;
            assert_eq!(
                searcher.quiesce(&mut position.clone(), -INFINITY, INFINITY, 0),
                Some(stand_pat)
            );
            assert_eq!(searcher.nodes, 0);
            assert_eq!(searcher.quiet_evasions, count);
        });
    }
    // 上限直前ならstand-patのβカットを使わず、少なくとも1手読む。
    with_root_searcher(&position, &[], |searcher| {
        searcher.quiet_evasions = QSEARCH_QUIET_EVASION_LIMIT - 1;
        searcher
            .quiesce(&mut position.clone(), -INFINITY, stand_pat, 0)
            .unwrap();
        assert!(searcher.nodes > 0);
        assert_eq!(searcher.quiet_evasions, QSEARCH_QUIET_EVASION_LIMIT - 1);
    });
}

#[test]
fn quiescence_after_quiet_evasion_neither_probes_nor_stores_tt() {
    // 成りを伴う捕獲があり、通常の静止探索でも置換表まで処理が進む。
    let capture_position = crate::parse_sfen("11k/12/6p5/12/6O5/12/12/12/12/12/12/K11 b").unwrap();
    for position in [capture_position, escapable_position()] {
        for count in 1..=QSEARCH_QUIET_EVASION_LIMIT {
            with_root_searcher(&position, &[], |searcher| {
                searcher.quiet_evasions = count;
                let key = search_key(&position);
                let expected = searcher
                    .quiesce(&mut position.clone(), -INFINITY, INFINITY, 0)
                    .unwrap();
                assert!(searcher.tt.probe(key, 0).is_none(), "格納しない");
                let sentinel = MATE - 10;
                searcher.tt.store(key, 0, sentinel, Bound::Exact, None, 0);
                assert_eq!(
                    searcher.quiesce(&mut position.clone(), -INFINITY, INFINITY, 0),
                    Some(expected),
                    "既存の値を照会して打ち切らない"
                );
                assert_eq!(searcher.tt.probe(key, 0).unwrap().score, sentinel);
                #[cfg(feature = "search-stats")]
                assert_eq!(searcher.stats.quiesce_tt_probes, 0);
            });
        }
    }
}

#[test]
fn quiescence_quiet_evasion_repetition_in_history_or_path_is_draw() {
    let position = escapable_position();
    let moves = legal_moves(&position);
    assert!(
        moves
            .iter()
            .all(|&mv| position.captured_squares(mv) == [None; 2])
    );
    let children = repeated_root_children(&position, &moves);
    // 履歴にないときの駒損と、反復による引き分けを区別する。
    assert!(run_quiesce(&position, -INFINITY, INFINITY, 0, &small_tt()).0 < DRAW_SCORE);
    for from_history in [true, false] {
        let history = if from_history {
            children.as_slice()
        } else {
            &[]
        };
        with_root_searcher(&position, history, |searcher| {
            if !from_history {
                searcher.path_keys.extend_from_slice(&children);
            }
            let path = searcher.path_keys.clone();
            assert_eq!(
                searcher.quiesce(&mut position.clone(), -INFINITY, INFINITY, 0),
                Some(DRAW_SCORE)
            );
            assert_eq!(searcher.nodes, moves.len() as u64);
            assert_eq!(searcher.path_keys, path);
            assert_eq!(searcher.quiet_evasions, 0);
        });
    }
}

#[test]
fn quiescence_quiet_evasion_repetition_respects_null_boundary() {
    let position = escapable_position();
    let children = repeated_root_children(&position, &legal_moves(&position));
    for inside_boundary in [true, false] {
        with_root_searcher(&position, &children, |searcher| {
            if inside_boundary {
                searcher.null_move_boundary = Some(searcher.path_keys.len());
                searcher.path_keys.extend_from_slice(&children);
            } else {
                searcher.path_keys.extend_from_slice(&children);
                searcher.null_move_boundary = Some(searcher.path_keys.len());
                searcher.path_keys.push(search_key(&position));
            }
            let path = searcher.path_keys.clone();
            let boundary = searcher.null_move_boundary;
            let score = searcher
                .quiesce(&mut position.clone(), -INFINITY, INFINITY, 0)
                .unwrap();
            if inside_boundary {
                assert_eq!(score, DRAW_SCORE);
            } else {
                assert!(score < DRAW_SCORE);
            }
            assert_eq!(searcher.path_keys, path);
            assert_eq!(searcher.null_move_boundary, boundary);
            assert_eq!(searcher.quiet_evasions, 0);
        });
    }
}

#[test]
fn quiescence_interrupted_quiet_evasion_restores_state() {
    // 逃げた直後にも別の飛車の捕獲が残り、再帰の内側でノード上限に達する。
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(10, 9), Color::Black, PieceKind::Pawn),
            (fs(6, 3), Color::White, PieceKind::Rook),
            (fs(10, 3), Color::White, PieceKind::Rook),
            (fs(1, 1), Color::White, PieceKind::King),
        ],
    );
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
    let pst = weights().unwrap();
    let table = small_tt();
    let mut searcher = new_searcher(&pst, &position, engine_rules(), &[], &shared, &table);
    // この逃げる手を先頭にして、子の捕獲探索での中断を確実に通す。
    let escape = Move {
        from: fs(6, 12),
        mid: None,
        to: fs(7, 12),
        promote: false,
    };
    assert!(legal_moves(&position).contains(&escape));
    table.store(
        search_key(&position),
        0,
        -INFINITY,
        Bound::Lower,
        Some(escape),
        0,
    );
    let path = searcher.path_keys.clone();
    let mut current = position.clone();
    assert_eq!(searcher.quiesce(&mut current, -INFINITY, INFINITY, 0), None);
    assert_eq!(searcher.nodes, 1);
    assert_eq!(searcher.quiet_evasions, 0);
    assert_eq!(searcher.path_keys, path);
    assert_eq!(current, position);
    assert!(table.probe(search_key(&position), 0).unwrap().best_move == Some(escape));
}

#[test]
fn quiescence_capture_evasions_ignore_repetition_and_pruning() {
    // 玉将の退路は自軍歩兵で塞がれ、残る捕獲手はすべて獅子に玉将を取られる。
    // 捕獲後のキーを履歴へ入れても、捕獲手では反復を判定しない。
    let position = position(
        Color::White,
        &[
            (fs(1, 12), Color::White, PieceKind::King),
            (fs(1, 11), Color::White, PieceKind::Pawn),
            (fs(2, 11), Color::White, PieceKind::Pawn),
            (fs(2, 12), Color::Black, PieceKind::Rook),
            (fs(3, 10), Color::Black, PieceKind::Lion),
            (fs(8, 5), Color::Black, PieceKind::King),
        ],
    );
    let moves = legal_moves(&position);
    assert!(!moves.is_empty());
    assert!(all_royals_under_attack(&position));
    assert!(
        moves
            .iter()
            .all(|&mv| position.captured_squares(mv) != [None; 2])
    );
    let history = repeated_root_children(&position, &moves);
    with_root_searcher(&position, &history, |searcher| {
        // 王駒が狙われている間は、delta pruningの閾値によらず全捕獲を読む。
        searcher.delta_margin = -INFINITY;
        assert_eq!(
            searcher.quiesce(&mut position.clone(), -INFINITY, INFINITY, 0),
            Some(-MATE + 2)
        );
        assert!(searcher.nodes > 0);
        assert_eq!(searcher.quiet_evasions, 0);
    });
}
