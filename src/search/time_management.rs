//! 局面適応の4係数と、探索間・反復間の履歴。
use super::{Move, TimeBudget};
use std::fmt;
use std::time::Duration;

/// 4係数の積に掛ける正規化の定数。
///
/// Stockfishの定数は深さ20〜30の探索で係数の積が1前後になるよう調整されており、
/// 深さ6〜12のminaseでは`reduction`が上限近くに張り付き、最善手の交替も多いため、
/// 積の中央値が2.08になった（[time-management-efficiency-stage3-bench]）。
/// 中央値の手が標準予算をそのまま使うよう、積を中央値で割る。
/// STCの開始前に1回だけ行う定数の見直しであり、以後は変えない。
///
/// [time-management-efficiency-stage3-bench]: ../../docs/measurements/time-management-efficiency-stage3-bench.md
const TOTAL_SCALE: f64 = 0.48;

/// 同じ対局の直前のgoから引き継ぐ値。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TimeHistory {
    pub(crate) average_score: Option<f64>,
    pub(crate) time_reduction: f64,
}

impl Default for TimeHistory {
    fn default() -> Self {
        Self {
            average_score: None,
            time_reduction: 1.0,
        }
    }
}

/// 最後の反復で停止判断に使った係数と、次の探索へ渡す状態。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeReport {
    pub(crate) history: TimeHistory,
    falling: f64,
    reduction: f64,
    instability: f64,
    effort: f64,
    total_ms: f64,
    soft_ms: f64,
    hard_ms: f64,
}

impl TimeReport {
    /// 完了反復がない場合は係数を掛けず、持ち越し値も更新しない。
    pub(super) fn new(budget: Option<TimeBudget>, history: TimeHistory) -> Self {
        let (soft_ms, hard_ms) = budget.map_or((0.0, 0.0), |budget| {
            (
                budget.soft.as_secs_f64() * 1000.0,
                budget.hard.as_secs_f64() * 1000.0,
            )
        });
        Self {
            history,
            falling: 1.0,
            reduction: 1.0,
            instability: 1.0,
            effort: 1.0,
            total_ms: soft_ms,
            soft_ms,
            hard_ms,
        }
    }

    /// 等号では継続し、totalとhardの小さい方を超えた完了反復で停止する。
    pub(super) fn should_stop(self, elapsed: Duration, budget: TimeBudget) -> bool {
        elapsed.as_secs_f64() * 1000.0 > self.total_ms.min(budget.hard.as_secs_f64() * 1000.0)
    }
}

impl fmt::Display for TimeReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "info string timeman falling={:.6} reduction={:.6} instability={:.6} effort={:.6} total={:.6} soft={:.6} hard={:.6}",
            self.falling,
            self.reduction,
            self.instability,
            self.effort,
            self.total_ms,
            self.soft_ms,
            self.hard_ms
        )
    }
}

pub(super) fn interpolate(x: f64, x1: f64, x2: f64, y1: f64, y2: f64) -> f64 {
    y1 + (y2 - y1) * ((x - x1) / (x2 - x1)).clamp(0.0, 1.0)
}

/// 完了反復だけを数えるgoごとの状態。
pub(super) struct TimeSignals {
    previous: TimeHistory,
    scores: Option<[i32; 4]>,
    score_index: usize,
    best: Option<Move>,
    last_change_depth: u32,
    total_changes: f64,
}

impl TimeSignals {
    pub(super) fn new(previous: TimeHistory) -> Self {
        Self {
            previous,
            scores: None,
            score_index: 0,
            best: None,
            last_change_depth: 1,
            total_changes: 0.0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn complete_iteration(
        &mut self,
        root_depth: u32,
        best: Move,
        score: i32,
        pawn_value: i32,
        changes: u32,
        best_nodes: u64,
        nodes: u64,
        budget: Option<TimeBudget>,
    ) -> TimeReport {
        let scores = self.scores.get_or_insert([score; 4]);
        let old_score = scores[self.score_index];
        scores[self.score_index] = score;
        self.score_index = (self.score_index + 1) % 4;
        if self.best.is_some_and(|previous| previous != best) {
            self.last_change_depth = root_depth;
        }
        self.best = Some(best);
        self.total_changes = self.total_changes / 2.0 + f64::from(changes);
        let time_reduction = interpolate(
            f64::from(root_depth - self.last_change_depth),
            4.96,
            18.79,
            0.639,
            1.712,
        )
        .clamp(0.629, 1.544);
        let score = f64::from(score);
        let previous_score = self.previous.average_score.unwrap_or(score);
        let units = 208.0 / f64::from(pawn_value);
        let mut report = TimeReport::new(
            budget,
            TimeHistory {
                average_score: None,
                time_reduction,
            },
        );
        report.falling = ((11.48
            + 2.30 * (previous_score - score) * units
            + 1.1 * (f64::from(old_score) - score) * units)
            / 100.0)
            .clamp(0.576, 1.728);
        report.reduction = (1.468 + self.previous.time_reduction) / (2.284 * time_reduction);
        report.instability = 1.077 + 2.229 * self.total_changes;
        // 完了反復は根のノードを含むためnodesは正である。
        let nodes_effort = best_nodes as f64 / nodes as f64 * 100_000.0;
        report.effort =
            interpolate(nodes_effort, 75_800.0, 104_510.0, 0.969, 0.714).clamp(0.693, 0.838);
        if budget.is_some_and(|budget| budget.adaptive) {
            report.total_ms *= TOTAL_SCALE
                * report.falling
                * report.reduction
                * report.instability
                * report.effort;
        } else {
            // 固定時間・深さ・ノード・無期限探索では係数を停止判断に使わない。
            report.falling = 1.0;
            report.reduction = 1.0;
            report.instability = 1.0;
            report.effort = 1.0;
        }
        report
    }
}

#[cfg(test)]
mod tests;
