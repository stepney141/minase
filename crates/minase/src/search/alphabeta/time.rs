//! 探索の時間予算と反復の開始判定。
//!
//! 予算式が係数表`params`を読み、反復の開始判定と先読みの的中処理が反復深化に結びつくため、αβ探索の側に置く。

use std::time::Duration;

use crate::search::limits::{ClockLimits, SearchLimits};
use minase_core::mv::Move;

use super::params;

/// 1手に使う時間の予算。softはhard以下、hardは1 ms以上である。
///
/// [`clock_budget_with_margin`]では残り時間と秒読みの和が余裕を超えるとき、
/// hardはその和から余裕を引いた値以下となる。
/// softは通常、反復の境界で照合し、hardは探索中の時計の検査点でも照合する。
/// 先読みから続く反復では、的中後の最初の時計検査でも[`iteration_prediction_fits`]で予算を当て直す。
/// 通常探索では探索開始から、先読みでは的中の時点から測る。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct TimeBudget {
    /// 完了イテレーションの境界で停止する目安時間。
    pub(super) soft: Duration,
    /// 探索途中でも打ち切る上限時間。
    pub(super) hard: Duration,
    /// 対象の探索の秒読み期では、予測を使わず締切まで反復を始める。
    pub(super) byoyomi_period: bool,
}

/// 次の探索へ持ち越す、直前の探索の評価値と返却手の平均値。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct PreviousSearch {
    pub(crate) score_and_average: Option<(i32, i32)>,
}

/// 評価値の下落の因子の調整係数。
#[derive(Clone, Copy)]
pub(super) struct FallingEvalCoefficients {
    pub(super) base: i32,
    pub(super) previous: i32,
    pub(super) iteration: i32,
}

impl FallingEvalCoefficients {
    pub(super) fn current() -> Self {
        Self {
            base: params::time_falling_base(),
            previous: params::time_falling_previous(),
            iteration: params::time_falling_iteration(),
        }
    }
}

/// 評価値の下落の因子の下限と上限。Stockfish `src/search.cpp:594-597`に合わせる。
const FALLING_EVAL_FACTOR_MIN: i32 = 576;
const FALLING_EVAL_FACTOR_MAX: i32 = 1728;

/// 完了反復ごとの時間伸縮に使う評価値の下落の因子の状態。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct InstabilityScaling {
    pub(super) previous_average: Option<i32>,
    pub(super) iteration_values: [i32; 4],
    pub(super) current: usize,
}

impl InstabilityScaling {
    /// 前の探索の評価値で環状配列を埋め、欠測時は0で埋める。
    pub(super) fn new(previous: PreviousSearch) -> Self {
        let initial = previous
            .score_and_average
            .map_or(0, |(score, _average)| score);
        Self {
            previous_average: previous.score_and_average.map(|(_score, average)| average),
            iteration_values: [initial; 4],
            current: 0,
        }
    }

    /// 現在位置の値から評価値の下落の因子を求めた後、完了反復の値を書いて位置を進める。
    pub(super) fn update_falling_eval(
        &mut self,
        value: i32,
        pawn_value: i32,
        coefficients: FallingEvalCoefficients,
    ) -> i32 {
        let factor = falling_eval_factor(
            self.previous_average,
            self.iteration_values[self.current],
            value,
            pawn_value,
            coefficients,
        );
        self.iteration_values[self.current] = value;
        self.current = (self.current + 1) % self.iteration_values.len();
        factor
    }
}

/// 評価値の下落による評価値の下落の因子を千分率で返す。
///
/// 2つの差と積は`i64`で求め、和を歩兵価値で1回だけ割る。
/// 前の探索の平均値がなければ、minaseの欠測時の規則として上限を返す。
pub(super) fn falling_eval_factor(
    previous_average: Option<i32>,
    iteration_value: i32,
    value: i32,
    pawn_value: i32,
    coefficients: FallingEvalCoefficients,
) -> i32 {
    let Some(previous_average) = previous_average else {
        return FALLING_EVAL_FACTOR_MAX;
    };
    assert!(pawn_value > 0, "pawn value must be positive");
    let previous_difference = i64::from(previous_average) - i64::from(value);
    let iteration_difference = i64::from(iteration_value) - i64::from(value);
    let numerator = i64::from(coefficients.previous) * previous_difference
        + i64::from(coefficients.iteration) * iteration_difference;
    (i64::from(coefficients.base) + numerator / i64::from(pawn_value)).clamp(
        i64::from(FALLING_EVAL_FACTOR_MIN),
        i64::from(FALLING_EVAL_FACTOR_MAX),
    ) as i32
}

/// 基礎softに千分率の因子を順に掛け、hardで抑える前の時間をmsで返す。
pub(super) fn scaled_soft_millis(base_soft: Duration, factors: &[u128]) -> u128 {
    let numerator = factors.iter().fold(base_soft.as_millis(), |time, &factor| {
        time.saturating_mul(factor)
    });
    let exponent = u32::try_from(factors.len()).expect("time factor count must fit in u32");
    let denominator = 1000_u128
        .checked_pow(exponent)
        .expect("time factor denominator must fit in u128");
    numerator / denominator
}

/// 基礎予算のsoftだけを、hardで抑えた伸縮後の値へ差し替える。
pub(super) fn scaled_time_budget(base: TimeBudget, time_ms: u128) -> TimeBudget {
    let soft_ms = time_ms.min(base.hard.as_millis());
    TimeBudget {
        soft: Duration::from_millis(to_u64_ms(soft_ms)),
        ..base
    }
}

/// 完了反復の最善手列から、予測完了時刻をsoftで抑えるかを返す。
///
/// `iterations`個の最善手が揃い、すべて同じ場合だけ真を返す。
/// `bests`は完了順に並び、末尾が最新の反復の最善手である。
/// 反復数は係数`StableIterations`で与え、既定値4の判断の理由は`docs/adr/stop-early-on-stable-best-move-without-extensions.md`にある。
pub(super) fn stable_signal(bests: &[Move], iterations: usize) -> bool {
    bests.len() >= iterations
        && bests[bests.len() - iterations..]
            .windows(2)
            .all(|pair| pair[0] == pair[1])
}

/// 時間予算内で次の反復を開始できるかを返す。
///
/// 通常は[`iteration_prediction_fits`]を満たし、さらに最善手が不安定なら的中後の経過時間がsoft未満であることを要求する。
/// 安定時のsoft判定は同関数の完了時刻予測に含まれる。
/// [`byoyomi_clock`]の対象で残り持ち時間が0の場合は予測と安定性を使わず、的中後の経過時間がhard未満かだけを判定する。
///
/// 固定倍の予測は`docs/adr/predict-next-iteration-by-fixed-ratio.md`に基づく。
/// 的中までの時間を予算に足す扱いは[`iteration_prediction_fits`]に従う。
pub(super) fn should_start_next_iteration(
    elapsed: Duration,
    hit: Duration,
    budget: TimeBudget,
    stable: bool,
) -> bool {
    if budget.byoyomi_period {
        return elapsed.saturating_sub(hit) < budget.hard;
    }
    iteration_prediction_fits(elapsed, hit, budget, stable)
        && (stable || elapsed.saturating_sub(hit) < budget.soft)
}

/// 反復の予測完了時刻が、的中時刻から測った予算に収まるかを返す。
///
/// `started`と`hit`は探索開始からの累積時間である。
/// `started × IterationRatio ≤ (hit + hard) × 100` を要求し、安定時はsoftにも同じ判定を行う。
/// 交差積で比較して、百分率の除算による丸めを避ける。
///
/// 先読みで始めた反復も、的中後にこの式へ当て直す。
/// 先読みでない探索は`hit = 0`なので、先読みなしの式と不等号まで一致する。
/// 理由は`docs/adr/continue-search-and-measure-budget-from-ponderhit.md`にある。
pub(super) fn iteration_prediction_fits(
    started: Duration,
    hit: Duration,
    budget: TimeBudget,
    stable: bool,
) -> bool {
    let predicted = started.as_nanos() * params::iteration_ratio() as u128;
    predicted <= (hit.as_nanos() + budget.hard.as_nanos()) * 100
        && (!stable || predicted <= (hit.as_nanos() + budget.soft.as_nanos()) * 100)
}

/// [`clock_budget_with_margin`]を安全余裕30 msで適用する。
/// 秒読みだけの特別な締切や、指定された秒読み余裕の適用は[`time_budget`]で選ぶ。
pub(super) fn clock_budget(clock: ClockLimits) -> TimeBudget {
    clock_budget_with_margin(clock, 30)
}

/// 持ち時間から探索時間を配分する式に渡す調整係数。
#[derive(Clone, Copy)]
pub(super) struct ClockCoefficients {
    pub(super) expected_plies: i32,
    pub(super) min_moves: i32,
    pub(super) increment_share: i32,
    pub(super) hard_soft_ratio: i32,
    pub(super) hard_remaining_share: i32,
}

impl ClockCoefficients {
    pub(super) fn current() -> Self {
        Self {
            expected_plies: params::expected_plies(),
            min_moves: params::min_moves(),
            increment_share: params::increment_share(),
            hard_soft_ratio: params::hard_soft_ratio(),
            hard_remaining_share: params::hard_remaining_share(),
        }
    }
    fn moves_to_go(self, ply: u32) -> u128 {
        (self.min_moves as u128)
            .max((self.expected_plies as u128).saturating_sub(u128::from(ply)) / 2)
    }
    pub(super) fn budget(self, clock: ClockLimits, margin_ms: u64) -> TimeBudget {
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
            byoyomi_period: false,
        }
    }
}

/// 残り時間R、加算I、秒読みB、余裕mをミリ秒で受け、softとhardを求める。
///
/// 残り着手数Mは[`ClockCoefficients::moves_to_go`]で求め、序盤係数の分子wはRが正なら`min(ply + 4, 40)`、0なら40とする。
/// `soft_raw = R / M + I × IncrementShare / 100 + B × 8 × w / 400` を各項ごとの整数除算で求める。
/// `safe_hard = max(1, R + B − m)` とし、減算は0で飽和させる。
/// `hard = max(1, min(soft_raw × HardSoftRatio / 100, R × HardRemainingShare / 100 + B × 8 / 10, safe_hard))` とする。
/// `soft = min(soft_raw, hard)` としてsoftがhardを超えないようにし、最終値は`u64`のミリ秒へ飽和変換する。
///
/// 秒読みが0なら序盤係数の項も0となり、時計の予算はこの係数に依存しない。
/// 残り手数とsoftの上限の理由は`docs/adr/estimate-moves-to-go-from-ply.md`、
/// 秒読みの項だけの序盤係数は`docs/adr/scale-only-the-byoyomi-term-in-the-opening.md`、
/// 持ち時間を序盤から配る理由は`docs/adr/spend-main-time-from-the-opening.md`にある。
/// 秒読みのない時計でhardを残り時間の割合でも抑え、途中結果を使わない理由は
/// `docs/adr/reject-fair-share-budget-with-partial-results.md`にある。
fn clock_budget_with_margin(clock: ClockLimits, margin_ms: u64) -> TimeBudget {
    ClockCoefficients::current().budget(clock, margin_ms)
}

/// 秒読み用の時間配分と途中結果の採用を適用する時計を返す。
/// 秒読みが正で、固定時間、深さ、ノード数の制限を併用しない有限探索だけを選ぶ。
pub(super) fn byoyomi_clock(limits: &SearchLimits) -> Option<ClockLimits> {
    let limits = limits.finite()?;
    limits.clock.filter(|clock| {
        clock.byoyomi_ms > 0
            && limits.movetime_ms.is_none()
            && limits.depth.is_none()
            && limits.nodes.is_none()
    })
}

/// 探索制限から時間予算を求める。
///
/// [`byoyomi_clock`]の対象では指定の秒読み余裕を使い、残り持ち時間が0なら`max(1 ms, 秒読み − 余裕)`をsoftとhardの両方にする。
/// 持ち時間があれば[`clock_budget_with_margin`]を使う。
/// 対象外では[`clock_budget`]を使い、`movetime`と時計の併用時はsoftとhardのそれぞれで小さい方を採る。
pub(super) fn time_budget(limits: &SearchLimits) -> Option<TimeBudget> {
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
                byoyomi_period: true,
            });
        }
        return Some(clock_budget_with_margin(clock, clock.byoyomi_margin_ms));
    }
    let limits = limits.finite()?;
    let movetime = limits.movetime_ms.map(|milliseconds| TimeBudget {
        soft: Duration::from_millis(milliseconds.get()),
        hard: Duration::from_millis(milliseconds.get()),
        byoyomi_period: false,
    });
    match (movetime, limits.clock.map(clock_budget)) {
        (Some(fixed), Some(clock)) => Some(TimeBudget {
            soft: fixed.soft.min(clock.soft),
            hard: fixed.hard.min(clock.hard),
            byoyomi_period: false,
        }),
        (Some(budget), None) | (None, Some(budget)) => Some(budget),
        (None, None) => None,
    }
}

/// ミリ秒をu64へ飽和変換する。
fn to_u64_ms(milliseconds: u128) -> u64 {
    milliseconds.min(u128::from(u64::MAX)) as u64
}
