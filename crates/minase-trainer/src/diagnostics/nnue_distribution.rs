use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use clap::Args;
use minase::eval::pst;
use serde::Serialize;

use super::{
    Result,
    network::{IntegerNetwork, read_baseline},
    sample::{FileIdentity, SampleArgs, SampleIdentity, file_identity, sample_identity},
    statistics::{
        Comparison, Interval, Summary, compare_intervals, compare_scores, scale_verdict, summarize,
    },
};

/// 全局面を母集団とする評価分布診断の引数。
#[derive(Args)]
pub struct DistributionArgs {
    #[command(flatten)]
    sample: SampleArgs,
    /// 比較名、MNPT版3、MNUE版5の組。複数回指定できる。
    #[arg(long, required = true, num_args = 3, value_names = ["NAME", "BASELINE", "CANDIDATE"], action = clap::ArgAction::Append)]
    pair: Vec<String>,
    /// 新規作成するJSON報告。
    #[arg(long)]
    pub(super) output: PathBuf,
}

#[derive(Serialize)]
struct Pair {
    name: String,
    baseline: FileIdentity,
    candidate: FileIdentity,
    sample_count: usize,
    baseline_statistics: Summary,
    candidate_statistics: Summary,
    #[serde(flatten)]
    comparison: Comparison,
    verdict: &'static str,
    by_phase_q: Vec<Interval>,
    by_baseline_absolute_cp: Vec<Interval>,
}

#[derive(Serialize)]
pub(super) struct Report {
    #[serde(flatten)]
    identity: SampleIdentity,
    population_count: u64,
    sampling: String,
    evaluation_perspective: &'static str,
    standard_deviation_ddof: u8,
    percentile_method: &'static str,
    spearman_tie_method: &'static str,
    regression_equation: &'static str,
    unchanged_scale_interval_inclusive: [f64; 2],
    pairs: Vec<Pair>,
}

pub(super) fn run(args: &DistributionArgs) -> Result<Report> {
    let mut names = HashSet::new();
    for pair in args.pair.as_chunks::<3>().0 {
        if !names.insert(&pair[0]) {
            return Err("pair names must be unique".into());
        }
    }
    let dataset = args.sample.open()?;
    let indices = args.sample.select(&dataset, false)?;
    let mut pairs = Vec::new();
    for pair in args.pair.as_chunks::<3>().0 {
        let (base_path, candidate_path) = (Path::new(&pair[1]), Path::new(&pair[2]));
        let baseline = read_baseline(base_path)?;
        let candidate = IntegerNetwork::read(candidate_path)?;
        let mut base_scores = Vec::with_capacity(indices.len());
        let mut candidate_scores = Vec::with_capacity(indices.len());
        let mut phases = Vec::with_capacity(indices.len());
        for &index in &indices {
            let (position, perspective) = dataset.record(index)?.to_position_for_evaluation()?;
            base_scores.push(f64::from(
                pst::evaluate_for_diagnostics(&baseline, &position, perspective).0,
            ));
            candidate_scores.push(f64::from(candidate.evaluate(&position, perspective)));
            phases.push(i64::from(
                position.occupied().popcount().saturating_sub(2).min(90),
            ));
        }
        let comparison = compare_scores(&base_scores, &candidate_scores)?;
        let absolute: Vec<_> = base_scores.iter().map(|v| v.abs() as i64).collect();
        pairs.push(Pair {
            name: pair[0].clone(),
            baseline: file_identity(base_path)?,
            candidate: file_identity(candidate_path)?,
            sample_count: indices.len(),
            baseline_statistics: summarize(&base_scores)?,
            candidate_statistics: summarize(&candidate_scores)?,
            verdict: scale_verdict(comparison.std_ratio_candidate_over_baseline),
            comparison,
            by_phase_q: compare_intervals(
                &base_scores,
                &candidate_scores,
                &phases,
                &[(0, Some(29)), (30, Some(59)), (60, Some(90))],
            )?,
            by_baseline_absolute_cp: compare_intervals(
                &base_scores,
                &candidate_scores,
                &absolute,
                &[
                    (0, Some(99)),
                    (100, Some(299)),
                    (300, Some(999)),
                    (1000, None),
                ],
            )?,
        });
    }
    Ok(Report {
        identity: sample_identity(&dataset, &indices, args.sample.seed)?,
        population_count: dataset.record_count(),
        sampling: args.sample.sampling(),
        evaluation_perspective: "side_to_move",
        standard_deviation_ddof: 0,
        percentile_method: "linear",
        spearman_tie_method: "average",
        regression_equation: "candidate = intercept + slope * baseline",
        unchanged_scale_interval_inclusive: [0.9, 1.1],
        pairs,
    })
}
