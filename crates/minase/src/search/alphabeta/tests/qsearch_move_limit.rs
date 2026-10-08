//! 設計書strength-stage12.md「項目4」の手数制限と適用除外。

use super::*;
use crate::search::alphabeta::params;

fn ordinary(from: Square, to: Square) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote: false,
    }
}

#[cfg(feature = "tuning")]
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

/// 上限より1手多い独立した歩兵の捕獲を用意する。
/// 先頭は置換表で指定し、残りは全て同価値の歩兵とする。
fn captures_over_limit() -> (Position, Vec<Move>) {
    let count = params::qsearch_move_limit() as u8 + 1;
    assert!(count <= 10, "王駒の筋を避けて捕獲を配置する");
    let mut pieces = vec![
        (sq(11, 0), Color::Black, PieceKind::King),
        (sq(11, 11), Color::White, PieceKind::King),
    ];
    let mut captures = Vec::new();
    for file in 0..count {
        pieces.push((sq(file, 4), Color::Black, PieceKind::Pawn));
        pieces.push((sq(file, 5), Color::White, PieceKind::Pawn));
        captures.push(ordinary(sq(file, 4), sq(file, 5)));
    }
    let board = position(Color::Black, &pieces);
    let actual: Vec<_> = legal_moves(&board)
        .into_iter()
        .filter(|&mv| board.captured_squares(mv).iter().any(Option::is_some))
        .collect();
    assert_eq!(actual.len(), captures.len());
    assert!(captures.iter().all(|mv| actual.contains(mv)));
    (board, captures)
}

fn capture_leaf_score(board: &Position, pst: &Pst, mv: Move) -> i32 {
    let mut child = board.clone();
    child.make_move_unchecked(mv, engine_rules());
    -evaluate(pst, &child)
}

// 複雑な捕獲の例外は上限1を明示し、必ず上限到達後の経路を通す。
// 他の並列テストへ係数を漏らさないよう、既存のtuningテストと同じく隔離する。
#[cfg(feature = "tuning")]
fn single_capture_limit(test: &str) -> bool {
    const CHILD: &str = "MINASE_QSEARCH_LIMIT_TEST";
    if std::env::var(CHILD).as_deref() == Ok(test) {
        params::set("QsearchMoveLimit", 1).unwrap();
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("search::alphabeta::tests::qsearch_move_limit::{test}"),
            "--nocapture",
        ])
        .env(CHILD, test)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

// 最大ply直前で呼び、探索された各捕獲の子を静的評価の葉にする。
// これにより探索ノード数は、このノードで実際に読んだ捕獲手数となる。
#[test]
fn qsearch_limit_counts_tt_capture_and_skips_later_ordinary_capture() {
    let (board, captures) = captures_over_limit();
    with_root_searcher(&board, &[], |searcher| {
        assert_eq!(searcher.previous_capture[0], None);
        let ply = MAX_PLY - 1;
        let tt_capture = *captures.last().unwrap();
        seed_first(searcher, &board, tt_capture, ply);
        // 同価値の捕獲は同じ段に入り、置換表の手の後は公開されている順序規則に従う。
        let mut ordered = captures.clone();
        ordered.pop();
        ordered.insert(0, tt_capture);
        let searched = &ordered[..params::qsearch_move_limit() as usize];
        let mut expected = evaluate(searcher.pst, &board);
        let mut best_move = None;
        for &mv in searched {
            assert!(!capture_is_pruned_by_see(
                &board,
                engine_rules(),
                searcher.pst,
                mv
            ));
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            let score = -evaluate(searcher.pst, &child);
            if score > expected {
                expected = score;
                best_move = Some(mv);
            }
        }
        assert!(best_move.is_some());
        assert_eq!(
            searcher.quiesce(&mut board.clone(), -INFINITY, INFINITY, ply),
            Some(expected)
        );
        assert_eq!(searcher.nodes, params::qsearch_move_limit() as u64);
        assert_eq!(
            searcher.previous_capture[MAX_PLY as usize],
            Some(searched.last().unwrap().to)
        );
        assert_eq!(
            searcher
                .tt
                .probe(search_key(&board), ply)
                .unwrap()
                .best_move,
            best_move
        );
    });
}

#[cfg(feature = "tuning")]
#[test]
fn qsearch_limit_reads_last_royal_after_tt_capture() {
    if !single_capture_limit("qsearch_limit_reads_last_royal_after_tt_capture") {
        return;
    }
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

#[cfg(feature = "tuning")]
#[test]
fn qsearch_limit_reads_double_lion_capture_after_tt_capture() {
    if !single_capture_limit("qsearch_limit_reads_double_lion_capture_after_tt_capture") {
        return;
    }
    // 香車2枚をこの配置に置き、二重捕獲による評価増分を駒価値とdelta_marginの和に収める。
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(0, 0), Color::Black, PieceKind::Rook),
            (sq(0, 2), Color::White, PieceKind::Pawn),
            (sq(3, 2), Color::Black, PieceKind::Lion),
            (sq(3, 3), Color::White, PieceKind::Lance),
            (sq(4, 3), Color::White, PieceKind::Lance),
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
        // 二重捕獲がαを上げた後も、もう一方をdelta pruningで切らない。
        let stand_pat = evaluate(searcher.pst, &board);
        let first_score = capture_leaf_score(&board, searcher.pst, first_capture());
        assert!(expected > stand_pat.max(first_score));
        for &mv in &doubles {
            assert!(!capture_is_pruned_by_see(
                &board,
                engine_rules(),
                searcher.pst,
                mv
            ));
            let captured_value: i32 = board
                .captured_squares(mv)
                .into_iter()
                .flatten()
                .map(|square| searcher.pst.piece_value(board.piece_at(square).unwrap()))
                .sum();
            assert!(stand_pat + captured_value + searcher.delta_margin > expected);
        }
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

#[cfg(feature = "tuning")]
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

#[cfg(feature = "tuning")]
#[test]
fn qsearch_limit_recaptures_mid_capture_after_normal_search() {
    if !single_capture_limit("qsearch_limit_recaptures_mid_capture_after_normal_search") {
        return;
    }
    check_lion_recapture(false, false);
}
#[cfg(feature = "tuning")]
#[test]
fn qsearch_limit_recaptures_igui_after_quiescence() {
    if !single_capture_limit("qsearch_limit_recaptures_igui_after_quiescence") {
        return;
    }
    check_lion_recapture(true, true);
}
#[cfg(feature = "tuning")]
#[test]
fn qsearch_limit_recaptures_mid_capture_after_quiescence() {
    if !single_capture_limit("qsearch_limit_recaptures_mid_capture_after_quiescence") {
        return;
    }
    check_lion_recapture(false, true);
}

#[test]
fn qsearch_limit_counts_exception_as_first_capture() {
    let (board, captures) = captures_over_limit();
    with_root_searcher(&board, &[], |searcher| {
        let ply = MAX_PLY - 1;
        seed_first(searcher, &board, captures[0], ply);
        searcher.previous_capture[ply as usize] = Some(captures[0].to);
        assert!(
            searcher
                .quiesce(&mut board.clone(), -INFINITY, INFINITY, ply)
                .is_some()
        );
        assert_eq!(
            searcher.nodes,
            params::qsearch_move_limit() as u64,
            "先頭が取り返しでも上限に数える"
        );
    });
}

#[test]
fn qsearch_limit_does_not_count_see_pruned_capture() {
    let (board, captures) = captures_over_limit();
    let mut pieces: Vec<_> = Square::all()
        .filter_map(|square| board.piece_at(square).map(|piece| (square, piece)))
        .collect();
    for (square, piece) in &mut pieces {
        if *square == captures[0].from {
            *piece = PieceCode::new(Color::Black, PieceKind::Rook).unwrap();
        }
    }
    pieces.push((
        sq(0, 8),
        PieceCode::new(Color::White, PieceKind::Rook).unwrap(),
    ));
    let board = position_from_codes(Color::Black, &pieces);
    with_root_searcher(&board, &[], |searcher| {
        let ply = MAX_PLY - 1;
        seed_first(searcher, &board, captures[0], ply);
        assert!(capture_is_pruned_by_see(
            &board,
            engine_rules(),
            searcher.pst,
            captures[0]
        ));
        let expected = captures[1..]
            .iter()
            .map(|&mv| {
                assert!(!capture_is_pruned_by_see(
                    &board,
                    engine_rules(),
                    searcher.pst,
                    mv
                ));
                capture_leaf_score(&board, searcher.pst, mv)
            })
            .max()
            .unwrap()
            .max(evaluate(searcher.pst, &board));
        assert_eq!(
            searcher.quiesce(&mut board.clone(), -INFINITY, INFINITY, ply),
            Some(expected)
        );
        assert_eq!(searcher.nodes, params::qsearch_move_limit() as u64);
        let best = searcher
            .tt
            .probe(search_key(&board), ply)
            .unwrap()
            .best_move
            .unwrap();
        assert!(captures[1..].contains(&best));
        assert_eq!(capture_leaf_score(&board, searcher.pst, best), expected);
    });
}

#[test]
fn qsearch_limit_does_not_count_delta_pruned_capture() {
    let (board, captures) = captures_over_limit();
    // 盲虎は歩兵より捕獲価値が高く、この配置では捕獲による評価増分が駒価値とdelta_marginの和に収まる。
    // 先頭の歩兵捕獲だけを枝刈りし、残りの捕獲でαが上がらない局面にする。
    let pieces: Vec<_> = Square::all()
        .filter_map(|square| {
            board.piece_at(square).map(|piece| {
                if captures[1..].iter().any(|mv| mv.to == square) {
                    (
                        square,
                        PieceCode::new(Color::White, PieceKind::BlindTiger).unwrap(),
                    )
                } else {
                    (square, piece)
                }
            })
        })
        .collect();
    let board = position_from_codes(Color::Black, &pieces);
    with_root_searcher(&board, &[], |searcher| {
        let ply = MAX_PLY - 1;
        seed_first(searcher, &board, captures[0], ply);
        let alpha = evaluate(searcher.pst, &board)
            + searcher.delta_margin
            + searcher
                .pst
                .piece_value(board.piece_at(captures[1].to).unwrap())
            - 1;
        for &mv in &captures[1..] {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            assert!(
                -evaluate(searcher.pst, &child) <= alpha,
                "捕獲後もdeltaの閾値は変わらない: {mv:?}"
            );
        }
        let expected = captures[1..]
            .iter()
            .map(|&mv| {
                assert!(!capture_is_pruned_by_see(
                    &board,
                    engine_rules(),
                    searcher.pst,
                    mv
                ));
                capture_leaf_score(&board, searcher.pst, mv)
            })
            .max()
            .unwrap()
            .max(evaluate(searcher.pst, &board));
        assert_eq!(
            searcher.quiesce(&mut board.clone(), alpha, INFINITY, ply),
            Some(expected)
        );
        assert_eq!(searcher.nodes, params::qsearch_move_limit() as u64);
        let best = searcher
            .tt
            .probe(search_key(&board), ply)
            .unwrap()
            .best_move
            .unwrap();
        assert!(captures[1..].contains(&best));
        assert_eq!(capture_leaf_score(&board, searcher.pst, best), expected);
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
