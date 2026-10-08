//! 探索方式に共通する1手の時間予算。

use std::time::Duration;

use crate::search::alphabeta::params;
use crate::search::limits::{ClockLimits, SearchLimits};

/// 1手に使う時間の予算。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct TimeBudget {
    /// 完了イテレーションの境界で停止する目安時間。
    pub(crate) soft: Duration,
    /// 探索途中でも打ち切る上限時間。
    pub(super) hard: Duration,
    /// 対象の探索の秒読み期では、予測を使わず締切まで反復を始める。
    pub(super) byoyomi_period: bool,
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
/// 秒読みが正の時計では、`time_budget`が`safe_hard`の30 msに代えてUSIオプション`ByoyomiMargin`の値を
/// [`clock_budget_with_margin`]へ渡し、持ち時間が尽きた手では秒読みから同じ余裕を引いた締切をsoftとhardの両方にする。
/// 係数を変更する場合は自己対局で採否を判定する。
pub(super) fn clock_budget(clock: ClockLimits) -> TimeBudget {
    clock_budget_with_margin(clock, 30)
}

/// 「締切の余裕を絶対値のUSIオプションにする理由」に従い安全上限の余裕を適用する。
///
/// 式は`docs/plans/byoyomi-time-usage.md`の「設計判断」に従う。
fn clock_budget_with_margin(clock: ClockLimits, margin_ms: u64) -> TimeBudget {
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
    let safe_hard = remaining
        .saturating_add(byoyomi)
        .saturating_sub(u128::from(margin_ms))
        .max(1);
    let hard = (soft_raw * params::hard_soft_ratio() as u128 / 100)
        .min(remaining * params::hard_remaining_share() as u128 / 100 + byoyomi_share)
        .min(safe_hard)
        .max(1);
    TimeBudget {
        soft: Duration::from_millis(to_u64_ms(soft_raw.min(hard))),
        hard: Duration::from_millis(to_u64_ms(hard)),
        byoyomi_period: false,
    }
}

/// 秒読みの変更対象となる時計を返す。
///
/// `docs/plans/byoyomi-time-usage.md`の「設計判断」の対象の探索の定義に従い、
/// 秒読みが正で、固定時間・深さ・ノード数の制限を併用しない時計だけを選ぶ。
pub(super) fn byoyomi_clock(limits: &SearchLimits) -> Option<ClockLimits> {
    let limits = limits.finite()?;
    limits.clock.filter(|clock| {
        clock.byoyomi_ms > 0
            && limits.movetime_ms.is_none()
            && limits.depth.is_none()
            && limits.nodes.is_none()
    })
}

/// 探索制限から時間予算を求める。`movetime`と時計の併用時は小さい方を採る。
pub(crate) fn time_budget(limits: &SearchLimits) -> Option<TimeBudget> {
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
