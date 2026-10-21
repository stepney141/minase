//! 静止探索を検査する。

use super::*;
use crate::search::alphabeta::capture_history::CaptureHistory;

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

// stand-pat、最大ply、候補なしの出口では置換表を参照も更新もしない。
#[test]
fn quiescence_early_exits_do_not_probe_or_store() {
    let nnue = test_nnue();
    let initial = Position::initial();
    let initial_eval = evaluate(nnue, &initial);
    let kings = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let capture = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(3, 10), Color::Black, PieceKind::Rook),
            (fs(3, 4), Color::White, PieceKind::Pawn),
        ],
    );
    assert_eq!(nnue.pawn_value(), 100, "固定ネットの歩兵価値");
    // 捕獲する歩兵の価値と、現在のdelta余裕値の和を境界にする。
    let threshold = evaluate(nnue, &capture)
        + nnue.pawn_value()
        + nnue.pawn_value() * crate::search::alphabeta::params::delta_margin() / 100;
    for (case, board, alpha, beta, ply, stored, depth) in [
        (
            "stand_pat_equal",
            &initial,
            -INFINITY,
            initial_eval,
            0,
            initial_eval + 123,
            4,
        ),
        (
            "stand_pat_above",
            &initial,
            -INFINITY,
            initial_eval - 1,
            0,
            initial_eval + 123,
            4,
        ),
        ("max_ply", &kings, -INFINITY, INFINITY, MAX_PLY, 12_345, 8),
        (
            "no_captures",
            &initial,
            -INFINITY,
            INFINITY,
            0,
            initial_eval + 123,
            4,
        ),
        (
            "filtered_captures",
            &capture,
            threshold,
            INFINITY,
            0,
            evaluate(nnue, &capture) + 123,
            4,
        ),
    ] {
        let key = search_key(board);
        let stand_pat = evaluate(nnue, board);
        let table = small_tt();
        assert_eq!(
            run_quiesce(board, alpha, beta, ply, &table),
            (stand_pat, 0),
            "{case}"
        );
        assert!(table.probe(key, ply).is_none(), "{case}");
        table.store(key, depth, stored, None, Bound::Exact, None, ply, false);
        let raw_before = table.raw_entry(key);
        assert_eq!(
            run_quiesce(board, alpha, beta, ply, &table),
            (stand_pat, 0),
            "{case}"
        );
        assert_ne!(stand_pat, stored, "{case}");
        assert_eq!(table.raw_entry(key), raw_before, "{case}");
        let hit = table.probe(key, ply).unwrap();
        assert_eq!(
            (
                hit.value.unwrap().score,
                u32::from(hit.value.unwrap().depth),
                hit.value.unwrap().bound
            ),
            (stored, depth, Bound::Exact),
            "{case}"
        );
    }
}

// Exact、
// score >= betaのLower、score <= alphaのUpperは深さ条件なしで返す。
#[test]
fn quiescence_tt_cuts_off_all_three_bounds_at_inclusive_edges() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(3, 10), Color::Black, PieceKind::Rook),
            (fs(3, 4), Color::White, PieceKind::Pawn),
        ],
    );
    let key = search_key(&position);
    let stand_pat = evaluate(test_nnue(), &position);
    let cases = [
        (Bound::Exact, stand_pat + 17, stand_pat - 10, stand_pat + 10),
        (
            Bound::Lower,
            stand_pat + 50,
            stand_pat - 100,
            stand_pat + 50,
        ),
        (
            Bound::Upper,
            stand_pat - 50,
            stand_pat - 50,
            stand_pat + 100,
        ),
    ];

    for (bound, score, alpha, beta) in cases {
        let table = small_tt();
        table.store(key, 0, score, None, bound, None, 0, false);
        let (actual, _) = run_quiesce(&position, alpha, beta, 0, &table);
        assert_eq!(actual, score);
    }
}

// 静止探索の通常出口は深さ0で記録され、同じ入口局面を
// 再訪するとExactヒットで捕獲展開を省く。
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
    assert_eq!(hit.value.unwrap().bound, Bound::Exact);
    let stored_move = hit.best_move.expect("捕獲手が最善なら記録手を持つ");
    assert_eq!(stored_move.from, capture.from);
    assert_eq!(stored_move.to, capture.to);
    assert_eq!(hit.value.unwrap().score, first_score);
    assert_eq!(hit.static_eval, Some(evaluate(test_nnue(), &position)));

    let (second_score, second_nodes) = run_quiesce(&position, -INFINITY, INFINITY, 0, &table);
    assert_eq!(second_score, first_score);
    assert_eq!(first_nodes, 1);
    assert_eq!(second_nodes, 0);
}

// 通常出口のUpper・Exactを深さ0で記録し、
// stand-patが最善なら手なしとする。
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
        ("exact", stand_pat - 1, stand_pat + 1, Bound::Exact),
        ("all_see_pruned", -INFINITY, INFINITY, Bound::Exact),
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
    assert!(buffers.initialize(board, &generator, nnue, &ranks, -1));
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
