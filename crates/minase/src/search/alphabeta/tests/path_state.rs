//! 探索経路の着手記録と補正の鍵を検査する。

use super::*;
use crate::search::alphabeta::history::{pawn_key, pawn_key_after_move};

fn recorded_move(board: &Position, mv: Move) -> MoveRecord {
    assert!(
        legal_moves(board).contains(&mv),
        "test move must be legal: {mv:?}"
    );
    let mut after = board.clone();
    let undo = after.make_move_unchecked(mv, engine_rules());
    MoveRecord::from_undo(test_nnue(), &undo)
}

fn board_with_kings(mut pieces: Vec<(Square, Color, PieceKind)>) -> Position {
    pieces.extend([
        (sq(0, 0), Color::Black, PieceKind::King),
        (sq(11, 11), Color::White, PieceKind::King),
    ]);
    position(Color::Black, &pieces)
}

#[test]
fn move_record_keeps_move_pre_move_piece_and_capture_kind() {
    let quiet = step(sq(5, 5), sq(5, 6));
    let quiet_board = board_with_kings(vec![(quiet.from, Color::Black, PieceKind::GoldGeneral)]);
    assert_eq!(
        recorded_move(&quiet_board, quiet),
        MoveRecord {
            mv: quiet,
            piece: PieceCode::new(Color::Black, PieceKind::GoldGeneral).unwrap(),
            captured: None,
        }
    );

    let capture = step(sq(4, 4), sq(4, 6));
    let capture_board = board_with_kings(vec![
        (capture.from, Color::Black, PieceKind::Rook),
        (capture.to, Color::White, PieceKind::Pawn),
    ]);
    assert_eq!(
        recorded_move(&capture_board, capture),
        MoveRecord {
            mv: capture,
            piece: PieceCode::new(Color::Black, PieceKind::Rook).unwrap(),
            captured: Some(PieceKind::Pawn),
        }
    );

    let promotion = Move {
        from: sq(4, 7),
        mid: None,
        to: sq(4, 8),
        promote: true,
    };
    let promotion_board = board_with_kings(vec![(promotion.from, Color::Black, PieceKind::Pawn)]);
    assert_eq!(
        recorded_move(&promotion_board, promotion),
        MoveRecord {
            mv: promotion,
            piece: PieceCode::new(Color::Black, PieceKind::Pawn).unwrap(),
            captured: None,
        }
    );
}

#[test]
fn move_record_double_capture_selects_value_then_destination() {
    let mv = Move {
        from: sq(5, 5),
        mid: Some(sq(5, 6)),
        to: sq(5, 7),
        promote: false,
    };
    let more_valuable_at_destination = board_with_kings(vec![
        (mv.from, Color::Black, PieceKind::Lion),
        (mv.mid.unwrap(), Color::White, PieceKind::Pawn),
        (mv.to, Color::White, PieceKind::Rook),
    ]);
    let pawn = more_valuable_at_destination
        .piece_at(mv.mid.unwrap())
        .unwrap();
    let rook = more_valuable_at_destination.piece_at(mv.to).unwrap();
    assert!(test_nnue().piece_value(rook) > test_nnue().piece_value(pawn));
    assert_eq!(
        recorded_move(&more_valuable_at_destination, mv).captured,
        Some(PieceKind::Rook)
    );

    let equal_value = position_with_promoted_pieces(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King, false),
            (mv.from, Color::Black, PieceKind::Lion, false),
            (mv.mid.unwrap(), Color::White, PieceKind::King, false),
            (mv.to, Color::White, PieceKind::CrownPrince, true),
        ],
    );
    assert_eq!(
        test_nnue().piece_value(equal_value.piece_at(mv.mid.unwrap()).unwrap()),
        test_nnue().piece_value(equal_value.piece_at(mv.to).unwrap())
    );
    assert_eq!(
        recorded_move(&equal_value, mv).captured,
        Some(PieceKind::CrownPrince)
    );
}

fn assert_incremental_pawn_key(name: &str, board: &Position, mv: Move) {
    assert!(
        legal_moves(board).contains(&mv),
        "{name}: test move must be legal: {mv:?}"
    );
    let before = pawn_key(board);
    let mut after = board.clone();
    let undo = after.make_move_unchecked(mv, engine_rules());
    assert_eq!(
        pawn_key_after_move(before, &undo),
        pawn_key(&after),
        "{name}: {mv:?}"
    );
}

#[test]
fn pawn_key_incremental_update_covers_moves_promotions_captures_igui_and_jitto() {
    let normal = step(sq(4, 4), sq(4, 5));
    assert_incremental_pawn_key(
        "通常の移動",
        &board_with_kings(vec![
            (normal.from, Color::Black, PieceKind::Rook),
            (sq(7, 7), Color::White, PieceKind::Pawn),
        ]),
        normal,
    );

    for (name, kind) in [
        ("歩兵の成る手", PieceKind::Pawn),
        ("仲人の成る手", PieceKind::GoBetween),
    ] {
        let promotion = Move {
            from: sq(4, 7),
            mid: None,
            to: sq(4, 8),
            promote: true,
        };
        assert_incremental_pawn_key(
            name,
            &board_with_kings(vec![(promotion.from, Color::Black, kind)]),
            promotion,
        );
    }

    let pawn_move = step(sq(4, 4), sq(4, 5));
    assert_incremental_pawn_key(
        "成らない歩兵の移動",
        &board_with_kings(vec![(pawn_move.from, Color::Black, PieceKind::Pawn)]),
        pawn_move,
    );

    let double_capture = Move {
        from: sq(5, 5),
        mid: Some(sq(5, 6)),
        to: sq(5, 7),
        promote: false,
    };
    assert_incremental_pawn_key(
        "獅子の2枚取り",
        &board_with_kings(vec![
            (double_capture.from, Color::Black, PieceKind::Lion),
            (double_capture.mid.unwrap(), Color::White, PieceKind::Pawn),
            (double_capture.to, Color::White, PieceKind::Rook),
        ]),
        double_capture,
    );

    let igui = Move {
        from: sq(5, 5),
        mid: Some(sq(5, 6)),
        to: sq(5, 5),
        promote: false,
    };
    assert_incremental_pawn_key(
        "居喰い",
        &board_with_kings(vec![
            (igui.from, Color::Black, PieceKind::Lion),
            (igui.mid.unwrap(), Color::White, PieceKind::Pawn),
        ]),
        igui,
    );

    let jitto = Move {
        from: sq(5, 5),
        mid: None,
        to: sq(5, 5),
        promote: false,
    };
    assert_incremental_pawn_key(
        "じっと",
        &board_with_kings(vec![
            (jitto.from, Color::Black, PieceKind::Lion),
            (sq(8, 8), Color::White, PieceKind::GoBetween),
        ]),
        jitto,
    );
}

#[test]
fn pawn_key_depends_on_absolute_pawn_placement_but_not_side_to_move() {
    let pieces = [
        (sq(4, 4), Color::Black, PieceKind::Pawn),
        (sq(7, 7), Color::White, PieceKind::GoBetween),
    ];
    let black = position(Color::Black, &pieces);
    let white = position(Color::White, &pieces);
    assert_eq!(pawn_key(&black), pawn_key(&white));

    let moved = position(
        Color::Black,
        &[
            (sq(4, 5), Color::Black, PieceKind::Pawn),
            (sq(7, 7), Color::White, PieceKind::GoBetween),
        ],
    );
    assert_ne!(pawn_key(&black), pawn_key(&moved));

    let mut after_null = black.clone();
    let null_undo = after_null.make_null_move();
    assert_eq!(pawn_key(&after_null), pawn_key(&black));
    after_null.unmake_null_move(null_undo);
    assert_eq!(after_null, black);
}

#[test]
fn searcher_initializes_and_propagates_pawn_keys() {
    let mv = step(sq(4, 4), sq(4, 5));
    let board = board_with_kings(vec![(mv.from, Color::Black, PieceKind::Pawn)]);
    let mut after = board.clone();
    let undo = after.make_move_unchecked(mv, engine_rules());
    let child_key = pawn_key_after_move(pawn_key(&board), &undo);

    with_root_searcher(&board, &[], |searcher| {
        let root_key = pawn_key(&board);
        assert_eq!(searcher.pawn_keys[0], root_key);

        searcher.pawn_keys[1] = !child_key;
        assert!(
            searcher
                .search_move(
                    &mut board.clone(),
                    mv,
                    1,
                    -INFINITY,
                    INFINITY,
                    0,
                    true,
                    0,
                    None,
                    false,
                )
                .is_some()
        );
        assert_eq!(searcher.pawn_keys[1], child_key);

        searcher.pawn_keys[1] = !root_key;
        assert!(
            searcher
                .search_null_move(&mut board.clone(), 0, INFINITY, 0)
                .is_some()
        );
        assert_eq!(searcher.pawn_keys[1], root_key);
    });
}

fn qsearch_path_board(mut pieces: Vec<(Square, Color, PieceKind)>) -> Position {
    pieces.extend([
        (sq(0, 0), Color::Black, PieceKind::King),
        (sq(11, 11), Color::White, PieceKind::King),
        // 対象の捕獲後に停止点へ到達させる、後手の独立した捕獲を置く。
        (sq(8, 6), Color::Black, PieceKind::Pawn),
        (sq(8, 8), Color::White, PieceKind::Rook),
    ]);
    position(Color::Black, &pieces)
}

fn qsearch_path_cases() -> Vec<(&'static str, Position, Move, SearchRepetition)> {
    let igui = Move {
        from: sq(5, 5),
        mid: Some(sq(5, 6)),
        to: sq(5, 5),
        promote: false,
    };
    let double_capture = Move {
        from: sq(5, 5),
        mid: Some(sq(5, 6)),
        to: sq(5, 7),
        promote: false,
    };
    let promoting_capture = Move {
        from: sq(4, 7),
        mid: None,
        to: sq(4, 8),
        promote: true,
    };
    vec![
        (
            "居喰い",
            qsearch_path_board(vec![
                (igui.from, Color::Black, PieceKind::Lion),
                (igui.mid.unwrap(), Color::White, PieceKind::Pawn),
            ]),
            igui,
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
        ),
        (
            "獅子の2枚取り",
            qsearch_path_board(vec![
                (double_capture.from, Color::Black, PieceKind::Lion),
                (double_capture.mid.unwrap(), Color::White, PieceKind::Pawn),
                (double_capture.to, Color::White, PieceKind::Rook),
            ]),
            double_capture,
            SearchRepetition::R2,
        ),
        (
            "成る捕獲",
            qsearch_path_board(vec![
                (promoting_capture.from, Color::Black, PieceKind::Pawn),
                (promoting_capture.to, Color::White, PieceKind::Pawn),
            ]),
            promoting_capture,
            SearchRepetition::R3,
        ),
    ]
}

fn expected_path_state(
    board: &Position,
    mv: Move,
    repetition: SearchRepetition,
) -> PathStateSnapshot {
    let mut child = board.clone();
    let undo = child.make_move_unchecked(mv, engine_rules());
    let correction_keys = correction_piece_keys(&child);
    let path_key = match repetition {
        SearchRepetition::R1 { .. } => search_key(&child),
        SearchRepetition::R2 | SearchRepetition::R3 => repetition_key(repetition.rule(), &child),
    };
    PathStateSnapshot {
        path_key,
        record: Some(MoveRecord::from_undo(test_nnue(), &undo)),
        material_key: material_key(&child),
        pawn_key: pawn_key(&child),
        minor_key: correction_keys.minor,
        non_pawn_key: correction_keys.non_pawn,
    }
}

fn quiescence_path_state(
    board: &Position,
    mv: Move,
    repetition: SearchRepetition,
) -> PathStateSnapshot {
    assert!(legal_moves(board).contains(&mv));
    assert!(board.captured_squares(mv).iter().any(Option::is_some));
    assert!(!capture_is_pruned_by_see(
        board,
        engine_rules(),
        test_nnue(),
        mv
    ));
    let mut snapshot = None;
    with_repetition_root_searcher(
        test_nnue(),
        board,
        engine_rules(),
        repetition,
        &[],
        |searcher| {
            // Upper境界では打ち切らず、検査対象の捕獲を候補の先頭に置く。
            searcher.tt.store(
                search_key(board),
                0,
                INFINITY,
                None,
                Bound::Upper,
                Some(mv),
                0,
                false,
            );
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            let child_capture = step(sq(8, 8), sq(8, 6));
            assert!(legal_moves(&child).contains(&child_capture));
            assert!(!capture_is_pruned_by_see(
                &child,
                engine_rules(),
                searcher.nnue,
                child_capture
            ));

            let path_keys = searcher.path_keys.clone();
            let path_moves_len = searcher.path_moves.len();
            let mut current = board.clone();
            searcher.interrupt_at = Some((1, StopReason::NodeLimit));
            assert_eq!(searcher.quiesce(&mut current, -INFINITY, INFINITY, 0), None);
            assert_eq!(current, *board);
            assert_eq!(searcher.path_keys, path_keys);
            assert_eq!(searcher.path_moves.len(), path_moves_len);
            snapshot = searcher.path_state_at_interrupt;
        },
    );
    snapshot.expect("child capture must reach the deterministic stop point")
}

fn normal_search_path_state(
    board: &Position,
    mv: Move,
    repetition: SearchRepetition,
) -> PathStateSnapshot {
    let mut snapshot = None;
    with_repetition_root_searcher(
        test_nnue(),
        board,
        engine_rules(),
        repetition,
        &[],
        |searcher| {
            let path_keys = searcher.path_keys.clone();
            let path_moves_len = searcher.path_moves.len();
            let mut current = board.clone();
            searcher.interrupt_at = Some((1, StopReason::NodeLimit));
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
                    false,
                ),
                None
            );
            assert_eq!(current, *board);
            assert_eq!(searcher.path_keys, path_keys);
            assert_eq!(searcher.path_moves.len(), path_moves_len);
            snapshot = searcher.path_state_at_interrupt;
        },
    );
    snapshot.expect("child capture must reach the deterministic stop point")
}

#[test]
fn quiescence_updates_keys_and_records_for_igui_double_capture_and_promotion() {
    for (name, board, mv, repetition) in qsearch_path_cases() {
        let expected = expected_path_state(&board, mv, repetition);
        let quiescence = quiescence_path_state(&board, mv, repetition);
        let normal = normal_search_path_state(&board, mv, repetition);
        assert_eq!(quiescence, expected, "{name}");
        assert_eq!(quiescence.record, normal.record, "{name}");
    }
}

#[test]
fn quiescence_restores_path_stacks_after_completion_and_interruption() {
    let completed = board_with_kings(vec![
        (sq(4, 4), Color::Black, PieceKind::Rook),
        (sq(4, 6), Color::White, PieceKind::Pawn),
    ]);
    with_root_searcher(&completed, &[], |searcher| {
        let path_keys = searcher.path_keys.clone();
        let path_moves_len = searcher.path_moves.len();
        let mut current = completed.clone();
        assert!(
            searcher
                .quiesce(&mut current, -INFINITY, INFINITY, 0)
                .is_some()
        );
        assert_eq!(current, completed);
        assert_eq!(searcher.path_keys, path_keys);
        assert_eq!(searcher.path_moves.len(), path_moves_len);
    });

    let (_, board, mv, repetition) = qsearch_path_cases().remove(0);
    let _ = quiescence_path_state(&board, mv, repetition);
}
