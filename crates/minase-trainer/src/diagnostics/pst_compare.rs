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
    /// 自己対局の分類のλを有限な0以上1以下の値で上書きし、人間対局には適用しない。省略時は来歴のλを使う。
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

/// `report.json`の`teacher_classes`の1要素。
/// [`TeacherClass`]の由来7項目に、適用条件4項目を加えたものである。
/// `generation_commit`は生成コミット、`network_checksum`は教師ネットの32バイトの検査和を小文字16進64文字で表した値、`nodes`はu32である。
/// `rule_set`は規則名、`search_condition`は`"in-game"`または`"standalone"`、
/// `result_origin`は`"selfplay"`または`"human"`、`start_origin`は`"random"`または`"human-game"`である。
/// `lookahead_gamma`はf64または`null`、`lookahead_plies`はu16または`null`である。
/// `lambda_override`はその分類に適用した上書き値で、人間対局の分類と上書きなしでは`null`、`lambda`は実効λである。
/// 学習の`config.json`の`preprocessing.teacher_classes`は検査和を32個の数値の配列で保存するが、この報告は16進文字列を使う。
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

/// `report.json`の`rescore_exclusions`。
/// `mate_band`、`tactical`、`depth_incomplete`、`total`のusizeの4欄を持つ。
/// この診断は再採点をしないため、すべて常に0である。
#[derive(Default, Serialize)]
struct Exclusions {
    mate_band: usize,
    tactical: usize,
    depth_incomplete: usize,
    total: usize,
}
/// `report.json`の`bands`の1要素で、教師分類と位相帯の組ごとの比較。
/// `generation`は0始まりの教師分類番号、`band`は[`pst_sample::band`]による0〜4の帯番号である。
/// `range`は帯Bのφの区間`[B/5, (B+1)/5)`を小数1桁で表し、帯4だけ右端を含む`"[0.8, 1.0]"`とする。
/// `training_records`と`validation_records`は分類と帯ごとの全局面数、
/// `samples = min(sample_size, validation_records)`、`indices_file`は標本の通算番号を保存したNPYのファイル名である。
/// `validation_loss`は`{base, candidate}`で、それぞれ全検証局面の平均二値交差エントロピーを持つ。
/// 計算は[`pst_statistics::validation`]に従い、検証局面0件なら両モデルの値を`null`にする。
///
/// 標本が空の帯だけ`reason = "empty band"`を持つ。
/// 標本が空でない帯だけ`teacher_comparison_excluded`を持ち、実効λが0なら標本数、それ以外は0である。
/// 標本が空でなくλが0の帯だけ`teacher_comparison_reason = "lambda is zero"`も持つ。
/// `base`と`candidate`は教師探索値との[`ErrorSummary`]で、空の帯とλが0の分類では`null`とする。
/// 標本の教師探索値を`raw`とし、先読み指定時はf64で再計算した先読み値を使う。
/// 換算値は`raw × (候補の出力用K / その分類の教師用K)`で、基準の比較にも候補のKを使う。
/// モデル評価は独立した整数PST参照による手番側視点のセンチポーンである。
/// 検証損失、量子化検査、整数評価の照合にはλが0の分類も含める。
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
/// `report.json`の`quantization`で、候補の実数PSTと候補の整数PSTの評価差を記録する。
/// 標本は全帯の標本の和集合で、実効λが0の分類も含み、`samples`に局面数を保存する。
/// `mean_absolute_error_cp`はf32の絶対誤差を[`numpy_sum`]の順で足し、局面数で割ってからf64へ広げた平均である。
/// `max_absolute_error_cp`は絶対誤差の最大、`limit_cp = 2.0`は上限である。
/// 平均が2.0を超えるか非有限なら診断は失敗する。
/// 実数評価は[`FloatWeights::evaluate`]のf32の2端点和を`linear::blend`でφ補間した値、
/// 整数評価は[`Weights::evaluate`]の独立参照である。
#[derive(Serialize)]
struct Quantization {
    samples: usize,
    mean_absolute_error_cp: f64,
    max_absolute_error_cp: f64,
    limit_cp: f64,
}
/// `report.json`の`derived_piece_values`で、端点ごとの静的な駒価値を固定の探索用駒価値と比べる。
/// `states`は[`pst_reference::STATES`]の35状態、`fixed`は基準MNPTの各状態の探索用駒価値である。
/// `base_middlegame`、`base_endgame`、`candidate_middlegame`、`candidate_endgame`は
/// 各モデルの序中盤端点0と終盤端点1から[`pst_reference::derived`]で導く値である。
#[derive(Serialize)]
struct Derived {
    states: Vec<usize>,
    fixed: Vec<i32>,
    base_middlegame: Vec<i32>,
    base_endgame: Vec<i32>,
    candidate_middlegame: Vec<i32>,
    candidate_endgame: Vec<i32>,
}

/// PST比較診断の`report.json`の最上位の欄。
/// JSONのキー順は構造体の欄の順である。
/// `skip_serializing_if`の欄は、値を計算する前に失敗した場合には出力しない。
/// 教師分類の番号は欄名で`generation`とも呼ぶ。
#[derive(Serialize)]
struct Report {
    /// `--sample-size`の値で、教師分類と位相帯ごとの最大標本数。
    sample_size: usize,
    /// `--seed`の値で、SplitMix64の抽出シード。
    seed: u64,
    /// 入力順に初めて現れた順の教師分類。各要素の欄は[`ClassMetadata`]に従う。
    teacher_classes: Vec<ClassMetadata>,
    /// 先読み条件`{gamma: f64, plies: u16}`。指定がなければ`null`。
    lookahead: Option<Lookahead>,
    /// `--lambda-override`の値。省略時は`null`。
    lambda_override: Option<f64>,
    /// 全検証集合の`{base: OutcomeMetrics, candidate: OutcomeMetrics}`。
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome_metrics: Option<Models<OutcomeMetrics>>,
    /// 再採点の除外件数。[`Exclusions`]の規約によりすべて0。
    rescore_exclusions: Exclusions,
    /// 実効λが0の分類に属する全検証局面数。抽出標本数ではない。
    teacher_comparison_excluded: usize,
    /// 基準と候補のMNPTの出力用Kを持つ`{base: f64, candidate: f64}`。
    #[serde(skip_serializing_if = "Option::is_none")]
    k: Option<Models<f64>>,
    /// 分類順の教師用K。λが0の分類は`null`。
    teacher_ks: Vec<Option<f64>>,
    /// 分類順の教師ネットの検査和。各分類の`network_checksum`と同じ小文字16進文字列。
    generation_checksums: Vec<String>,
    /// 基準MNPTの駒状態0〜46の探索用駒価値47個。成功時は候補との全件一致を検査済み。
    piece_values: Vec<i32>,
    /// 分類順、各分類内では帯0〜4の順の[`Band`]の配列。
    bands: Vec<Band>,
    /// 全帯の標本の和集合で、エンジン評価と独立整数PST参照が一致した件数の`{base, candidate}`。
    /// 1件でも不一致なら失敗するため、記録される値は各モデルとも標本局面数に等しい。
    #[serde(skip_serializing_if = "Option::is_none")]
    rust_agreement: Option<Models<usize>>,
    /// 候補の実数と整数の評価差を測る[`Quantization`]。
    #[serde(skip_serializing_if = "Option::is_none")]
    quantization: Option<Quantization>,
    /// 代表局面の全除去で基準と候補のPST除去差分の積が負となった件数。非0なら不合格。
    #[serde(skip_serializing_if = "Option::is_none")]
    pst_removal_sign_reversals: Option<usize>,
    /// 除去局面の整数評価の照合件数を持つ`{base: Agreement, candidate: Agreement}`。
    #[serde(skip_serializing_if = "Option::is_none")]
    rust_removal_agreement: Option<Models<Agreement>>,
    /// 初期局面、次に帯番号の昇順に並べた[`Representative`]の配列。
    #[serde(skip_serializing_if = "Option::is_none")]
    representatives: Option<Vec<Representative>>,
    /// 代表局面の成り手で、エンジン評価と独立参照の一致を確かめた手の総数の`{base, candidate}`。
    #[serde(skip_serializing_if = "Option::is_none")]
    rust_promotion_agreement: Option<Models<usize>>,
    /// 端点から導いた静的な駒価値と固定の探索用駒価値を持つ[`Derived`]。
    #[serde(skip_serializing_if = "Option::is_none")]
    derived_piece_values: Option<Derived>,
    /// 失敗時だけ加える失敗理由の文字列。
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// MNPT版2のPSTを比較し、検査結果を報告先へ保存する。
///
/// [`super::run`]は`--output-dir`が既に存在すれば何も作らずに拒否する。
/// 基準か候補がFMを含むMNPT版3なら、この関数も出力ディレクトリを作る前に拒否する。
/// 版3は`minase-trainer fm scale --fm-scale 0`で版2へ変換できる。
/// それ以外は`fs::create_dir`で出力先を作り、親ディレクトリは作らない。
/// 作成後の診断エラーでは、その時点までの欄に失敗理由の`error`を加えた`report.json`を保存してから返す。
/// 作成済みの`.npy`と`.bin`も残し、エラーを返すとコマンドは終了コード1で終わる。
/// 報告自体の入出力エラーでは保存に失敗することがある。
///
/// `--sample-size = 0`、先読み条件の片方だけの指定、重みと実数PSTの読み込み失敗、
/// 基準と候補の47個の探索用駒価値の不一致、データ読み込みと教師用K推定の失敗を拒否する。
/// 全帯の標本が空の場合も停止する。
/// 標本、代表局面、復元できる除去局面、成り手について、エンジンと独立した整数PST参照が不一致なら失敗する。
/// 量子化の平均絶対誤差が2 cpを超えるか非有限の場合、またはPST除去差分の厳密な符号反転が1件以上の場合も不合格である。
///
/// 保存物は[`Report`]の`report.json`、分類と帯ごとの`diagnostic-indices-generationN-bandB.npy`、
/// および[`pst_positions`]に定める`diagnostic-samples.bin`、`representatives.bin`、`removals.bin`である。
/// NPYの仕様は[`pst_sample::write_indices`]に従う。
/// 教師分類、先読み、λの上書き、対局単位の検証分割は学習と同じ[`Dataset::open`]で決まる。
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
