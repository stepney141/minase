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
    /// 比較名、次元32のFMを持つMNPT版3、MNUE版5の組。複数回指定できる。
    #[arg(long, required = true, num_args = 3, value_names = ["NAME", "BASELINE", "CANDIDATE"], action = clap::ArgAction::Append)]
    pair: Vec<String>,
    /// 新規作成するJSON報告。
    #[arg(long)]
    pub(super) output: PathBuf,
}

/// `Report.pairs`の1要素で、同じ標本における基準と候補の比較。
/// `name`は比較名、`baseline`と`candidate`は重みの[`FileIdentity`]、`sample_count`は標本数である。
/// `baseline_statistics`と`candidate_statistics`は全標本の[`Summary`]で、[`Comparison`]の全欄も平坦化して持つ。
/// `verdict`は[`scale_verdict`]の文字列である。
///
/// `by_phase_q`は位相分子`q = min(max(盤上総駒数−2, 0), 90)`で分けた
/// [0, 29]、[30, 59]、[60, 90]の3帯の[`Interval`]を持つ。
/// `by_baseline_absolute_cp`は基準評価の絶対値を0方向へ切り捨てた整数で分けた
/// [0, 99]、[100, 299]、[300, 999]、[1000, 上限なし]の4帯を持つ。
/// 全体または計7帯のいずれかで標本が2件未満、あるいは基準か候補の分散が0なら、報告を作らずに失敗する。
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

/// `diag nnue-distribution`が保存するJSON報告。
/// [`SampleIdentity`]の4欄を最上位へ平坦化し、`population_count`に全局面数、
/// `sampling`に[`SampleArgs::sampling`]の文字列、`pairs`に指定順の[`Pair`]の配列を持つ。
/// `evaluation_perspective = "side_to_move"`、`standard_deviation_ddof = 0`、
/// `percentile_method = "linear"`、`spearman_tie_method = "average"`、
/// `regression_equation = "candidate = intercept + slope * baseline"`、
/// `unchanged_scale_interval_inclusive = [0.9, 1.1]`を固定値として保存する。
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

/// `--pair NAME BASELINE CANDIDATE`の各組を同じ標本で比較する。
/// 引数は1組以上を要求し、NAMEは組の間で一意でなければならない。
/// BASELINEは次元32のFMを持つMNPT版3、CANDIDATEはMNUE版5である。
/// 各局面は[`Record::to_position_for_evaluation`](minase::training::records::Record::to_position_for_evaluation)で復元し、記録された手番側の視点から評価する。
/// 基準は[`pst::evaluate_for_diagnostics`]のPSTとFMを合わせた整数評価、候補は[`IntegerNetwork::evaluate`]の整数評価であり、単位はセンチポーンである。
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
