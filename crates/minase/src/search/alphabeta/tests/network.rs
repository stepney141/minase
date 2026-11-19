//! 構造の異なるネットで探索の入口と差し替えの契約を検査する。
use super::*;
use crate::eval::nnue::{
    input::{AnchorError, FeatureSet},
    network::test_util::fixed_simple,
};
use crate::search::{HistoryTables, SearchOptionValue, StartError};

struct NetworkPath(std::path::PathBuf);
impl NetworkPath {
    fn new(network: &Network, tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("minase-phase2b-{}-{tag}.bin", std::process::id()));
        std::fs::write(&path, network.encode()).unwrap();
        Self(path)
    }
}
impl Drop for NetworkPath {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).unwrap();
    }
}
fn prepared(path: &std::path::Path) -> crate::search::Searcher {
    let mut engine = crate::search::Searcher::alpha_beta();
    engine
        .set_option("USI_Hash", SearchOptionValue::Spin(1))
        .unwrap();
    engine
        .set_option(
            "EvalFile",
            SearchOptionValue::String(path.to_str().unwrap().to_owned()),
        )
        .unwrap();
    engine.prepare().unwrap();
    engine
}
fn finish(engine: crate::search::Searcher, snapshot: SearchSnapshot) -> crate::search::Searcher {
    let handle = engine.start(snapshot, depth_limits(1), 1, false).unwrap();
    loop {
        if matches!(
            handle.events().recv().unwrap(),
            SearchEvent::Finished { .. }
        ) {
            break;
        }
    }
    handle.join().unwrap()
}

#[test]
fn both_structures_search_all_bench_positions_to_depth_three() {
    for network in [fixed_network(), Network::Simple(fixed_simple())] {
        for (index, position) in minase_core::test_util::bench_positions().iter().enumerate() {
            network
                .check_search_root(position)
                .unwrap_or_else(|error| panic!("bench position {}: {error}", index + 1));
            let snapshot = snapshot_for(position);
            let result = crate::search::search(
                &network,
                &snapshot,
                &depth_limits(3),
                DEFAULT_THREADS,
                &mut small_tt(),
                &mut HistoryTables::new(DEFAULT_THREADS),
            )
            .unwrap();
            assert_eq!(result.depth, 3, "bench position {}", index + 1);
            assert!(legal_moves(position).contains(&result.best_move));
        }
    }
}

#[test]
fn ambiguous_prince_candidates_are_rejected_at_both_search_entries() {
    let piece = |kind| PieceCode::new(Color::Black, kind).unwrap();
    let position = minase_core::test_util::position_from_codes(
        Color::Black,
        &[
            (sq(0, 11), piece(PieceKind::King)),
            (
                sq(1, 10),
                PieceCode::new_promoted(Color::Black, PieceKind::CrownPrince).unwrap(),
            ),
            (sq(2, 10), piece(PieceKind::DrunkElephant)),
            (
                sq(11, 0),
                PieceCode::new(Color::White, PieceKind::King).unwrap(),
            ),
        ],
    );
    let expected = AnchorError::PrinceCandidates {
        color: Color::Black,
    };
    for network in [fixed_network(), Network::Simple(fixed_simple())] {
        let simple = network.feature_set() == FeatureSet::HalfKpVm;
        let result = crate::search::search(
            &network,
            &snapshot_for(&position),
            &depth_limits(1),
            DEFAULT_THREADS,
            &mut small_tt(),
            &mut HistoryTables::new(DEFAULT_THREADS),
        );
        if simple {
            assert_eq!(result.unwrap_err(), SearchError::UnsupportedRoot(expected));
        } else {
            assert!(legal_moves(&position).contains(&result.unwrap().best_move));
        }
        let path = NetworkPath::new(&network, "root");
        let engine = prepared(&path.0);
        if simple {
            let error = engine
                .start(snapshot_for(&position), depth_limits(1), 1, false)
                .err()
                .unwrap()
                .error;
            assert!(matches!(error, StartError::UnsupportedRoot(cause) if cause == expected));
        } else {
            finish(engine, snapshot_for(&position));
        }
    }
}

#[test]
fn eval_file_switch_discards_network_table_and_every_history() {
    let layer = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/nets/test-layer-stack.bin"
    ));
    let simple = NetworkPath::new(&Network::Simple(fixed_simple()), "switch");
    let position = Position::initial();
    let mut engine = finish(prepared(layer), snapshot_for(&position));
    let key = search_key(&position);
    assert!(
        engine
            .transposition_table()
            .unwrap()
            .probe(key, 0)
            .is_some()
    );
    // 深さ1で更新されない表も含め、全表に識別できる値を置く。
    let histories = engine.histories_mut_for_test();
    for worker in &mut histories.workers {
        for value in worker.iter_mut().flatten().flatten() {
            *value = 123;
        }
    }
    for history in &mut histories.capture {
        history.values_mut().fill(123);
    }
    for history in &mut histories.continuation_correction {
        history.values_mut().fill(123);
    }
    for values in [
        histories.correction.values(),
        histories.continuation.values(),
        histories.pawn.values(),
    ] {
        for value in values {
            value.store(123, AtomicOrdering::Relaxed);
        }
    }
    engine
        .set_option(
            "EvalFile",
            SearchOptionValue::String(simple.0.to_str().unwrap().to_owned()),
        )
        .unwrap();
    assert!(engine.network().is_none());
    assert!(
        engine
            .transposition_table()
            .unwrap()
            .probe(key, 0)
            .is_none()
    );
    let actual = engine.histories_for_test();
    let fresh = HistoryTables::new(DEFAULT_THREADS);
    assert_eq!(actual.workers, fresh.workers);
    for (a, b) in actual.capture.iter().zip(&fresh.capture) {
        assert_eq!(a.values(), b.values());
    }
    for (a, b) in actual
        .continuation_correction
        .iter()
        .zip(&fresh.continuation_correction)
    {
        assert_eq!(a.values(), b.values());
    }
    for (a, b) in [
        (actual.correction.values(), fresh.correction.values()),
        (actual.continuation.values(), fresh.continuation.values()),
        (actual.pawn.values(), fresh.pawn.values()),
    ] {
        assert_eq!(a.len(), b.len());
        assert!(
            a.iter()
                .zip(b)
                .all(|(a, b)| a.load(AtomicOrdering::Relaxed) == b.load(AtomicOrdering::Relaxed))
        );
    }
    assert_eq!(actual.previous_search.score_and_average, None);
    engine.prepare().unwrap();
    assert_eq!(
        engine.network().unwrap().feature_set(),
        FeatureSet::HalfKpVm
    );
    let engine = finish(engine, snapshot_for(&position));
    assert_eq!(
        engine.network().unwrap().feature_set(),
        FeatureSet::HalfKpVm
    );
    assert!(engine.network().unwrap().take_accumulator_stats().updates() > 0);
}
