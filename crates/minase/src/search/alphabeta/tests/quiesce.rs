//! 静止探索を検査する。

use super::*;
use crate::eval::nnue::EVALUATION_LIMIT;
use crate::search::REPETITION_BAND_START;
use crate::search::alphabeta::capture_history::CaptureHistory;
use crate::search::alphabeta::quiesce::{qsearch_entry_cutoff_score, qsearch_post_search_score};

#[test]
fn qsearch_entry_cutoff_score_blends_toward_beta() {
    assert_eq!(qsearch_entry_cutoff_score(300, 100), 186);
    assert_eq!(qsearch_entry_cutoff_score(-50, -200), -135);
    assert_eq!(qsearch_entry_cutoff_score(1024, 0), 441);
    assert_eq!(qsearch_entry_cutoff_score(0, -1024), -583);
}

#[test]
fn qsearch_post_search_score_blends_only_strict_fail_highs() {
    assert_eq!(qsearch_post_search_score(400, 100), 235);
    assert_eq!(qsearch_post_search_score(1024, 0), 462);
    assert_eq!(qsearch_post_search_score(0, -1024), -562);
    assert_eq!(qsearch_post_search_score(100, 100), 100);
}

#[test]
fn qsearch_score_blending_skips_values_outside_the_normal_range() {
    let beta = -29_999;
    assert_eq!(qsearch_entry_cutoff_score(-50, beta), -50);
    assert_eq!(qsearch_post_search_score(-50, beta), -50);
    assert_eq!(
        qsearch_post_search_score(REPETITION_BAND_START, 100),
        REPETITION_BAND_START
    );
}

// 深さ0ではstand-patと捕獲手を探索する。
// 守られた歩兵を飛車で取る損な交換は静止探索で見抜かれ、最善手にならない。
#[test]
fn quiescence_avoids_a_losing_rook_for_pawn_capture() {
    // 先手: 飛車3十、王将6十二。後手: 歩兵3四、飛車3一、玉将6一。
    // 3筋の間は空で、後手飛車は歩兵越しに先手飛車から攻撃されない。
    let position = position(
        Color::Black,
        &[
            (fs(3, 10), Color::Black, PieceKind::Rook),
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(3, 4), Color::White, PieceKind::Pawn),
            (fs(3, 1), Color::White, PieceKind::Rook),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let moves = legal_moves(&position);

    let result = run_search(
        &position,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    assert!(
        result.best_move.from != fs(3, 10) || result.best_move.to != fs(3, 4),
        "守られた歩兵を飛車で取る手は最善手にならない"
    );
    assert!(result.score.abs() < 29_000);
}

// 飛車で角行を取って歩兵に取り返される交換を使い、SEEの2段の境界を分離する。
#[test]
fn quiescence_applies_see_threshold_then_floor_with_fixed_bounds() {
    let position = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 4), Color::Black, PieceKind::Rook),
            (sq(5, 5), Color::White, PieceKind::Bishop),
            (sq(5, 6), Color::White, PieceKind::Pawn),
        ],
    );
    let capture = step(sq(5, 4), sq(5, 5));
    let nnue = test_nnue();
    assert!(legal_moves(&position).contains(&capture));
    let loss = nnue.piece_value(position.piece_at(capture.from).unwrap())
        - nnue.piece_value(position.piece_at(capture.to).unwrap());
    assert!(loss > 1);
    assert!(!see_prunes(&position, engine_rules(), nnue, capture, -loss));
    assert!(see_prunes(
        &position,
        engine_rules(),
        nnue,
        capture,
        -loss + 1
    ));

    for (case, delta_margin, see_floor, threshold_prunes, floor_prunes, nodes) in [
        ("threshold", loss - 1, -INFINITY, true, false, 0),
        (
            "threshold before floor",
            loss - 1,
            -loss + 1,
            true,
            false,
            0,
        ),
        ("floor", loss + 1, -loss + 1, false, true, 0),
        ("within floor", loss + 1, -loss, false, false, 1),
    ] {
        with_root_searcher(&position, &[], |searcher| {
            let ply = MAX_PLY - 1;
            set_test_path_ply(searcher, ply);
            zero_position_correction(searcher, &position);
            searcher.delta_margin = delta_margin;
            searcher.qsearch_see_floor = see_floor;
            assert_eq!(
                see_prunes(
                    &position,
                    searcher.rules,
                    searcher.nnue,
                    capture,
                    -delta_margin,
                ),
                threshold_prunes,
                "{case}"
            );
            assert_eq!(
                !threshold_prunes
                    && see_prunes(&position, searcher.rules, searcher.nnue, capture, see_floor,),
                floor_prunes,
                "{case}"
            );
            assert!(
                searcher
                    .quiesce(&mut position.clone(), -INFINITY, INFINITY, ply)
                    .is_some(),
                "{case}"
            );
            assert_eq!(searcher.nodes, nodes, "{case}");
            #[cfg(feature = "search-stats")]
            {
                assert_eq!(
                    searcher.stats.quiesce_see_threshold_prunes,
                    u64::from(threshold_prunes),
                    "{case}"
                );
                assert_eq!(
                    searcher.stats.quiesce_see_floor_prunes,
                    u64::from(floor_prunes),
                    "{case}"
                );
            }
        });
    }
}

fn single_low_value_capture() -> (Position, Move) {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(0, 0), Color::Black, PieceKind::Rook),
            (sq(0, 2), Color::White, PieceKind::Pawn),
        ],
    );
    let capture = step(sq(0, 0), sq(0, 2));
    assert!(legal_moves(&board).contains(&capture));
    (board, capture)
}

// 入口時のdelta pruning境界上の捕獲も候補として取り出し、見積りを返して保存する。
#[test]
fn quiescence_delta_pruned_low_value_capture_raises_and_stores_best() {
    let (board, capture) = single_low_value_capture();
    with_root_searcher(&board, &[], |searcher| {
        zero_position_correction(searcher, &board);
        searcher.delta_margin = 37;
        searcher.qsearch_see_floor = -INFINITY;
        let stand_pat = evaluate(searcher.nnue, &board);
        let captured_value = searcher
            .nnue
            .piece_value(board.piece_at(capture.to).unwrap());
        let expected = stand_pat + searcher.delta_margin + captured_value;
        assert!(expected < REPETITION_BAND_START);

        assert_eq!(
            searcher.quiesce(&mut board.clone(), expected, INFINITY, 0),
            Some(expected)
        );
        assert_eq!(searcher.nodes, 0, "delta pruningした捕獲は読まない");
        let hit = searcher.tt.probe(search_key(&board), 0).unwrap();
        assert_eq!(hit.value.unwrap().score, expected);
        assert_eq!(hit.value.unwrap().depth, 0);
        assert_eq!(hit.value.unwrap().bound, Bound::Upper);
        assert_eq!(hit.static_eval, Some(stand_pat));
        assert_eq!(hit.best_move, None);
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.quiesce_delta_best_score_raises, 1);
            assert_eq!(searcher.stats.quiesce_see_threshold_best_score_raises, 0);
            assert_eq!(searcher.stats.quiesce_low_value_group_nodes, 1);
        }
    });
}

// 低価値の置換表手は候補として処理するが、低価値群へ達したノードの計数には含めない。
#[cfg(feature = "search-stats")]
#[test]
fn quiescence_low_value_group_stat_excludes_leading_tt_capture() {
    let (board, capture) = single_low_value_capture();
    with_root_searcher(&board, &[], |searcher| {
        zero_position_correction(searcher, &board);
        searcher.delta_margin = 37;
        searcher.qsearch_see_floor = -INFINITY;
        let stand_pat = evaluate(searcher.nnue, &board);
        let captured_value = searcher
            .nnue
            .piece_value(board.piece_at(capture.to).unwrap());
        let expected = stand_pat + searcher.delta_margin + captured_value;
        searcher.tt.store(
            search_key(&board),
            0,
            INFINITY,
            None,
            Bound::Upper,
            Some(capture),
            0,
            false,
        );

        assert_eq!(
            searcher.quiesce(&mut board.clone(), expected, INFINITY, 0),
            Some(expected)
        );
        assert_eq!(searcher.stats.quiesce_delta_best_score_raises, 1);
        assert_eq!(searcher.stats.quiesce_low_value_group_nodes, 0);
    });
}

// SEEの閾値で読まない候補は、現在のαとfutility baseの小さい方まで最善値を上げる。
#[test]
fn quiescence_see_threshold_pruned_capture_raises_best() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 4), Color::Black, PieceKind::Rook),
            (sq(5, 5), Color::White, PieceKind::Bishop),
            (sq(5, 6), Color::White, PieceKind::Pawn),
        ],
    );
    let capture = step(sq(5, 4), sq(5, 5));
    assert!(legal_moves(&board).contains(&capture));
    with_root_searcher(&board, &[], |searcher| {
        zero_position_correction(searcher, &board);
        searcher.delta_margin = 2;
        searcher.qsearch_see_floor = -INFINITY;
        let stand_pat = evaluate(searcher.nnue, &board);
        let alpha = stand_pat + 1;
        assert!(see_prunes(
            &board,
            searcher.rules,
            searcher.nnue,
            capture,
            -1,
        ));

        assert_eq!(
            searcher.quiesce(&mut board.clone(), alpha, INFINITY, 0),
            Some(alpha)
        );
        assert_eq!(searcher.nodes, 0, "SEEの閾値で枝刈りした捕獲は読まない");
        let hit = searcher.tt.probe(search_key(&board), 0).unwrap();
        assert_eq!(hit.value.unwrap().score, alpha);
        assert_eq!(hit.value.unwrap().bound, Bound::Upper);
        assert_eq!(hit.best_move, None);
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.quiesce_see_threshold_prunes, 1);
            assert_eq!(searcher.stats.quiesce_delta_best_score_raises, 0);
            assert_eq!(searcher.stats.quiesce_see_threshold_best_score_raises, 1);
            assert_eq!(searcher.stats.quiesce_low_value_group_nodes, 0);
        }
    });
}

// 枝刈りの見積りから反復専用帯の値を作らない。
#[test]
fn quiescence_does_not_raise_best_to_repetition_band() {
    let (board, capture) = single_low_value_capture();
    with_root_searcher(&board, &[], |searcher| {
        zero_position_correction(searcher, &board);
        let stand_pat = evaluate(searcher.nnue, &board);
        let captured_value = searcher
            .nnue
            .piece_value(board.piece_at(capture.to).unwrap());
        searcher.delta_margin = REPETITION_BAND_START - stand_pat - captured_value;
        searcher.qsearch_see_floor = -INFINITY;
        assert!(searcher.delta_margin > 0);

        assert_eq!(
            searcher.quiesce(&mut board.clone(), REPETITION_BAND_START, INFINITY, 0,),
            Some(stand_pat)
        );
        assert_eq!(searcher.nodes, 0);
        let hit = searcher.tt.probe(search_key(&board), 0).unwrap();
        assert_eq!(hit.value.unwrap().score, stand_pat);
        assert_eq!(hit.value.unwrap().bound, Bound::Upper);
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.quiesce_delta_best_score_raises, 0);
            assert_eq!(searcher.stats.quiesce_low_value_group_nodes, 1);
        }
    });
}

// 捕獲候補がなくても通常出口を通り、stand-patを深さ0の置換表項目へ保存する。
#[test]
fn quiescence_without_captures_stores_stand_pat() {
    let board = bare_kings();
    with_root_searcher(&board, &[], |searcher| {
        zero_position_correction(searcher, &board);
        let stand_pat = evaluate(searcher.nnue, &board);
        assert_eq!(
            searcher.quiesce(&mut board.clone(), -INFINITY, INFINITY, 0),
            Some(stand_pat)
        );
        assert_eq!(searcher.nodes, 0);
        let hit = searcher.tt.probe(search_key(&board), 0).unwrap();
        assert_eq!(hit.value.unwrap().score, stand_pat);
        assert_eq!(hit.value.unwrap().depth, 0);
        assert_eq!(hit.value.unwrap().bound, Bound::Upper);
        assert_eq!(hit.static_eval, Some(stand_pat));
        assert_eq!(hit.best_move, None);
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.quiesce_tt_probes, 1);
            assert_eq!(searcher.stats.quiesce_tt_hits, 0);
        }
    });
}

// 先の捕獲でαが上がると、初期αでは読んだはずの後続捕獲を現在のαでdelta pruningする。
#[test]
fn quiescence_delta_pruning_uses_alpha_raised_by_an_earlier_capture() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(0, 4), Color::Black, PieceKind::Rook),
            (sq(0, 5), Color::White, PieceKind::Rook),
            (sq(5, 4), Color::Black, PieceKind::Rook),
            (sq(5, 5), Color::White, PieceKind::Pawn),
        ],
    );
    let first = step(sq(0, 4), sq(0, 5));
    let later = step(sq(5, 4), sq(5, 5));
    assert!(legal_moves(&board).contains(&first));
    assert!(legal_moves(&board).contains(&later));
    with_root_searcher(&board, &[], |searcher| {
        let ply = MAX_PLY - 1;
        set_test_path_ply(searcher, ply);
        zero_position_correction(searcher, &board);
        searcher.delta_margin = 0;
        searcher.qsearch_see_floor = -INFINITY;
        let stand_pat = evaluate(searcher.nnue, &board);
        let later_estimate =
            stand_pat + searcher.nnue.piece_value(board.piece_at(later.to).unwrap());
        let mut child = board.clone();
        child.make_move_unchecked(first, engine_rules());
        let first_score = -evaluate(searcher.nnue, &child);
        assert!(stand_pat < later_estimate);
        assert!(later_estimate <= first_score);
        assert!(!see_prunes(&board, searcher.rules, searcher.nnue, first, 0,));

        assert_eq!(
            searcher.quiesce(&mut board.clone(), -INFINITY, INFINITY, ply),
            Some(first_score)
        );
        assert_eq!(searcher.nodes, 1, "後続捕獲は上昇後のαで枝刈りする");
        let hit = searcher.tt.probe(search_key(&board), ply).unwrap();
        assert_eq!(hit.value.unwrap().score, first_score);
        assert_eq!(hit.best_move, Some(first));
        #[cfg(feature = "search-stats")]
        {
            assert_eq!(searcher.stats.quiesce_delta_best_score_raises, 0);
            assert_eq!(searcher.stats.quiesce_see_threshold_prunes, 0);
        }
    });
}

// 捕獲手のない葉ではstand-patを返すため、
// depth=1の根評価は各合法手の子局面に対する静的評価の最大値と一致する。
#[test]
fn quiescence_without_captures_matches_static_evaluation() {
    let mut position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::GoldGeneral),
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(6, 3), Color::White, PieceKind::GoldGeneral),
        ],
    );
    let moves = legal_moves(&position);
    let expected = moves
        .iter()
        .map(|&mv| {
            let undo = position.make_move_unchecked(mv, engine_rules());
            let score = -crate::eval::evaluate(test_nnue(), &position);
            position.unmake_move(undo);
            score
        })
        .max()
        .expect("root must have a legal move");

    let result = run_search(
        &position,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    assert_eq!(result.score, expected);
    // 各合法手を1回適用し、子の静止探索では捕獲がないので追加の適用はない。
    assert_eq!(result.nodes, moves.len() as u64);
}

// 最大plyでは置換表を参照も更新もせず、補正前の静的評価を返す。
#[test]
fn quiescence_max_ply_does_not_probe_or_store() {
    let board = bare_kings();
    let key = search_key(&board);
    let stand_pat = evaluate(test_nnue(), &board);
    let table = small_tt();
    table.store(key, 8, 12_345, None, Bound::Exact, None, MAX_PLY, false);
    let raw_before = table.raw_entry(key);

    assert_eq!(
        run_quiesce(&board, -INFINITY, INFINITY, MAX_PLY, &table),
        (stand_pat, 0)
    );
    assert_eq!(table.raw_entry(key), raw_before);
}

// Exact、score >= betaのLower、score <= alphaのUpperは、窓の幅が1の場合だけ返す。
// 幅が2以上なら打ち切り条件を満たす記録でも使わず、通常のstand-pat処理へ進む。
#[test]
fn quiescence_tt_cutoff_requires_a_zero_window() {
    let board = bare_kings();
    let key = search_key(&board);
    let stand_pat = evaluate(test_nnue(), &board);

    let exact = small_tt();
    exact.store(key, 0, stand_pat + 7, None, Bound::Exact, None, 0, false);
    let raw = exact.raw_entry(key);
    assert_eq!(
        run_quiesce(&board, stand_pat + 20, stand_pat + 21, 0, &exact).0,
        stand_pat + 7
    );
    assert_eq!(exact.raw_entry(key), raw, "零窓では通常出口を通らない");
    let exact = small_tt();
    exact.store(key, 0, stand_pat + 7, None, Bound::Exact, None, 0, false);
    assert_eq!(
        run_quiesce(&board, -INFINITY, INFINITY, 0, &exact).0,
        stand_pat + 7
    );
    let hit = exact.probe(key, 0).unwrap();
    assert_eq!(hit.value.unwrap().bound, Bound::Upper);
    assert_eq!(hit.static_eval, Some(stand_pat));

    let lower = small_tt();
    lower.store(key, 0, stand_pat - 2, None, Bound::Lower, None, 0, false);
    assert_eq!(
        run_quiesce(&board, stand_pat - 3, stand_pat - 2, 0, &lower).0,
        stand_pat - 2
    );
    let lower = small_tt();
    lower.store(key, 0, stand_pat, None, Bound::Lower, None, 0, false);
    assert_eq!(
        run_quiesce(&board, stand_pat - 2, stand_pat, 0, &lower).0,
        stand_pat
    );

    let upper = small_tt();
    upper.store(key, 0, stand_pat + 2, None, Bound::Upper, None, 0, false);
    assert_eq!(
        run_quiesce(&board, stand_pat + 2, stand_pat + 3, 0, &upper).0,
        stand_pat + 2
    );
    let upper = small_tt();
    upper.store(key, 0, stand_pat + 2, None, Bound::Upper, None, 0, false);
    assert_eq!(
        run_quiesce(&board, stand_pat + 2, stand_pat + 4, 0, &upper).0,
        stand_pat
    );
}

// 置換表の通常域の値は、Exactなら大小によらず、Lowerならstand-patより大きい場合だけ、
// Upperならstand-pat以下の場合だけ最善値の初期値にする。専用の帯と詰み帯は使わない。
#[test]
fn quiescence_tt_value_replaces_stand_pat_only_for_the_matching_bound_direction() {
    let board = bare_kings();
    let key = search_key(&board);
    let stand_pat = evaluate(test_nnue(), &board);
    for (case, bound, stored, expected) in [
        ("exact below", Bound::Exact, stand_pat - 7, stand_pat - 7),
        ("exact above", Bound::Exact, stand_pat + 7, stand_pat + 7),
        ("lower below", Bound::Lower, stand_pat - 7, stand_pat),
        ("lower above", Bound::Lower, stand_pat + 7, stand_pat + 7),
        ("upper below", Bound::Upper, stand_pat - 7, stand_pat - 7),
        ("upper above", Bound::Upper, stand_pat + 7, stand_pat),
        (
            "repetition band",
            Bound::Exact,
            REPETITION_BAND_START,
            stand_pat,
        ),
        ("mate band", Bound::Exact, MATE - 1, stand_pat),
    ] {
        let table = small_tt();
        table.store(key, 0, stored, Some(stand_pat), bound, None, 0, false);
        assert_eq!(
            run_quiesce(&board, -INFINITY, INFINITY, 0, &table),
            (expected, 0),
            "{case}"
        );
    }
}

// 詰み帯は零窓の打ち切りに使うが、専用の帯は使わない。
#[test]
fn quiescence_tt_cutoff_uses_mate_but_not_repetition_band_values() {
    let board = bare_kings();
    let key = search_key(&board);
    let stand_pat = evaluate(test_nnue(), &board);

    let mate = small_tt();
    mate.store(key, 0, MATE - 5, None, Bound::Exact, None, 0, false);
    assert_eq!(run_quiesce(&board, 0, 1, 0, &mate), (MATE - 5, 0));

    let repetition = small_tt();
    repetition.store(
        key,
        0,
        REPETITION_BAND_START,
        None,
        Bound::Exact,
        None,
        0,
        false,
    );
    assert_eq!(
        run_quiesce(&board, stand_pat, stand_pat + 1, 0, &repetition),
        (stand_pat, 0)
    );
}

// 置換表で置き換えた初期値がβ以上なら、幅が2の窓でも候補を作らず、その値を返す。
#[test]
fn quiescence_tt_replacement_at_beta_returns_before_candidates() {
    let (board, _) = single_low_value_capture();
    with_root_searcher(&board, &[], |searcher| {
        zero_position_correction(searcher, &board);
        let stand_pat = evaluate(searcher.nnue, &board);
        let stored = stand_pat + 20;
        let key = search_key(&board);
        searcher.tt.store(
            key,
            0,
            stored,
            Some(stand_pat),
            Bound::Exact,
            None,
            0,
            false,
        );
        let raw = searcher.tt.raw_entry(key);

        assert_eq!(
            searcher.quiesce(&mut board.clone(), stored - 2, stored, 0),
            Some(stored)
        );
        assert_eq!(searcher.nodes, 0);
        assert_eq!(searcher.tt.raw_entry(key), raw);
    });
}

// delta pruningのfutility baseは、置換表で置き換えた値ではなく元のstand-patから求める。
#[test]
fn quiescence_delta_pruning_uses_stand_pat_before_tt_replacement() {
    let (board, capture) = single_low_value_capture();
    with_root_searcher(&board, &[], |searcher| {
        zero_position_correction(searcher, &board);
        searcher.delta_margin = 37;
        searcher.qsearch_see_floor = -INFINITY;
        let stand_pat = evaluate(searcher.nnue, &board);
        let captured_value = searcher
            .nnue
            .piece_value(board.piece_at(capture.to).unwrap());
        let stored = stand_pat + searcher.delta_margin + captured_value;
        searcher.tt.store(
            search_key(&board),
            0,
            stored,
            Some(stand_pat),
            Bound::Exact,
            None,
            0,
            false,
        );

        assert_eq!(
            searcher.quiesce(&mut board.clone(), -INFINITY, INFINITY, 0),
            Some(stored)
        );
        assert_eq!(searcher.nodes, 0, "境界上の捕獲はdelta pruningする");
    });
}

// 照会ミスでstand-patがβ以上なら探索値なしの記録を書き、次の照会では値で打ち切らない。
#[test]
fn quiescence_stand_pat_cutoff_stores_static_eval_without_a_search_value() {
    let board = bare_kings();
    let key = search_key(&board);
    let stand_pat = evaluate(test_nnue(), &board);
    let table = small_tt();

    assert_eq!(
        run_quiesce(&board, stand_pat - 2, stand_pat, 0, &table),
        (stand_pat, 0)
    );
    let hit = table.probe(key, 0).unwrap();
    assert_eq!(hit.value, None);
    assert_eq!(hit.static_eval, Some(stand_pat));
    assert_eq!(hit.best_move, None);
    assert!(!hit.is_pv);

    assert_eq!(
        run_quiesce(&board, stand_pat + 1, stand_pat + 2, 0, &table),
        (stand_pat, 0)
    );
    let hit = table.probe(key, 0).unwrap();
    assert_eq!(hit.value.unwrap().score, stand_pat);
    assert_eq!(hit.value.unwrap().bound, Bound::Upper);
}

// 入口の返り値は補正後のstand-patからβへ寄せるが、置換表には寄せる前の
// 補正前静的評価だけを保存する。
#[test]
fn quiescence_stand_pat_cutoff_blends_return_but_stores_unadjusted_static_eval() {
    let board = bare_kings();
    let key = search_key(&board);
    let unadjusted = evaluate(test_nnue(), &board);
    with_root_searcher(&board, &[], |searcher| {
        zero_position_correction(searcher, &board);
        searcher.correction.store(
            board.side_to_move(),
            material_key(&board),
            CorrectionTableKind::Material,
            1024,
        );
        let correction = position_correction(searcher, board.side_to_move(), 0);
        assert_ne!(correction, 0);
        let stand_pat = (unadjusted + correction).clamp(-EVALUATION_LIMIT, EVALUATION_LIMIT);
        assert_ne!(stand_pat, unadjusted);
        let beta = stand_pat - 1024;
        let expected = stand_pat - 583;
        assert!(beta.abs() < REPETITION_BAND_START);

        assert_eq!(
            searcher.quiesce(&mut board.clone(), beta - 1, beta, 0),
            Some(expected)
        );
        let hit = searcher.tt.probe(key, 0).unwrap();
        assert_eq!(hit.value, None);
        assert_eq!(hit.static_eval, Some(unadjusted));
        assert_eq!(hit.best_move, None);
    });
}

// 照会に当たったノードでは、stand-patで打ち切っても探索値なしの記録へ置き換えない。
#[test]
fn quiescence_stand_pat_cutoff_does_not_store_on_a_tt_hit() {
    let board = bare_kings();
    let key = search_key(&board);
    let stand_pat = evaluate(test_nnue(), &board);
    let table = small_tt();
    table.store(key, 0, stand_pat, None, Bound::Upper, None, 0, true);
    let raw = table.raw_entry(key);

    assert_eq!(
        run_quiesce(&board, stand_pat - 2, stand_pat, 0, &table),
        (stand_pat, 0)
    );
    assert_eq!(table.raw_entry(key), raw);
    let hit = table.probe(key, 0).unwrap();
    assert!(hit.value.is_some());
    assert_eq!(hit.static_eval, None);
}

// 探索値なしの記録の静的評価を、評価関数の出力に代えてstand-patとして使う。
#[test]
fn quiescence_uses_static_eval_from_a_value_less_tt_entry() {
    let board = bare_kings();
    let key = search_key(&board);
    let evaluated = evaluate(test_nnue(), &board);
    let stored = if evaluated == 1_234 { -1_234 } else { 1_234 };
    let table = small_tt();
    table.store_static_eval(key, stored, false);

    assert_eq!(
        run_quiesce(&board, -INFINITY, INFINITY, 0, &table),
        (stored, 0)
    );
    let hit = table.probe(key, 0).unwrap();
    assert_eq!(hit.static_eval, Some(stored));
    assert_eq!(hit.value.unwrap().score, stored);
}

// 静止探索の通常出口は深さ0の上界として記録され、同じ入口局面を
// 上界が打ち切れる零窓で再訪すると捕獲展開を省く。
#[test]
fn quiescence_stores_depth_zero_and_reuses_it_on_revisit() {
    // 項目4の手数制限でも捕獲が最善になるよう、成否の分岐を持たない局面を使う。
    let position = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 0), Color::Black, PieceKind::Rook),
            (sq(0, 2), Color::White, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    let capture = Move {
        from: sq(0, 0),
        mid: None,
        to: sq(0, 2),
        promote: false,
    };
    assert!(legal_moves(&position).contains(&capture));

    let table = small_tt();
    let (first_score, first_nodes) = run_quiesce(&position, -INFINITY, INFINITY, 0, &table);
    let hit = table.probe(search_key(&position), 0).unwrap();
    assert_eq!(hit.value.unwrap().depth, 0);
    assert_eq!(hit.value.unwrap().bound, Bound::Upper);
    let stored_move = hit.best_move.expect("捕獲手が最善なら記録手を持つ");
    assert_eq!(stored_move.from, capture.from);
    assert_eq!(stored_move.to, capture.to);
    assert_eq!(hit.value.unwrap().score, first_score);
    assert_eq!(hit.static_eval, Some(evaluate(test_nnue(), &position)));

    let (second_score, second_nodes) =
        run_quiesce(&position, first_score, first_score + 1, 0, &table);
    assert_eq!(second_score, first_score);
    assert_eq!(first_nodes, 1);
    assert_eq!(second_nodes, 0);
}

// 通常出口は窓内の値も含めて上界を深さ0で記録し、stand-patが最善なら手なしとする。
#[test]
fn quiescence_records_bounds_and_no_move_by_exit_path() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(3, 10), Color::Black, PieceKind::Rook),
            (fs(3, 4), Color::White, PieceKind::Pawn),
            (fs(3, 1), Color::White, PieceKind::Rook),
        ],
    );
    let key = search_key(&position);
    let stand_pat = evaluate(test_nnue(), &position);
    let nnue = test_nnue();
    let mut captures = Vec::new();
    MoveGenerator::standard().generate_captures(&position, &mut captures);
    assert!(!captures.is_empty());
    assert!(
        captures
            .iter()
            .all(|&mv| see_prunes(&position, engine_rules(), nnue, mv, 0,))
    );
    let cases = [
        ("upper", stand_pat, INFINITY, Bound::Upper),
        ("inside", stand_pat - 1, stand_pat + 1, Bound::Upper),
        ("all_see_pruned", -INFINITY, INFINITY, Bound::Upper),
    ];

    for (case, alpha, beta, expected_bound) in cases {
        let table = small_tt();
        let (score, nodes) = run_quiesce(&position, alpha, beta, 0, &table);
        let hit = table.probe(key, 0).unwrap();
        assert_eq!((score, nodes), (stand_pat, 0), "{case}");
        assert_eq!(hit.value.unwrap().score, stand_pat, "{case}");
        assert_eq!(hit.value.unwrap().depth, 0, "{case}");
        assert_eq!(hit.value.unwrap().bound, expected_bound, "{case}");
        assert_eq!(hit.best_move, None, "{case}");
    }
}

// 静止探索で最後の王駒を取ったMATE−(ply+1)は、現在plyを渡した
// 置換表の変換で往復する。捕獲を評価する最大plyでも詰み帯に収まる。
#[test]
fn quiescence_mate_score_round_trips_through_the_tt() {
    let position = position(
        Color::Black,
        &[
            (fs(1, 12), Color::Black, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::Rook),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let mate_move = Move {
        from: fs(6, 10),
        mid: None,
        to: fs(6, 1),
        promote: false,
    };
    assert!(legal_moves(&position).contains(&mate_move));

    for ply in [0, 5, MAX_PLY - 1] {
        let table = small_tt();
        let (score, nodes) = run_quiesce(&position, -INFINITY, INFINITY, ply, &table);
        assert_eq!(nodes, 0);
        let hit = table.probe(search_key(&position), ply).unwrap();
        assert_eq!(score, MATE - (ply + 1) as i32);
        assert!(score >= MATE_THRESHOLD);
        assert_eq!(hit.value.unwrap().score, score);
        assert_eq!(hit.value.unwrap().depth, 0);
        assert_eq!(hit.value.unwrap().bound, Bound::Upper);
        assert_eq!(hit.best_move, Some(mate_move));
        // 同じ局面を根から評価すると、捕獲までの距離は1手になる。
        assert_eq!(
            table
                .probe(search_key(&position), 0)
                .unwrap()
                .value
                .unwrap()
                .score,
            MATE - 1
        );
    }
}

// 候補の探索値がβ以上になった通常出口は下界として保存する。
#[test]
fn quiescence_normal_exit_stores_a_lower_bound_on_fail_high() {
    let board = position(
        Color::Black,
        &[
            (fs(1, 12), Color::Black, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::Rook),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let expected = MATE - 1;
    let table = small_tt();
    assert_eq!(
        run_quiesce(&board, expected - 2, expected - 1, 0, &table).0,
        expected
    );
    let hit = table.probe(search_key(&board), 0).unwrap();
    assert_eq!(hit.value.unwrap().score, expected);
    assert_eq!(hit.value.unwrap().bound, Bound::Lower);
}

// 候補を調べた後にβを超えた通常域の最善値はβへ寄せ、同じ値を返して下界として保存する。
#[test]
fn quiescence_post_search_fail_high_blends_return_and_stored_value() {
    let (board, capture) = single_low_value_capture();
    let mut child = board.clone();
    child.make_move_unchecked(capture, engine_rules());

    with_root_searcher(&board, &[], |searcher| {
        zero_position_correction(searcher, &board);
        searcher.delta_margin = INFINITY;
        searcher.qsearch_see_floor = -INFINITY;
        searcher.tt.store_static_eval(search_key(&board), 0, false);
        searcher.tt.store(
            search_key(&child),
            0,
            -400,
            None,
            Bound::Exact,
            None,
            1,
            false,
        );

        assert_eq!(searcher.quiesce(&mut board.clone(), 99, 100, 0), Some(235));
        let hit = searcher.tt.probe(search_key(&board), 0).unwrap();
        assert_eq!(hit.value.unwrap().score, 235);
        assert_eq!(hit.value.unwrap().bound, Bound::Lower);
        assert_eq!(hit.best_move, Some(capture));
        assert_eq!(hit.static_eval, Some(0));
    });
}

fn history_ordering_position() -> (Position, [Move; 6]) {
    let mut pieces = vec![
        (sq(11, 0), Color::Black, PieceKind::King),
        (sq(11, 11), Color::White, PieceKind::King),
    ];
    let captures = core::array::from_fn(|i| {
        let from = sq(2 * i as u8, 4);
        let to = sq(2 * i as u8, 5);
        let attacker = if i < 2 {
            PieceKind::Pawn
        } else {
            PieceKind::Rook
        };
        let victim = if i == 5 {
            PieceKind::Rook
        } else {
            PieceKind::Pawn
        };
        pieces.push((from, Color::Black, attacker));
        pieces.push((to, Color::White, victim));
        Move {
            from,
            to,
            mid: None,
            promote: false,
        }
    });
    (position(Color::Black, &pieces), captures)
}

fn captures_with_history(
    board: &Position,
    nnue: &Nnue,
    history: &CaptureHistory,
    tt_move: Option<Move>,
) -> Vec<Move> {
    let generator = MoveGenerator::standard();
    let ranks = CaptureRanks::new(nnue);
    let mut buffers = QsearchBuffers::default();
    buffers.reset(board);
    buffers.initialize(board, &generator, nnue, &ranks);
    buffers.set_tt_move(board, &generator, tt_move);
    std::iter::from_fn(|| buffers.next(board, &generator, nnue, &ranks, history))
        .map(|candidate| candidate.capture.mv)
        .collect()
}

// 群内は生の履歴、攻撃駒価値、生成順で並べ、置換表の手を最優先する。
// 採用理由はdocs/adr/update-capture-history-in-stockfish-form-and-use-it-in-ordering-qsearch-and-lmr.md。
#[test]
fn quiescence_capture_history_preserves_group_order_and_tt_priority() {
    let (board, moves) = history_ordering_position();
    let nnue = test_nnue();
    for (case, cutoff, tt_index, expected) in [
        ("no_history", None, None, [5, 0, 1, 2, 3, 4]),
        ("raw_history", Some((4, 0, 1)), None, [5, 4, 1, 2, 3, 0]),
        (
            "group_boundary",
            Some((4, 5, 256)),
            None,
            [5, 4, 0, 1, 2, 3],
        ),
        (
            "tt_priority",
            Some((4, 0, 256)),
            Some(0),
            [0, 5, 4, 1, 2, 3],
        ),
    ] {
        let mut history = empty_capture_history();
        if let Some((winner, loser, depth)) = cutoff {
            let bonus = if depth == 1 {
                1
            } else {
                super::super::capture_history::CAPTURE_HISTORY_LIMIT
            };
            history.update(&board, nnue, moves[winner], bonus);
            history.update(&board, nnue, moves[loser], -bonus);
            assert!(history.read(&board, nnue, moves[winner]) > 0, "{case}");
            assert!(history.read(&board, nnue, moves[loser]) < 0, "{case}");
            if depth == 1 {
                assert_eq!(history.read(&board, nnue, moves[winner]), 1, "{case}");
                assert_eq!(history.read(&board, nnue, moves[loser]), -1, "{case}");
                assert_eq!(history.adjustment(&board, nnue, moves[winner]), 0, "{case}");
                assert_eq!(history.adjustment(&board, nnue, moves[loser]), 0, "{case}");
            }
        }
        assert_eq!(
            captures_with_history(&board, nnue, &history, tt_index.map(|index| moves[index])),
            expected.map(|index| moves[index]),
            "{case}"
        );
    }
}

// 特殊駒も履歴順で通常駒と合流し、同点は公開生成順を保つ。
#[test]
fn quiescence_capture_history_orders_special_and_ordinary_captures_together() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(0, 4), Color::Black, PieceKind::Pawn),
            (sq(0, 5), Color::White, PieceKind::Pawn),
            (sq(4, 4), Color::Black, PieceKind::Rook),
            (sq(4, 5), Color::White, PieceKind::Pawn),
            (sq(8, 4), Color::Black, PieceKind::Lion),
            (sq(9, 4), Color::Black, PieceKind::Lion),
            (sq(8, 6), Color::White, PieceKind::Pawn),
        ],
    );
    let nnue = test_nnue();
    let mut public = Vec::new();
    MoveGenerator::standard().generate_captures(&board, &mut public);
    let mut reference: Vec<_> = public
        .into_iter()
        .map(|mv| (mv, move_order_key(&board, nnue, mv).unwrap()))
        .collect();
    order_captures(&mut reference);
    let reference: Vec<_> = reference.into_iter().map(|(mv, _)| mv).collect();
    let history = empty_capture_history();
    assert_eq!(
        captures_with_history(&board, nnue, &history, None),
        reference
    );
    let lion_moves: Vec<_> = reference
        .iter()
        .copied()
        .filter(|mv| mv.to == sq(8, 6))
        .collect();
    assert!(
        lion_moves.len() >= 2,
        "同じ履歴の特殊捕獲で生成順も検査する"
    );
    let ordinary: Vec<_> = reference
        .iter()
        .copied()
        .filter(|mv| mv.to != sq(8, 6))
        .collect();
    assert_eq!(ordinary.len(), 2);
    for reward_special in [true, false] {
        let mut history = empty_capture_history();
        if reward_special {
            history.update(&board, nnue, lion_moves[0], 1);
            for &mv in &ordinary {
                history.update(&board, nnue, mv, -1);
            }
        } else {
            history.update(&board, nnue, ordinary[1], 1);
            history.update(&board, nnue, lion_moves[0], -1);
        }
        let expected: Vec<_> = if reward_special {
            lion_moves.iter().chain(&ordinary).copied().collect()
        } else {
            [ordinary[1], ordinary[0]]
                .into_iter()
                .chain(lion_moves.iter().copied())
                .collect()
        };
        assert_eq!(
            captures_with_history(&board, nnue, &history, None),
            expected
        );
    }
}
