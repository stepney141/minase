//! 調整した係数とエンジン異常件数の表示。

use super::{model::Settings, params};
use minase::harness::FailureCounts;

pub(super) fn print_theta(settings: &Settings, theta: &[f64]) {
    for (p, value) in settings.parameters.iter().zip(theta) {
        println!("{} = {value}", p.name);
    }
}

pub(super) fn parameter_line(d: &params::Declaration) -> String {
    format!(
        "{}, {}, {}, {}, {}, 0.002",
        d.name,
        d.default,
        d.min,
        d.max,
        (f64::from(d.max) - f64::from(d.min)) / 6.0
    )
}

/// 異常の理由別件数を`match_runner`の最終サマリと同じ書式で表す。
pub(super) fn failure_text(failures: FailureCounts) -> String {
    format!(
        "illegal_moves={} crashes={} timeouts={} time_forfeits={} rejected_moves={}",
        failures.illegal_moves,
        failures.crashes,
        failures.timeouts,
        failures.time_forfeits,
        failures.rejected_moves
    )
}
