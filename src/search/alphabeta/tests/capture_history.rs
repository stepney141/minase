//! docs/plans/strength-stage12.md「項目3　捕獲履歴」「検証」の契約。

use super::*;
use crate::search::alphabeta::capture_history::CaptureHistory;
use crate::search::alphabeta::params;

fn capture(from: Square, to: Square) -> Move {
    Move {
        from,
        to,
        mid: None,
        promote: false,
    }
}

/// 2枚取りでは価値の大きい駒種を選び、到達升を添字にする。
#[test]
fn capture_history_double_capture_uses_more_valuable_victim() {
    let pst = weights().unwrap();
    for (mid_kind, to_kind) in [
        (PieceKind::Pawn, PieceKind::Rook),
        (PieceKind::Rook, PieceKind::Pawn),
    ] {
        let board = position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(11, 11), Color::White, PieceKind::King),
                (sq(5, 5), Color::Black, PieceKind::Lion),
                (sq(5, 6), Color::White, mid_kind),
                (sq(6, 6), Color::White, to_kind),
            ],
        );
        let mv = Move {
            mid: Some(sq(5, 6)),
            ..capture(sq(5, 5), sq(6, 6))
        };
        assert!(legal_moves(&board).contains(&mv));
        let mut history = CaptureHistory::new();
        history.record_cutoff(&board, &pst, mv, &[], 3);
        for (victim, expected) in [(PieceKind::Pawn, 0), (PieceKind::Rook, 9)] {
            let single = position(
                Color::Black,
                &[
                    (sq(5, 5), Color::Black, PieceKind::Lion),
                    (sq(6, 6), Color::White, victim),
                ],
            );
            assert_eq!(
                history.read(&single, &pst, capture(mv.from, mv.to)),
                expected
            );
        }
        assert_eq!(history.read(&board, &pst, capture(mv.from, sq(5, 6))), 0);
    }
}

/// 同値の2枚取りでは到達升の駒種を選ぶ。王将と太子の価値は等しい。
#[test]
fn capture_history_equal_victims_prefer_destination() {
    let pst = weights().unwrap();
    let board = position_with_promoted_pieces(
        Color::Black,
        &[
            (sq(5, 5), Color::Black, PieceKind::Lion, false),
            (sq(5, 6), Color::White, PieceKind::King, false),
            (sq(6, 6), Color::White, PieceKind::CrownPrince, true),
        ],
    );
    assert_eq!(
        pst.piece_value(board.piece_at(sq(5, 6)).unwrap()),
        pst.piece_value(board.piece_at(sq(6, 6)).unwrap())
    );
    let mv = Move {
        mid: Some(sq(5, 6)),
        ..capture(sq(5, 5), sq(6, 6))
    };
    let mut history = CaptureHistory::new();
    history.record_cutoff(&board, &pst, mv, &[], 2);
    assert_eq!(history.read(&board, &pst, capture(mv.from, mv.to)), 4);
    let king = position(
        Color::Black,
        &[
            (mv.from, Color::Black, PieceKind::Lion),
            (mv.to, Color::White, PieceKind::King),
        ],
    );
    assert_eq!(history.read(&king, &pst, capture(mv.from, mv.to)), 0);
}

/// 成る手は着手前の駒種を使い、成駒と先後を区別する。
#[test]
fn capture_history_promotion_uses_pre_move_piece_and_color() {
    let pst = weights().unwrap();
    let mv = Move {
        promote: true,
        ..capture(sq(5, 7), sq(5, 8))
    };
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (mv.from, Color::Black, PieceKind::Rook),
            (mv.to, Color::White, PieceKind::Pawn),
        ],
    );
    assert!(legal_moves(&board).contains(&mv));
    let mut history = CaptureHistory::new();
    history.record_cutoff(&board, &pst, mv, &[], 4);
    assert_eq!(
        history.read(
            &board,
            &pst,
            Move {
                promote: false,
                ..mv
            }
        ),
        16
    );
    for (color, kind, promoted, expected) in [
        (Color::Black, PieceKind::Rook, false, 16),
        (Color::White, PieceKind::Rook, false, 0),
        (Color::Black, PieceKind::DragonKing, true, 0),
    ] {
        let other = position_with_promoted_pieces(
            color,
            &[
                (mv.from, color, kind, promoted),
                (mv.to, color.opposite(), PieceKind::Pawn, false),
            ],
        );
        assert_eq!(history.read(&other, &pst, mv), expected);
    }
}

fn capture_fixture() -> Position {
    position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Rook),
            (sq(5, 7), Color::White, PieceKind::Pawn),
            (sq(3, 5), Color::White, PieceKind::Pawn),
            (sq(7, 5), Color::White, PieceKind::Pawn),
            (sq(5, 3), Color::White, PieceKind::Pawn),
        ],
    )
}

fn picked(searcher: &Searcher<'_>, board: &Position, tt: Option<Move>) -> Vec<(Move, bool)> {
    let mut picker = MovePicker::new(tt, [None; KILLER_COUNT]);
    std::iter::from_fn(|| {
        picker.next(
            board,
            searcher.pst,
            &searcher.generator,
            searcher.history,
            &searcher.capture_history,
        )
    })
    .collect()
}

/// 非ゼロ履歴は捕獲段だけを変え、TT手を先頭に保ち、合法手の重複を作らない。
#[test]
fn capture_history_changes_picker_but_preserves_root_and_tt_order() {
    let board = capture_fixture();
    with_root_searcher(&board, &[], |searcher| {
        let before = picked(searcher, &board, None);
        let captures: Vec<_> = before
            .iter()
            .filter(|(_, c)| *c)
            .map(|(mv, _)| *mv)
            .collect();
        assert_eq!(captures.len(), 4);
        let winner = captures[3];
        let mut root_before = legal_moves(&board);
        searcher.order_moves(&board, &mut root_before, None, 0);
        searcher
            .capture_history
            .record_cutoff(&board, searcher.pst, winner, &[captures[0]], 256);
        assert_eq!(
            searcher.capture_history.read(&board, searcher.pst, winner),
            params::capture_history_limit()
        );
        assert_eq!(
            searcher
                .capture_history
                .read(&board, searcher.pst, captures[0]),
            -params::capture_history_limit()
        );
        assert_eq!(
            searcher
                .capture_history
                .adjustment(&board, searcher.pst, winner),
            searcher.pst.pawn_value() * params::capture_history_scale() / 100
        );
        assert_eq!(
            searcher
                .capture_history
                .adjustment(&board, searcher.pst, captures[0]),
            -searcher.pst.pawn_value() * params::capture_history_scale() / 100
        );
        let after = picked(searcher, &board, None);
        assert_eq!(after[0], (winner, true));
        assert_eq!(
            after[1..4],
            [
                (captures[1], true),
                (captures[2], true),
                (captures[0], true)
            ]
        );
        assert_eq!(after[4..], before[4..]);
        let mut root_after = legal_moves(&board);
        searcher.order_moves(&board, &mut root_after, None, 0);
        assert_eq!(root_after, root_before);
        assert_eq!(
            picked(searcher, &board, Some(captures[0]))[0],
            (captures[0], true)
        );
        let unique: std::collections::HashSet<_> = after.iter().map(|&(mv, _)| mv).collect();
        assert_eq!(unique.len(), after.len());
        assert_eq!(unique, legal_moves(&board).into_iter().collect());
    });
}

/// 子局面の既知の値で打ち切り位置を固定し、通常探索による加減点を調べる。
#[test]
fn capture_history_negamax_updates_only_searched_captures_on_capture_cutoff() {
    let board = capture_fixture();
    for quiet_cutoff in [false, true] {
        with_root_searcher(&board, &[], |searcher| {
            let moves = picked(searcher, &board, None);
            let winner_index = if quiet_cutoff { 4 } else { 2 };
            for (index, &(mv, _)) in moves.iter().enumerate() {
                let mut child = board.clone();
                child.make_move_unchecked(mv, engine_rules());
                let score = if index == winner_index { 20 } else { 0 };
                searcher
                    .tt
                    .store(search_key(&child), 1, -score, Bound::Exact, None, 1);
            }
            assert_eq!(
                searcher.negamax(&mut board.clone(), 2, -10, 10, 0),
                Some(20)
            );
            assert_eq!(searcher.nodes, winner_index as u64 + 1);
            for (index, &(mv, is_capture)) in moves.iter().enumerate() {
                if is_capture {
                    let expected = if quiet_cutoff || index > winner_index {
                        0
                    } else if index == winner_index {
                        4
                    } else {
                        -4
                    };
                    assert_eq!(
                        searcher.capture_history.read(&board, searcher.pst, mv),
                        expected,
                        "move {index}, quiet={quiet_cutoff}"
                    );
                }
            }
        });
    }
}

/// 新しく作る探索ワーカーの表は、先行するワーカーの更新を引き継がない。
#[test]
fn capture_history_is_zero_for_each_new_searcher() {
    let board = capture_fixture();
    let mv = capture(sq(5, 5), sq(5, 7));
    with_root_searcher(&board, &[], |first| {
        first
            .capture_history
            .record_cutoff(&board, first.pst, mv, &[], 3);
        with_root_searcher(&board, &[], |second| {
            assert_eq!(second.capture_history.read(&board, second.pst, mv), 0);
        });
        assert_eq!(first.capture_history.read(&board, first.pst, mv), 9);
    });
}

/// 歩兵1枚未満の補正は、正負とも0方向へ丸める。
#[test]
fn capture_history_adjustment_truncates_toward_zero() {
    let board = capture_fixture();
    let pst = weights().unwrap();
    let winner = capture(sq(5, 5), sq(5, 7));
    let loser = capture(sq(5, 5), sq(3, 5));
    let mut history = CaptureHistory::new();
    history.record_cutoff(&board, &pst, winner, &[loser], 1);
    assert!(pst.pawn_value() < 20_755);
    assert_eq!(history.adjustment(&board, &pst, winner), 0);
    assert_eq!(history.adjustment(&board, &pst, loser), 0);
}

/// SEEで飛ばした捕獲手と先行する静かなTT手は、捕獲打ち切り時にも減点しない。
#[test]
fn capture_history_excludes_see_pruned_captures_and_quiet_tt_move() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::FreeKing),
            (sq(5, 4), Color::White, PieceKind::Rook),
            (sq(5, 5), Color::White, PieceKind::Pawn),
            (sq(3, 3), Color::Black, PieceKind::Pawn),
            (sq(3, 4), Color::White, PieceKind::Pawn),
        ],
    );
    let quiet = capture(sq(11, 0), sq(10, 0));
    let pruned = capture(sq(5, 3), sq(5, 4));
    let winner = capture(sq(3, 3), sq(3, 4));
    with_root_searcher(&board, &[], |searcher| {
        let moves = picked(searcher, &board, Some(quiet));
        assert_eq!(
            &moves[..3],
            &[(quiet, false), (pruned, true), (winner, true)]
        );
        assert!(!royal_under_attack(&board));
        let margin = crate::search::alphabeta::pruning::see_margin(searcher.pst.pawn_value(), 3);
        assert!(see_prunes(
            &board,
            engine_rules(),
            searcher.pst,
            pruned,
            margin
        ));
        assert!(!see_prunes(
            &board,
            engine_rules(),
            searcher.pst,
            winner,
            margin
        ));
        searcher
            .tt
            .store(search_key(&board), 0, 0, Bound::Upper, Some(quiet), 0);
        for (mv, score) in [(quiet, 0), (winner, 10_001)] {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            searcher
                .tt
                .store(search_key(&child), 2, -score, Bound::Exact, None, 1);
        }
        // このテストでは実着手による打ち切りだけを検査する。
        searcher.null_move_ply = Some(0);
        assert_eq!(
            searcher.negamax(&mut board.clone(), 3, 10_000, 10_001, 0),
            Some(10_001)
        );
        assert_eq!(searcher.nodes, 2);
        assert_eq!(
            searcher.capture_history.read(&board, searcher.pst, pruned),
            0
        );
        assert_eq!(
            searcher.capture_history.read(&board, searcher.pst, winner),
            9
        );
    });
}
