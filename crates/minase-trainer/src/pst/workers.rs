use super::{Error, Result};
use std::collections::HashSet;

/// 指定された並列度、または物理コア数−2を使う。最小値は1。
pub(super) fn workers(threads: Option<usize>) -> Result<usize> {
    if let Some(threads) = threads {
        return if threads == 0 {
            Err(Error::Invalid("--threads must be positive"))
        } else {
            Ok(threads)
        };
    }
    physical_workers(&std::fs::read_to_string("/proc/cpuinfo")?)
}

fn physical_workers(cpuinfo: &str) -> Result<usize> {
    let mut cores = HashSet::new();
    for processor in cpuinfo.split("\n\n").filter(|s| !s.trim().is_empty()) {
        let field = |key: &str| {
            processor.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                (name.trim() == key).then_some(value.trim())
            })
        };
        let package = field("physical id").ok_or(Error::Invalid(
            "CPU topology has no physical id; set --threads explicitly",
        ))?;
        let core = field("core id").ok_or(Error::Invalid(
            "CPU topology has no core id; set --threads explicitly",
        ))?;
        cores.insert((package, core));
    }
    if cores.is_empty() {
        return Err(Error::Invalid(
            "CPU topology is empty; set --threads explicitly",
        ));
    }
    Ok(cores.len().saturating_sub(2).max(1))
}
