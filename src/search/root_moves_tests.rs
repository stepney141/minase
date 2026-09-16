//! 根の結果保持の検証。期待値はtime-management-efficiency.mdの第0段階に従う。

use super::*;
use crate::core::piece::{Color, PieceKind};
use crate::eval::weights;
use crate::test_util::{position, sq};

/// 離れた王2枚の局面を使い、必要なら全子局面を反復による引き分けにする。
fn with_root_results(
    repeated: bool,
    node_limit: Option<u64>,
    test: impl FnOnce(&Position, &[Move], &mut Searcher<'_>),
) {
    let position = position(
        Color::Black,
        &[
            (sq(5, 0), Color::Black, PieceKind::King),
            (sq(5, 11), Color::White, PieceKind::King),
        ],
    );
    let rules = MoveRules::standard();
    let mut moves = Vec::new();
    MoveGenerator::new(rules).generate_moves(&position, &mut moves);
    let history: Vec<_> = if repeated {
        moves
            .iter()
            .map(|&mv| {
                let mut child = position.clone();
                child.make_move_unchecked(mv, rules);
                search_key(&child)
            })
            .collect()
    } else {
        vec![search_key(&position)]
    };
    let external_stop = AtomicBool::new(false);
    let shared = SharedSearch {
        external_stop: &external_stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        node_limit,
        started: Instant::now(),
        hard_limit: None,
    };
    let pst = weights().unwrap();
    let tt = TranspositionTable::new(1).unwrap();
    let mut results = RootMoves::new(&moves);
    let mut searcher = new_searcher(&pst, &position, rules, &history, &shared, &tt);
    searcher.root_results = Some(&mut results);
    test(&position, &moves, &mut searcher);
}

// 同第0段階。各手の探索窓に対する境界を記録し、β打ち切り後は未探索にする。
#[test]
fn root_results_record_bounds_pv_and_nodes_without_reordering() {
    for (alpha, beta, first_bound, cutoff) in [
        (0, 1, Bound::Upper, false),
        (-1, 0, Bound::Lower, true),
        (-1, 1, Bound::Exact, false),
    ] {
        with_root_results(true, None, |position, moves, searcher| {
            let results = searcher.root_results.as_ref().unwrap();
            assert!(results.entries.iter().all(|entry| entry.score.is_none()
                && entry.previous_score.is_none()
                && entry.pv.is_empty()
                && entry.nodes == 0));
            let (best, score) = searcher
                .search_root(position, moves, 1, alpha, beta)
                .unwrap();
            assert_eq!(score, DRAW_SCORE);
            let results = searcher.root_results.as_ref().unwrap();
            assert_eq!(
                results
                    .entries
                    .iter()
                    .map(|entry| entry.mv)
                    .collect::<Vec<_>>(),
                moves
            );
            for entry in &results.entries {
                assert_eq!(entry.previous_score, None);
                if cutoff && entry.mv != best {
                    assert_eq!(entry.score, None);
                    assert!(entry.pv.is_empty());
                    assert_eq!(entry.nodes, 0);
                } else {
                    let bound = if entry.mv == best {
                        first_bound
                    } else {
                        Bound::Upper
                    };
                    assert_eq!(entry.score, Some((DRAW_SCORE, bound)));
                    assert_eq!(entry.pv, [entry.mv]);
                    assert_eq!(entry.nodes, 1);
                }
            }
            assert_eq!(
                results.entries.iter().map(|entry| entry.nodes).sum::<u64>(),
                searcher.nodes - 1
            );
        });
    }
}

// 各手の主変化は根の着手だけでなく、その後の応手も保持する。
#[test]
fn root_results_preserve_the_best_moves_full_pv() {
    with_root_results(false, None, |position, moves, searcher| {
        let (best, score) = searcher.search_iteration(position, moves, 3, None).unwrap();
        let results = searcher.root_results.as_ref().unwrap();
        let entry = results
            .entries
            .iter()
            .find(|entry| entry.mv == best)
            .unwrap();
        assert_eq!(entry.score, Some((score, Bound::Exact)));
        assert_eq!(entry.pv, searcher.pv[0]);
        assert!(entry.pv.len() >= 2, "fixture must include a reply");
        let mut child = position.clone();
        for &mv in &entry.pv {
            let mut legal = Vec::new();
            searcher.generator.generate_moves(&child, &mut legal);
            assert!(legal.contains(&mv));
            child.make_move_unchecked(mv, searcher.rules);
        }
    });
}

// 窓の再探索は同じ反復であり、直前反復の値を上書きせずノード数を合算する。
#[test]
fn root_results_carry_previous_iteration_across_aspiration_research() {
    with_root_results(true, None, |position, moves, searcher| {
        searcher.search_iteration(position, moves, 1, None).unwrap();
        let before = searcher.nodes;
        let delta = searcher.pst.pawn_value() / 2;
        let (best, _) = searcher
            .search_iteration(position, moves, 5, Some(-delta))
            .unwrap();
        let results = searcher.root_results.as_ref().unwrap();
        for entry in &results.entries {
            assert_eq!(entry.previous_score, Some(DRAW_SCORE));
            assert_eq!(entry.pv, [entry.mv]);
            let bound = if entry.mv == best {
                Bound::Exact
            } else {
                Bound::Upper
            };
            assert_eq!(entry.score, Some((DRAW_SCORE, bound)));
            assert_eq!(entry.nodes, if entry.mv == best { 2 } else { 1 });
        }
        assert_eq!(searcher.nodes - before, moves.len() as u64 + 3);
    });
}

// 中断した手の評価値と主変化は採用せず、その手が消費したノードは残す。
#[test]
fn root_results_count_interrupted_move_without_marking_it_complete() {
    with_root_results(false, Some(2), |position, moves, searcher| {
        assert_eq!(searcher.search_iteration(position, moves, 3, None), None);
        let results = searcher.root_results.as_ref().unwrap();
        assert!(results.entries.iter().all(|entry| entry.score.is_none()
            && entry.previous_score.is_none()
            && entry.pv.is_empty()));
        assert_eq!(
            results.entries.iter().map(|entry| entry.nodes).sum::<u64>(),
            1
        );
        assert_eq!(searcher.nodes, 2);
        assert_eq!(searcher.stop_reason, Some(StopReason::NodeLimit));
    });
}

// 次の窓の根へ入る前に停止しても、前の窓の評価値と主変化を残さない。
#[test]
fn root_results_clear_previous_window_even_when_root_is_interrupted() {
    with_root_results(true, None, |position, moves, searcher| {
        searcher.search_iteration(position, moves, 1, None).unwrap();
        searcher.shared.stop(StopReason::HardLimit);
        assert_eq!(searcher.search_iteration(position, moves, 2, Some(0)), None);
        let results = searcher.root_results.as_ref().unwrap();
        assert!(results.entries.iter().all(|entry| entry.score.is_none()
            && entry.previous_score == Some(DRAW_SCORE)
            && entry.pv.is_empty()
            && entry.nodes == 0));
    });
}
