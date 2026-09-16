//! 第3段階の規定値と境界。期待値は指示書の式から独立に算出する。
use super::*;
use crate::search::{RootMoves, tt::Bound};
use crate::{Game, Rules};

fn moves() -> Vec<Move> {
    Game::new(
        Rules::from_codes(&[
            crate::RuleCode::L0,
            crate::RuleCode::P0,
            crate::RuleCode::R1,
            crate::RuleCode::E0,
        ])
        .unwrap(),
    )
    .legal_moves()
}
fn budget() -> TimeBudget {
    TimeBudget {
        adaptive: true,
        soft: Duration::from_millis(100),
        hard: Duration::from_millis(500),
    }
}
fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "actual={actual}, expected={expected}"
    );
}

#[test]
fn coefficients_match_representative_values_and_interpolation_endpoints() {
    close(interpolate(-1.0, 0.0, 10.0, 2.0, 4.0), 2.0);
    close(interpolate(0.0, 0.0, 10.0, 2.0, 4.0), 2.0);
    close(interpolate(5.0, 0.0, 10.0, 2.0, 4.0), 3.0);
    close(interpolate(10.0, 0.0, 10.0, 2.0, 4.0), 4.0);
    close(interpolate(11.0, 0.0, 10.0, 2.0, 4.0), 4.0);
    let mut signals = TimeSignals::new(TimeHistory::default());
    let report = signals.complete_iteration(1, moves()[0], 100, 100, 0, 1, 10, Some(budget()));
    close(report.falling, 0.576);
    close(report.instability, 1.077);
    close(report.history.time_reduction, 0.639);
    close(report.reduction, 2.468 / (2.284 * 0.639));
    close(report.effort, 0.838);
    close(
        report.total_ms,
        100.0 * 0.48 * 0.576 * (2.468 / (2.284 * 0.639)) * 1.077 * 0.838,
    );
}

#[test]
fn falling_evaluation_uses_pawn_units_and_previous_go_average() {
    for (pawn, score, average) in [(100, 80, 100.0), (200, 160, 200.0)] {
        let mut signals = TimeSignals::new(TimeHistory {
            average_score: Some(average),
            time_reduction: 1.2,
        });
        let report =
            signals.complete_iteration(1, moves()[0], score, pawn, 0, 100, 100, Some(budget()));
        close(report.falling, 1.0716);
        close(report.reduction, 2.668 / (2.284 * 0.639));
        close(report.effort, 0.969 - 0.255 * 24_200.0 / 28_710.0);
    }
}

#[test]
fn evaluation_ring_reads_four_iterations_ago_and_pads_with_first_score() {
    let mut signals = TimeSignals::new(TimeHistory::default());
    let mv = moves()[0];
    let scores = [100, 90, 80, 70, 60, 50];
    let expected = [0.576, 0.576, 0.576, 0.8012, 1.03, 1.03];
    for (index, (score, falling)) in scores.into_iter().zip(expected).enumerate() {
        let report =
            signals.complete_iteration(index as u32 + 1, mv, score, 100, 0, 1, 10, Some(budget()));
        close(report.falling, falling);
    }
}

#[test]
fn instability_decays_and_last_change_depth_resets_only_on_completed_best_change() {
    let mut signals = TimeSignals::new(TimeHistory::default());
    let moves = moves();
    let first = signals.complete_iteration(1, moves[0], 0, 100, 2, 1, 10, Some(budget()));
    close(first.instability, 5.535);
    let second = signals.complete_iteration(2, moves[0], 0, 100, 1, 1, 10, Some(budget()));
    close(second.instability, 5.535);
    let third = signals.complete_iteration(3, moves[0], 0, 100, 0, 1, 10, Some(budget()));
    close(third.instability, 3.306);
    let late = signals.complete_iteration(30, moves[0], 0, 100, 0, 1, 10, Some(budget()));
    close(late.history.time_reduction, 1.544);
    let changed = signals.complete_iteration(31, moves[1], 0, 100, 0, 1, 10, Some(budget()));
    close(changed.history.time_reduction, 0.639);
}

#[test]
fn stopping_uses_strict_minimum_of_total_and_hard() {
    let budget = budget();
    let mut report = TimeReport::new(Some(budget), TimeHistory::default());
    for (total, threshold) in [(50.0, 50), (800.0, 500)] {
        report.total_ms = total;
        assert!(!report.should_stop(Duration::from_millis(threshold - 1), budget));
        assert!(!report.should_stop(Duration::from_millis(threshold), budget));
        assert!(report.should_stop(
            Duration::from_millis(threshold) + Duration::from_nanos(1),
            budget
        ));
    }
}

#[test]
fn fixed_time_ignores_adaptive_coefficients_even_with_clock() {
    let budget = TimeBudget {
        adaptive: false,
        ..budget()
    };
    let mut signals = TimeSignals::new(TimeHistory {
        average_score: Some(1000.0),
        time_reduction: 1.0,
    });
    let report = signals.complete_iteration(1, moves()[0], -1000, 100, 10, 1, 10, Some(budget));
    close(report.total_ms, 100.0);
    close(
        report.falling * report.reduction * report.instability * report.effort,
        1.0,
    );
    assert!(!report.should_stop(Duration::from_millis(100), budget));
    assert!(report.should_stop(Duration::from_millis(101), budget));
}

#[test]
fn root_averages_update_once_per_complete_iteration_and_nodes_accumulate_across_windows() {
    let moves = moves();
    let mut roots = RootMoves::new(&moves[..2]);
    roots.begin_iteration();
    roots.record(moves[0], Some(100), (-1000, 1000), &[], 10);
    roots.record(moves[1], Some(50), (-1000, 1000), &[], 20);
    assert_eq!(roots.entry(moves[0]).average_score, None);
    roots.complete_iteration();
    assert_eq!(roots.entry(moves[0]).average_score, Some(100.0));
    assert_eq!(roots.entry(moves[1]).average_score, Some(50.0));
    roots.begin_iteration();
    roots.begin_window();
    roots.record(moves[0], Some(300), (0, 200), &[], 30);
    roots.begin_window();
    roots.record(moves[0], Some(200), (-1000, 1000), &[], 40);
    roots.record(moves[1], Some(-50), (-1000, 1000), &[], 50);
    roots.complete_iteration();
    assert_eq!(roots.entry(moves[0]).average_score, Some(150.0));
    assert_eq!(roots.entry(moves[1]).average_score, Some(0.0));
    assert_eq!(roots.entry(moves[0]).total_nodes, 80);
    assert_eq!(roots.entry(moves[0]).nodes, 70);
    roots.begin_iteration();
    roots.begin_window();
    roots.record(moves[0], None, (-1000, 1000), &[], 60);
    assert_eq!(roots.entry(moves[0]).average_score, Some(150.0));
    assert_eq!(roots.entry(moves[0]).total_nodes, 140);
    assert_eq!(roots.entry(moves[1]).score, None);
    assert_ne!(roots.entry(moves[0]).score, Some((200, Bound::Exact)));
}
