//! null move pruningの適用条件、打ち切り回数、および検証探索を検査する。

use super::*;
use crate::search::REPETITION_BAND_START;
#[cfg(feature = "search-stats")]
use crate::search::alphabeta::pruning::NULL_MOVE_VERIFICATION_MIN_DEPTH;
use crate::search::alphabeta::pruning::{null_move_beta_minimum, razoring_margin};

fn quiet_position_with_material() -> Position {
    position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 1), Color::Black, PieceKind::GoldGeneral),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    )
}

fn attacked_position_with_material() -> Position {
    position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 1), Color::Black, PieceKind::GoldGeneral),
            (sq(4, 1), Color::Black, PieceKind::Rook),
            (sq(11, 6), Color::White, PieceKind::Rook),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    )
}

/// 深さ1と2でもnull moveが打ち切れる、静的評価とnull moveの値より十分低い零窓を作る。
fn shallow_null_window(board: &Position) -> (i32, i32) {
    let static_eval = evaluate(test_nnue(), board);
    let mut after_null = board.clone();
    after_null.make_null_move();
    let null_score = -evaluate(test_nnue(), &after_null);
    let beta = static_eval.min(null_score) - 300;
    assert!(beta >= null_move_beta_minimum(test_nnue().pawn_value()));
    assert!(beta.abs() < REPETITION_BAND_START);
    (beta - 1, beta)
}

#[allow(clippy::too_many_arguments)]
fn run_candidate(
    board: &Position,
    depth: u32,
    alpha: i32,
    beta: i32,
    ply: u32,
    cut_node: bool,
    nmp_min_ply: Option<u32>,
    previous_null: bool,
) -> (Option<i32>, u32, u64) {
    let mut result = None;
    with_root_searcher(board, &[], |searcher| {
        // ttPvを立てて、null moveより前のreverse futilityだけを除外する。
        searcher.tt.store(
            search_key(board),
            0,
            beta - 1,
            Bound::Upper,
            None,
            ply,
            true,
        );
        searcher.follow_pv[ply as usize] = true;
        searcher.nmp_min_ply = nmp_min_ply;
        if previous_null {
            searcher.null_move_ply = Some(ply);
        }
        let score = searcher.negamax(&mut board.clone(), depth, alpha, beta, ply, cut_node);
        result = Some((
            score,
            searcher.prior_nmp_fail_high[ply as usize],
            searcher.nodes,
        ));
    });
    result.unwrap()
}

#[test]
fn null_move_requires_cut_node_safe_window_and_unattacked_royal() {
    let board = quiet_position_with_material();
    assert!(!royal_under_attack(&board));
    let (alpha, beta) = shallow_null_window(&board);
    assert_eq!(
        run_candidate(&board, 1, alpha, beta, 0, true, None, false).1,
        1
    );
    assert_eq!(
        run_candidate(&board, 1, alpha, beta, 0, false, None, false).1,
        0,
        "cutNodeでないノード"
    );
    assert_eq!(
        run_candidate(
            &board,
            1,
            REPETITION_BAND_START,
            REPETITION_BAND_START + 1,
            0,
            true,
            None,
            false,
        )
        .1,
        0,
        "αが帯の内側にあるノード"
    );

    let attacked = attacked_position_with_material();
    assert!(royal_under_attack(&attacked));
    let (alpha, beta) = shallow_null_window(&attacked);
    assert_eq!(
        run_candidate(&attacked, 1, alpha, beta, 0, true, None, false).1,
        0,
        "王駒に疑似利きが届くノード"
    );
}

#[test]
fn null_move_requires_beta_floor_material_and_non_null_predecessor() {
    let board = quiet_position_with_material();
    let minimum = null_move_beta_minimum(test_nnue().pawn_value());
    assert_eq!(minimum, -961, "100 × 2000 / 208は0方向へ丸める");
    assert_eq!(
        run_candidate(&board, 1, minimum - 2, minimum - 1, 0, true, None, false,).1,
        0,
        "βが下限より小さいノード"
    );

    let (alpha, beta) = shallow_null_window(&board);
    assert_eq!(
        run_candidate(&board, 1, alpha, beta, 0, true, None, true).1,
        0,
        "直前の手がnull moveであるノード"
    );

    let kings = bare_kings();
    let (alpha, beta) = shallow_null_window(&kings);
    assert_eq!(
        run_candidate(&kings, 1, alpha, beta, 0, true, None, false).1,
        0,
        "王駒以外の自駒がないノード"
    );
}

#[test]
fn null_move_runs_at_depths_one_and_two_and_respects_nmp_min_ply() {
    let board = quiet_position_with_material();
    let (alpha, beta) = shallow_null_window(&board);
    for depth in [1, 2] {
        assert_eq!(
            run_candidate(&board, depth, alpha, beta, 0, true, None, false).1,
            1,
            "depth={depth}"
        );
    }
    assert_eq!(
        run_candidate(&board, 1, alpha, beta, 2, true, Some(3), false).1,
        0,
        "plyがnmpMinPlyより浅い"
    );
    assert_eq!(
        run_candidate(&board, 1, alpha, beta, 2, true, Some(2), false).1,
        1,
        "plyがnmpMinPlyに達した"
    );
}

#[test]
fn prior_nmp_fail_high_accumulates_and_child_slot_resets_on_node_entry() {
    let board = quiet_position_with_material();
    let (alpha, beta) = shallow_null_window(&board);
    with_root_searcher(&board, &[], |searcher| {
        assert!(searcher.nmp_min_ply.is_none());
        assert!(searcher.prior_nmp_fail_high.iter().all(|&count| count == 0));
        searcher
            .tt
            .store(search_key(&board), 0, beta - 1, Bound::Upper, None, 0, true);
        searcher.prior_nmp_fail_high[1] = 9;
        for expected in [1, 2] {
            assert!(
                searcher
                    .negamax(&mut board.clone(), 1, alpha, beta, 0, true)
                    .is_some()
            );
            assert_eq!(searcher.prior_nmp_fail_high[0], expected);
            assert_eq!(
                searcher.prior_nmp_fail_high[1], 0,
                "子の回数は親ノードの各開始時に初期化する"
            );
            searcher.prior_nmp_fail_high[1] = 9;
        }
    });
}

#[test]
fn root_resets_child_nmp_count_on_every_window_search() {
    let board = bare_kings();
    let moves = legal_moves(&board);
    with_root_searcher(&board, &[], |searcher| {
        for count in [7, 9] {
            searcher.prior_nmp_fail_high[1] = count;
            assert!(
                searcher
                    .search_root(&board, &moves, 1, -INFINITY, INFINITY)
                    .is_some()
            );
            assert_eq!(searcher.prior_nmp_fail_high[1], 0);
        }
    });
}

#[cfg(feature = "invariants")]
#[test]
fn root_rejects_active_null_move_verification_state() {
    let board = bare_kings();
    let moves = legal_moves(&board);
    with_root_searcher(&board, &[], |searcher| {
        searcher.nmp_min_ply = Some(1);
        let result = catch_unwind(AssertUnwindSafe(|| {
            searcher.search_root(&board, &moves, 1, -INFINITY, INFINITY)
        }));
        assert!(result.is_err());
    });
}

#[test]
fn verification_restores_same_ply_state_after_completed_failure() {
    let board = quiet_position_with_material();
    with_root_searcher(&board, &[], |searcher| {
        let saved_static_eval = 12_345;
        searcher.static_evals[0] = saved_static_eval;
        searcher.tt_pv[0] = true;
        searcher.nmp_min_ply = Some(77);
        searcher.pv[0].push(legal_moves(&board)[0]);
        let beta =
            evaluate(searcher.nnue, &board) + razoring_margin(searcher.nnue.pawn_value(), 4) + 2;
        let score = searcher
            .search_null_move_verification(&mut board.clone(), 4, beta, 0)
            .unwrap();
        assert!(score < beta);
        assert_eq!(searcher.nmp_min_ply, Some(77));
        assert_eq!(searcher.static_evals[0], saved_static_eval);
        assert!(searcher.tt_pv[0]);
        assert!(searcher.pv[0].is_empty());
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.null_move_verifications, 1);
            assert_eq!(searcher.stats.null_move_verification_failures, 1);
        }
    });
}

#[test]
fn verification_restores_same_ply_state_before_propagating_interruption() {
    let board = quiet_position_with_material();
    with_root_searcher(&board, &[], |searcher| {
        let saved_static_eval = -12_345;
        searcher.static_evals[0] = saved_static_eval;
        searcher.tt_pv[0] = true;
        searcher.pv[0].push(legal_moves(&board)[0]);
        searcher.interrupt_at = Some((0, StopReason::ExternalStop));
        let beta = evaluate(searcher.nnue, &board) + 1;
        assert_eq!(
            searcher.search_null_move_verification(&mut board.clone(), 4, beta, 0),
            None
        );
        assert_eq!(searcher.nmp_min_ply, None);
        assert_eq!(searcher.static_evals[0], saved_static_eval);
        assert!(searcher.tt_pv[0]);
        assert!(searcher.pv[0].is_empty());
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.null_move_verifications, 0);
            assert_eq!(searcher.stats.null_move_verification_failures, 0);
        }
    });
}

/// null moveの子と同じ局面の置換表値を用意し、実着手を読まずにfail-highを固定する。
#[cfg(feature = "search-stats")]
fn store_null_fail_high(searcher: &Searcher<'_>, board: &Position, depth: u32, beta: i32) {
    let mut after_null = board.clone();
    after_null.make_null_move();
    searcher.tt.store(
        search_key(&after_null),
        depth,
        -beta - 50,
        Bound::Upper,
        None,
        1,
        false,
    );
}

#[cfg(feature = "search-stats")]
#[test]
fn verification_starts_at_depth_threshold_and_returns_original_null_score() {
    assert_eq!(NULL_MOVE_VERIFICATION_MIN_DEPTH, 16);
    let board = quiet_position_with_material();
    for (depth, expected_verifications) in [(15, 0), (16, 1)] {
        with_root_searcher(&board, &[], |searcher| {
            let static_eval = evaluate(searcher.nnue, &board);
            let beta = static_eval - 200;
            assert!(beta >= null_move_beta_minimum(searcher.nnue.pawn_value()));
            searcher.follow_pv[0] = true;
            searcher.tt.store(
                search_key(&board),
                depth - 1,
                beta,
                Bound::Lower,
                None,
                0,
                true,
            );
            store_null_fail_high(searcher, &board, depth, beta);
            assert_eq!(
                searcher.negamax(&mut board.clone(), depth, beta - 1, beta, 0, true),
                Some(beta + 50),
                "検証値ではなく元のnull moveの値を返す"
            );
            assert_eq!(searcher.prior_nmp_fail_high[0], 1);
            assert_eq!(
                searcher.stats.null_move_verifications,
                expected_verifications
            );
            assert_eq!(searcher.stats.null_move_verification_failures, 0);
            assert_eq!(searcher.nmp_min_ply, None);
            assert_eq!(searcher.static_evals[0], static_eval);
            assert!(searcher.tt_pv[0]);
            assert!(searcher.pv[0].is_empty());
        });
    }
}

#[cfg(feature = "search-stats")]
#[test]
fn failed_verification_continues_outer_search_after_restoring_state() {
    let board = quiet_position_with_material();
    with_root_searcher(&board, &[], |searcher| {
        let static_eval = evaluate(searcher.nnue, &board);
        let beta = static_eval - 200;
        let depth = 20;
        searcher.follow_pv[0] = true;
        searcher.tt.store(
            search_key(&board),
            depth - 1,
            beta - 1,
            Bound::Upper,
            None,
            0,
            true,
        );
        store_null_fail_high(searcher, &board, depth, beta);
        searcher.interrupt_at = Some((0, StopReason::ExternalStop));
        assert_eq!(
            searcher.negamax(&mut board.clone(), depth, beta - 1, beta, 0, true),
            None,
            "検証がβ未満なら外側の最初の実着手で中断する"
        );
        assert_eq!(searcher.stats.null_move_verifications, 1);
        assert_eq!(searcher.stats.null_move_verification_failures, 1);
        assert_eq!(searcher.prior_nmp_fail_high[0], 0);
        assert_eq!(searcher.nmp_min_ply, None);
        assert_eq!(searcher.static_evals[0], static_eval);
        assert!(searcher.tt_pv[0]);
        assert!(searcher.pv[0].is_empty());
        assert_eq!(searcher.nodes, 0);
    });
}
