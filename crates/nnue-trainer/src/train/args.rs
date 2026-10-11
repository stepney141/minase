use super::{Error, Result};
use crate::{
    data::{InputFeatureSet, Lookahead, MAX_BATCH_SIZE},
    model::Shape,
};
use serde::Serialize;
use std::{collections::HashSet, path::PathBuf};

/// GPU学習の入力と実行条件。最適化器の係数は固定する。
#[derive(Clone, Debug, clap::Args, Serialize)]
pub struct Args {
    /// Input feature set.
    #[arg(long, value_enum)]
    pub feature_set: InputFeatureSet,
    /// Even hidden width, 2..=512.
    #[arg(long)]
    pub width: usize,
    /// MNSD files, including their provenance sidecars, in global-record order.
    #[arg(long, required = true, num_args = 1..)]
    pub data: Vec<PathBuf>,
    /// Uncompressed NPZ with the initial floating-point PST.
    #[arg(long)]
    pub init_pst: PathBuf,
    /// Baseline MNPT supplying the PST, piece values, output K and rule-set name.
    #[arg(long)]
    pub base: PathBuf,
    /// Directory for configuration, logs, checkpoints and exported networks.
    #[arg(long)]
    pub output_dir: PathBuf,
    /// Lookahead discount, strictly between 0 and 1; requires --lookahead-plies.
    #[arg(long, requires = "lookahead_plies")]
    pub lookahead_gamma: Option<f64>,
    /// Lookahead window, 1..=4096; requires --lookahead-gamma.
    #[arg(long, requires = "lookahead_gamma")]
    pub lookahead_plies: Option<u16>,
    /// Maximum positions per update; the final batch can be shorter.
    #[arg(long, default_value_t = 16_384)]
    pub batch: usize,
    /// Total epoch budget, including completed epochs on resume, 1..=50.
    #[arg(long, default_value_t = 50)]
    pub epochs: u64,
    /// Stop after this many completed epochs without improvement.
    #[arg(long, default_value_t = 5)]
    pub patience: u64,
    /// Initialization, permutation and validation-sample seed.
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// Cumulative wall-clock budget including preprocessing and validation, at most 24 hours.
    #[arg(long, default_value_t = 24.0)]
    pub time_limit_hours: f64,
    /// Maximum validation positions for the quantization error check.
    #[arg(long, default_value_t = 10_000)]
    pub validation_sample: usize,
    /// Batch workers; omit for physical cores minus 2, with a minimum of 1.
    #[arg(long)]
    pub threads: Option<usize>,
    /// Resume the latest checkpoint in this directory; must equal --output-dir.
    #[arg(long)]
    pub resume: Option<PathBuf>,
}

impl Args {
    /// ファイルやGPUに触れる前に値と引数の組合せを検査する。
    pub fn validate(&self) -> Result<()> {
        Shape::new(self.feature_set, self.width, 1.0)?;
        if self.data.is_empty() {
            return Err(Error::Invalid("--data requires at least one file").into());
        }
        if !(1..=MAX_BATCH_SIZE).contains(&self.batch) {
            return Err(
                Error::Invalid("--batch exceeds the reverse-index capacity or is zero").into(),
            );
        }
        if !(1..=50).contains(&self.epochs) {
            return Err(Error::Invalid("--epochs must be in 1..=50").into());
        }
        if self.patience == 0 || self.validation_sample == 0 || self.threads == Some(0) {
            return Err(Error::Invalid(
                "--patience, --validation-sample and --threads must be positive",
            )
            .into());
        }
        if !self.time_limit_hours.is_finite()
            || !(0.0 < self.time_limit_hours && self.time_limit_hours <= 24.0)
        {
            return Err(Error::Invalid("--time-limit-hours must be finite and in (0, 24]").into());
        }
        self.lookahead()?;
        Ok(())
    }

    pub(super) fn lookahead(&self) -> Result<Option<Lookahead>> {
        match (self.lookahead_gamma, self.lookahead_plies) {
            (None, None) => Ok(None),
            (Some(gamma), Some(plies)) => Ok(Some(Lookahead::new(gamma, plies)?)),
            _ => Err(Error::Invalid(
                "lookahead requires both --lookahead-gamma and --lookahead-plies",
            )
            .into()),
        }
    }

    pub(super) fn workers(&self) -> Result<usize> {
        if let Some(n) = self.threads {
            return Ok(n);
        }
        let cpuinfo = std::fs::read_to_string("/proc/cpuinfo")?;
        physical_workers(&cpuinfo)
    }
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
        return Err(Error::Invalid("CPU topology is empty; set --threads explicitly").into());
    }
    Ok(cores.len().saturating_sub(2).max(1))
}
