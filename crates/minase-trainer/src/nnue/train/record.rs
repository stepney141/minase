use super::{Args, Error, Result, backend::Validation};
use crate::nn::{Sha256Hasher, atomic_write, hex};
use crate::nnue::options::DeviceKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

/// 学習前と各superbatchの確定時に記録する統計。
#[derive(Debug, Serialize, Deserialize)]
pub struct SuperbatchRecord {
    /// 学習を実行するデバイス。
    pub device: DeviceKind,
    /// 確定したsuperbatch数。0は学習前の検証。
    pub superbatch: u64,
    /// 更新数を1周の更新数で割った値。
    pub epochs: f64,
    /// 訓練局面数で重み付けした平均損失。
    pub train_loss: Option<f64>,
    /// 全検証局面の平均損失。
    pub validation_loss: Option<f64>,
    /// 構造固有の重みと検証の統計。
    pub statistics: Value,
    /// 供給と転送を含む当該superbatchの学習秒数。
    pub training_seconds: Option<f64>,
    /// 検証の所要秒数。
    pub validation_seconds: Option<f64>,
    /// 更新局面数を学習秒数で割った速度。
    pub training_positions_per_second: Option<f64>,
    /// 1更新の平均計算秒数。CUDAではイベントで測る。
    pub update_seconds: Option<f64>,
    /// プロセス内で観測したデバイス全体の最大使用バイト数。
    pub gpu_memory_used_bytes: Option<usize>,
    /// デバイス全体の総バイト数。
    pub gpu_memory_total_bytes: Option<usize>,
    /// 再開前を含む累積秒数。
    pub elapsed_seconds: f64,
    /// 当該superbatchの更新数。
    pub steps: u64,
    /// 確定した累積更新数。
    pub total_steps: u64,
    /// 最初の更新の学習率倍率。
    pub lr_multiplier_first: Option<f64>,
    /// 最後の更新の学習率倍率。
    pub lr_multiplier_last: Option<f64>,
}

fn file_record(path: &Path) -> Result<Value> {
    let mut hash = Sha256Hasher::default();
    let mut file = std::fs::File::open(path)?;
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(json!({"path": path.canonicalize()?, "sha256": hex(&hash.finalize())}))
}

pub(super) fn configuration(
    args: &Args,
    workers: usize,
    dataset: super::store::DatasetInfo<'_>,
    architecture: Value,
) -> Result<Value> {
    let mut settings = serde_json::to_value(args)?;
    let object = settings
        .as_object_mut()
        .ok_or(Error::Invalid("arguments are not an object"))?;
    for key in [
        "output_dir",
        "resume",
        "data",
        "superbatches",
        "time_limit_hours",
        "threads",
        "validation_superbatches",
        "validation_sample",
        "bucket_positions",
        "scratch_dir",
    ] {
        object.remove(key);
    }
    if args.validation_patience.is_some() {
        object.insert(
            "validation_superbatches".into(),
            json!(args.validation_superbatches),
        );
    }
    object.insert("rules".into(), json!(args.parsed_rules()?.to_string()));
    object.insert("architecture".into(), architecture.clone());
    let files = dataset
        .paths
        .into_iter()
        .map(file_record)
        .collect::<Result<Vec<_>>>()?;
    let mut preprocessing = json!({
        "teacher_classes": dataset.classes, "teacher_lambdas": dataset.lambdas,
        "teacher_cp_per_logit": args.teacher_cp_per_logit,
        "training_positions": dataset.training, "validation_positions": dataset.validation,
        "training_indices_sha256": dataset.training_hash,
        "validation_indices_sha256": dataset.validation_hash,
    });
    if let Some(subsample) = dataset.subsample {
        preprocessing["train_subsample"] = serde_json::to_value(subsample)?;
    }
    Ok(
        json!({"device": args.device, "arguments": args, "resolved_threads": workers,
        "architecture": architecture, "settings": settings, "inputs": {"files": files},
        "preprocessing": preprocessing, "resumes": []}),
    )
}

/// 再開の時刻、変更できる予算と実行資源、開始時点のsuperbatch数を保存する。
pub(super) fn append_resume(
    path: &Path,
    config: &mut Value,
    args: &Args,
    workers: usize,
    superbatch: u64,
    resumed_at: SystemTime,
) -> Result<()> {
    let resumed_at = resumed_at.duration_since(UNIX_EPOCH)?.as_secs();
    let resumes = config
        .as_object_mut()
        .ok_or(Error::Invalid("configuration must be an object"))?
        .get_mut("resumes")
        .and_then(Value::as_array_mut)
        .ok_or(Error::Invalid("configuration resumes must be an array"))?;
    resumes.push(json!({
        "resumed_at_unix_seconds": resumed_at,
        "arguments": args,
        "superbatches": args.superbatches,
        "time_limit_hours": args.time_limit_hours,
        "workers": workers,
        "validation_superbatches": args.validation_superbatches,
        "completed_superbatches": superbatch,
        "bucket_positions": args.bucket_positions,
        "scratch_dir": args.scratch_dir,
    }));
    write_json(path, config)
}

pub(super) fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(atomic_write(path, &bytes)?)
}

pub(super) fn append_superbatch(path: &Path, record: &SuperbatchRecord) -> Result<()> {
    append_record(path, record)
}

fn append_record(path: &Path, record: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec(record)?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

#[derive(Default)]
pub(super) struct ValidationHistory {
    pub last: Option<(u64, f64)>,
    pub best: Option<(u64, f64)>,
    pub unimproved: u64,
}

impl ValidationHistory {
    /// 更新前の測定は記録だけに使い、同率なら先の重みを残す。
    pub fn record(&mut self, superbatch: u64, loss: f64) {
        self.last = Some((superbatch, loss));
        if superbatch > 0 {
            if self.best.is_none_or(|(_, best)| loss < best) {
                self.best = self.last;
                self.unimproved = 0;
            } else {
                self.unimproved += 1;
            }
        }
    }
}

/// 確定したsuperbatchまで連番を検査し、以後の行と改行のない末尾を捨てる。
/// ログの追記後に`latest.npz`を保存し、その保存を確定の時点とする。
/// 確定済みの行だけから検証の履歴を復元する。
pub(super) fn reconcile_log(path: &Path, superbatch: u64) -> Result<ValidationHistory> {
    let bytes = std::fs::read(path)?;
    let mut keep = 0;
    let mut history = ValidationHistory::default();
    for (expected, line) in bytes.split_inclusive(|&b| b == b'\n').enumerate() {
        let expected = u64::try_from(expected)?;
        if line.last() != Some(&b'\n') {
            break;
        }
        let record: SuperbatchRecord = serde_json::from_slice(line)?;
        if record.superbatch != expected
            || record
                .validation_loss
                .is_some_and(|loss| !loss.is_finite() || loss < 0.0)
        {
            return Err(
                Error::Invalid("training log does not match checkpoint superbatches").into(),
            );
        }
        if let Some(loss) = record.validation_loss {
            history.record(record.superbatch, loss);
        }
        if expected == 0 && history.last.is_none() {
            return Err(Error::Invalid("training log has no initial validation").into());
        }
        keep += line.len();
        if expected == superbatch {
            if keep != bytes.len() {
                eprintln!(
                    "discarding training log entries after checkpoint superbatch {superbatch}"
                );
                atomic_write(path, &bytes[..keep])?;
            }
            return Ok(history);
        }
    }
    Err(Error::Invalid("training log is missing a completed superbatch").into())
}

/// 予算を保存済みの進捗まで短縮した場合に、最後の行の検証を補完する。
/// 確定した重みと訓練記録は保ち、検証の統計と累積秒数だけを更新する。
pub(super) fn complete_final_validation(
    path: &Path,
    validation: &Validation,
    statistics: Value,
    seconds: f64,
    elapsed: f64,
) -> Result<()> {
    let text = std::fs::read_to_string(path)?;
    let terminated = text
        .strip_suffix('\n')
        .ok_or(Error::Invalid("training log has an incomplete final row"))?;
    let (prefix, line) = terminated
        .rsplit_once('\n')
        .ok_or(Error::Invalid("training log has no completed superbatch"))?;
    let mut record: SuperbatchRecord = serde_json::from_str(line)?;
    record.validation_loss = Some(validation.loss());
    record.validation_seconds = Some(seconds);
    record.statistics = statistics;
    record.elapsed_seconds = elapsed;
    let mut bytes = prefix.as_bytes().to_vec();
    bytes.push(b'\n');
    serde_json::to_writer(&mut bytes, &record)?;
    bytes.push(b'\n');
    Ok(atomic_write(path, &bytes)?)
}
