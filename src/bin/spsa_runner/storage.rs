//! 実行条件の一致検査と、適用順によるθの復元。

use super::model::{IterationRecord, Settings, flips, update};
use minase::harness::{
    CpuRecord, EngineIdentity, FailureCounts, HarnessRecord, RunLock, TerminationRecord,
    atomic_write_json, failure_from_stored, lock_run_directory,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub engine: EngineIdentity,
    pub engine_sha256: String,
    pub settings: Settings,
    pub each: String,
    pub rules_source: String,
    pub canonical_rules: Vec<String>,
    pub max_ply: u32,
    pub response_timeout_secs: u64,
    pub cpu: CpuRecord,
    pub runner: HarnessRecord,
}

#[derive(Debug)]
pub(super) struct Store {
    root: PathBuf,
    _lock: RunLock,
}

pub(super) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn read_json<T: for<'a> Deserialize<'a>>(path: &Path) -> io::Result<T> {
    serde_json::from_reader(File::open(path)?)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

impl Store {
    pub fn create(root: &Path, manifest: &Manifest) -> io::Result<Self> {
        fs::create_dir(root)?;
        let lock = lock_run_directory(root)?;
        fs::create_dir(root.join("iterations"))?;
        atomic_write_json(
            root,
            &root.join(".manifest.json.tmp"),
            &root.join("manifest.json"),
            manifest,
        )?;
        Ok(Self {
            root: root.to_owned(),
            _lock: lock,
        })
    }

    pub fn resume(
        root: &Path,
        expected: &Manifest,
    ) -> io::Result<(Self, BTreeMap<u64, IterationRecord>)> {
        let lock = lock_run_directory(root)?;
        let manifest: Manifest = read_json(&root.join("manifest.json"))?;
        if &manifest != expected {
            return Err(invalid("manifest does not match requested session"));
        }
        let mut records = BTreeMap::new();
        let directory = root.join("iterations");
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                return Err(invalid("unexpected non-file in iterations"));
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| invalid("non-UTF-8 iteration filename"))?;
            if let Some(digits) = name
                .strip_prefix('.')
                .and_then(|s| s.strip_suffix(".json.tmp"))
                && digits.len() == 20
                && digits.bytes().all(|b| b.is_ascii_digit())
            {
                fs::remove_file(entry.path())?;
                continue;
            }
            let k = name
                .strip_suffix(".json")
                .filter(|s| s.len() == 20 && s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| invalid("invalid iteration filename"))?;
            let record: IterationRecord = read_json(&entry.path())?;
            if record.k != k || k == 0 || k > expected.settings.iterations {
                return Err(invalid("iteration filename or number mismatch"));
            }
            if records.insert(k, record).is_some() {
                return Err(invalid("duplicate iteration"));
            }
        }
        validate_chain(&expected.settings, &records)?;
        Ok((
            Self {
                root: root.to_owned(),
                _lock: lock,
            },
            records,
        ))
    }

    pub fn save(&self, record: &IterationRecord) -> io::Result<()> {
        let directory = self.root.join("iterations");
        atomic_write_json(
            &directory,
            &directory.join(format!(".{:020}.json.tmp", record.k)),
            &directory.join(format!("{:020}.json", record.k)),
            record,
        )
    }
}

/// 適用番号の連続性、摂動の再現性、および保存θの更新連鎖を検査する。
pub(super) fn validate_chain(
    settings: &Settings,
    records: &BTreeMap<u64, IterationRecord>,
) -> io::Result<Vec<f64>> {
    let mut by_application: Vec<_> = records.values().collect();
    by_application.sort_by_key(|r| r.application);
    let mut theta = settings.initial_theta();
    let mut numbers = BTreeSet::new();
    for (index, record) in by_application.iter().enumerate() {
        if record.application != index as u64 + 1
            || record.k == 0
            || record.k > settings.iterations
            || !numbers.insert(record.k)
        {
            return Err(invalid(
                "application numbers must be unique and contiguous from 1",
            ));
        }
        if record.flip != flips(settings, record.k)
            || record.pairs.len() != settings.pairs_per_iteration
            || record.theta.len() != theta.len()
        {
            return Err(invalid("invalid iteration dimensions or flip"));
        }
        let mut failures = FailureCounts::default();
        for (j, pair) in record.pairs.iter().enumerate() {
            if pair.values.number
                != (record.k - 1) * settings.pairs_per_iteration as u64 + j as u64 + 1
                || pair.category.is_some_and(|c| c > 4)
                || pair.values.plus.len() != theta.len()
                || pair.values.minus.len() != theta.len()
            {
                return Err(invalid("invalid pair number, category or dimensions"));
            }
            for ((p, plus), minus) in settings
                .parameters
                .iter()
                .zip(&pair.values.plus)
                .zip(&pair.values.minus)
            {
                if !(p.min..=p.max).contains(plus) || !(p.min..=p.max).contains(minus) {
                    return Err(invalid("saved integer outside parameter range"));
                }
            }
            // 手数上限で打ち切った局を含むペアだけが破棄され、分類を持たない。
            let cutoff = pair
                .terminations
                .iter()
                .any(|t| matches!(t, TerminationRecord::Cutoff));
            if cutoff != pair.category.is_none() {
                return Err(invalid("pair category does not match cutoff terminations"));
            }
            let mut observed = FailureCounts::default();
            for termination in &pair.terminations {
                if let TerminationRecord::Forfeit { reason, .. } = termination {
                    observed.record(failure_from_stored(*reason));
                }
            }
            if observed != pair.failures {
                return Err(invalid("failure counts do not match terminations"));
            }
            let abnormal: Vec<_> = pair
                .terminations
                .iter()
                .filter(|t| matches!(t, TerminationRecord::Forfeit { .. }))
                .collect();
            if abnormal.len() != pair.abnormal_games.len()
                || abnormal
                    .iter()
                    .zip(&pair.abnormal_games)
                    .any(|(t, saved)| **t != saved.game.termination)
            {
                return Err(invalid("abnormal game records do not match terminations"));
            }
            failures.add(observed);
        }
        if record.d != record.pairs.iter().map(|p| p.difference()).sum::<i64>()
            || record.failures != failures
        {
            return Err(invalid("iteration totals do not match pairs"));
        }
        theta = update(settings, &theta, record.k, &record.flip, record.d);
        if theta
            .iter()
            .zip(&record.theta)
            .any(|(a, b)| a.to_bits() != b.to_bits())
        {
            return Err(invalid("saved theta does not match application chain"));
        }
    }
    Ok(theta)
}
