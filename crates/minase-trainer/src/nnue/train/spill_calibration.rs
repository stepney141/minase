//! 一時ファイルの訓練集合と、保存済み較正JSONの照合。
use crate::data::{
    CpPerLogitEstimate, Error, Result, SpillStore, TeacherCalibration, TeacherClass,
    TeacherCpPerLogitSource,
};
use crate::nn::hex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;

/// 保存済み較正ファイルの形式識別子。
const FORMAT: &str = "minase-trainer-teacher-calibration";

/// 訓練集合だけから尺度を推定するか、6属性で較正項目を照合する。
pub(super) fn resolve(
    dataset: &SpillStore,
    source: &TeacherCpPerLogitSource,
    cp_per_logit: f32,
) -> Result<TeacherCalibration> {
    let (estimates, source) = match source {
        TeacherCpPerLogitSource::Calibrate => {
            (dataset.estimate_teacher_cp_per_logit()?, json!("calibrate"))
        }
        TeacherCpPerLogitSource::File(path) => {
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
    /// 教師分類からλを除いた照合用の6属性を取り出す。
    fn new(class: &TeacherClass) -> Self {
        let value = json!(class);
        Self {
            generation_commit: value["generation_commit"]
                .as_str()
                .expect("serialized class string")
                .to_owned(),
            network_checksum: hex(class.network_checksum()),
            search_budget: class.search_budget(),
            rule_set: value["rule_set"]
                .as_str()
                .expect("serialized class string")
                .to_owned(),
            origin: value["origin"].clone(),
            lookahead: value["lookahead"].clone(),
        }
    }
}

/// 較正JSONの分類ごとの尺度と、作成時の訓練局面数。
#[derive(Serialize, Deserialize)]
struct CalibrationEntry {
    #[serde(flatten)]
    key: ClassKey,
    training_positions: u64,
    teacher_cp_per_logit: f64,
    at_search_bound: bool,
}

/// 保存された較正条件。必須欄と型はデータ層の較正形式に従う。
#[derive(Serialize, Deserialize)]
struct CalibrationConditions {
    inputs: Vec<Value>,
    validation_split: String,
    lookahead: Value,
    precision: String,
    search_range: [f64; 2],
    iterations: u32,
}

/// 較正ファイル全体の必須欄。尺度の値域は項目ごとに検査する。
#[derive(Serialize, Deserialize)]
struct CalibrationFile {
    format: String,
    trainer_version: String,
    conditions: CalibrationConditions,
    // 数値以外（nullなど）の尺度も専用エラーに分類してから復号する。
    classes: Vec<Value>,
}

/// 重複、過不足、空の訓練分類と尺度の値域を検査する。
fn match_calibration(
    document: &Value,
    dataset: &SpillStore,
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

/// 訓練局面だけを走査し、分類別の件数を求める。
fn training_positions(dataset: &SpillStore) -> Result<Vec<u64>> {
    let mut counts = vec![0; dataset.teacher_classes().len()];
    dataset.for_each_training(|record| {
        counts[record.teacher_class as usize] += 1;
        Ok(())
    })?;
    Ok(counts)
}

/// 推定が探索範囲の端へ達した分類を標準エラーへ報告する。
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
