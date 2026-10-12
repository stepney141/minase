//! 固定した帯別標本で基準PSTと候補PSTを比較する。

use super::{
    Result,
    pst_positions::{self, Agreement, Representative},
    pst_reference::{self, Features, FloatWeights, Models, Weights},
    pst_sample,
    pst_statistics::{self, ErrorSummary, OutcomeMetrics},
};
use crate::{
    data::{Dataset, Lookahead, TeacherClass},
    pst::export::numpy_sum,
};
use clap::Args;
use minase::training::provenance::{ResultOrigin, SearchCondition, StartOrigin};
use serde::Serialize;
use std::{collections::BTreeMap, fs, path::PathBuf};

/// 教師分類・位相帯の損失、整数一致、量子化と局面操作を診断する引数。
#[derive(Args)]
pub struct PstCompareArgs {
    /// 来歴を伴うMNSD。実行中は元ファイルを変更・切り詰めしないこと。
    #[arg(long, required = true, num_args = 1..)]
    pub data: Vec<PathBuf>,
    /// 自己対局の教師の混合比λを上書きする。省略時は来歴を使う。
    #[arg(long)]
    pub lambda_override: Option<f64>,
    /// 教師探索値の先読みの減衰率。
    #[arg(long, requires = "lookahead_plies")]
    pub lookahead_gamma: Option<f64>,
    /// 教師探索値の先読みの手数。
    #[arg(long, requires = "lookahead_gamma")]
    pub lookahead_plies: Option<u16>,
    /// 基準のMNPT。
    #[arg(long)]
    pub base: PathBuf,
    /// 候補のMNPT。
    #[arg(long)]
    pub candidate: PathBuf,
    /// 候補の実数PSTのmiddlegame、endgame配列を持つNPZ。
    #[arg(long)]
    pub candidate_float: PathBuf,
    /// 成果物を保存する新規ディレクトリ。
    #[arg(long)]
    pub output_dir: PathBuf,
    /// 教師分類・帯ごとの最大標本数。
    #[arg(long, default_value_t = 10_000)]
    pub sample_size: usize,
    /// SplitMix64による標本抽出のシード。
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
}

#[derive(Serialize)]
struct ClassMetadata {
    generation_commit: String,
    network_checksum: String,
    nodes: u32,
    rule_set: String,
    search_condition: SearchCondition,
    result_origin: ResultOrigin,
    start_origin: StartOrigin,
    lookahead_gamma: Option<f64>,
    lookahead_plies: Option<u16>,
    lambda_override: Option<f64>,
    lambda: f64,
}
impl ClassMetadata {
    fn new(class: &TeacherClass, lambda: f64) -> Self {
        Self {
            generation_commit: class.generation_commit.clone(),
            network_checksum: class
                .network_checksum
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            nodes: class.nodes,
            rule_set: class.rule_set.clone(),
            search_condition: class.search_condition,
            result_origin: class.result_origin,
            start_origin: class.start_origin,
            lookahead_gamma: class.lookahead.map(Lookahead::gamma),
            lookahead_plies: class.lookahead.map(Lookahead::plies),
            lambda_override: class.lambda_override,
            lambda,
        }
    }
}

#[derive(Default, Serialize)]
struct Exclusions {
    mate_band: usize,
    tactical: usize,
    depth_incomplete: usize,
    total: usize,
}
#[derive(Serialize)]
struct Band {
    generation: usize,
    band: usize,
    range: String,
    training_records: usize,
    validation_records: usize,
    samples: usize,
    indices_file: String,
    validation_loss: Models<Option<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    teacher_comparison_excluded: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    teacher_comparison_reason: Option<&'static str>,
    base: Option<ErrorSummary>,
    candidate: Option<ErrorSummary>,
}
#[derive(Serialize)]
struct Quantization {
    samples: usize,
    mean_absolute_error_cp: f64,
    max_absolute_error_cp: f64,
    limit_cp: f64,
}
#[derive(Serialize)]
struct Derived {
    states: Vec<usize>,
    fixed: Vec<i32>,
    base_middlegame: Vec<i32>,
    base_endgame: Vec<i32>,
    candidate_middlegame: Vec<i32>,
    candidate_endgame: Vec<i32>,
}

// 構造体のフィールド順でJSONのキー順を定める。
#[derive(Serialize)]
struct Report {
    sample_size: usize,
    seed: u64,
    teacher_classes: Vec<ClassMetadata>,
    lookahead: Option<Lookahead>,
    lambda_override: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome_metrics: Option<Models<OutcomeMetrics>>,
    rescore_exclusions: Exclusions,
    teacher_comparison_excluded: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    k: Option<Models<f64>>,
    teacher_ks: Vec<Option<f64>>,
    generation_checksums: Vec<String>,
    piece_values: Vec<i32>,
    bands: Vec<Band>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rust_agreement: Option<Models<usize>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quantization: Option<Quantization>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pst_removal_sign_reversals: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rust_removal_agreement: Option<Models<Agreement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    representatives: Option<Vec<Representative>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rust_promotion_agreement: Option<Models<usize>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    derived_piece_values: Option<Derived>,
    // 失敗時だけ理由を加え、報告の保存を保証する。
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub(super) fn run(args: &PstCompareArgs) -> Result<()> {
    // 版3だけは報告先の作成前に拒否し、読み込みエラーは診断報告へ保存する。
    let models = Models {
        base: Weights::read(&args.base),
        candidate: Weights::read(&args.candidate),
    };
    for (name, model) in models.each() {
        if let Ok(weights) = model
            && weights.engine.fm_weights().is_some()
        {
            return Err(format!(
                "--{name}: PST comparison diagnostics require PST-only weights (MNPT version 2); \
                 MNPT version 3 includes FM. Convert it to version 2 with \
                 `minase-trainer fm scale --fm-scale 0` before comparing."
            )
            .into());
        }
    }
    fs::create_dir(&args.output_dir)?;
    let mut report = Report {
        sample_size: args.sample_size,
        seed: args.seed,
        teacher_classes: Vec::new(),
        lookahead: None,
        lambda_override: args.lambda_override,
        outcome_metrics: None,
        rescore_exclusions: Exclusions::default(),
        teacher_comparison_excluded: 0,
        k: None,
        teacher_ks: Vec::new(),
        generation_checksums: Vec::new(),
        piece_values: Vec::new(),
        bands: Vec::new(),
        rust_agreement: None,
        quantization: None,
        pst_removal_sign_reversals: None,
        rust_removal_agreement: None,
        representatives: None,
        rust_promotion_agreement: None,
        derived_piece_values: None,
        error: None,
    };
    let result = diagnose(args, models, &mut report);
    if let Err(error) = &result {
        report.error = Some(error.to_string());
    }
    super::save(
        &args.output_dir.join("report.json"),
        &report,
        report.error.as_deref(),
    )?;
    result
}

fn diagnose(
    args: &PstCompareArgs,
    models: Models<Result<Weights>>,
    report: &mut Report,
) -> Result<()> {
    if args.sample_size == 0 {
        return Err("diagnostic sample size must be positive".into());
    }
    report.lookahead = match (args.lookahead_gamma, args.lookahead_plies) {
        (None, None) => None,
        (Some(gamma), Some(plies)) => Some(Lookahead::new(gamma, plies)?),
        _ => return Err("both lookahead-gamma and lookahead-plies are required".into()),
    };
    let models = Models {
        base: models.base?,
        candidate: models.candidate?,
    };
    report.piece_values = (0..47)
        .map(|s| models.base.engine.piece_value_of_state(s))
        .collect();
    if (0..47).any(|s| report.piece_values[s] != models.candidate.engine.piece_value_of_state(s)) {
        return Err("candidate piece values differ from the base".into());
    }
    report.k = Some(Models {
        base: models.base.k(),
        candidate: models.candidate.k(),
    });
    let float = FloatWeights::read(&args.candidate_float)?;
    eprintln!("loading diagnostic data");
    // SAFETY: --dataの実行中の不変性をCLI契約で要求する。この診断は入力を書き換えない。
    let dataset = unsafe { Dataset::open(&args.data, report.lookahead, args.lambda_override)? };
    report.teacher_classes = dataset
        .teacher_classes()
        .iter()
        .zip(dataset.teacher_lambdas())
        .map(|(class, &lambda)| ClassMetadata::new(class, lambda))
        .collect();
    report.generation_checksums = report
        .teacher_classes
        .iter()
        .map(|class| class.network_checksum.clone())
        .collect();
    eprintln!("recomputing f64 teacher scores and calibrating teacher K");
    let teacher_scores = dataset.diagnostic_teacher_scores()?;
    let teacher_ks = pst_statistics::teacher_ks(&dataset, &teacher_scores)?;
    report.teacher_ks = teacher_ks
        .iter()
        .map(|&k| k.is_finite().then_some(k))
        .collect();
    eprintln!("selecting phase-band samples and computing validation statistics");
    let sampling = pst_sample::select(&dataset, args.sample_size, args.seed)?;
    let validation = pst_statistics::validation(&dataset, &models, &teacher_scores, &teacher_ks)?;
    report.outcome_metrics = Some(validation.outcomes);
    report.teacher_comparison_excluded = sampling
        .validation
        .iter()
        .zip(dataset.teacher_lambdas())
        .filter(|(_, lambda)| **lambda == 0.0)
        .map(|(counts, _)| counts.iter().sum::<usize>())
        .sum();
    let mut union = Vec::new();
    let mut representatives = BTreeMap::<usize, u64>::new();
    for (generation, bands) in sampling.indices.iter().enumerate() {
        for (band, indices) in bands.iter().enumerate() {
            let filename = pst_sample::filename(generation, band);
            pst_sample::write_indices(&args.output_dir.join(&filename), indices)?;
            let count = sampling.validation[generation][band];
            let empty = indices.is_empty();
            let inactive = dataset.teacher_lambdas()[generation] == 0.0;
            let mut entry = Band {
                generation,
                band,
                range: format!(
                    "[{:.1}, {:.1}{}",
                    band as f64 / 5.0,
                    (band + 1) as f64 / 5.0,
                    if band == 4 { "]" } else { ")" }
                ),
                training_records: sampling.training[generation][band],
                validation_records: count,
                samples: indices.len(),
                indices_file: filename,
                validation_loss: Models {
                    base: (count != 0)
                        .then(|| validation.losses.base[generation][band] / count as f64),
                    candidate: (count != 0)
                        .then(|| validation.losses.candidate[generation][band] / count as f64),
                },
                reason: empty.then_some("empty band"),
                teacher_comparison_excluded: (!empty).then_some(if inactive {
                    indices.len()
                } else {
                    0
                }),
                teacher_comparison_reason: (!empty && inactive).then_some("lambda is zero"),
                base: None,
                candidate: None,
            };
            if let Some(&first) = indices.first() {
                representatives
                    .entry(band)
                    .and_modify(|previous| *previous = (*previous).min(first))
                    .or_insert(first);
                union.extend(indices);
                if !inactive {
                    let raw: Vec<_> = indices
                        .iter()
                        .map(|&index| teacher_scores[index as usize])
                        .collect();
                    // 換算尺度は基準の比較にも候補のKを使う。
                    let scaled: Vec<_> = raw
                        .iter()
                        .map(|score| score * (models.candidate.k() / teacher_ks[generation]))
                        .collect();
                    let mut base = Vec::with_capacity(indices.len());
                    let mut candidate = Vec::with_capacity(indices.len());
                    for &index in indices {
                        let features = Features::new(&dataset.record(index)?);
                        base.push(f64::from(models.base.evaluate(&features)));
                        candidate.push(f64::from(models.candidate.evaluate(&features)));
                    }
                    entry.base = Some(pst_statistics::error_summary(&base, &scaled, &raw));
                    entry.candidate =
                        Some(pst_statistics::error_summary(&candidate, &scaled, &raw));
                }
            }
            report.bands.push(entry);
        }
    }
    if union.is_empty() {
        return Err("every band is empty".into());
    }
    union.sort_unstable();
    let records = union
        .iter()
        .map(|&index| dataset.record(index))
        .collect::<crate::data::Result<Vec<_>>>()?;
    pst_positions::write_mnsd(
        &args.output_dir.join("diagnostic-samples.bin"),
        &records,
        *models.base.engine.checksum(),
    )?;
    eprintln!(
        "checking {} sampled positions against the engine",
        records.len()
    );
    report.rust_agreement = Some(Models {
        base: pst_positions::check(&models.base, &records, "base")?,
        candidate: pst_positions::check(&models.candidate, &records, "candidate")?,
    });
    let errors: Vec<f32> = records
        .iter()
        .map(|record| {
            let features = Features::new(record);
            (float.evaluate(&features) - models.candidate.evaluate(&features) as f32).abs()
        })
        .collect();
    let mean = numpy_sum(&errors) / errors.len() as f32;
    report.quantization = Some(Quantization {
        samples: errors.len(),
        mean_absolute_error_cp: f64::from(mean),
        max_absolute_error_cp: f64::from(errors.iter().copied().fold(0.0, f32::max)),
        limit_cp: 2.0,
    });
    if !mean.is_finite() || mean > 2.0 {
        return Err(format!("quantization mean absolute error {mean} exceeds 2 cp").into());
    }
    let selected = representatives
        .into_iter()
        .map(|(band, index)| Ok((band, index, dataset.record(index)?)))
        .collect::<Result<Vec<_>>>()?;
    let positions = pst_positions::diagnose(&models, &selected, &args.output_dir)?;
    report.pst_removal_sign_reversals = Some(positions.reversals);
    report.rust_removal_agreement = Some(positions.removals);
    report.representatives = Some(positions.representatives);
    report.rust_promotion_agreement = Some(positions.promotions);
    report.derived_piece_values = Some(Derived {
        states: pst_reference::STATES.to_vec(),
        fixed: pst_reference::STATES
            .iter()
            .map(|&s| models.base.engine.piece_value_of_state(s))
            .collect(),
        base_middlegame: pst_reference::derived(&models.base, 0),
        base_endgame: pst_reference::derived(&models.base, 1),
        candidate_middlegame: pst_reference::derived(&models.candidate, 0),
        candidate_endgame: pst_reference::derived(&models.candidate, 1),
    });
    if positions.reversals != 0 {
        return Err("PST removal delta reverses the baseline sign".into());
    }
    Ok(())
}
