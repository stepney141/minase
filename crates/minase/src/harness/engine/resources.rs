//! エンジンのCPU時間と最大常駐メモリの取得。

use crate::harness::engine::process::EngineProcess;
#[cfg(target_os = "linux")]
use procfs::process::Process;

/// 終局時に読み取る1エンジンの資源使用量。
#[derive(Clone, Copy, Default)]
pub(in crate::harness) struct EngineResourceUsage {
    /// プロセス全体のユーザー時間とシステム時間の合計(ns)。
    pub(in crate::harness) cpu_time_ns: Option<u64>,
    /// プロセスが記録した最大常駐メモリ(byte)。
    pub(in crate::harness) peak_rss_bytes: Option<u64>,
}

impl EngineProcess {
    /// 子プロセスが終了する前にCPU時間と最大常駐メモリを読み取る。
    pub(in crate::harness) fn resource_usage(&self) -> EngineResourceUsage {
        process_resource_usage(self.child.id())
    }
}

/// Linuxのprocfsからプロセス全体の資源使用量を読み取る。
#[cfg(target_os = "linux")]
fn process_resource_usage(pid: u32) -> EngineResourceUsage {
    let Ok(pid) = i32::try_from(pid) else {
        return EngineResourceUsage::default();
    };
    let Ok(process) = Process::new(pid) else {
        return EngineResourceUsage::default();
    };
    resource_usage_from_counters(
        process.stat().ok().map(|stat| (stat.utime, stat.stime)),
        procfs::ticks_per_second(),
        process.status().ok().and_then(|status| status.vmhwm),
    )
}

/// CPU tickをnsへ、VmHWMのKiBをbyteへ換算する。欠測や桁あふれはNoneとする。
#[cfg(target_os = "linux")]
fn resource_usage_from_counters(
    cpu_ticks: Option<(u64, u64)>,
    ticks_per_second: u64,
    peak_rss_kib: Option<u64>,
) -> EngineResourceUsage {
    let cpu_time_ns = cpu_ticks.and_then(|(user, system)| {
        let ticks = u128::from(user) + u128::from(system);
        let nanos = ticks
            .checked_mul(1_000_000_000)?
            .checked_div(u128::from(ticks_per_second))?;
        u64::try_from(nanos).ok()
    });
    EngineResourceUsage {
        cpu_time_ns,
        peak_rss_bytes: peak_rss_kib.and_then(|kib| kib.checked_mul(1024)),
    }
}

/// procfsがないOSでは欠測を明示する。
#[cfg(not(target_os = "linux"))]
const fn process_resource_usage(_pid: u32) -> EngineResourceUsage {
    EngineResourceUsage {
        cpu_time_ns: None,
        peak_rss_bytes: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn resource_counters_convert_units_and_preserve_missing_values() {
        // (7+4) / 3秒をnsへ換算し、0方向へ丸める。123 KiB = 125952 byte。
        let usage = resource_usage_from_counters(Some((7, 4)), 3, Some(123));
        assert_eq!(usage.cpu_time_ns, Some(3_666_666_666));
        assert_eq!(usage.peak_rss_bytes, Some(125_952));
        let missing = resource_usage_from_counters(None, 100, None);
        assert_eq!(missing.cpu_time_ns, None);
        assert_eq!(missing.peak_rss_bytes, None);
        let invalid = resource_usage_from_counters(Some((1, 1)), 0, Some(u64::MAX));
        assert_eq!(invalid.cpu_time_ns, None);
        assert_eq!(invalid.peak_rss_bytes, None);
        let overflow = resource_usage_from_counters(Some((u64::MAX, u64::MAX)), 1, Some(0));
        assert_eq!(overflow.cpu_time_ns, None);
        assert_eq!(overflow.peak_rss_bytes, Some(0));
        let wide = resource_usage_from_counters(Some((u64::MAX, u64::MAX)), u64::MAX, None);
        assert_eq!(wide.cpu_time_ns, Some(2_000_000_000));
        assert_eq!(wide.peak_rss_bytes, None);
    }
}
