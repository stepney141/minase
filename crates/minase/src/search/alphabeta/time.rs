//! αβ探索の反復の開始判定。

use std::time::Duration;

use crate::search::time::TimeBudget;
use minase_core::mv::Move;

use super::params;

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
/// `docs/plans/byoyomi-time-usage.md`の「秒読み期の締切」に従い、
/// 対象の探索の秒読み期では的中後の経過時間だけを締切と比べる。
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
