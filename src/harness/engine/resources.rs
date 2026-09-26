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
    let cpu_time_ns = process.stat().ok().and_then(|stat| {
        let ticks = u128::from(stat.utime) + u128::from(stat.stime);
        let nanos = ticks
            .checked_mul(1_000_000_000)?
            .checked_div(u128::from(procfs::ticks_per_second()))?;
        u64::try_from(nanos).ok()
    });
    let peak_rss_bytes = process
        .status()
        .ok()
        .and_then(|status| status.vmhwm)
        .and_then(|kib| kib.checked_mul(1024));
    EngineResourceUsage {
        cpu_time_ns,
        peak_rss_bytes,
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
    use std::process;

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_resource_probe_reports_current_process_cpu_and_memory() {
        let usage = process_resource_usage(process::id());
        assert!(usage.cpu_time_ns.is_some());
        assert!(usage.peak_rss_bytes.is_some_and(|bytes| bytes > 0));
    }
}
