//! 探索の時間予算と反復の開始判定。

use std::time::Duration;

use crate::core::mv::Move;
use crate::search::limits::{ClockLimits, SearchLimits};

use super::params;

/// 1手に使う時間の予算。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct TimeBudget {
    /// 完了イテレーションの境界で停止する目安時間。
    pub(super) soft: Duration,
    /// 探索途中でも打ち切る上限時間。
    pub(super) hard: Duration,
}

/// 最善手の安定を判定する直近の完了反復数。
///
/// `docs/plans/strength-stage6.md`の「最善手安定時の早期終了」節に従う。
/// 「反復深化の診断」の時間条件を含む模擬で、失う良い結果の割合が10%以下に
/// なる最小の反復数がk = 4だったことに基づく。
const STABLE_ITERATIONS: usize = 4;

/// 完了反復の最善手列から、予測完了時刻をsoftで抑えるかを返す。
///
/// `docs/plans/strength-stage6.md`の「最善手安定時の早期終了」節に従い、
/// 直近4反復の最善手がすべて同じ場合だけ真を返す。
/// `bests`は完了順に並び、末尾が最新の反復の最善手である。
pub(super) fn stable_signal(bests: &[Move]) -> bool {
    bests.len() >= STABLE_ITERATIONS
        && bests[bests.len() - STABLE_ITERATIONS..]
            .windows(2)
            .all(|pair| pair[0] == pair[1])
}

/// 時間予算内で次の反復を開始できるかを返す。
///
/// `docs/plans/strength-stage6.md`の「最善手安定時の早期終了」節に従い、
/// `stable`が真なら経過時間に固定比を掛けた予測完了時刻がsoft以下であることを、
/// 偽なら経過時間がsoft未満であることを要求し、hardの予測による上限は常に守る。
/// 比の既定値は`params`の表に定める。
pub(super) fn should_start_next_iteration(
    elapsed: Duration,
    hit: Duration,
    budget: TimeBudget,
    stable: bool,
) -> bool {
    iteration_prediction_fits(elapsed, hit, budget, stable)
        && (stable || elapsed.saturating_sub(hit) < budget.soft)
}

/// 的中から予測した完了時刻が、開始時の安定性に応じた予算に収まるか。
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

/// 現在の手数から、手番側が今後指すと見込む手数を返す。
pub(super) fn moves_to_go(ply: u32) -> u128 {
    (params::min_moves() as u128)
        .max((params::expected_plies() as u128).saturating_sub(u128::from(ply)) / 2)
}

/// 持ち時間制の予算式を1箇所に集約する。
///
/// 既定係数では、以下の式と一致する。
/// `moves_to_go = max(88, 432.saturating_sub(ply) / 2)`、
/// `soft_raw = remaining / moves_to_go + 0.76 * increment + 0.8 * byoyomi * w`、
/// `safe_hard = max(1ms, (remaining + byoyomi).saturating_sub(30ms))`、
/// `hard = max(1ms, min(4.51 * soft_raw, 0.27 * remaining + 0.8 * byoyomi, safe_hard))`、
/// `soft = min(soft_raw, hard)`とする。
/// `w = min(1, (ply + 4) / 40)`は序盤の係数で、残り時間が正の手の秒読みの項にだけ掛け、
/// 対局開始直後の数手が秒読み相当の長考を使うことを防ぐ
/// （`docs/plans/time-management-efficiency.md`の「採用した方式」）。
/// 秒読みのない時計では序盤の係数が掛かる項は0になる。
/// 係数を変更する場合は自己対局で採否を判定する。
pub(super) fn clock_budget(clock: ClockLimits) -> TimeBudget {
    let remaining = u128::from(clock.remaining_ms);
    let increment = u128::from(clock.increment_ms);
    let byoyomi = u128::from(clock.byoyomi_ms);
    let byoyomi_share = byoyomi * 8 / 10;
    let opening = if remaining > 0 {
        u128::from(clock.ply.saturating_add(4).min(40))
    } else {
        40
    };
    let soft_raw = remaining / moves_to_go(clock.ply)
        + increment * params::increment_share() as u128 / 100
        + byoyomi * 8 * opening / 400;
    let safe_hard = remaining.saturating_add(byoyomi).saturating_sub(30).max(1);
    let hard = (soft_raw * params::hard_soft_ratio() as u128 / 100)
        .min(remaining * params::hard_remaining_share() as u128 / 100 + byoyomi_share)
        .min(safe_hard)
        .max(1);
    TimeBudget {
        soft: Duration::from_millis(to_u64_ms(soft_raw.min(hard))),
        hard: Duration::from_millis(to_u64_ms(hard)),
    }
}

/// 探索制限から時間予算を求める。`movetime`と時計の併用時は小さい方を採る。
pub(super) fn time_budget(limits: &SearchLimits) -> Option<TimeBudget> {
    let limits = limits.finite()?;
    let movetime = limits.movetime_ms.map(|milliseconds| TimeBudget {
        soft: Duration::from_millis(milliseconds.get()),
        hard: Duration::from_millis(milliseconds.get()),
    });
    match (movetime, limits.clock.map(clock_budget)) {
        (Some(fixed), Some(clock)) => Some(TimeBudget {
            soft: fixed.soft.min(clock.soft),
            hard: fixed.hard.min(clock.hard),
        }),
        (Some(budget), None) | (None, Some(budget)) => Some(budget),
        (None, None) => None,
    }
}

/// ミリ秒をu64へ飽和変換する。
fn to_u64_ms(milliseconds: u128) -> u64 {
    milliseconds.min(u128::from(u64::MAX)) as u64
}
