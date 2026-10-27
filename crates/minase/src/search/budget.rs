//! 探索方式が共有する持ち時間の配分と入力の選別。

use crate::search::alphabeta::params;
use crate::search::limits::{ClockLimits, SearchLimits};
use std::time::Duration;

/// 1手に使う時間の予算。softはhard以下、hardは1 ms以上である。
///
/// [`ClockCoefficients::budget`]では残り時間と秒読みの和が余裕を超えるとき、
/// hardはその和から余裕を引いた値以下となる。
/// softは通常、反復の境界で照合し、hardは探索中の時計の検査点でも照合する。
/// [`byoyomi_clock`]の対象で残り持ち時間が0の場合と[`movetime_deadline`]の対象では、
/// softとhardを兼ねる締切まで、予測と最善手の安定性を使わず反復を始める。
/// それ以外の先読みから続く反復では、的中後の最初の時計検査でも反復の完了時刻予測で予算を当て直す。
/// 通常探索では探索開始から、先読みでは的中の時点から測る。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct TimeBudget {
    /// 完了イテレーションの境界で停止する目安時間。
    pub(super) soft: Duration,
    /// 探索途中でも打ち切る上限時間。
    pub(super) hard: Duration,
    /// 予測を使わず、softとhardを兼ねる締切まで反復を始める予算を示す。
    pub(super) deadline: bool,
}

/// 自分と相手の残り時間からtime advantageの因子を千分率で返す。
///
/// 相手の残り時間が多い場合だけ差を取り、すべての演算を`u128`で行う。
/// 正の整数除算は床方向へ丸める。Stockfish `src/timeman.cpp:151-156`に当たる。
pub(super) fn time_advantage_factor(
    remaining_ms: u64,
    opponent_remaining_ms: u64,
    time_advantage: u128,
) -> u128 {
    assert!(
        time_advantage <= 1000,
        "time advantage must not exceed 1000"
    );
    let remaining = u128::from(remaining_ms);
    let opponent = u128::from(opponent_remaining_ms);
    let difference = opponent.saturating_sub(remaining);
    1000 - time_advantage * difference / (remaining + opponent + 1)
}

/// [`ClockCoefficients::budget`]を安全余裕30 msで適用する。
/// 秒読みだけの特別な締切や、指定された秒読み余裕の適用は[`time_budget`]で選ぶ。
#[cfg(test)]
pub(super) fn clock_budget(clock: ClockLimits) -> TimeBudget {
    ClockCoefficients::current().budget(clock, 30)
}

/// 持ち時間から探索時間を配分する式に渡す調整係数。
#[derive(Clone, Copy)]
pub(super) struct ClockCoefficients {
    pub(super) expected_plies: i32,
    pub(super) min_moves: i32,
    pub(super) increment_share: i32,
    pub(super) hard_soft_ratio: i32,
    pub(super) hard_remaining_share: i32,
    pub(super) time_advantage: u128,
}

impl ClockCoefficients {
    pub(super) fn current() -> Self {
        Self {
            expected_plies: params::expected_plies(),
            min_moves: params::min_moves(),
            increment_share: params::increment_share(),
            hard_soft_ratio: params::hard_soft_ratio(),
            hard_remaining_share: params::hard_remaining_share(),
            time_advantage: u128::try_from(params::time_advantage())
                .expect("time advantage must not be negative"),
        }
    }
    fn moves_to_go(self, ply: u32) -> u128 {
        (self.min_moves as u128)
            .max((self.expected_plies as u128).saturating_sub(u128::from(ply)) / 2)
    }
    /// 残り時間R、加算I、秒読みB、余裕mをミリ秒で受け、softとhardを求める。
    ///
    /// 残り着手数Mは[`Self::moves_to_go`]で求め、序盤係数の分子wはRが正なら`min(ply + 4, 40)`、0なら40とする。
    /// `soft_raw = R / M + I × IncrementShare / 100 + B × 8 × w / 400` を各項ごとの整数除算で求める。
    /// 秒読みが0の時計では、相手の残り時間を`R_o`として
    /// `f = 1000 − floor(TimeAdvantage × max(0, R_o − R) / (R + R_o + 1))`を求め、
    /// `soft_raw`を`floor(soft_raw × f / 1000)`に置き換える。
    /// `safe_hard = max(1, R + B − m)` とし、減算は0で飽和させる。
    /// `hard = max(1, min(soft_raw × HardSoftRatio / 100, R × HardRemainingShare / 100 + B × 8 / 10, safe_hard))` とする。
    /// `soft = min(soft_raw, hard)` としてsoftがhardを超えないようにし、最終値は`u64`のミリ秒へ飽和変換する。
    ///
    /// 秒読みが0なら序盤係数の項も0となり、時計の予算はこの係数に依存しない。
    /// 残り手数とsoftの上限の理由は`docs/adr/estimate-moves-to-go-from-ply.md`、
    /// 秒読みの項だけの序盤係数は`docs/adr/scale-only-the-byoyomi-term-in-the-opening.md`、
    /// 持ち時間を序盤から配る理由は`docs/adr/spend-main-time-from-the-opening.md`にある。
    /// 秒読みのない時計でhardを残り時間の割合でも抑え、途中結果を使わない理由は
    /// `docs/adr/keep-discarding-hard-interrupted-iterations-and-reject-fair-share-budgets.md`にある。
    pub(super) fn budget(self, clock: ClockLimits, margin_ms: u64) -> TimeBudget {
        self.budget_with_time_advantage(clock, margin_ms, clock.byoyomi_ms == 0)
    }
    fn budget_without_time_advantage(self, clock: ClockLimits, margin_ms: u64) -> TimeBudget {
        self.budget_with_time_advantage(clock, margin_ms, false)
    }
    fn budget_with_time_advantage(
        self,
        clock: ClockLimits,
        margin_ms: u64,
        apply_time_advantage: bool,
    ) -> TimeBudget {
        let remaining = u128::from(clock.remaining_ms);
        let increment = u128::from(clock.increment_ms);
        let byoyomi = u128::from(clock.byoyomi_ms);
        let byoyomi_share = byoyomi * 8 / 10;
        let opening = if remaining > 0 {
            u128::from(clock.ply.saturating_add(4).min(40))
        } else {
            40
        };
        let soft_raw = remaining / self.moves_to_go(clock.ply)
            + increment * self.increment_share as u128 / 100
            + byoyomi * 8 * opening / 400;
        let soft_raw = if apply_time_advantage {
            let opponent_remaining_ms = clock
                .opponent_remaining_ms
                .expect("time advantage clock must contain the opponent's remaining time");
            soft_raw
                * time_advantage_factor(
                    clock.remaining_ms,
                    opponent_remaining_ms,
                    self.time_advantage,
                )
                / 1000
        } else {
            soft_raw
        };
        let safe_hard = remaining
            .saturating_add(byoyomi)
            .saturating_sub(u128::from(margin_ms))
            .max(1);
        let hard = (soft_raw * self.hard_soft_ratio as u128 / 100)
            .min(remaining * self.hard_remaining_share as u128 / 100 + byoyomi_share)
            .min(safe_hard)
            .max(1);
        TimeBudget {
            soft: Duration::from_millis(to_u64_ms(soft_raw.min(hard))),
            hard: Duration::from_millis(to_u64_ms(hard)),
            deadline: false,
        }
    }
}

/// 秒読み用の時間配分を適用する時計を返す。
/// 秒読みが正で、固定時間、深さ、ノード数の制限を併用しない有限探索だけを選ぶ。
/// 根の順序と途中結果の採用は、固定時間だけの探索も含めてαβ探索の途中結果の採用条件で判定する。
pub(super) fn byoyomi_clock(limits: &SearchLimits) -> Option<ClockLimits> {
    let limits = limits.finite()?;
    limits.clock.filter(|clock| {
        clock.byoyomi_ms > 0
            && limits.movetime_ms.is_none()
            && limits.depth.is_none()
            && limits.nodes.is_none()
    })
}

/// 固定時間だけを指定した探索の締切を返す。
/// 時計、深さ、ノード数の制限を併用しない有限探索だけを選ぶ。
pub(super) fn movetime_deadline(limits: &SearchLimits) -> Option<Duration> {
    let limits = limits.finite()?;
    limits
        .movetime_ms
        .filter(|_| limits.clock.is_none() && limits.depth.is_none() && limits.nodes.is_none())
        .map(|milliseconds| Duration::from_millis(milliseconds.get()))
}

/// 探索制限から時間予算を求める。
///
/// [`byoyomi_clock`]の対象では指定の秒読み余裕を使い、残り持ち時間が0なら`max(1 ms, 秒読み − 余裕)`をsoftとhardの両方にする。
/// 持ち時間があれば[`ClockCoefficients::budget`]を使う。
/// [`movetime_deadline`]の対象では、指定時間をsoftとhardの両方にする。
/// 秒読み期と固定時間だけを指定した探索ではdeadlineの印を立て、予測を使わず締切まで反復を始める。
/// 秒読みが0で`movetime`がなければtime advantageを適用し、深さやノード数の制限を併用しても除外しない。
/// 秒読みが正の時計と`movetime`を併用する時計では適用しない。
/// `movetime`と時計の併用時はsoftとhardのそれぞれで小さい方を採る。
pub(super) fn time_budget(limits: &SearchLimits) -> Option<TimeBudget> {
    time_budget_with_coefficients(limits, ClockCoefficients::current())
}

/// [`time_budget`]の予算計算を指定された時計係数で行う。
pub(super) fn time_budget_with_coefficients(
    limits: &SearchLimits,
    coefficients: ClockCoefficients,
) -> Option<TimeBudget> {
    if let Some(deadline) = movetime_deadline(limits) {
        return Some(TimeBudget {
            soft: deadline,
            hard: deadline,
            deadline: true,
        });
    }
    if let Some(clock) = byoyomi_clock(limits) {
        if clock.remaining_ms == 0 {
            let deadline = Duration::from_millis(
                clock
                    .byoyomi_ms
                    .saturating_sub(clock.byoyomi_margin_ms)
                    .max(1),
            );
            return Some(TimeBudget {
                soft: deadline,
                hard: deadline,
                deadline: true,
            });
        }
        return Some(coefficients.budget(clock, clock.byoyomi_margin_ms));
    }
    let limits = limits.finite()?;
    let movetime = limits.movetime_ms.map(|milliseconds| TimeBudget {
        soft: Duration::from_millis(milliseconds.get()),
        hard: Duration::from_millis(milliseconds.get()),
        deadline: false,
    });
    let clock = limits.clock.map(|clock| {
        if limits.movetime_ms.is_some() {
            coefficients.budget_without_time_advantage(clock, 30)
        } else {
            coefficients.budget(clock, 30)
        }
    });
    match (movetime, clock) {
        (Some(fixed), Some(clock)) => Some(TimeBudget {
            soft: fixed.soft.min(clock.soft),
            hard: fixed.hard.min(clock.hard),
            deadline: false,
        }),
        (Some(budget), None) | (None, Some(budget)) => Some(budget),
        (None, None) => None,
    }
}

/// ミリ秒をu64へ飽和変換する。
pub(super) fn to_u64_ms(milliseconds: u128) -> u64 {
    milliseconds.min(u128::from(u64::MAX)) as u64
}
