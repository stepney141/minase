//! 時間配分の算術と入力条件を検査する。

use super::budget::*;
use super::{ClockLimits, SearchLimits};
use std::time::Duration;

fn depth_limits(depth: u32) -> SearchLimits {
    SearchLimits::new(Some(depth), None, None, None).expect("test depth must be valid")
}

fn nodes_limits(nodes: u64) -> SearchLimits {
    SearchLimits::new(None, Some(nodes), None, None).expect("test node limit must be valid")
}

fn infinite_limits() -> SearchLimits {
    SearchLimits::infinite()
}

fn clock_at_ply(remaining_ms: u64, increment_ms: u64, byoyomi_ms: u64, ply: u32) -> ClockLimits {
    ClockLimits::new(
        remaining_ms,
        increment_ms,
        byoyomi_ms,
        Some(remaining_ms),
        ply,
    )
    .expect("test clock must contain non-zero time")
}

// 時計の予算はsoft ≤ hardかつhard ≥ 1 msを満たす。
// 残り時間と秒読みの合計が30 msを超えるとき、hardはその合計から30 msを引いた値以下になる。
#[test]
fn clock_budget_preserves_bounds_over_a_deterministic_grid() {
    let remaining_values = [0, 20, 31, 200, 10_000, 60_000, u64::MAX];
    let increment_values = [0, 100, 1_000, u64::MAX];
    let byoyomi_values = [0, 100, 200, u64::MAX];
    let ply_values = [0, 100, 300, 431, 432, 1_000, u32::MAX];

    for remaining in remaining_values {
        for increment in increment_values {
            for byoyomi in byoyomi_values {
                if remaining == 0 && increment == 0 && byoyomi == 0 {
                    continue;
                }
                for ply in ply_values {
                    let budget = clock_budget(clock_at_ply(remaining, increment, byoyomi, ply));
                    assert!(budget.soft <= budget.hard);
                    assert!(budget.hard >= Duration::from_millis(1));

                    let clock_total = u128::from(remaining) + u128::from(byoyomi);
                    if clock_total > 30 {
                        assert!(budget.hard.as_millis() <= clock_total - 30);
                    }
                }
            }
        }
    }
}

// 秒読みつき時計では、指定余裕を安全上限から引き、予算の下限は1msとする。
#[test]
fn byoyomi_with_remaining_time_changes_only_the_safety_margin() {
    for (remaining, increment, byoyomi, ply) in [
        (60000, 1000, 1000, 0),
        (60000, 1000, 1000, 36),
        (100, 1000, 100, 0),
        (1, 0, 1, 0),
    ] {
        let clock = clock_at_ply(remaining, increment, byoyomi, ply);
        let budget = |margin| {
            time_budget(
                &SearchLimits::new(None, None, None, Some(clock.with_byoyomi_margin_ms(margin)))
                    .unwrap(),
            )
            .unwrap()
        };
        let uncapped = budget(0);
        assert_eq!(budget(30), clock_budget(clock));
        for margin in [
            0,
            30,
            remaining + byoyomi - 1,
            remaining + byoyomi,
            remaining + byoyomi + 1,
        ] {
            let actual = budget(margin);
            let cap = Duration::from_millis((remaining + byoyomi).saturating_sub(margin).max(1));
            assert_eq!(actual.hard, uncapped.hard.min(cap));
            assert_eq!(actual.soft, uncapped.soft.min(cap));
            assert!(!actual.deadline);
        }
    }
}

// 深さやノード数を併用する固定時間の予算は指定値のままで、予測による開始判定を使う。
#[test]
fn movetime_with_depth_or_nodes_keeps_prediction_based_budget() {
    for movetime in [1, 30, 1000, 60_000, u64::MAX] {
        for (depth, nodes) in [(Some(2), None), (None, Some(100)), (Some(2), Some(100))] {
            let limits = SearchLimits::new(depth, nodes, Some(movetime), None).unwrap();
            assert_eq!(
                time_budget(&limits),
                Some(TimeBudget {
                    soft: Duration::from_millis(movetime),
                    hard: Duration::from_millis(movetime),
                    deadline: false,
                })
            );
        }
    }
}

// 時間を指定しない深さ制限、ノード数制限、無限探索は、時間予算を持たない。
#[test]
fn limits_without_time_constraints_have_no_budget() {
    for limits in [depth_limits(2), nodes_limits(100), infinite_limits()] {
        assert_eq!(time_budget(&limits), None);
    }
}

// 固定時間を併用する時計では、相手の残り時間によって予算を変えない。
#[test]
fn movetime_clock_does_not_apply_time_advantage() {
    let coefficients = ClockCoefficients {
        expected_plies: 20,
        min_moves: 10,
        increment_share: 0,
        hard_soft_ratio: 200,
        hard_remaining_share: 50,
        time_advantage: 900,
    };
    // 残り1000 ms、残り10手ではsoft=100 ms、hard=200 msとなる。
    // 秒読み100 msは序盤にsoftへ8 msを足し、持ち時間0なら安全上限70 msで抑える。
    for (remaining, byoyomi, movetime, soft, hard) in [
        (1000, 0, 50, 50, 50),
        (1000, 0, 150, 100, 150),
        (1000, 0, 500, 100, 200),
        (1000, 100, 500, 108, 216),
        (0, 100, 500, 70, 70),
        (0, 100, 50, 50, 50),
    ] {
        let budget = |opponent_remaining_ms| {
            let clock =
                ClockLimits::new(remaining, 0, byoyomi, Some(opponent_remaining_ms), 0).unwrap();
            let limits = SearchLimits::new(None, None, Some(movetime), Some(clock)).unwrap();
            time_budget_with_coefficients(&limits, coefficients).unwrap()
        };
        let expected = TimeBudget {
            soft: Duration::from_millis(soft),
            hard: Duration::from_millis(hard),
            deadline: false,
        };
        assert_eq!(budget(1000), expected);
        assert_eq!(budget(3000), expected);
    }
}

// 秒読みが正の時計では、相手の残り時間によって予算を変えない。
#[test]
fn byoyomi_clock_does_not_apply_time_advantage() {
    let coefficients = ClockCoefficients {
        expected_plies: 20,
        min_moves: 10,
        increment_share: 0,
        hard_soft_ratio: 200,
        hard_remaining_share: 50,
        time_advantage: 900,
    };
    let budget = |opponent_remaining_ms| {
        let clock = ClockLimits::new(1000, 0, 100, Some(opponent_remaining_ms), 0).unwrap();
        let limits = SearchLimits::new(None, None, None, Some(clock)).unwrap();
        time_budget_with_coefficients(&limits, coefficients).unwrap()
    };
    assert_eq!(budget(1000), budget(3000));
    assert_eq!(budget(1000).soft, Duration::from_millis(108));
    assert_eq!(budget(1000).hard, Duration::from_millis(216));
    assert!(!budget(1000).deadline);
    assert!(!budget(3000).deadline);
}

// 深さやノード数を併用しても、秒読みと固定時間がなければtime advantageを適用する。
#[test]
fn depth_and_node_limited_clocks_apply_time_advantage() {
    let coefficients = ClockCoefficients {
        expected_plies: 20,
        min_moves: 10,
        increment_share: 0,
        hard_soft_ratio: 200,
        hard_remaining_share: 50,
        time_advantage: 900,
    };
    for (depth, nodes) in [
        (None, None),
        (Some(2), None),
        (None, Some(100)),
        (Some(2), Some(100)),
    ] {
        let budget = |opponent_remaining_ms| {
            let clock = ClockLimits::new(1000, 0, 0, Some(opponent_remaining_ms), 0).unwrap();
            let limits = SearchLimits::new(depth, nodes, None, Some(clock)).unwrap();
            time_budget_with_coefficients(&limits, coefficients).unwrap()
        };
        assert_eq!(budget(1000).soft, Duration::from_millis(100));
        assert_eq!(budget(1000).hard, Duration::from_millis(200));
        assert_eq!(budget(3000).soft, Duration::from_millis(55));
        assert_eq!(budget(3000).hard, Duration::from_millis(110));
        assert!(!budget(1000).deadline);
        assert!(!budget(3000).deadline);
    }
}

/// time advantageはsoft_rawとその倍数で決まるhardだけを縮める。
/// 残り時間の割合と安全上限で決まるhardは変えない。
#[test]
fn time_advantage_scales_soft_and_only_the_soft_multiple_hard_candidate() {
    let coefficients = |hard_remaining_share| ClockCoefficients {
        expected_plies: 20,
        min_moves: 10,
        increment_share: 0,
        hard_soft_ratio: 200,
        hard_remaining_share,
        time_advantage: 900,
    };
    let clock = |opponent_remaining_ms| {
        ClockLimits::new(1000, 0, 0, Some(opponent_remaining_ms), 0).unwrap()
    };

    let neutral = coefficients(50).budget(clock(1000), 30);
    let scaled = coefficients(50).budget(clock(3000), 30);
    assert_eq!(neutral.soft, Duration::from_millis(100));
    assert_eq!(neutral.hard, Duration::from_millis(200));
    assert_eq!(scaled.soft, Duration::from_millis(55));
    assert_eq!(scaled.hard, Duration::from_millis(110));

    let remaining_limited = coefficients(10);
    assert_eq!(
        remaining_limited.budget(clock(1000), 30).hard,
        Duration::from_millis(100)
    );
    assert_eq!(
        remaining_limited.budget(clock(3000), 30).hard,
        Duration::from_millis(100)
    );

    let safety_limited = coefficients(50);
    assert_eq!(
        safety_limited.budget(clock(1000), 950).hard,
        Duration::from_millis(50)
    );
    assert_eq!(
        safety_limited.budget(clock(3000), 950).hard,
        Duration::from_millis(50)
    );
}

/// time advantageの因子は相手の残り時間が多いときだけ減り、正の除算を床方向へ丸める。
#[test]
fn time_advantage_factor_uses_remaining_time_difference_and_floor_division() {
    assert_eq!(time_advantage_factor(5, 5, 900), 1000);
    assert_eq!(time_advantage_factor(6, 5, 900), 1000);
    assert_eq!(time_advantage_factor(5, 0, 900), 1000);

    // 900 × 3 / 8は337余り4なので、因子は663になる。
    assert_eq!(time_advantage_factor(2, 5, 900), 663);
    assert_eq!(time_advantage_factor(1, 4, 0), 1000);
    assert_eq!(time_advantage_factor(1, 4, 1000), 500);
}

/// 時計のdocコメントの式から手計算した例。各項の丸めと上限を個別に検査する。
#[test]
fn tuning_default_clock_budget_matches_numeric_examples() {
    // f7c0925cの時間配分の算術例で使った係数を固定する。
    let coefficients = crate::search::budget::ClockCoefficients {
        expected_plies: 458,
        min_moves: 40,
        increment_share: 87,
        hard_soft_ratio: 730,
        hard_remaining_share: 39,
        time_advantage: 900,
    };
    // Mの初期値229と下限40の境界、加算87%、hard倍率730%、残り39%を検査する。
    // 相手の残り時間は自分と同じなのでtime advantageは働かず、既存の算術例を保つ。
    // 秒読みの序盤係数は変わらず、各項の除算を先に行ってから合算する。
    for (name, remaining, increment, byoyomi, ply, soft, hard) in [
        ("initial division below", 228, 0, 0, 0, 0, 1),
        ("initial division exact", 229, 0, 0, 0, 1, 7),
        ("initial division above", 230, 0, 0, 0, 1, 7),
        ("increment rounds down", 229, 1, 0, 0, 1, 7),
        ("increment fraction", 229, 9, 0, 0, 8, 58),
        ("increment next integer", 229, 10, 0, 0, 9, 65),
        ("increment above integer", 229, 11, 0, 0, 10, 73),
        ("separate truncations", 230, 1, 12, 0, 1, 7),
        ("opening fraction reaches integer", 229, 0, 13, 0, 2, 14),
        ("opening below cap", 21100, 0, 1000, 35, 880, 6424),
        ("opening at cap", 21100, 0, 1000, 36, 900, 6570),
        ("opening after cap", 21000, 0, 1000, 37, 900, 6570),
        ("moves before floor", 8200, 0, 0, 376, 200, 1460),
        ("moves at floor", 8000, 0, 0, 377, 200, 1460),
        ("moves after floor", 8000, 0, 0, 378, 200, 1460),
        ("moves beyond game", 8000, 0, 0, u32::MAX, 200, 1460),
        ("remaining cap", 1000, 1000, 0, 0, 390, 390),
        ("safety cap", 31, 1000, 0, 0, 1, 1),
        ("safety below zero", 29, 1000, 0, 0, 1, 1),
        ("byoyomi without main time", 0, 100, 1000, 0, 800, 800),
        ("large increment is capped", 1000, u64::MAX, 0, 0, 390, 390),
        (
            "final duration saturates",
            u64::MAX,
            u64::MAX,
            u64::MAX,
            458,
            u64::MAX,
            u64::MAX,
        ),
    ] {
        let actual = coefficients.budget(clock_at_ply(remaining, increment, byoyomi, ply), 30);
        assert_eq!(
            actual,
            TimeBudget {
                deadline: false,
                soft: Duration::from_millis(soft),
                hard: Duration::from_millis(hard),
            },
            "{name}"
        );
    }
}
