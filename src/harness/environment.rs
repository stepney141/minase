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
    let text = fs::read_to_string("/sys/devices/system/cpu/online").ok()?;
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
    let mut cores = BTreeSet::new();
    for cpu in online {
        let topology = PathBuf::from(format!("/sys/devices/system/cpu/cpu{cpu}/topology"));
        let package = fs::read_to_string(topology.join("physical_package_id"))
            .ok()?
            .trim()
            .parse::<u32>()
            .ok()?;
        let core = fs::read_to_string(topology.join("core_id"))
            .ok()?
            .trim()
            .parse::<u32>()
            .ok()?;
        cores.insert((package, core));
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
    fs::read_to_string("/proc/meminfo")
        .ok()?
        .lines()
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
