//! 探索の時間予算と反復の開始判定。
//!
//! 予算式が係数表`params`を読み、反復の開始判定と先読みの的中処理が反復深化に結びつくため、αβ探索の側に置く。

use core::num::NonZeroUsize;
use std::time::Duration;

use crate::search::MATE;
use crate::search::limits::{ClockLimits, SearchLimits};
use minase_core::mv::Move;

use super::params;

/// 1手に使う時間の予算。softはhard以下、hardは1 ms以上である。
///
/// [`ClockCoefficients::budget`]では残り時間と秒読みの和が余裕を超えるとき、
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

/// 次の探索へ持ち越す、直前の探索の値。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct PreviousSearch {
    pub(crate) score_and_average: Option<(i32, i32)>,
    /// 主ワーカーが最後に求めたbest move stabilityの短縮率。単位は千分率である。
    pub(crate) time_reduction: i32,
}

/// 前の探索の短縮率の初期値。Stockfish `src/thread.cpp:271`に合わせる。
const INITIAL_PREVIOUS_TIME_REDUCTION: i32 = 850;

impl Default for PreviousSearch {
    fn default() -> Self {
        Self {
            score_and_average: None,
            time_reduction: INITIAL_PREVIOUS_TIME_REDUCTION,
        }
    }
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

/// best move stabilityによる時間の因子の調整係数。
#[derive(Clone, Copy)]
pub(super) struct TimeReductionCoefficients {
    pub(super) intercept: i32,
    pub(super) slope: i32,
    pub(super) scale: i32,
}

impl TimeReductionCoefficients {
    pub(super) fn current() -> Self {
        Self {
            intercept: params::time_reduction_intercept(),
            slope: params::time_reduction_slope(),
            scale: params::time_reduction_scale(),
        }
    }
}

/// best move instabilityによる時間の因子の調整係数。
#[derive(Clone, Copy)]
pub(super) struct TimeInstabilityCoefficients {
    pub(super) base: u128,
    pub(super) slope: u128,
}

impl TimeInstabilityCoefficients {
    pub(super) fn current() -> Self {
        Self {
            base: u128::try_from(params::time_instability_base())
                .expect("time instability base must not be negative"),
            slope: u128::try_from(params::time_instability_slope())
                .expect("time instability slope must not be negative"),
        }
    }
}

/// best move effortによる時間の因子の調整係数。
#[derive(Clone, Copy)]
pub(super) struct TimeEffortCoefficients {
    pub(super) intercept: i32,
    pub(super) slope: i32,
}

impl TimeEffortCoefficients {
    pub(super) fn current() -> Self {
        Self {
            intercept: params::time_effort_intercept(),
            slope: params::time_effort_slope(),
        }
    }
}

/// 評価値の下落の因子の下限と上限。Stockfish `src/search.cpp:594-597`に合わせる。
const FALLING_EVAL_FACTOR_MIN: i32 = 576;
const FALLING_EVAL_FACTOR_MAX: i32 = 1728;

/// 最善手の安定性から求める短縮率の下限と上限。Stockfish `src/search.cpp:600-602`に合わせる。
const TIME_REDUCTION_MIN: i64 = 629;
const TIME_REDUCTION_MAX: i64 = 1544;
/// 前の探索の短縮率に加える定数。Stockfish `src/search.cpp:604-605`に合わせる。
const TIME_REDUCTION_FACTOR_BASE: i64 = 1468;

/// 最善手のノード占有率を表す尺度。100000が占有率100%に当たる。
const BEST_MOVE_EFFORT_SCALE: u128 = 100_000;
/// 最善手のノード占有率による因子の下限と上限。
/// Stockfish `src/search.cpp:592, 609-610`に合わせる。
const BEST_MOVE_EFFORT_FACTOR_MIN: i64 = 693;
const BEST_MOVE_EFFORT_FACTOR_MAX: i64 = 838;

/// 根の候補が1手のときの時間上限。Stockfish `src/search.cpp:615-617`に合わせる。
const SINGLE_ROOT_MOVE_TIME_CAP_MS: u128 = 500;
/// 早期停止の対象とする、勝ち側の詰みまでの手数。
const EARLY_STOP_MATE_IN: i32 = 3;
/// 早期停止の対象とする、負け側の詰みまでの手数。
const EARLY_STOP_MATED_IN: i32 = 2;
/// increaseDepthで深さを増やしてよい経過時間の上限。hardで抑える前の時間に対する百分率。
const INCREASE_DEPTH_THRESHOLD_PERCENT: u128 = 50;
/// 百分率の尺度。
const PERCENT_SCALE: u128 = 100;

/// 完了反復ごとの時間伸縮に使う探索の不安定さの状態。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct InstabilityScaling {
    pub(super) previous_average: Option<i32>,
    pub(super) iteration_values: [i32; 4],
    pub(super) current: usize,
    pub(super) previous_time_reduction: i32,
    pub(super) time_reduction: i32,
    /// 最善手の変化回数の減衰つき累計。単位は1/1024回である。
    pub(super) best_move_change_accumulator: u128,
    worker_count: NonZeroUsize,
}

impl InstabilityScaling {
    /// 前の探索の評価値で環状配列を埋め、欠測時は0とする。
    /// 短縮率は前の値を保持し、現在値を中立値1000で初期化する。
    pub(super) fn new(previous: PreviousSearch, worker_count: NonZeroUsize) -> Self {
        let initial = previous
            .score_and_average
            .map_or(0, |(score, _average)| score);
        Self {
            previous_average: previous.score_and_average.map(|(_score, average)| average),
            iteration_values: [initial; 4],
            current: 0,
            previous_time_reduction: previous.time_reduction,
            time_reduction: 1000,
            best_move_change_accumulator: 0,
            worker_count,
        }
    }

    /// 実際に反復を始めるとき、最善手の変化回数の累計を半分へ減衰させる。
    pub(super) fn begin_iteration(&mut self) {
        self.best_move_change_accumulator /= 2;
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

    /// 完了反復の最善手列からbest move stabilityの因子を求め、現在の短縮率を更新する。
    pub(super) fn update_best_move_stability(
        &mut self,
        bests: &[Move],
        coefficients: TimeReductionCoefficients,
    ) -> i32 {
        let reduction = time_reduction(best_move_stability_distance(bests), coefficients);
        let factor =
            best_move_stability_factor(self.previous_time_reduction, reduction, coefficients.scale);
        self.time_reduction = reduction;
        factor
    }

    /// 完了反復までに回収した変化回数を累計へ加え、best move instabilityの因子を返す。
    pub(super) fn update_best_move_instability(
        &mut self,
        changes: u64,
        coefficients: TimeInstabilityCoefficients,
    ) -> u128 {
        let added = 1024_u128
            .checked_mul(u128::from(changes))
            .expect("best move change increment must fit in u128");
        self.best_move_change_accumulator = self
            .best_move_change_accumulator
            .checked_add(added)
            .expect("best move change accumulator must fit in u128");
        best_move_instability_factor(
            self.best_move_change_accumulator,
            self.worker_count.get(),
            coefficients,
        )
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

/// 完了反復の最善手列の末尾で、同じ手が続いた回数から1を引いた距離を返す。
pub(super) fn best_move_stability_distance(bests: &[Move]) -> usize {
    let last = bests
        .last()
        .expect("best move stability requires a completed iteration");
    bests
        .iter()
        .rev()
        .take_while(|&&best| best == *last)
        .count()
        - 1
}

/// 最善手が最後に変わった完了反復からの距離を、固定範囲内の短縮率へ変換する。
pub(super) fn time_reduction(x: usize, coefficients: TimeReductionCoefficients) -> i32 {
    let x = i64::try_from(x).expect("best move stability distance must fit in i64");
    let reduction = i64::from(coefficients.intercept) + i64::from(coefficients.slope) * x;
    i32::try_from(reduction.clamp(TIME_REDUCTION_MIN, TIME_REDUCTION_MAX))
        .expect("clamped time reduction must fit in i32")
}

/// 前の探索と現在の短縮率から、best move stabilityによる時間の因子を千分率で返す。
///
/// 分子と分母は`i64`で求め、正の整数除算によって床方向へ丸める。
pub(super) fn best_move_stability_factor(
    previous_time_reduction: i32,
    time_reduction: i32,
    scale: i32,
) -> i32 {
    assert!(
        previous_time_reduction >= 0,
        "previous time reduction must not be negative"
    );
    assert!(time_reduction > 0, "time reduction must be positive");
    assert!(scale > 0, "time reduction scale must be positive");
    let numerator = (TIME_REDUCTION_FACTOR_BASE + i64::from(previous_time_reduction)) * 1_000_000;
    let denominator = i64::from(scale) * i64::from(time_reduction);
    i32::try_from(numerator / denominator).expect("best move stability factor must fit in i32")
}

/// 最善手の変化回数の減衰つき累計から、best move instabilityの因子を千分率で返す。
///
/// 累計は1/1024回の単位であり、全ワーカーの合計を設定上のワーカー数で割る。
pub(super) fn best_move_instability_factor(
    change_accumulator: u128,
    worker_count: usize,
    coefficients: TimeInstabilityCoefficients,
) -> u128 {
    assert!(worker_count > 0, "worker count must be positive");
    let workers = u128::try_from(worker_count).expect("worker count must fit in u128");
    let numerator = coefficients
        .slope
        .checked_mul(change_accumulator)
        .expect("best move instability numerator must fit in u128");
    let denominator = 1024_u128
        .checked_mul(workers)
        .expect("best move instability denominator must fit in u128");
    coefficients
        .base
        .checked_add(numerator / denominator)
        .expect("best move instability factor must fit in u128")
}

/// 最善手の部分木の累計ノード数が全ノード数に占める割合を100000倍して返す。
pub(super) fn best_move_effort_ratio(best_move_nodes: u64, total_nodes: u64) -> u128 {
    BEST_MOVE_EFFORT_SCALE * u128::from(best_move_nodes) / u128::from(total_nodes.max(1))
}

/// 最善手のノード占有率を、固定範囲内のbest move effortの因子へ変換する。
///
/// 占有率、傾きの項、および直線はそれぞれ`u128`、`i64`、`i64`で求める。
/// 負になり得る傾きの項はRustの整数除算によって0の方向へ丸める。
pub(super) fn best_move_effort_factor(
    best_move_nodes: u64,
    total_nodes: u64,
    coefficients: TimeEffortCoefficients,
) -> i32 {
    let ratio = i64::try_from(best_move_effort_ratio(best_move_nodes, total_nodes))
        .expect("best move effort ratio must fit in i64");
    let slope_term = i64::from(coefficients.slope)
        .checked_mul(ratio)
        .expect("best move effort slope term must fit in i64")
        / 100_000;
    let factor = i64::from(coefficients.intercept)
        .checked_add(slope_term)
        .expect("best move effort factor must fit in i64");
    i32::try_from(factor.clamp(BEST_MOVE_EFFORT_FACTOR_MIN, BEST_MOVE_EFFORT_FACTOR_MAX))
        .expect("clamped best move effort factor must fit in i32")
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

/// 基礎softに千分率の因子を順に掛け、根の候補が1手なら500 msで抑えた時間を返す。
pub(super) fn scaled_soft_millis(
    base_soft: Duration,
    factors: &[u128],
    root_move_count: usize,
) -> u128 {
    let numerator = factors.iter().fold(base_soft.as_millis(), |time, &factor| {
        time.saturating_mul(factor)
    });
    let exponent = u32::try_from(factors.len()).expect("time factor count must fit in u32");
    let denominator = 1000_u128
        .checked_pow(exponent)
        .expect("time factor denominator must fit in u128");
    let time_ms = numerator / denominator;
    if root_move_count == 1 {
        time_ms.min(SINGLE_ROOT_MOVE_TIME_CAP_MS)
    } else {
        time_ms
    }
}

/// 基礎予算のsoftだけを、hardで抑えた伸縮後の値へ差し替える。
pub(super) fn scaled_time_budget(base: TimeBudget, time_ms: u128) -> TimeBudget {
    let soft_ms = time_ms.min(base.hard.as_millis());
    TimeBudget {
        soft: Duration::from_millis(to_u64_ms(soft_ms)),
        ..base
    }
}

/// 完了反復の値が3手以内の勝ち、またはちょうど2手の負けを表すかを返す。
///
/// 双方の着手をそれぞれ1手と数える。反復による勝敗の専用の帯は詰み帯より低いため、
/// この条件には入らない。Stockfish `src/search.cpp:621-624`に当たる。
pub(super) fn mate_early_stop(value: i32) -> bool {
    value >= MATE - EARLY_STOP_MATE_IN || value == -(MATE - EARLY_STOP_MATED_IN)
}

/// 的中後の経過時間がhardで抑える前の時間の50%以下なら、深さを増やしてよいと判定する。
///
/// 的中前は常に真を返す。時間はナノ秒へ揃えて交差積で比較し、ミリ秒への切り詰めを行わない。
/// Stockfish `src/search.cpp:634`に当たる。
pub(super) fn should_increase_depth(
    elapsed: Duration,
    hit: Option<Duration>,
    scaled_time_ms: u128,
) -> bool {
    let Some(hit) = hit else {
        return true;
    };
    let elapsed_ns = elapsed
        .saturating_sub(hit)
        .as_nanos()
        .saturating_mul(PERCENT_SCALE);
    let threshold_ns = scaled_time_ms
        .saturating_mul(1_000_000)
        .saturating_mul(INCREASE_DEPTH_THRESHOLD_PERCENT);
    elapsed_ns <= threshold_ns
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
    /// `docs/adr/reject-fair-share-budget-with-partial-results.md`にある。
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
            byoyomi_period: false,
        }
    }
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
/// 持ち時間があれば[`ClockCoefficients::budget`]を使う。
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
        return Some(coefficients.budget(clock, clock.byoyomi_margin_ms));
    }
    let limits = limits.finite()?;
    let movetime = limits.movetime_ms.map(|milliseconds| TimeBudget {
        soft: Duration::from_millis(milliseconds.get()),
        hard: Duration::from_millis(milliseconds.get()),
        byoyomi_period: false,
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
