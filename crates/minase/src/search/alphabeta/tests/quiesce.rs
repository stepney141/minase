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

// β以上の静的評価は置換表を照合も保存もしない。
#[test]
fn quiescence_stand_pat_cutoff_does_not_probe_or_store() {
    let position = Position::initial();
    let stand_pat = evaluate(test_nnue(), &position);
    let key = search_key(&position);
    for beta in [stand_pat, stand_pat - 1] {
        let table = small_tt();
        let (score, nodes) = run_quiesce(&position, -INFINITY, beta, 0, &table);
        assert_eq!((score, nodes), (stand_pat, 0));
        assert!(table.probe(key, 0).is_none());
        table.store(key, 4, stand_pat + 123, Bound::Exact, None, 0);
        let (score, nodes) = run_quiesce(&position, -INFINITY, beta, 0, &table);
        assert_eq!((score, nodes), (stand_pat, 0));
        let hit = table.probe(key, 0).unwrap();
        assert_eq!(
            (hit.score, hit.depth, hit.bound),
            (stand_pat + 123, 4, Bound::Exact)
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
        table.store(key, 0, score, bound, None, 0);
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
    assert_eq!(hit.depth, 0);
    assert_eq!(hit.bound, Bound::Exact);
    let stored_move = hit.best_move.expect("捕獲手が最善なら記録手を持つ");
    assert_eq!(stored_move.from, capture.from);
    assert_eq!(stored_move.to, capture.to);
    assert_eq!(hit.score, first_score);

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
    let cases = [
        (stand_pat, INFINITY, Bound::Upper),
        (stand_pat - 1, stand_pat + 1, Bound::Exact),
    ];

    for (alpha, beta, expected_bound) in cases {
        let table = small_tt();
        let (score, _) = run_quiesce(&position, alpha, beta, 0, &table);
        let hit = table.probe(key, 0).unwrap();
        assert_eq!(score, stand_pat);
        assert_eq!(hit.depth, 0);
        assert_eq!(hit.bound, expected_bound);
        assert_eq!(hit.best_move, None);
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
        assert_eq!(hit.score, score);
        assert_eq!(hit.depth, 0);
        assert_eq!(hit.best_move, Some(mate_move));
        // 同じ局面を根から評価すると、捕獲までの距離は1手になる。
        assert_eq!(
            table.probe(search_key(&position), 0).unwrap().score,
            MATE - 1
        );
    }
}

// 最大ply葉は置換表照会より先に静的評価を返し、既存の
// エントリを参照も更新もしない。
#[test]
fn quiescence_max_ply_leaf_does_not_probe_or_store() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let key = search_key(&position);
    let table = small_tt();
    table.store(key, 8, 12_345, Bound::Exact, None, MAX_PLY);
    let raw_before = table.raw_entry(key);

    let (score, _) = run_quiesce(&position, -INFINITY, INFINITY, MAX_PLY, &table);
    assert_eq!(score, evaluate(test_nnue(), &position));
    assert_ne!(score, 12_345);
    assert_eq!(table.raw_entry(key), raw_before);
}

// 合法な捕獲がない場合と、入口の閾値で
// 全捕獲が消える場合は、既存の置換表の値にかかわらず静的評価を返す。
#[test]
fn quiescence_empty_candidates_do_not_probe_or_store() {
    let nnue = test_nnue();
    let capture_position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(3, 10), Color::Black, PieceKind::Rook),
            (fs(3, 4), Color::White, PieceKind::Pawn),
        ],
    );
    let stand_pat = evaluate(nnue, &capture_position);
    for (position, alpha) in [
        (Position::initial(), -INFINITY),
        (
            capture_position,
            stand_pat
                + nnue.pawn_value()
                + nnue.pawn_value() * crate::search::alphabeta::params::delta_margin() / 100,
        ),
    ] {
        let key = search_key(&position);
        let stand_pat = evaluate(nnue, &position);
        let table = small_tt();
        let (score, nodes) = run_quiesce(&position, alpha, INFINITY, 0, &table);
        assert_eq!((score, nodes), (stand_pat, 0));
        assert!(table.probe(key, 0).is_none());
        table.store(key, 4, stand_pat + 123, Bound::Exact, None, 0);
        let (score, nodes) = run_quiesce(&position, alpha, INFINITY, 0, &table);
        assert_eq!((score, nodes), (stand_pat, 0));
        let hit = table.probe(key, 0).unwrap();
        assert_eq!((hit.score, hit.depth), (stand_pat + 123, 4));
    }
}

// stand-patは損な捕獲より優先される。
// 候補があり、SEEで全て捨てるノードは保存する。
#[test]
fn quiescence_see_pruned_candidates_still_store() {
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
    let nnue = test_nnue();
    let mut captures = Vec::new();
    MoveGenerator::standard().generate_captures(&position, &mut captures);
    assert!(!captures.is_empty());
    assert!(captures.iter().all(|&mv| capture_is_pruned_by_see(
        &position,
        engine_rules(),
        nnue,
        mv
    )));
    let table = small_tt();
    let stand_pat = evaluate(nnue, &position);
    let (score, nodes) = run_quiesce(&position, -INFINITY, INFINITY, 0, &table);
    assert_eq!((score, nodes), (stand_pat, 0));
    let hit = table.probe(search_key(&position), 0).unwrap();
    assert_eq!(
        (hit.score, hit.bound, hit.best_move),
        (stand_pat, Bound::Exact, None)
    );
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

// 生の履歴を優先し、同点なら攻撃駒価値と生成順を保つ。
// 採用理由はdocs/adr/bound-capture-history-and-use-it-in-qsearch.md。
#[test]
fn quiescence_capture_history_orders_within_group_using_raw_values() {
    let (board, moves) = history_ordering_position();
    let nnue = test_nnue();
    let mut history = CaptureHistory::new();
    assert_eq!(
        captures_with_history(&board, nnue, &history, None),
        [moves[5], moves[0], moves[1], moves[2], moves[3], moves[4]]
    );
    history.record_cutoff(&board, nnue, moves[4], &[moves[0]], 1);
    assert_eq!(history.read(&board, nnue, moves[4]), 1);
    assert_eq!(history.read(&board, nnue, moves[0]), -1);
    // 歩兵価値への換算では両方0になる小さな値でも順序が変わる。
    assert_eq!(history.adjustment(&board, nnue, moves[4]), 0);
    assert_eq!(history.adjustment(&board, nnue, moves[0]), 0);
    assert_eq!(
        captures_with_history(&board, nnue, &history, None),
        [moves[5], moves[4], moves[1], moves[2], moves[3], moves[0]]
    );
}

// 正負の履歴があっても捕獲価値の群を越えない。
#[test]
fn quiescence_capture_history_preserves_group_boundaries() {
    let (board, moves) = history_ordering_position();
    let nnue = test_nnue();
    let mut history = CaptureHistory::new();
    history.record_cutoff(&board, nnue, moves[4], &[moves[5]], 256);
    assert!(history.read(&board, nnue, moves[4]) > 0);
    assert!(history.read(&board, nnue, moves[5]) < 0);
    assert_eq!(
        captures_with_history(&board, nnue, &history, None),
        [moves[5], moves[4], moves[0], moves[1], moves[2], moves[3]]
    );
}

// 置換表の記録手は群と履歴に優先し、重複しない。
#[test]
fn quiescence_capture_history_keeps_tt_capture_first() {
    let (board, moves) = history_ordering_position();
    let nnue = test_nnue();
    let mut history = CaptureHistory::new();
    history.record_cutoff(&board, nnue, moves[4], &[moves[0]], 256);
    assert_eq!(
        captures_with_history(&board, nnue, &history, Some(moves[0])),
        [moves[0], moves[5], moves[4], moves[1], moves[2], moves[3]]
    );
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
    let history = CaptureHistory::new();
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
        let mut history = CaptureHistory::new();
        if reward_special {
            history.record_cutoff(&board, nnue, lion_moves[0], &ordinary, 1);
        } else {
            history.record_cutoff(&board, nnue, ordinary[1], &[lion_moves[0]], 1);
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
