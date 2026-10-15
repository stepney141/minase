//! 教師の勝率尺度の較正、較正ファイルの照合と出所の記録。

use super::{CpPerLogitEstimate, Dataset, Error, Lookahead, Result, TeacherClass};
use crate::{identity::file_identity, nn::hex};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    fs::{self, OpenOptions},
    io::Write,
    num::NonZeroUsize,
    path::PathBuf,
    str::FromStr,
};

const FORMAT: &str = "minase-trainer-teacher-calibration";

/// 教師の勝率尺度を訓練集合から推定するか、較正ファイルから読むかの指定。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TeacherCpPerLogitSource {
    /// この学習の訓練集合から分類ごとに推定する。
    Calibrate,
    /// 指定したJSONの分類と照合し、記録済みの値を使う。
    File(PathBuf),
}

impl FromStr for TeacherCpPerLogitSource {
    type Err = Infallible;

    fn from_str(text: &str) -> std::result::Result<Self, Self::Err> {
        Ok(if text == "calibrate" {
            Self::Calibrate
        } else {
            Self::File(text.into())
        })
    }
}

impl Serialize for TeacherCpPerLogitSource {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Calibrate => serializer.serialize_str("calibrate"),
            Self::File(path) => path.serialize(serializer),
        }
    }
}

/// 学習の分類順に並べた尺度、導出量、境界到達状況と出所。
pub(crate) struct TeacherCalibration {
    pub teacher_cp_per_logit: Vec<Option<f64>>,
    pub teacher_score_multiplier: Vec<Option<f64>>,
    pub at_search_bound: Vec<Option<bool>>,
    pub source: Value,
}

impl TeacherCalibration {
    /// 教師値の計算ではλが0の分類をNaNで表す。
    pub fn target_values(&self) -> Vec<f64> {
        self.teacher_cp_per_logit
            .iter()
            .map(|value| match value {
                Some(value) => *value,
                None => f64::NAN,
            })
            .collect()
    }
}

impl TeacherCpPerLogitSource {
    pub(crate) fn resolve(
        &self,
        dataset: &Dataset,
        cp_per_logit: f32,
    ) -> Result<TeacherCalibration> {
        self.resolve_estimates(dataset, cp_per_logit, || {
            dataset.estimate_teacher_cp_per_logit()
        })
    }

    pub(crate) fn resolve_with_scores(
        &self,
        dataset: &Dataset,
        scores: &[f64],
        cp_per_logit: f32,
    ) -> Result<TeacherCalibration> {
        if scores.len() as u64 != dataset.record_count() {
            return Err(Error::TeacherSamples);
        }
        self.resolve_estimates(dataset, cp_per_logit, || {
            dataset.estimate_teacher_cp_per_logit_with_scores(scores)
        })
    }

    fn resolve_estimates(
        &self,
        dataset: &Dataset,
        cp_per_logit: f32,
        estimate: impl FnOnce() -> Result<Vec<Option<CpPerLogitEstimate>>>,
    ) -> Result<TeacherCalibration> {
        let (estimates, source) = match self {
            Self::Calibrate => (estimate()?, json!("calibrate")),
            Self::File(path) => {
                let path = path.canonicalize()?;
                let document: Value = serde_json::from_slice(&fs::read(&path)?)?;
                let estimates = match_calibration(&document, dataset)?;
                (
                    estimates,
                    json!({"calibration_file": path, "calibration": document}),
                )
            }
        };
        warn_search_bounds(&estimates);
        Ok(TeacherCalibration {
            teacher_cp_per_logit: estimates
                .iter()
                .map(|value| value.map(|v| v.value))
                .collect(),
            teacher_score_multiplier: estimates
                .iter()
                .map(|value| value.map(|v| f64::from(cp_per_logit) / v.value))
                .collect(),
            at_search_bound: estimates
                .iter()
                .map(|value| value.map(|v| v.at_search_bound))
                .collect(),
            source,
        })
    }
}

/// λを除く6属性で較正項目と教師分類を照合する。
/// 由来と先読みはTeacherClassのJSON表現を使い、エンジン側の型に復号経路を加えない。
#[derive(Serialize, Deserialize, PartialEq)]
struct ClassKey {
    generation_commit: String,
    network_checksum: String,
    search_budget: u64,
    rule_set: String,
    origin: Value,
    lookahead: Value,
}

impl ClassKey {
    fn new(class: &TeacherClass) -> Self {
        Self {
            generation_commit: class.generation_commit.clone(),
            network_checksum: hex(&class.network_checksum),
            search_budget: class.search_budget,
            rule_set: class.rule_set.clone(),
            origin: json!(class.origin),
            lookahead: json!(class.lookahead),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct CalibrationEntry {
    #[serde(flatten)]
    key: ClassKey,
    training_positions: usize,
    teacher_cp_per_logit: f64,
    at_search_bound: bool,
}

#[derive(Serialize, Deserialize)]
struct CalibrationConditions {
    inputs: Vec<Value>,
    validation_split: String,
    lookahead: Value,
    precision: String,
    search_range: [f64; 2],
    iterations: u32,
}

#[derive(Serialize, Deserialize)]
struct CalibrationFile {
    format: String,
    trainer_version: String,
    conditions: CalibrationConditions,
    // 数値以外（nullなど）の尺度も専用エラーに分類してから復号する。
    classes: Vec<Value>,
}

fn match_calibration(
    document: &Value,
    dataset: &Dataset,
) -> Result<Vec<Option<CpPerLogitEstimate>>> {
    if document.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(Error::CalibrationFormat);
    }
    let file: CalibrationFile = serde_json::from_value(document.clone())?;
    let mut entries: Vec<CalibrationEntry> = Vec::with_capacity(file.classes.len());
    for (entry, value) in file.classes.into_iter().enumerate() {
        if !value
            .get("teacher_cp_per_logit")
            .and_then(Value::as_f64)
            .is_some_and(|v| v.is_finite() && v > 0.0)
        {
            return Err(Error::CalibrationScale { entry });
        }
        let value: CalibrationEntry = serde_json::from_value(value)?;
        if entries.iter().any(|previous| previous.key == value.key) {
            return Err(Error::DuplicateCalibrationClass { entry });
        }
        entries.push(value);
    }
    if dataset.teacher_lambdas().len() != dataset.teacher_classes().len() {
        return Err(Error::TeacherLambda);
    }
    let counts = training_positions(dataset)?;
    let keys: Vec<_> = dataset
        .teacher_classes()
        .iter()
        .map(ClassKey::new)
        .collect();
    let mut estimates = vec![None; keys.len()];
    for (entry, value) in entries.into_iter().enumerate() {
        let class = keys
            .iter()
            .position(|key| *key == value.key)
            .ok_or(Error::UnexpectedCalibrationClass { entry })?;
        if dataset.teacher_lambdas()[class] != 0.0 {
            estimates[class] = Some(CpPerLogitEstimate {
                value: value.teacher_cp_per_logit,
                at_search_bound: value.at_search_bound,
            });
        }
    }
    for (class, &lambda) in dataset.teacher_lambdas().iter().enumerate() {
        if lambda != 0.0 {
            if counts[class] == 0 {
                return Err(Error::EmptyTeacherClass { class });
            }
            if estimates[class].is_none() {
                return Err(Error::MissingCalibrationClass { class });
            }
        }
    }
    Ok(estimates)
}

fn training_positions(dataset: &Dataset) -> Result<Vec<usize>> {
    let mut counts = vec![0; dataset.teacher_classes().len()];
    for &index in dataset.training_indices() {
        counts[dataset.teacher_class(index)? as usize] += 1;
    }
    Ok(counts)
}

fn warn_search_bounds(estimates: &[Option<CpPerLogitEstimate>]) {
    for (class, estimate) in estimates.iter().enumerate() {
        if let Some(estimate) = estimate.filter(|value| value.at_search_bound) {
            eprintln!(
                "warning: teacher class {class}: teacher_cp_per_logit={} reached a search bound",
                estimate.value
            );
        }
    }
}

/// 教師の勝率尺度を訓練集合から較正し、JSONへ保存する引数。
#[derive(Clone, Debug, clap::Args)]
pub struct CalibrateArgs {
    /// MNSDを入力順に指定する。較正中は変更しないこと。
    #[arg(long, required = true, num_args = 1..)]
    pub data: Vec<PathBuf>,
    /// 先読み割引率。0より大きく1より小さい有限値。
    #[arg(long, requires = "lookahead_plies")]
    pub lookahead_gamma: Option<f64>,
    /// 先読み幅。1以上4096以下。
    #[arg(long, requires = "lookahead_gamma")]
    pub lookahead_plies: Option<u16>,
    /// 新規に作成する較正JSON。既存のパスは拒否する。
    #[arg(long)]
    pub output: PathBuf,
    /// CPUのスレッド数。省略時は物理コア数−2、最小1。
    #[arg(long)]
    pub threads: Option<usize>,
}

/// 学習と同じ分割・先読み・f32の保持値で較正し、既存ファイルを上書きせずに保存する。
/// 実戦対局は探索値を使わないので、較正ファイルの項目に含めない。
pub fn calibrate(args: CalibrateArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let lookahead = match (args.lookahead_gamma, args.lookahead_plies) {
        (None, None) => None,
        (Some(gamma), Some(plies)) => Some(Lookahead::new(gamma, plies)?),
        _ => return Err(Error::LookaheadParameters.into()),
    };
    match fs::symlink_metadata(&args.output) {
        Ok(_) => return Err(Error::CalibrationOutputExists { path: args.output }.into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let workers = NonZeroUsize::new(crate::nn::workers::workers(args.threads)?)
        .ok_or(crate::nn::Error::Invalid("worker count must be positive"))?;
    let dataset = Dataset::open(&args.data, lookahead, workers)?.with_lambda(1.0)?;
    let estimates = dataset.estimate_teacher_cp_per_logit()?;
    warn_search_bounds(&estimates);
    let counts = training_positions(&dataset)?;
    let mut classes = Vec::new();
    for (class, estimate) in estimates.into_iter().enumerate() {
        if let Some(estimate) = estimate {
            classes.push(serde_json::to_value(CalibrationEntry {
                key: ClassKey::new(&dataset.teacher_classes()[class]),
                training_positions: counts[class],
                teacher_cp_per_logit: estimate.value,
                at_search_bound: estimate.at_search_bound,
            })?);
        }
    }
    let file = CalibrationFile {
        format: FORMAT.into(),
        trainer_version: env!("CARGO_PKG_VERSION").into(),
        conditions: CalibrationConditions {
            inputs: dataset
                .paths()
                .map(|path| Ok(serde_json::to_value(file_identity(path)?)?))
                .collect::<Result<_>>()?,
            validation_split: "hash64(seed, game) % 20 == 0".into(),
            lookahead: json!(lookahead),
            precision: "f64".into(),
            search_range: [50.0, 4000.0],
            iterations: 64,
        },
        classes,
    };
    let bytes = serde_json::to_vec_pretty(&file)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.output)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                Error::CalibrationOutputExists {
                    path: args.output.clone(),
                }
            } else {
                Error::Io(error)
            }
        })?;
    output.write_all(&bytes)?;
    output.write_all(b"\n")?;
    Ok(())
}
