//! 静止探索は候補の列から取り出した順番で捕獲手を制限する。
//! 最後の王駒の捕獲と免除の手に加え、獅子の2枚取りを手数制限から外す。
//! 枝刈りした手と免除または例外の手も候補数に含める。採用理由はdocs/adr/limit-searched-captures-in-qsearch.md。

use super::*;
#[cfg(feature = "tuning")]
use crate::search::alphabeta::{
    params, pruning::corrected_static_eval, quiesce::qsearch_post_search_score,
};

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

#[cfg(feature = "tuning")]
fn seed_first(searcher: &Searcher<'_>, board: &Position, mv: Move, ply: u32) {
    assert!(legal_moves(board).contains(&mv));
    // 窓を打ち切らないUpper記録で、対象外の捕獲を先頭に指定する。
    searcher.tt.store(
        search_key(board),
        0,
        INFINITY,
        None,
        Bound::Upper,
        Some(mv),
        ply,
        false,
    );
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

/// 指定した上限より1手多い独立した歩兵の捕獲を用意する。
/// 先頭は置換表で指定し、残りは全て同価値の歩兵とする。
#[cfg(feature = "tuning")]
fn captures_over_limit(limit: usize) -> (Position, Vec<Move>) {
    let count = u8::try_from(limit + 1).unwrap();
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

#[cfg(feature = "tuning")]
fn capture_leaf_score(board: &Position, nnue: &Nnue, mv: Move) -> i32 {
    let mut child = board.clone();
    child.make_move_unchecked(mv, engine_rules());
    -evaluate(nnue, &child)
}

// 手数制限を使うテストは上限を明示し、既定値や他の並列テストに依存させない。
// 他の並列テストへ係数を漏らさないよう、既存のtuningテストと同じく隔離する。
#[cfg(feature = "tuning")]
fn fixed_capture_limit(test: &str, limit: i32) -> bool {
    const CHILD: &str = "MINASE_QSEARCH_LIMIT_TEST";
    if std::env::var(CHILD).as_deref() == Ok(test) {
        params::set("QsearchMoveLimit", limit).unwrap();
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
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout
            .lines()
            .any(|line| line
                .starts_with("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;")),
        "{stdout}"
    );
    false
}

#[cfg(feature = "tuning")]
macro_rules! fixed_capture_limit_test {
    ($name:ident, $limit:literal, $body:block) => {
        #[test]
        fn $name() {
            if fixed_capture_limit(stringify!($name), $limit) {
                $body
            }
        }
    };
}

// 最大ply直前で呼び、探索された各捕獲の子を静的評価の葉にする。
// これにより探索ノード数は、このノードで実際に読んだ捕獲手数となる。
#[cfg(feature = "tuning")]
#[test]
fn qsearch_limit_counts_tt_capture_once_and_prunes_at_limit() {
    if !fixed_capture_limit(
        stringify!(qsearch_limit_counts_tt_capture_once_and_prunes_at_limit),
        2,
    ) {
        return;
    }
    let (board, captures) = captures_over_limit(2);
    with_root_searcher(&board, &[], |searcher| {
        assert_eq!(searcher.previous_move_to[0], None);
        let ply = MAX_PLY - 1;
        set_test_path_ply(searcher, ply);
        let tt_capture = *captures.last().unwrap();
        seed_first(searcher, &board, tt_capture, ply);
        // 同価値の捕獲は同じ段に入り、置換表の手の後は公開されている順序規則に従う。
        let mut ordered = captures.clone();
        ordered.pop();
        ordered.insert(0, tt_capture);
        let searched = &ordered[..2];
        let mut expected = evaluate(searcher.nnue, &board);
        let mut best_move = None;
        for &mv in searched {
            assert!(!see_prunes(
                &board,
                engine_rules(),
                searcher.nnue,
                mv,
                searcher.qsearch_see_floor,
            ));
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            let score = -evaluate(searcher.nnue, &child);
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
        assert_eq!(searcher.nodes, 2);
        #[cfg(feature = "search-stats")]
        assert_eq!(
            searcher.stats.quiesce_move_count_prunes, 1,
            "群の置換表手は重複して候補に数えない"
        );
        assert_eq!(
            searcher.previous_move_to[MAX_PLY as usize],
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
fixed_capture_limit_test!(
    qsearch_limit_counts_last_royal_without_counting_a_search_node,
    1,
    {
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
            set_test_path_ply(searcher, ply);
            seed_first(searcher, &board, first_capture(), ply);
            let mate = ordinary(sq(5, 0), sq(5, 8));
            assert!(legal_moves(&board).contains(&mate));
            assert_eq!(
                searcher.quiesce(&mut board.clone(), -INFINITY, INFINITY, ply),
                Some(MATE - MAX_PLY as i32)
            );
            assert_eq!(
                searcher.nodes, 1,
                "王駒を取る候補は数えるが探索ノードには数えない"
            );
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
);

#[cfg(feature = "tuning")]
fixed_capture_limit_test!(qsearch_limit_reads_double_lion_capture_after_limit, 1, {
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
        set_test_path_ply(searcher, ply);
        // 手数上限の例外だけを検査し、delta pruningの閾値を全二重捕獲より大きくする。
        searcher.delta_margin = INFINITY;
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
                -evaluate(searcher.nnue, &child)
            })
            .max()
            .unwrap();
        // 二重捕獲がαを上げた後も、もう一方をdelta pruningで切らない。
        let stand_pat = evaluate(searcher.nnue, &board);
        let first_score = capture_leaf_score(&board, searcher.nnue, first_capture());
        assert!(expected > stand_pat.max(first_score));
        for &mv in &doubles {
            assert!(!see_prunes(
                &board,
                engine_rules(),
                searcher.nnue,
                mv,
                searcher.qsearch_see_floor,
            ));
            let captured_value: i32 = board
                .captured_squares(mv)
                .into_iter()
                .flatten()
                .map(|square| searcher.nnue.piece_value(board.piece_at(square).unwrap()))
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
});

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
        set_test_path_ply(searcher, ply);
        let qsearch_child_beta = quiescence_parent.then(|| {
            -corrected_static_eval(
                evaluate(searcher.nnue, &board),
                position_correction(searcher, board.side_to_move(), ply),
            )
        });
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
                        0,
                        None,
                        false
                    )
                    .is_some()
            );
        }
        assert_eq!(
            searcher.previous_move_to[(ply + 1) as usize],
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
        let unblended = -evaluate(searcher.nnue, &after);
        let expected =
            qsearch_child_beta.map_or(unblended, |beta| qsearch_post_search_score(unblended, beta));
        assert_eq!(hit.value.unwrap().score, expected);
        assert_eq!(
            searcher.nodes, 3,
            "親の捕獲、子の置換表の捕獲、取り返しを読む"
        );
    });
}

#[cfg(feature = "tuning")]
fixed_capture_limit_test!(
    qsearch_limit_recaptures_mid_capture_after_normal_search,
    1,
    {
        check_lion_recapture(false, false);
    }
);
#[cfg(feature = "tuning")]
fixed_capture_limit_test!(qsearch_limit_recaptures_igui_after_quiescence, 1, {
    check_lion_recapture(true, true);
});
#[cfg(feature = "tuning")]
fixed_capture_limit_test!(qsearch_limit_recaptures_mid_capture_after_quiescence, 1, {
    check_lion_recapture(false, true);
});

#[cfg(feature = "tuning")]
#[test]
fn qsearch_limit_counts_exception_as_first_candidate() {
    if !fixed_capture_limit(
        stringify!(qsearch_limit_counts_exception_as_first_candidate),
        2,
    ) {
        return;
    }
    let (board, captures) = captures_over_limit(2);
    with_root_searcher(&board, &[], |searcher| {
        let ply = MAX_PLY - 1;
        set_test_path_ply(searcher, ply);
        seed_first(searcher, &board, captures[0], ply);
        searcher.previous_move_to[ply as usize] = Some(captures[0].to);
        assert!(
            searcher
                .quiesce(&mut board.clone(), -INFINITY, INFINITY, ply)
                .is_some()
        );
        assert_eq!(searcher.nodes, 2, "先頭が取り返しでも候補の順番に数える");
        #[cfg(feature = "search-stats")]
        assert_eq!(searcher.stats.quiesce_move_count_prunes, 1);
    });
}

#[cfg(feature = "tuning")]
#[test]
fn qsearch_limit_counts_see_pruned_candidate() {
    if !fixed_capture_limit(stringify!(qsearch_limit_counts_see_pruned_candidate), 2) {
        return;
    }
    let (board, captures) = captures_over_limit(2);
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
        set_test_path_ply(searcher, ply);
        searcher.delta_margin = INFINITY;
        seed_first(searcher, &board, captures[0], ply);
        assert!(see_prunes(
            &board,
            engine_rules(),
            searcher.nnue,
            captures[0],
            searcher.qsearch_see_floor,
        ));
        assert!(!see_prunes(
            &board,
            engine_rules(),
            searcher.nnue,
            captures[1],
            searcher.qsearch_see_floor,
        ));
        let expected = capture_leaf_score(&board, searcher.nnue, captures[1])
            .max(evaluate(searcher.nnue, &board));
        assert_eq!(
            searcher.quiesce(&mut board.clone(), -INFINITY, INFINITY, ply),
            Some(expected)
        );
        assert_eq!(searcher.nodes, 1);
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.quiesce_see_floor_prunes, 1);
            assert_eq!(searcher.stats.quiesce_move_count_prunes, 1);
        }
        let best = searcher
            .tt
            .probe(search_key(&board), ply)
            .unwrap()
            .best_move
            .unwrap();
        assert!(captures[1..].contains(&best));
        assert_eq!(capture_leaf_score(&board, searcher.nnue, best), expected);
    });
}

#[cfg(feature = "tuning")]
#[test]
fn qsearch_limit_counts_delta_pruned_candidate_and_uses_its_estimate() {
    if !fixed_capture_limit(
        stringify!(qsearch_limit_counts_delta_pruned_candidate_and_uses_its_estimate),
        2,
    ) {
        return;
    }
    let (board, captures) = captures_over_limit(2);
    let nnue = test_nnue();
    // 香車は歩兵より捕獲価値が高く、隣の筋を守らないためSEEの閾値を通る。
    // エポック0では評価増分が駒価値とdelta_marginの和に収まる。
    // 先頭の歩兵捕獲だけを枝刈りし、残りの捕獲でαが上がらない局面にする。
    let pieces: Vec<_> = Square::all()
        .filter_map(|square| {
            board.piece_at(square).map(|piece| {
                if captures[1..].iter().any(|mv| mv.to == square) {
                    (
                        square,
                        PieceCode::new(Color::White, PieceKind::Lance).unwrap(),
                    )
                } else {
                    (square, piece)
                }
            })
        })
        .collect();
    let board = position_from_codes(Color::Black, &pieces);
    with_nnue_root_searcher(nnue, &board, &[], |searcher| {
        let ply = MAX_PLY - 1;
        set_test_path_ply(searcher, ply);
        zero_position_correction(searcher, &board);
        // 固定したdelta境界で手数の算入を検査する。
        searcher.delta_margin = 500;
        seed_first(searcher, &board, captures[0], ply);
        let stand_pat = evaluate(searcher.nnue, &board);
        let alpha = stand_pat
            + searcher.delta_margin
            + searcher
                .nnue
                .piece_value(board.piece_at(captures[1].to).unwrap())
            - 1;
        for &mv in &captures[1..] {
            let mut child = board.clone();
            child.make_move_unchecked(mv, engine_rules());
            assert!(
                -evaluate(searcher.nnue, &child) <= alpha,
                "捕獲後もdeltaの閾値は変わらない: {mv:?}"
            );
        }
        assert!(!see_prunes(
            &board,
            engine_rules(),
            searcher.nnue,
            captures[1],
            searcher.qsearch_see_floor,
        ));
        let searched_best = capture_leaf_score(&board, searcher.nnue, captures[1]);
        let pruned_estimate = stand_pat
            + searcher.delta_margin
            + searcher
                .nnue
                .piece_value(board.piece_at(captures[0].to).unwrap());
        assert!(pruned_estimate <= alpha);
        assert!(pruned_estimate > searched_best);
        let expected = stand_pat.max(searched_best).max(pruned_estimate);
        assert_eq!(
            searcher.quiesce(&mut board.clone(), alpha, INFINITY, ply),
            Some(expected)
        );
        assert_eq!(searcher.nodes, 1);
        let hit = searcher.tt.probe(search_key(&board), ply).unwrap();
        assert_eq!(hit.value.unwrap().score, expected);
        assert_eq!(
            hit.best_move, None,
            "見積りによる引き上げは最善手を変えない"
        );
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.quiesce_delta_best_score_raises, 1);
            assert_eq!(searcher.stats.quiesce_move_count_prunes, 1);
        }
    });
}

#[test]
fn qsearch_limit_quiet_move_records_previous_destination() {
    let board = two_captures(PieceKind::Pawn);
    with_root_searcher(&board, &[], |searcher| {
        let quiet = ordinary(sq(11, 0), sq(10, 0));
        assert!(legal_moves(&board).contains(&quiet));
        searcher.previous_move_to[1] = Some(sq(5, 3));
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
                    0,
                    None,
                    false
                )
                .is_some()
        );
        assert_eq!(searcher.previous_move_to[1], Some(quiet.to));
    });
}

#[test]
fn qsearch_limit_null_move_clears_previous_destination() {
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
        zero_position_correction(searcher, &after_null);
        let expected = -evaluate(searcher.nnue, &after_null);
        searcher.previous_move_to[1] = Some(sq(5, 3));
        assert_eq!(
            searcher.search_null_move(&mut board.clone(), 0, expected, 0),
            Some(expected)
        );
        assert_eq!(searcher.nodes, 0, "null move直後の静止探索を調べる");
        assert_eq!(searcher.previous_move_to[1], None);
    });
}
