//! 静止探索の免除と、免除に使う王手判定の契約を検査する。

use super::*;
#[cfg(feature = "tuning")]
use crate::search::alphabeta::params;
use minase_core::test_util::{bench_positions, sampled_random_positions};

fn capture(from: Square, to: Square, promote: bool) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote,
    }
}

fn single_rook_capture(white_king: Square, defended: bool) -> (Position, Move) {
    let mut pieces = vec![
        (sq(0, 0), Color::Black, PieceKind::King),
        (sq(5, 2), Color::Black, PieceKind::Rook),
        (sq(5, 4), Color::White, PieceKind::Pawn),
        (white_king, Color::White, PieceKind::King),
    ];
    if defended {
        pieces.push((sq(0, 4), Color::White, PieceKind::Rook));
    }
    (
        position(Color::Black, &pieces),
        capture(sq(5, 2), sq(5, 4), false),
    )
}

fn promoting_rook_capture(white_king: Square, defended: bool) -> (Position, Move) {
    let mut pieces = vec![
        (sq(0, 0), Color::Black, PieceKind::King),
        (sq(5, 7), Color::Black, PieceKind::Rook),
        (sq(5, 8), Color::White, PieceKind::Pawn),
        (white_king, Color::White, PieceKind::King),
    ];
    if defended {
        pieces.push((sq(0, 8), Color::White, PieceKind::Rook));
    }
    (
        position(Color::Black, &pieces),
        capture(sq(5, 7), sq(5, 8), true),
    )
}

fn qsearch_at_leaf(
    searcher: &mut Searcher<'_>,
    board: &Position,
    alpha: i32,
    previous_move_to: Option<Square>,
) -> Option<i32> {
    let ply = MAX_PLY - 1;
    set_test_path_ply(searcher, ply);
    searcher.previous_move_to[ply as usize] = previous_move_to;
    searcher.quiesce(&mut board.clone(), alpha, INFINITY, ply)
}

fn generated_moves(generator: &MoveGenerator, position: &Position) -> Vec<Move> {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    moves
}

/// 着手後の局面で、着手側に相手の残存王駒をすべて取る合法手があるかを調べる。
fn gives_check_reference(position: &Position, generator: &MoveGenerator, mv: Move) -> bool {
    let mover = position.side_to_move();
    let opponent = mover.opposite();
    let mut after = position.clone();
    after.make_move_unchecked(mv, generator.rules());
    let remaining_royals = after.royal_pieces(opponent);
    if remaining_royals.is_empty() {
        return false;
    }

    // null moveは盤面と成り権を変えず、着手側へ手番を戻す。
    // 直前の獅子捕獲の制約は相手側だけに適用されるため、この列挙では消えてよい。
    let mut checking_position = after;
    checking_position.make_null_move();
    generated_moves(generator, &checking_position)
        .into_iter()
        .any(|candidate| {
            let captured = checking_position.captured_squares(candidate);
            remaining_royals
                .into_iter()
                .all(|royal| captured.contains(&Some(royal)))
        })
}

fn falcon_or_eagle_can_capture_two_royals_after(
    position: &Position,
    generator: &MoveGenerator,
    mv: Move,
) -> bool {
    let mover = position.side_to_move();
    let opponent = mover.opposite();
    let mut after = position.clone();
    after.make_move_unchecked(mv, generator.rules());
    let royals = after.royal_pieces(opponent);
    if royals.popcount() != 2 {
        return false;
    }

    let mut checking_position = after;
    checking_position.make_null_move();
    generated_moves(generator, &checking_position)
        .into_iter()
        .any(|candidate| {
            let kind = checking_position
                .piece_at(candidate.from)
                .and_then(PieceCode::kind);
            let captured = checking_position.captured_squares(candidate);
            matches!(
                kind,
                Some(PieceKind::HornedFalcon | PieceKind::SoaringEagle)
            ) && royals
                .into_iter()
                .all(|royal| captured.contains(&Some(royal)))
        })
}

fn gives_check_edge_positions() -> Vec<Position> {
    vec![
        // 居喰いと獅子の2枚取り。
        position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(5, 5), Color::Black, PieceKind::Lion),
                (sq(5, 6), Color::White, PieceKind::Pawn),
                (sq(6, 6), Color::White, PieceKind::Pawn),
                (sq(11, 11), Color::White, PieceKind::King),
            ],
        ),
        // 角鷹の2段階移動。
        position_with_promoted_pieces(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King, false),
                (sq(5, 4), Color::Black, PieceKind::HornedFalcon, true),
                (sq(5, 5), Color::White, PieceKind::Pawn, false),
                (sq(5, 6), Color::White, PieceKind::Pawn, false),
                (sq(11, 11), Color::White, PieceKind::King, false),
            ],
        ),
        // 飛鷲の2段階移動。
        position_with_promoted_pieces(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King, false),
                (sq(5, 4), Color::Black, PieceKind::SoaringEagle, true),
                (sq(4, 5), Color::White, PieceKind::Pawn, false),
                (sq(3, 6), Color::White, PieceKind::Pawn, false),
                (sq(11, 11), Color::White, PieceKind::King, false),
            ],
        ),
        // 捕獲と同時に成れる麒麟。
        position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(5, 7), Color::Black, PieceKind::Kirin),
                (sq(5, 9), Color::White, PieceKind::Pawn),
                (sq(11, 11), Color::White, PieceKind::King),
            ],
        ),
        // 歩兵を取った角鷹が、残る王将と太子を次の2段階移動で取れる。
        position_with_promoted_pieces(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King, false),
                (sq(5, 4), Color::Black, PieceKind::HornedFalcon, true),
                (sq(4, 4), Color::White, PieceKind::Pawn, false),
                (sq(4, 5), Color::White, PieceKind::King, false),
                (sq(4, 6), Color::White, PieceKind::CrownPrince, true),
            ],
        ),
    ]
}

#[test]
fn qsearch_gives_check_matches_post_move_reference_for_sampled_captures() {
    let rules = engine_rules();
    let generator = MoveGenerator::new(rules);
    let positions = bench_positions()
        .into_iter()
        .chain(sampled_random_positions(rules))
        .chain(gives_check_edge_positions());
    let mut compared = 0_usize;
    let mut igui = 0_usize;
    let mut lion_two_stage = 0_usize;
    let mut falcon_two_stage = 0_usize;
    let mut eagle_two_stage = 0_usize;
    let mut promotions = 0_usize;
    let mut two_royal_falcon_or_eagle_checks = 0_usize;

    for position in positions {
        for mv in generated_moves(&generator, &position)
            .into_iter()
            .filter(|&mv| position.captured_squares(mv).iter().any(Option::is_some))
        {
            let before = position.clone();
            let expected = gives_check_reference(&position, &generator, mv);
            let actual = position.gives_check(&generator, mv);
            assert_eq!(actual, expected, "mv={mv:?}");
            assert_eq!(position, before, "gives_check must not change the position");

            let kind = position.piece_at(mv.from).and_then(PieceCode::kind);
            igui += usize::from(mv.from == mv.to);
            lion_two_stage += usize::from(mv.mid.is_some() && kind == Some(PieceKind::Lion));
            falcon_two_stage +=
                usize::from(mv.mid.is_some() && kind == Some(PieceKind::HornedFalcon));
            eagle_two_stage +=
                usize::from(mv.mid.is_some() && kind == Some(PieceKind::SoaringEagle));
            promotions += usize::from(mv.promote);
            two_royal_falcon_or_eagle_checks += usize::from(
                actual && falcon_or_eagle_can_capture_two_royals_after(&position, &generator, mv),
            );
            compared += 1;
        }
    }

    assert!(compared > 0);
    assert!(igui > 0, "居喰いを照合する");
    assert!(lion_two_stage > 0, "獅子の2段階移動を照合する");
    assert!(falcon_two_stage > 0, "角鷹の2段階移動を照合する");
    assert!(eagle_two_stage > 0, "飛鷲の2段階移動を照合する");
    assert!(promotions > 0, "成る捕獲手を照合する");
    assert!(
        two_royal_falcon_or_eagle_checks > 0,
        "角鷹または飛鷲が王駒2枚を取れる王手を照合する"
    );
}

#[test]
fn qsearch_delta_pruning_exempts_recapture_promotion_and_check() {
    let (recapture_board, recapture) = single_rook_capture(sq(11, 11), false);
    with_root_searcher(&recapture_board, &[], |searcher| {
        assert!(
            qsearch_at_leaf(searcher, &recapture_board, INFINITY - 1, Some(recapture.to),)
                .is_some()
        );
        assert_eq!(searcher.nodes, 1);
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.quiesce_recapture_exemptions, 1);
            assert_eq!(searcher.stats.quiesce_gives_check_calls, 0);
        }
    });

    let promotion_board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 7), Color::Black, PieceKind::Kirin),
            (sq(5, 9), Color::White, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    let promotion = capture(sq(5, 7), sq(5, 9), true);
    assert!(legal_moves(&promotion_board).contains(&promotion));
    with_root_searcher(&promotion_board, &[], |searcher| {
        assert!(qsearch_at_leaf(searcher, &promotion_board, INFINITY - 1, None).is_some());
        assert_eq!(searcher.nodes, 1, "成る捕獲だけをdelta pruningから外す");
        #[cfg(feature = "search-stats")]
        assert_eq!(searcher.stats.quiesce_promotion_exemptions, 1);
    });

    let (checking_board, checking) = single_rook_capture(sq(5, 8), false);
    let generator = MoveGenerator::new(engine_rules());
    assert!(checking_board.gives_check(&generator, checking));
    with_root_searcher(&checking_board, &[], |searcher| {
        assert!(qsearch_at_leaf(searcher, &checking_board, INFINITY - 1, None).is_some());
        assert_eq!(searcher.nodes, 1);
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.quiesce_check_exemptions, 1);
            assert_eq!(searcher.stats.quiesce_gives_check_calls, 1);
        }
    });
}

#[test]
fn qsearch_see_threshold_exempts_recapture_promotion_and_check() {
    let (recapture_board, recapture) = single_rook_capture(sq(11, 11), true);
    with_root_searcher(&recapture_board, &[], |searcher| {
        zero_position_correction(searcher, &recapture_board);
        searcher.delta_margin = 0;
        searcher.qsearch_see_floor = -INFINITY;
        assert!(see_prunes(
            &recapture_board,
            engine_rules(),
            searcher.nnue,
            recapture,
            0,
        ));
        let alpha = evaluate(searcher.nnue, &recapture_board);
        assert!(qsearch_at_leaf(searcher, &recapture_board, alpha, Some(recapture.to)).is_some());
        assert_eq!(searcher.nodes, 1);
        #[cfg(feature = "search-stats")]
        assert_eq!(searcher.stats.quiesce_recapture_exemptions, 1);
    });

    let (promotion_board, promotion) = promoting_rook_capture(sq(11, 11), true);
    assert!(legal_moves(&promotion_board).contains(&promotion));
    with_root_searcher(&promotion_board, &[], |searcher| {
        zero_position_correction(searcher, &promotion_board);
        searcher.delta_margin = 0;
        searcher.qsearch_see_floor = -INFINITY;
        assert!(see_prunes(
            &promotion_board,
            engine_rules(),
            searcher.nnue,
            promotion,
            0,
        ));
        let alpha = evaluate(searcher.nnue, &promotion_board);
        assert!(qsearch_at_leaf(searcher, &promotion_board, alpha, None).is_some());
        assert_eq!(searcher.nodes, 1);
        #[cfg(feature = "search-stats")]
        assert_eq!(searcher.stats.quiesce_promotion_exemptions, 1);
    });

    let (checking_board, checking) = single_rook_capture(sq(5, 8), true);
    let generator = MoveGenerator::new(engine_rules());
    assert!(checking_board.gives_check(&generator, checking));
    with_root_searcher(&checking_board, &[], |searcher| {
        zero_position_correction(searcher, &checking_board);
        searcher.delta_margin = 0;
        searcher.qsearch_see_floor = -INFINITY;
        assert!(see_prunes(
            &checking_board,
            engine_rules(),
            searcher.nnue,
            checking,
            0,
        ));
        let alpha = evaluate(searcher.nnue, &checking_board);
        assert!(qsearch_at_leaf(searcher, &checking_board, alpha, None).is_some());
        assert_eq!(searcher.nodes, 1);
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.quiesce_check_exemptions, 1);
            assert_eq!(searcher.stats.quiesce_gives_check_calls, 1);
        }
    });
}

#[test]
fn qsearch_see_floor_still_prunes_each_exemption_kind() {
    let (recapture_board, recapture) = single_rook_capture(sq(11, 11), true);
    with_root_searcher(&recapture_board, &[], |searcher| {
        zero_position_correction(searcher, &recapture_board);
        searcher.delta_margin = 0;
        searcher.qsearch_see_floor = 0;
        let alpha = evaluate(searcher.nnue, &recapture_board);
        assert!(qsearch_at_leaf(searcher, &recapture_board, alpha, Some(recapture.to)).is_some());
        assert_eq!(searcher.nodes, 0);
        #[cfg(feature = "search-stats")]
        assert_eq!(searcher.stats.quiesce_see_floor_prunes, 1);
    });

    let (promotion_board, promotion) = promoting_rook_capture(sq(11, 11), true);
    assert!(legal_moves(&promotion_board).contains(&promotion));
    with_root_searcher(&promotion_board, &[], |searcher| {
        zero_position_correction(searcher, &promotion_board);
        searcher.delta_margin = 0;
        searcher.qsearch_see_floor = 0;
        let alpha = evaluate(searcher.nnue, &promotion_board);
        assert!(qsearch_at_leaf(searcher, &promotion_board, alpha, None).is_some());
        assert_eq!(searcher.nodes, 0);
        #[cfg(feature = "search-stats")]
        assert_eq!(searcher.stats.quiesce_see_floor_prunes, 1);
    });

    let (checking_board, checking) = single_rook_capture(sq(5, 8), true);
    let generator = MoveGenerator::new(engine_rules());
    assert!(checking_board.gives_check(&generator, checking));
    with_root_searcher(&checking_board, &[], |searcher| {
        zero_position_correction(searcher, &checking_board);
        searcher.delta_margin = 0;
        searcher.qsearch_see_floor = 0;
        let alpha = evaluate(searcher.nnue, &checking_board);
        assert!(qsearch_at_leaf(searcher, &checking_board, alpha, None).is_some());
        assert_eq!(searcher.nodes, 0);
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.quiesce_check_exemptions, 1);
            assert_eq!(searcher.stats.quiesce_see_floor_prunes, 1);
        }
    });
}

#[test]
fn qsearch_exemption_stats_use_recapture_then_promotion_then_check_priority() {
    let (board, promotion) = promoting_rook_capture(sq(5, 11), false);
    let generator = MoveGenerator::new(engine_rules());
    assert!(board.gives_check(&generator, promotion));

    with_root_searcher(&board, &[], |searcher| {
        assert!(qsearch_at_leaf(searcher, &board, INFINITY - 1, None).is_some());
        assert_eq!(searcher.nodes, 2);
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.quiesce_recapture_exemptions, 0);
            assert_eq!(searcher.stats.quiesce_promotion_exemptions, 1);
            assert_eq!(searcher.stats.quiesce_check_exemptions, 1);
            assert_eq!(searcher.stats.quiesce_gives_check_calls, 1);
        }
    });

    with_root_searcher(&board, &[], |searcher| {
        assert!(qsearch_at_leaf(searcher, &board, INFINITY - 1, Some(promotion.to)).is_some());
        assert_eq!(searcher.nodes, 2);
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.quiesce_recapture_exemptions, 2);
            assert_eq!(searcher.stats.quiesce_promotion_exemptions, 0);
            assert_eq!(searcher.stats.quiesce_check_exemptions, 0);
            assert_eq!(searcher.stats.quiesce_gives_check_calls, 0);
        }
    });
}

#[test]
fn qsearch_does_not_judge_check_without_a_pruning_condition() {
    let (board, mv) = single_rook_capture(sq(11, 11), false);
    let generator = MoveGenerator::new(engine_rules());
    assert!(!board.gives_check(&generator, mv));
    with_root_searcher(&board, &[], |searcher| {
        assert!(qsearch_at_leaf(searcher, &board, -INFINITY, None).is_some());
        assert_eq!(searcher.nodes, 1);
        #[cfg(feature = "search-stats")]
        assert_eq!(searcher.stats.quiesce_gives_check_calls, 0);
    });
}

#[cfg(feature = "tuning")]
fn seed_first_capture(searcher: &Searcher<'_>, board: &Position, mv: Move, ply: u32) {
    assert!(legal_moves(board).contains(&mv));
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

#[cfg(feature = "tuning")]
fn fixed_move_limit(test: &str, limit: i32) -> bool {
    const CHILD: &str = "MINASE_QSEARCH_EXEMPTION_LIMIT_TEST";
    if std::env::var(CHILD).as_deref() == Ok(test) {
        params::set("QsearchMoveLimit", limit).unwrap();
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("search::alphabeta::tests::qsearch_exemptions::{test}"),
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
#[test]
fn qsearch_move_count_exempts_non_capture_recapture_promotion_and_check() {
    if !fixed_move_limit(
        stringify!(qsearch_move_count_exempts_non_capture_recapture_promotion_and_check),
        1,
    ) {
        return;
    }

    let parent = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 6), Color::Black, PieceKind::Pawn),
            (sq(5, 4), Color::Black, PieceKind::Pawn),
            (sq(0, 8), Color::White, PieceKind::Rook),
            (sq(5, 8), Color::White, PieceKind::Rook),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    let quiet = capture(sq(5, 4), sq(5, 5), false);
    let first = capture(sq(0, 8), sq(0, 6), false);
    let recapture = capture(sq(5, 8), sq(5, 5), false);
    assert!(parent.captured_squares(quiet).iter().all(Option::is_none));
    let mut child = parent.clone();
    child.make_move_unchecked(quiet, engine_rules());
    let child_captures: Vec<_> = legal_moves(&child)
        .into_iter()
        .filter(|&mv| child.captured_squares(mv).iter().any(Option::is_some))
        .collect();
    assert_eq!(child_captures.len(), 2);
    assert!(child_captures.contains(&first));
    assert!(child_captures.contains(&recapture));
    with_root_searcher(&parent, &[], |searcher| {
        let ply = MAX_PLY - 2;
        set_test_path_ply(searcher, ply);
        seed_first_capture(searcher, &child, first, ply + 1);
        assert!(
            searcher
                .search_move(
                    &mut parent.clone(),
                    quiet,
                    1,
                    -INFINITY,
                    INFINITY,
                    ply,
                    true,
                    0,
                    None,
                    false,
                )
                .is_some()
        );
        assert_eq!(
            searcher.previous_move_to[(ply + 1) as usize],
            Some(quiet.to)
        );
        assert_eq!(searcher.nodes, 3, "非捕獲手の後の取り返しまで読む");
        #[cfg(feature = "search-stats")]
        assert_eq!(searcher.stats.quiesce_recapture_exemptions, 1);
    });

    let promotion_board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 0), Color::Black, PieceKind::Rook),
            (sq(0, 2), Color::White, PieceKind::Pawn),
            (sq(5, 7), Color::Black, PieceKind::Kirin),
            (sq(5, 9), Color::White, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    let first = capture(sq(0, 0), sq(0, 2), false);
    let promotion = capture(sq(5, 7), sq(5, 9), true);
    assert!(legal_moves(&promotion_board).contains(&promotion));
    with_root_searcher(&promotion_board, &[], |searcher| {
        let ply = MAX_PLY - 1;
        set_test_path_ply(searcher, ply);
        seed_first_capture(searcher, &promotion_board, first, ply);
        assert!(
            searcher
                .quiesce(&mut promotion_board.clone(), -INFINITY, INFINITY, ply)
                .is_some()
        );
        assert_eq!(searcher.nodes, 2, "上限後も成る捕獲を読む");
        #[cfg(feature = "search-stats")]
        assert_eq!(searcher.stats.quiesce_promotion_exemptions, 1);
    });

    let checking_board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 0), Color::Black, PieceKind::Rook),
            (sq(0, 2), Color::White, PieceKind::Pawn),
            (sq(5, 0), Color::Black, PieceKind::Rook),
            (sq(5, 3), Color::White, PieceKind::Pawn),
            (sq(5, 8), Color::White, PieceKind::King),
        ],
    );
    let first = capture(sq(0, 0), sq(0, 2), false);
    let checking = capture(sq(5, 0), sq(5, 3), false);
    assert!(checking_board.gives_check(&MoveGenerator::new(engine_rules()), checking));
    with_root_searcher(&checking_board, &[], |searcher| {
        let ply = MAX_PLY - 1;
        set_test_path_ply(searcher, ply);
        seed_first_capture(searcher, &checking_board, first, ply);
        assert!(
            searcher
                .quiesce(&mut checking_board.clone(), -INFINITY, INFINITY, ply)
                .is_some()
        );
        assert_eq!(searcher.nodes, 2, "上限後も王手になる捕獲を読む");
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.quiesce_check_exemptions, 1);
            assert_eq!(searcher.stats.quiesce_gives_check_calls, 1);
        }
    });
}

#[cfg(feature = "tuning")]
#[test]
fn qsearch_double_lion_only_avoids_move_count_and_delta_pruning_still_applies() {
    if !fixed_move_limit(
        stringify!(qsearch_double_lion_only_avoids_move_count_and_delta_pruning_still_applies),
        1,
    ) {
        return;
    }
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
    let first = capture(sq(0, 0), sq(0, 2), false);
    let doubles: Vec<_> = legal_moves(&board)
        .into_iter()
        .filter(|&mv| {
            board.piece_at(mv.from).and_then(PieceCode::kind) == Some(PieceKind::Lion)
                && board.captured_squares(mv).iter().all(Option::is_some)
        })
        .collect();
    assert!(!doubles.is_empty());
    assert!(
        doubles
            .iter()
            .all(|&mv| !board.gives_check(&MoveGenerator::new(engine_rules()), mv))
    );
    with_root_searcher(&board, &[], |searcher| {
        let ply = MAX_PLY - 1;
        set_test_path_ply(searcher, ply);
        zero_position_correction(searcher, &board);
        searcher.delta_margin = 0;
        searcher.qsearch_see_floor = -INFINITY;
        seed_first_capture(searcher, &board, first, ply);
        assert!(
            searcher
                .quiesce(&mut board.clone(), INFINITY - 1, INFINITY, ply)
                .is_some()
        );
        assert_eq!(
            searcher.nodes, 0,
            "獅子の2枚取りもdelta pruningでは読まない"
        );
        #[cfg(feature = "search-stats")]
        assert_eq!(
            searcher.stats.quiesce_lion_double_move_count_exemptions,
            doubles.len() as u64
        );
    });
}

#[cfg(feature = "tuning")]
#[test]
fn qsearch_falcon_and_eagle_double_captures_do_not_avoid_move_count() {
    if !fixed_move_limit(
        stringify!(qsearch_falcon_and_eagle_double_captures_do_not_avoid_move_count),
        1,
    ) {
        return;
    }

    let fixtures = [
        (PieceKind::HornedFalcon, sq(5, 2), sq(5, 3), sq(5, 4)),
        (PieceKind::SoaringEagle, sq(5, 2), sq(4, 3), sq(3, 4)),
    ];
    for (kind, from, mid, to) in fixtures {
        let board = position_with_promoted_pieces(
            Color::Black,
            &[
                (sq(11, 0), Color::Black, PieceKind::King, false),
                (sq(11, 11), Color::White, PieceKind::King, false),
                (sq(0, 0), Color::Black, PieceKind::Rook, false),
                (sq(0, 2), Color::White, PieceKind::Pawn, false),
                (from, Color::Black, kind, true),
                (mid, Color::White, PieceKind::Lance, false),
                (to, Color::White, PieceKind::Lance, false),
            ],
        );
        let first = capture(sq(0, 0), sq(0, 2), false);
        let double = Move {
            from,
            mid: Some(mid),
            to,
            promote: false,
        };
        assert!(legal_moves(&board).contains(&double), "kind={kind:?}");
        assert!(!board.gives_check(&MoveGenerator::new(engine_rules()), double));
        with_root_searcher(&board, &[], |searcher| {
            let ply = MAX_PLY - 1;
            set_test_path_ply(searcher, ply);
            searcher.delta_margin = INFINITY;
            searcher.qsearch_see_floor = -INFINITY;
            seed_first_capture(searcher, &board, first, ply);
            assert!(
                searcher
                    .quiesce(&mut board.clone(), -INFINITY, INFINITY, ply)
                    .is_some()
            );
            assert_eq!(searcher.nodes, 1, "kind={kind:?}");
            #[cfg(feature = "search-stats")]
            {
                assert!(searcher.stats.quiesce_move_count_prunes > 0);
                assert_eq!(searcher.stats.quiesce_lion_double_move_count_exemptions, 0);
            }
        });
    }
}
