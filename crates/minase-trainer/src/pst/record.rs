use super::{Args, Error, Result, export::QuantizationReport};
use crate::{data::Dataset, nn::atomic_write};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

/// PSTの1エポックの記録。学習前の損失と速度はnull。
#[derive(Debug, Serialize)]
pub struct EpochRecord {
    /// CPUで実行したことを記録する。
    pub device: &'static str,
    /// 完了したエポック。0は学習前の検証。
    pub epoch: u64,
    /// 訓練集合の二値交差エントロピーの平均。
    pub train_loss: Option<f64>,
    /// 係数を掛ける前の除去損失の平均。
    pub removal_loss: Option<f64>,
    /// BCEと係数を掛けた除去損失の和。
    pub total_loss: Option<f64>,
    /// 全検証集合のBCE平均。
    pub validation_loss: f64,
    /// 検証局面数。
    pub samples: usize,
    /// エポックの訓練秒数。
    pub training_seconds: Option<f64>,
    /// 検証秒数。
    pub validation_seconds: f64,
    /// 訓練局面数を訓練秒数で割った速度。
    pub training_positions_per_second: Option<f64>,
    /// 1更新の平均計算秒数。
    pub update_seconds: Option<f64>,
    /// 再開前を含む累積秒数。
    pub elapsed_seconds: f64,
    /// エポックの更新回数。
    pub steps: u64,
    /// 累積更新回数。
    pub total_steps: u64,
}

pub(super) fn configuration(
    args: &Args,
    workers: usize,
    dataset: &Dataset,
    ks: &[f64],
    k: f32,
) -> Result<Configuration> {
    let mut settings = serde_json::to_value(args)?;
    let object = settings
        .as_object_mut()
        .ok_or(Error::Invalid("arguments are not an object"))?;
    // 予算と並列度だけは再開で変更できる。ファイルは検査和と正規化したパスで照合する。
    for key in [
        "output_dir",
        "resume",
        "data",
        "init",
        "epochs",
        "patience",
        "threads",
    ] {
        object.remove(key);
    }
    object.insert("model".into(), json!("mirrored_pst"));
    object.insert(
        "optimizer".into(),
        json!({"lr":args.lr, "weight_decay":0.0, "betas":[0.9,0.999], "epsilon":1e-8}),
    );
    let files = dataset.paths().zip(dataset.headers()).map(|(path, header)| {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(".provenance.json");
        Ok(json!({"data":file_record(path)?, "provenance":file_record(Path::new(&sidecar))?, "header_sha256":hex(&digest(&header.encode()))}))
    }).collect::<Result<Vec<_>>>()?;
    let inputs = json!({"files":files, "init":file_record(&args.init)?});
    let preprocessing = json!({"teacher_classes":dataset.teacher_classes(), "teacher_ks":ks,
        "teacher_lambdas":dataset.teacher_lambdas(), "teacher_k_source":"training", "output_k":k,
        "training_positions":dataset.training_indices().len(), "validation_positions":dataset.validation_indices().len(),
        "training_indices_sha256":hash_u64(dataset.training_indices()), "validation_indices_sha256":hash_u64(dataset.validation_indices())});
    let config_hash = digest(&serde_json::to_vec(&settings)?);
    let input_hash = digest(&serde_json::to_vec(
        &json!({"inputs":inputs,"preprocessing":preprocessing}),
    )?);
    Ok(Configuration {
        config_hash,
        input_hash,
        document: json!({"device":"cpu", "arguments":args, "resolved_threads":workers, "settings":settings,
            "inputs":inputs, "preprocessing":preprocessing, "resumes":[], "config_sha256":hex(&config_hash), "input_sha256":hex(&input_hash)}),
    })
}

pub(crate) fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}

pub(crate) fn file_record(path: &Path) -> Result<Value> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(json!({"path": path.canonicalize()?, "sha256": hex(&hasher.finalize())}))
}

pub(crate) fn hash_u64(values: &[u64]) -> String {
    let mut hash = Sha256::new();
    for v in values {
        hash.update(v.to_le_bytes());
    }
    hex(&hash.finalize())
}

pub(crate) struct Configuration {
    pub document: Value,
    pub config_hash: [u8; 32],
    pub input_hash: [u8; 32],
}

pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(atomic_write(path, &bytes)?)
}

pub(crate) fn append_epoch(path: &Path, record: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec(record)?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

#[derive(serde::Deserialize)]
struct LogProgress {
    epoch: u64,
    validation_loss: f64,
}

// チェックポイントを永続化する前に中断したログの末尾は再実行する。
pub(crate) fn reconcile_log(path: &Path, epoch: u64) -> Result<f64> {
    let bytes = std::fs::read(path)?;
    let mut keep = 0;
    let mut expected = 0;
    let mut initial = None;
    for line in bytes.split_inclusive(|&b| b == b'\n') {
        if expected > epoch || line.last() != Some(&b'\n') {
            break;
        }
        let record: LogProgress = serde_json::from_slice(line)?;
        if record.epoch != expected || !record.validation_loss.is_finite() {
            return Err(Error::Invalid(
                "training log does not match checkpoint epochs",
            ));
        }
        if record.epoch == 0 {
            initial = Some(record.validation_loss);
        }
        keep += line.len();
        expected += 1;
    }
    if expected != epoch + 1 {
        return Err(Error::Invalid("training log is missing a completed epoch"));
    }
    if keep != bytes.len() {
        eprintln!("discarding training log entries after checkpoint epoch {epoch}");
        atomic_write(path, &bytes[..keep])?;
    }
    initial.ok_or(Error::Invalid("training log has no epoch zero"))
}

/// 学習を停止した理由。診断値だけでは停止しない。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// 指定されたエポック数を完了した。
    MaxEpochs,
    /// 指定された連続エポック数にわたって改善しなかった。
    EarlyStopping,
}

/// 終了理由、最良エポック、量子化検査をsummary.jsonへ保存する。
#[derive(Debug, Serialize)]
pub struct Summary {
    /// candidate、quantization_error、またはerror。
    pub status: &'static str,
    /// 検査失敗または学習未完了の理由。
    pub reason: Option<String>,
    /// 学習の停止理由。
    pub stopped_by: StopReason,
    /// 収束に関する注記。PSTではnull。
    pub convergence: Option<&'static str>,
    /// 検証を終えた最終エポック。
    pub last_epoch: u64,
    /// 1以上の最良エポック。未完了ならnull。
    pub best_epoch: Option<u64>,
    /// 最良エポックの検証損失。
    pub best_validation_loss: Option<f64>,
    /// 初期モデルの検証損失。
    pub initial_validation_loss: Option<f64>,
    /// 最良損失が初期損失を下回るか。採否には使わない。
    pub improved_over_epoch_zero: Option<bool>,
    /// 永続化した更新回数。
    pub total_steps: u64,
    /// 再開前を含む学習実行の秒数。
    pub elapsed_seconds: f64,
    /// 量子化誤差の検査結果。
    pub quantization: Option<QuantizationReport>,
}
