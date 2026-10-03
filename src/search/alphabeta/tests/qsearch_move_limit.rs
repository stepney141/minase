//! 設計書strength-stage12.md「項目4」の手数制限と適用除外。

use super::*;

fn ordinary(from: Square, to: Square) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote: false,
    }
}

fn first_capture() -> Move {
    ordinary(sq(0, 0), sq(0, 2))
}

fn seed_first(searcher: &Searcher<'_>, board: &Position, mv: Move, ply: u32) {
    assert!(legal_moves(board).contains(&mv));
    // 窓を打ち切らないUpper記録で、対象外の捕獲を先頭に指定する。
    searcher
        .tt
        .store(search_key(board), 0, INFINITY, Bound::Upper, Some(mv), ply);
}

fn two_captures(victim: PieceKind) -> Position {
    position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(0, 0), Color::Black, PieceKind::Rook),
            (sq(0, 2), Color::White, PieceKind::Pawn),
            (sq(5, 0), Color::Black, PieceKind::Rook),
            (sq(5, 3), Color::White, victim),
        ],
    )
}

// 最大ply直前で呼び、探索された各捕獲の子を静的評価の葉にする。
// これにより探索ノード数は、このノードで実際に読んだ捕獲手数となる。
#[test]
fn qsearch_limit_counts_tt_capture_and_skips_later_ordinary_capture() {
    let board = two_captures(PieceKind::Bishop);
    with_root_searcher(&board, &[], |searcher| {
        assert_eq!(searcher.previous_capture[0], None);
        let ply = MAX_PLY - 1;
        seed_first(searcher, &board, first_capture(), ply);
        let mut child = board.clone();
        child.make_move_unchecked(first_capture(), engine_rules());
        let expected = evaluate(searcher.pst, &board).max(-evaluate(searcher.pst, &child));
        let score = searcher.quiesce(&mut board.clone(), -INFINITY, INFINITY, ply);
        assert_eq!(score, Some(expected));
        assert_eq!(searcher.nodes, 1);
        assert_eq!(
            searcher.previous_capture[MAX_PLY as usize],
            Some(first_capture().to)
        );
        assert_eq!(
            searcher
                .tt
                .probe(search_key(&board), ply)
                .unwrap()
                .best_move,
            Some(first_capture())
        );
    });
}

#[test]
fn qsearch_limit_reads_last_royal_after_tt_capture() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(5, 8), Color::White, PieceKind::King),
            (sq(0, 0), Color::Black, PieceKind::Rook),
            (sq(0, 2), Color::White, PieceKind::Pawn),
            (sq(5, 0), Color::Black, PieceKind::Rook),
        ],
    );
    with_root_searcher(&board, &[], |searcher| {
        let ply = MAX_PLY - 1;
        seed_first(searcher, &board, first_capture(), ply);
        let mate = ordinary(sq(5, 0), sq(5, 8));
        assert!(legal_moves(&board).contains(&mate));
        assert_eq!(
            searcher.quiesce(&mut board.clone(), -INFINITY, INFINITY, ply),
            Some(MATE - MAX_PLY as i32)
        );
        assert_eq!(searcher.nodes, 1, "先頭の捕獲を読んだ後で王駒を取る");
        assert_eq!(
            searcher
                .tt
                .probe(search_key(&board), ply)
                .unwrap()
                .best_move,
            Some(mate)
        );
    });
}

#[test]
fn qsearch_limit_reads_double_lion_capture_after_tt_capture() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(0, 0), Color::Black, PieceKind::Rook),
            (sq(0, 2), Color::White, PieceKind::Pawn),
            (sq(5, 5), Color::Black, PieceKind::Lion),
            (sq(5, 6), Color::White, PieceKind::Pawn),
            (sq(6, 6), Color::White, PieceKind::Pawn),
        ],
    );
    with_root_searcher(&board, &[], |searcher| {
        let ply = MAX_PLY - 1;
        seed_first(searcher, &board, first_capture(), ply);
        let doubles: Vec<_> = legal_moves(&board)
            .into_iter()
            .filter(|&mv| board.captured_squares(mv).iter().all(Option::is_some))
            .collect();
        assert!(!doubles.is_empty());
        let expected = doubles
            .iter()
            .map(|&mv| {
                let mut child = board.clone();
                child.make_move_unchecked(mv, engine_rules());
                -evaluate(searcher.pst, &child)
            })
            .max()
            .unwrap();
        assert_eq!(
            searcher.quiesce(&mut board.clone(), -INFINITY, INFINITY, ply),
            Some(expected)
        );
        assert_eq!(searcher.nodes, 1 + doubles.len() as u64);
        assert!(
            doubles.contains(
                &searcher
                    .tt
                    .probe(search_key(&board), ply)
                    .unwrap()
                    .best_move
                    .unwrap()
            )
        );
    });
}

fn check_lion_recapture(igui: bool, quiescence_parent: bool) {
    // 静止探索は獅子の経由升だけの捕獲を既存仕様で除くため、この経路では角鷹を使う。
    let falcon = !igui && quiescence_parent;
    let destination = if igui {
        sq(5, 5)
    } else if falcon {
        sq(5, 3)
    } else {
        sq(6, 5)
    };
    let rook_from = sq(if igui || falcon { 5 } else { 6 }, 0);
    let victim = if falcon { sq(5, 4) } else { sq(5, 6) };
    let board = position_with_promoted_pieces(
        Color::White,
        &[
            (sq(11, 0), Color::Black, PieceKind::King, false),
            (sq(11, 11), Color::White, PieceKind::King, false),
            (sq(0, 0), Color::Black, PieceKind::Rook, false),
            (sq(0, 2), Color::White, PieceKind::Pawn, false),
            (rook_from, Color::Black, PieceKind::Rook, false),
            (
                sq(5, 5),
                Color::White,
                if falcon {
                    PieceKind::HornedFalcon
                } else {
                    PieceKind::Lion
                },
                falcon,
            ),
            (victim, Color::Black, PieceKind::Rook, false),
        ],
    );
    let capture = Move {
        from: sq(5, 5),
        mid: Some(victim),
        to: destination,
        promote: false,
    };
    assert!(legal_moves(&board).contains(&capture));
    assert!(!board.captured_squares(capture).contains(&Some(destination)));
    let mut child = board.clone();
    child.make_move_unchecked(capture, engine_rules());
    let recapture = ordinary(rook_from, destination);
    assert!(legal_moves(&child).contains(&recapture));
    with_root_searcher(&board, &[], |searcher| {
        let ply = MAX_PLY - 2;
        seed_first(searcher, &child, first_capture(), ply + 1);
        if quiescence_parent {
            seed_first(searcher, &board, capture, ply);
            assert!(
                searcher
                    .quiesce(&mut board.clone(), -INFINITY, INFINITY, ply)
                    .is_some()
            );
        } else {
            assert!(
                searcher
                    .search_move(
                        &mut board.clone(),
                        capture,
                        1,
                        -INFINITY,
                        INFINITY,
                        ply,
                        true,
                        0
                    )
                    .is_some()
            );
        }
        assert_eq!(
            searcher.previous_capture[(ply + 1) as usize],
            Some(destination)
        );
        let hit = searcher.tt.probe(search_key(&child), ply + 1).unwrap();
        assert_eq!(
            hit.best_move,
            Some(recapture),
            "先頭の歩兵捕獲の後でも直前に捕獲した駒を取り返す"
        );
        let mut after = child.clone();
        after.make_move_unchecked(recapture, engine_rules());
        assert_eq!(hit.score, -evaluate(searcher.pst, &after));
        assert_eq!(
            searcher.nodes, 3,
            "親の捕獲、子の置換表の捕獲、取り返しを読む"
        );
    });
}

#[test]
fn qsearch_limit_recaptures_igui_after_normal_search() {
    check_lion_recapture(true, false);
}
#[test]
fn qsearch_limit_recaptures_mid_capture_after_normal_search() {
    check_lion_recapture(false, false);
}
#[test]
fn qsearch_limit_recaptures_igui_after_quiescence() {
    check_lion_recapture(true, true);
}
#[test]
fn qsearch_limit_recaptures_mid_capture_after_quiescence() {
    check_lion_recapture(false, true);
}

#[test]
fn qsearch_limit_counts_exception_as_first_capture() {
    let board = two_captures(PieceKind::Bishop);
    with_root_searcher(&board, &[], |searcher| {
        let ply = MAX_PLY - 1;
        seed_first(searcher, &board, first_capture(), ply);
        searcher.previous_capture[ply as usize] = Some(first_capture().to);
        assert!(
            searcher
                .quiesce(&mut board.clone(), -INFINITY, INFINITY, ply)
                .is_some()
        );
        assert_eq!(searcher.nodes, 1, "先頭が取り返しでも上限の1手に数える");
    });
}

#[test]
fn qsearch_limit_does_not_count_see_pruned_capture() {
    // 先頭の歩兵を守る後手飛車を追加した局面。
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(0, 0), Color::Black, PieceKind::Rook),
            (sq(0, 2), Color::White, PieceKind::Pawn),
            (sq(0, 5), Color::White, PieceKind::Rook),
            (sq(5, 0), Color::Black, PieceKind::Rook),
            (sq(5, 3), Color::White, PieceKind::Pawn),
        ],
    );
    with_root_searcher(&board, &[], |searcher| {
        let ply = MAX_PLY - 1;
        seed_first(searcher, &board, first_capture(), ply);
        assert!(capture_is_pruned_by_see(
            &board,
            engine_rules(),
            searcher.pst,
            first_capture()
        ));
        assert!(
            searcher
                .quiesce(&mut board.clone(), -INFINITY, INFINITY, ply)
                .is_some()
        );
        assert_eq!(searcher.nodes, 1);
        assert_eq!(
            searcher
                .tt
                .probe(search_key(&board), ply)
                .unwrap()
                .best_move,
            Some(ordinary(sq(5, 0), sq(5, 3)))
        );
    });
}

#[test]
fn qsearch_limit_does_not_count_delta_pruned_capture() {
    let board = two_captures(PieceKind::Rook);
    with_root_searcher(&board, &[], |searcher| {
        let ply = MAX_PLY - 1;
        seed_first(searcher, &board, first_capture(), ply);
        let alpha =
            evaluate(searcher.pst, &board) + searcher.delta_margin + searcher.pst.pawn_value();
        assert!(
            searcher
                .quiesce(&mut board.clone(), alpha, INFINITY, ply)
                .is_some()
        );
        assert_eq!(searcher.nodes, 1);
        assert_eq!(
            searcher
                .tt
                .probe(search_key(&board), ply)
                .unwrap()
                .best_move,
            Some(ordinary(sq(5, 0), sq(5, 3)))
        );
    });
}

#[test]
fn qsearch_limit_quiet_move_clears_previous_capture() {
    let board = two_captures(PieceKind::Pawn);
    with_root_searcher(&board, &[], |searcher| {
        let quiet = ordinary(sq(11, 0), sq(10, 0));
        assert!(legal_moves(&board).contains(&quiet));
        searcher.previous_capture[1] = Some(sq(5, 3));
        assert!(
            searcher
                .search_move(
                    &mut board.clone(),
                    quiet,
                    1,
                    -INFINITY,
                    INFINITY,
                    0,
                    true,
                    0
                )
                .is_some()
        );
        assert_eq!(searcher.previous_capture[1], None);
    });
}

#[test]
fn qsearch_limit_null_move_clears_previous_capture() {
    let board = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::GoldGeneral),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let mut after_null = board.clone();
    after_null.make_null_move();
    with_root_searcher(&board, &[], |searcher| {
        let expected = -evaluate(searcher.pst, &after_null);
        searcher.previous_capture[1] = Some(sq(5, 3));
        assert_eq!(
            searcher.negamax(&mut board.clone(), 4, expected - 1, expected, 0),
            Some(expected)
        );
        assert_eq!(searcher.nodes, 0, "null moveで打ち切った直後を調べる");
        assert_eq!(searcher.previous_capture[1], None);
    });
}
