use super::{Args, Error, Result};
use crate::{
    data::{Dataset, Observations},
    model::{Model, Parameter},
    options::DeviceKind,
    reference::Validation,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Movement {
    mean_absolute_cp: f64,
    max_absolute_cp: f64,
}

/// 完了した1エポックの記録。学習前のエポック0では訓練の欄はnull。
#[derive(Debug, Serialize, Deserialize)]
pub struct EpochRecord {
    /// 学習を実行したデバイス。
    pub device: DeviceKind,
    /// 完了したエポック番号。0は学習前の検証。
    pub epoch: u64,
    /// 訓練集合の局面数で重み付けしたBCE。
    pub train_loss: Option<f64>,
    /// 指定された係数を含む補助損失の局面平均。
    pub removal_loss: Option<f64>,
    /// BCEと補助損失の和。
    pub total_loss: Option<f64>,
    /// 全検証集合のBCE。
    pub validation_loss: f64,
    /// 引き分けを除いた符号一致率。分母が0ならnull。
    pub sign_accuracy: Option<f64>,
    /// 検証局面数。
    pub samples: usize,
    /// 引き分けでない検証局面数。
    pub decisive_samples: u64,
    /// 非線形出力の母標準偏差、センチポーン。
    pub nonlinear_std_cp: f64,
    /// 活性化の飽和率。
    pub saturation: Saturation,
    /// 初期線形重みからの絶対移動量、センチポーン。
    pub linear_movement: LinearMovement,
    /// 当該エポックの学習秒数。順列作成と供給、転送を含む。
    pub training_seconds: Option<f64>,
    /// 当該エポックの検証秒数。
    pub validation_seconds: f64,
    /// 訓練集合の局面数を学習秒数で割った値。
    pub training_positions_per_second: Option<f64>,
    /// 1更新の平均秒数。CUDAではイベント、CPUではホストの経過時間を用いる。
    pub update_seconds: Option<f64>,
    /// 学習・検証の配列確保後に測ったGPU全体の最大使用量、バイト。
    pub gpu_memory_used_bytes: Option<usize>,
    /// GPUの総メモリ、バイト。
    pub gpu_memory_total_bytes: Option<usize>,
    /// 再開前を含む累積秒数。
    pub elapsed_seconds: f64,
    /// このエポックの更新回数。
    pub steps: u64,
    /// 全エポックの更新回数。
    pub total_steps: u64,
}

/// 累算器が0以下と1以上であった要素の割合。
#[derive(Debug, Serialize, Deserialize)]
pub struct Saturation {
    /// 0以下の割合。
    pub acc_le_zero: f64,
    /// 1以上の割合。
    pub acc_ge_one: f64,
}

/// 線形重みの2端点についての移動量。
#[derive(Debug, Serialize, Deserialize)]
pub struct LinearMovement {
    middlegame: Movement,
    endgame: Movement,
}

pub(super) fn linear_movement(model: &Model) -> LinearMovement {
    // 正準行は全て鏡映の対を持つので、埋め草を除いた平均は展開後と等しい。
    let rows = model.initial_linear().len() / 2 - 1;
    let endpoint = |i| {
        let mut sum = 0.0;
        let mut max: f64 = 0.0;
        for row in 0..rows {
            let delta = (f64::from(model.parameters()[Parameter::Linear][2 * row + i])
                - f64::from(model.initial_linear()[2 * row + i]))
            .abs();
            sum += delta;
            max = max.max(delta);
        }
        Movement {
            mean_absolute_cp: sum / rows as f64,
            max_absolute_cp: max,
        }
    };
    LinearMovement {
        middlegame: endpoint(0),
        endgame: endpoint(1),
    }
}

pub(super) fn validate_statistics(v: &Validation) -> Result<()> {
    if v.positions == 0
        || v.activations == 0
        || ![v.loss_sum, v.nonlinear_sum, v.nonlinear_square_sum]
            .iter()
            .all(|v| v.is_finite())
        || v.loss_sum < 0.0
        || v.nonlinear_square_sum < 0.0
        || v.correct > v.decisive
        || v.decisive > v.positions as u64
        || v.lower + v.upper > v.activations as u64
    {
        return Err(Error::Invalid("non-finite or invalid validation statistics").into());
    }
    Ok(())
}

pub(super) fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}

fn file_record(path: &Path) -> Result<Value> {
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

fn hash_u64(values: &[u64]) -> String {
    let mut hash = Sha256::new();
    for v in values {
        hash.update(v.to_le_bytes());
    }
    hex(&hash.finalize())
}

pub(super) struct Configuration {
    pub document: Value,
    pub config_hash: [u8; 32],
    pub input_hash: [u8; 32],
}

pub(super) fn configuration(
    args: &Args,
    workers: usize,
    dataset: &Dataset,
    observations: &Observations,
    teacher_ks: &[f64],
    model: &Model,
) -> Result<Configuration> {
    let mut settings = serde_json::to_value(args)?;
    let object = settings
        .as_object_mut()
        .ok_or(Error::Invalid("arguments are not an object"))?;
    // 保存先と再開操作は数値計算の設定ではない。入力パスは別途正規化して照合する。
    // エポック数、改善なしの許容数、時間の上限、ワーカー数は予算と並列度であり、
    // 学習の軌跡を変えないので、再開時に変えてよい（ハッシュに含めない）。
    for key in [
        "output_dir",
        "resume",
        "data",
        "init_pst",
        "base",
        "epochs",
        "patience",
        "time_limit_hours",
        "threads",
    ] {
        object.remove(key);
    }
    let resolved_threads = workers;
    object.insert(
        "optimizer".into(),
        serde_json::to_value(args.training.optimizer())?,
    );
    object.insert("initial_standard_deviation".into(), json!(0.05));
    let files = dataset
        .paths()
        .zip(dataset.headers())
        .map(|(path, header)| {
            let mut sidecar = path.as_os_str().to_owned();
            sidecar.push(".provenance.json");
            Ok(
                json!({"data": file_record(path)?, "provenance": file_record(Path::new(&sidecar))?,
            "header_sha256": hex(&digest(&header.encode()))}),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let inputs = json!({"files": files, "init_pst": file_record(&args.init_pst)?, "base": file_record(&args.base)?});
    let count = |mask: &[bool]| {
        // 埋め草は観測対象に含めない。
        let rows = if mask.is_empty() {
            mask
        } else {
            &mask[..mask.len() - 1]
        };
        let observed = rows.iter().filter(|&&v| v).count();
        json!({"observed": observed, "unobserved": rows.len()-observed})
    };
    let preprocessing = json!({
        "teacher_classes": dataset.teacher_classes(), "teacher_ks": teacher_ks,
        "teacher_lambdas": dataset.teacher_lambdas(),
        "teacher_k_source": "training", "output_k": model.shape().k(),
        "training_positions": dataset.training_indices().len(),
        "validation_positions": dataset.validation_indices().len(),
        "training_indices_sha256": hash_u64(dataset.training_indices()),
        "validation_indices_sha256": hash_u64(dataset.validation_indices()),
        "observations": {"linear_rows": count(model.masks().canonical()),
            "feature_rows": count(model.masks().first_layer()), "virtual_rows": count(model.masks().virtual_rows())},
        "canonical_counts_sha256": hash_u64(observations.canonical_counts()),
        "input_counts_sha256": hash_u64(observations.input_counts()),
    });
    let config_hash = digest(&serde_json::to_vec(&settings)?);
    let input_hash = digest(&serde_json::to_vec(
        &json!({"inputs": inputs, "preprocessing": preprocessing}),
    )?);
    Ok(Configuration {
        document: json!({"device": args.device, "arguments": args, "resolved_threads": resolved_threads, "settings": settings,
        "inputs": inputs, "preprocessing": preprocessing,
        "resumes": [],
        "config_sha256": hex(&config_hash), "input_sha256": hex(&input_hash)}),
        config_hash,
        input_hash,
    })
}

pub(super) fn append_resume(
    path: &Path,
    config: &mut Value,
    args: &Args,
    workers: usize,
    epoch: u64,
) -> Result<()> {
    let resumed_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let resumes = config
        .as_object_mut()
        .ok_or(Error::Invalid("configuration must be an object"))?
        .entry("resumes")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or(Error::Invalid("configuration resumes must be an array"))?;
    resumes.push(json!({
        "resumed_at_unix_seconds": resumed_at,
        "epochs": args.epochs,
        "patience": args.patience,
        "time_limit_hours": args.time_limit_hours,
        "workers": workers,
        "epoch": epoch,
    }));
    write_json(path, config)
}

pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let mut file = File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(temporary, path)?;
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

pub(super) fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

pub(super) fn append_epoch(path: &Path, record: &EpochRecord) -> Result<()> {
    let mut bytes = serde_json::to_vec(record)?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

// チェックポイントを永続化する前に中断したログの末尾は再実行する。
pub(super) fn reconcile_log(path: &Path, epoch: u64) -> Result<f64> {
    let bytes = std::fs::read(path)?;
    let mut keep = 0;
    let mut expected = 0;
    let mut initial = None;
    for line in bytes.split_inclusive(|&b| b == b'\n') {
        if expected > epoch || line.last() != Some(&b'\n') {
            break;
        }
        let record: EpochRecord = serde_json::from_slice(line)?;
        if record.epoch != expected || !record.validation_loss.is_finite() {
            return Err(Error::Invalid("training log does not match checkpoint epochs").into());
        }
        if record.epoch == 0 {
            initial = Some(record.validation_loss);
        }
        keep += line.len();
        expected += 1;
    }
    if expected != epoch + 1 {
        return Err(Error::Invalid("training log is missing a completed epoch").into());
    }
    if keep != bytes.len() {
        eprintln!("discarding training log entries after checkpoint epoch {epoch}");
        atomic_write(path, &bytes[..keep])?;
    }
    initial.ok_or_else(|| Error::Invalid("training log has no epoch zero").into())
}
