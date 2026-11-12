//! 評価を読む時点まで累算を保留し、実着手とnull moveの復元を保つ契約。

use super::super::searcher::{AccumulatorUpdate, PendingAccumulator};
use super::*;

#[test]
fn lazy_accumulator_skips_normal_and_qsearch_tt_cutoffs() {
    let net = epoch_zero_nnue();
    let root = Position::initial();
    let mv = legal_moves(&root)[0];
    let mut child = root.clone();
    child.make_move_unchecked(mv, engine_rules());
    for depth in [0, 1] {
        with_nnue_root_searcher(&net, &root, &[], |searcher| {
            searcher.tt.store(
                search_key(&child),
                8,
                1234,
                None,
                Bound::Exact,
                None,
                1,
                false,
            );
            net.take_accumulator_stats();
            let mut current = root.clone();
            assert_eq!(
                searcher.search_move(&mut current, mv, 2, 0, 1, 0, true, depth, None, false),
                Some(-1234)
            );
            assert_eq!(net.take_accumulator_stats().updates(), 0);
            assert_eq!(current, root);
            assert!(searcher.pending_accumulators.iter().all(Option::is_none));
        });
    }
}

#[test]
fn lazy_accumulator_skips_tt_static_eval_when_stand_pat_cuts_off() {
    let net = epoch_zero_nnue();
    let root = Position::initial();
    let mv = legal_moves(&root)[0];
    let mut child = root.clone();
    child.make_move_unchecked(mv, engine_rules());
    with_nnue_root_searcher(&net, &root, &[], |searcher| {
        assert!(
            searcher
                .tt
                .store_static_eval(search_key(&child), 1000, false)
        );
        net.take_accumulator_stats();
        let mut current = root.clone();
        assert!(
            searcher
                .search_move(&mut current, mv, 1, 0, 1, 0, true, 0, None, false)
                .is_some()
        );
        assert_eq!(net.take_accumulator_stats().updates(), 0);
        assert_eq!(searcher.nodes, 1);
        assert_eq!(current, root);
        assert!(searcher.pending_accumulators.iter().all(Option::is_none));
    });
}

#[test]
fn lazy_accumulator_computes_tt_static_eval_parent_before_descending() {
    let net = epoch_zero_nnue();
    let root = Position::initial();
    let mv = legal_moves(&root)[0];
    let mut child = root.clone();
    child.make_move_unchecked(mv, engine_rules());
    let expected = net.refresh_accumulator(&child);
    with_nnue_root_searcher(&net, &root, &[], |searcher| {
        assert!(searcher.tt.store_static_eval(
            search_key(&child),
            net.evaluate_accumulator(&expected, child.side_to_move()),
            false
        ));
        net.take_accumulator_stats();
        let mut current = root.clone();
        assert!(
            searcher
                .search_move(
                    &mut current,
                    mv,
                    2,
                    -INFINITY,
                    INFINITY,
                    0,
                    true,
                    1,
                    None,
                    false
                )
                .is_some()
        );
        assert!(searcher.nodes > 1);
        assert!(net.take_accumulator_stats().incremental_updates > 0);
        assert_eq!(searcher.accumulators[1], expected);
        assert_eq!(current, root);
        assert!(searcher.pending_accumulators.iter().all(Option::is_none));
    });
}

#[test]
fn lazy_accumulator_skips_null_tt_cutoff_and_applies_lion_expiry_on_read() {
    let net = epoch_zero_nnue();
    let mut root = position(
        Color::Black,
        &[
            (sq(0, 11), Color::Black, PieceKind::King),
            (sq(11, 0), Color::White, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Rook),
            (sq(5, 3), Color::White, PieceKind::Lion),
        ],
    );
    root.make_move_unchecked(step(sq(5, 5), sq(5, 3)), engine_rules());
    assert!(root.lion_taken_by_non_lion().is_some());
    let mut child = root.clone();
    child.make_null_move();
    let expected = net.refresh_accumulator(&child);
    for cutoff in [true, false] {
        with_nnue_root_searcher(&net, &root, &[], |searcher| {
            if cutoff {
                searcher.tt.store(
                    search_key(&child),
                    8,
                    1234,
                    None,
                    Bound::Exact,
                    None,
                    1,
                    false,
                );
            }
            net.take_accumulator_stats();
            let mut current = root.clone();
            assert!(searcher.search_null_move(&mut current, 0, 0, 0).is_some());
            let counts = net.take_accumulator_stats();
            if cutoff {
                assert_eq!(counts.updates(), 0);
            } else {
                assert!(counts.incremental_updates > 0);
                assert_eq!(searcher.accumulators[1], expected);
            }
            assert_eq!(current, root);
            assert!(searcher.pending_accumulators.iter().all(Option::is_none));
        });
    }
}

#[test]
fn lazy_accumulator_matches_eager_update_once_per_move_across_rules() {
    let net = epoch_zero_nnue();
    for specification in [
        "JCSA",
        "lishogi",
        "L1,L3,P0,P5,P6,R2,E1,E2",
        "L0,P1,R1,E0",
        "L1,L2,P2,P3,P4,R2,E0",
    ] {
        let rules = minase_core::Rules::from_codes(
            &minase_core::rules::parse_rule_set(specification).unwrap(),
        )
        .unwrap();
        let positions = minase_core::test_util::sampled_random_positions(rules.moves)
            .into_iter()
            .chain(
                minase_core::test_util::CAPTURE_EDGE_SFENS
                    .iter()
                    .map(|sfen| minase_core::parse_sfen(sfen).unwrap()),
            );
        for root in positions {
            let mut moves = Vec::new();
            MoveGenerator::new(rules.moves).generate_moves(&root, &mut moves);
            with_nnue_root_searcher(&net, &root, &[], |searcher| {
                for mv in moves {
                    let mut child = root.clone();
                    let undo = child.make_move_unchecked(mv, rules.moves);
                    // 従来の、子へ進む前に無条件で更新する処理を参照とする。
                    let mut expected = searcher.accumulators[0].clone();
                    net.update_accumulator_after_move(
                        &searcher.accumulators[0],
                        &mut expected,
                        &child,
                        &undo,
                    );
                    searcher.pending_accumulators[1] = Some(PendingAccumulator {
                        update: AccumulatorUpdate::Move(undo),
                        computed: false,
                    });
                    net.take_accumulator_stats();
                    assert_eq!(
                        searcher.evaluate_at(&child, 1),
                        net.evaluate_accumulator(&expected, child.side_to_move())
                    );
                    assert_eq!(searcher.accumulators[1], expected);
                    assert_eq!(net.take_accumulator_stats().incremental_updates, 1);
                    searcher.evaluate_at(&child, 1);
                    assert_eq!(net.take_accumulator_stats().updates(), 0);
                    child.unmake_move(searcher.take_accumulator_move(1));
                    assert_eq!(child, root);
                }
            });
        }
    }
}
