//! 測定機とrunnerの識別。

use super::sha256_file;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, io, path::PathBuf};

/// 実行バイナリの識別情報。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessRecord {
    /// Cargoパッケージの版。
    pub version: String,
    /// 実行ファイル全体のSHA-256。
    pub sha256: String,
}

/// 測定に使うCPUの識別情報。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpuRecord {
    /// CPUの機種名。
    pub model: String,
    /// 取得できた場合の物理コア数。
    pub physical_cores: Option<usize>,
    /// OSが報告した論理コア数。
    pub logical_cores: usize,
    /// OSが報告した実メモリ容量(byte)。
    pub physical_memory_bytes: Option<u64>,
}

/// Linuxで取得できるCPU機種名を返す。
pub fn cpu_model() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("model name")
                    .and_then(|line| line.split_once(':'))
                    .map(|(_, value)| value.trim().to_owned())
            })
        })
        .unwrap_or_else(|| "unreported".to_owned())
}

/// Linuxがオンラインと報告する論理CPU番号を展開する。
#[cfg(target_os = "linux")]
fn online_cpu_indices() -> Option<BTreeSet<usize>> {
    parse_online_cpu_indices(&fs::read_to_string("/sys/devices/system/cpu/online").ok()?)
}

/// sysfsのCPU一覧を展開し、逆順や整数でない範囲を拒否する。
#[cfg(target_os = "linux")]
fn parse_online_cpu_indices(text: &str) -> Option<BTreeSet<usize>> {
    let mut indices = BTreeSet::new();
    for range in text.trim().split(',') {
        let (start, end) = range
            .split_once('-')
            .map_or((range, range), |(start, end)| (start, end));
        let start = start.parse::<usize>().ok()?;
        let end = end.parse::<usize>().ok()?;
        if start > end {
            return None;
        }
        indices.extend(start..=end);
    }
    Some(indices)
}

/// LinuxのCPUトポロジーからオンライン物理コア数を得る。
#[cfg(target_os = "linux")]
pub fn physical_core_count() -> Option<usize> {
    let online = online_cpu_indices()?;
    let topology = online
        .into_iter()
        .map(|cpu| {
            let path = PathBuf::from(format!("/sys/devices/system/cpu/cpu{cpu}/topology"));
            Some((
                fs::read_to_string(path.join("physical_package_id")).ok()?,
                fs::read_to_string(path.join("core_id")).ok()?,
            ))
        })
        .collect::<Option<Vec<_>>>()?;
    count_physical_cores(&topology)
}

/// 同じパッケージ内の同じコアを1個として数える。欠測はNoneとする。
#[cfg(target_os = "linux")]
fn count_physical_cores(topology: &[(String, String)]) -> Option<usize> {
    let mut cores = BTreeSet::new();
    for (package, core) in topology {
        cores.insert((
            package.trim().parse::<u32>().ok()?,
            core.trim().parse::<u32>().ok()?,
        ));
    }
    (!cores.is_empty()).then_some(cores.len())
}

/// CPUトポロジーを提供しないOSでは物理コア数を欠測とする。
#[cfg(not(target_os = "linux"))]
pub const fn physical_core_count() -> Option<usize> {
    None
}

/// Linuxの`MemTotal`から実メモリ容量を得る。
#[cfg(target_os = "linux")]
pub fn physical_memory_bytes() -> Option<u64> {
    parse_physical_memory_bytes(&fs::read_to_string("/proc/meminfo").ok()?)
}

/// /proc/meminfoのMemTotalをKiBからbyteへ換算する。
#[cfg(target_os = "linux")]
fn parse_physical_memory_bytes(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

/// 実メモリ容量を提供しないOSでは欠測とする。
#[cfg(not(target_os = "linux"))]
pub const fn physical_memory_bytes() -> Option<u64> {
    None
}

/// 現在の対局ハーネス実行ファイルをSHA-256で識別する。
pub fn harness_record() -> io::Result<HarnessRecord> {
    Ok(HarnessRecord {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        sha256: sha256_file(&std::env::current_exe()?)?,
    })
}

/// 現在の測定機の構成を記録する。
pub fn cpu_record() -> io::Result<CpuRecord> {
    Ok(CpuRecord {
        model: cpu_model(),
        physical_cores: physical_core_count(),
        logical_cores: std::thread::available_parallelism()?.get(),
        physical_memory_bytes: physical_memory_bytes(),
    })
}

/// 前回の起動からrunnerまたは測定機が変わった場合に1行通知する。
pub fn notify_invocation_change(
    runner: &HarnessRecord,
    cpu: &CpuRecord,
    previous_runner: &HarnessRecord,
    previous_cpu: &CpuRecord,
) {
    if runner.sha256 != previous_runner.sha256 || cpu != previous_cpu {
        eprintln!(
            "notice: runner SHA-256 or measurement machine differs from the previous invocation"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_host_resource_text_is_parsed_without_host_dependencies() {
        assert_eq!(
            parse_online_cpu_indices("0-2,4,6-7\n"),
            Some(BTreeSet::from([0, 1, 2, 4, 6, 7]))
        );
        assert_eq!(
            parse_online_cpu_indices("0-2,1-3"),
            Some(BTreeSet::from([0, 1, 2, 3]))
        );
        for invalid in ["", "2-1", "x", "0-1-2"] {
            assert_eq!(parse_online_cpu_indices(invalid), None);
        }
        let topology = [("0\n", "0\n"), ("0", "0"), ("0", "1"), ("1", "0")]
            .map(|(package, core)| (package.to_owned(), core.to_owned()));
        assert_eq!(count_physical_cores(&topology), Some(3));
        assert_eq!(count_physical_cores(&[]), None);
        assert_eq!(count_physical_cores(&[("bad".into(), "0".into())]), None);
        assert_eq!(count_physical_cores(&[("0".into(), "bad".into())]), None);
        // 12345 KiB × 1024 = 12641280 byte。MemFreeではなくMemTotalを読む。
        assert_eq!(
            parse_physical_memory_bytes("MemFree: 12 kB\nMemTotal: 12345 kB\n"),
            Some(12_641_280)
        );
        for missing in [
            "",
            "MemFree: 12345 kB",
            "MemTotal:",
            "MemTotal: bad kB",
            "MemTotal: 18446744073709551615 kB",
        ] {
            assert_eq!(parse_physical_memory_bytes(missing), None);
        }
    }
}
