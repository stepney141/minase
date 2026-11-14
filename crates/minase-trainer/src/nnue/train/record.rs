use super::{Args, Error, Result, initial_standard_deviation};
use crate::nn::{Sha256Hasher, atomic_write, hex};
use crate::nnue::{
    data::Observations,
    model::{Model, Parameter},
    options::DeviceKind,
    reference::Validation,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

/// 線形重みの1端点について、現在値と初期値の差の絶対値をセンチポーンで集計する。
/// 埋め草を除く正準行6,840行の平均が`mean_absolute_cp`、最大が`max_absolute_cp`である。
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Movement {
    mean_absolute_cp: f64,
    max_absolute_cp: f64,
}

/// 完了した1 superbatchの記録。学習前の更新0回では訓練の欄はnull。
/// `training.jsonl`へ1 superbatchにつき1行を保存し、最初と最後の更新で使った倍率を
/// `lr_multiplier_first`と`lr_multiplier_last`に記録する。更新0回では両欄ともnull。
#[derive(Debug, Serialize, Deserialize)]
pub struct SuperbatchRecord {
    /// 学習を実行したデバイス。
    pub device: DeviceKind,
    /// 完了したsuperbatch数。0は学習前の検証。
    pub superbatch: u64,
    /// 更新回数を1周の更新回数で割った周回数。
    pub epochs: f64,
    /// 訓練集合の局面数で重み付けした勝率の差の2乗。
    pub train_loss: Option<f64>,
    /// 全検証集合の勝率の差の2乗。
    pub validation_loss: Option<f64>,
    /// 引き分けを除いた符号一致率。分母が0ならnull。
    pub sign_accuracy: Option<f64>,
    /// 検証局面数。
    pub samples: Option<usize>,
    /// 引き分けでない検証局面数。
    pub decisive_samples: Option<u64>,
    /// 非線形出力の母標準偏差、センチポーン。
    pub nonlinear_std_cp: Option<f64>,
    /// 活性化の飽和率。
    pub saturation: Option<Saturation>,
    /// 幅15の層のz≤0とz≥1の割合。clipped ReLUの枝の飽和を記録する。
    /// 2乗の枝はzが負でも勾配が流れ、|z|≥√(128/127)で飽和する。
    /// 境界点そのものの逆伝播は設計の規約に従い、内側の微分を使う。
    pub hidden1_saturation: Option<Saturation>,
    /// 幅32の層のa≤0とa≥1の割合。
    pub hidden2_saturation: Option<Saturation>,
    /// 全検証局面、全視点、全要素積のうち、両因子の累算値が正である割合。
    pub pairwise_both_positive: Option<f64>,
    /// superbatch境界のホストの第1層の展開後の重みのうち、絶対値が上限に達した割合。
    /// HalfKPはW+Vで判定し、未観測行と埋め草を分母から除く。
    pub first_layer_at_limit: f64,
    /// 初期線形重みからの絶対移動量、センチポーン。
    pub linear_movement: LinearMovement,
    /// 当該superbatchの学習秒数。順列作成と供給、転送を含む。
    pub training_seconds: Option<f64>,
    /// 当該superbatchの終了時の検証秒数。
    pub validation_seconds: Option<f64>,
    /// 更新に使った局面数を学習秒数で割った値。
    pub training_positions_per_second: Option<f64>,
    /// 1更新の平均秒数。CUDAではイベント、CPUではホストの経過時間を用いる。
    pub update_seconds: Option<f64>,
    /// CUDAが報告するデバイス全体の使用量（総量−空き容量）の、このプロセス内での最大値。単位はバイト。
    /// 他のプロセスの確保も含む。
    /// バックエンド確保直後、各検証の評価器確保後、各superbatchの訓練開始前と入力面確保後に測る。
    /// `--device cpu`では`null`である。
    pub gpu_memory_used_bytes: Option<usize>,
    /// CUDAが報告するデバイス全体の総メモリ、バイト。
    /// 測定時点は[`Self::gpu_memory_used_bytes`]と同じで、直近の総量を保存する。
    /// `--device cpu`では`null`である。
    pub gpu_memory_total_bytes: Option<usize>,
    /// 再開前を含む累積秒数。
    pub elapsed_seconds: f64,
    /// このsuperbatchの更新回数。
    pub steps: u64,
    /// 全superbatchの更新回数。
    pub total_steps: u64,
    /// このsuperbatchの最初の更新で学習率に掛けた倍率。更新0回ではnull。
    pub lr_multiplier_first: Option<f64>,
    /// このsuperbatchの最後の更新で学習率に掛けた倍率。更新0回ではnull。
    pub lr_multiplier_last: Option<f64>,
}

impl SuperbatchRecord {
    /// 検証した行だけ、検証集合全体の統計と所要時間を記録する。
    pub(super) fn set_validation(&mut self, validation: &Validation, seconds: f64) {
        let saturation = |values: [f64; 2]| Saturation {
            acc_le_zero: values[0],
            acc_ge_one: values[1],
        };
        self.validation_loss = Some(validation.loss());
        self.sign_accuracy = validation.sign_accuracy();
        self.samples = Some(validation.positions);
        self.decisive_samples = Some(validation.decisive);
        self.nonlinear_std_cp = Some(validation.nonlinear_std_cp());
        self.saturation = Some(saturation(validation.saturation()));
        self.hidden1_saturation = Some(saturation(validation.hidden1_saturation()));
        self.hidden2_saturation = Some(saturation(validation.hidden2_saturation()));
        self.pairwise_both_positive = Some(validation.pairwise_both_positive());
        self.validation_seconds = Some(seconds);
    }
}

/// 累算器が0以下と1以上であった要素の割合。
#[derive(Debug, Serialize, Deserialize)]
pub struct Saturation {
    /// 0以下の割合。
    pub acc_le_zero: f64,
    /// 1以上の割合。
    pub acc_ge_one: f64,
}

/// [`SuperbatchRecord::linear_movement`]へ保存する線形重みの移動量。
/// `middlegame`と`endgame`は2端点の集計で、各欄は`Movement`の平均と最大の定義に従う。
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
        || v.positions.checked_mul(15) != Some(v.hidden1_activations)
        || v.positions.checked_mul(32) != Some(v.hidden2_activations)
        || v.pairwise_products.checked_mul(2) != Some(v.activations)
        || ![v.loss_sum, v.nonlinear_sum, v.nonlinear_square_sum]
            .iter()
            .all(|v| v.is_finite())
        || v.loss_sum < 0.0
        || v.nonlinear_square_sum < 0.0
        || v.correct > v.decisive
        || v.decisive > v.positions as u64
        || v.hidden1_lower + v.hidden1_upper > v.hidden1_activations as u64
        || v.hidden2_lower + v.hidden2_upper > v.hidden2_activations as u64
        || v.pairwise_positive > v.pairwise_products as u64
        || v.lower + v.upper > v.activations as u64
    {
        return Err(Error::Invalid("non-finite or invalid validation statistics").into());
    }
    Ok(())
}

fn file_record(path: &Path) -> Result<Value> {
    Ok(serde_json::to_value(crate::identity::file_identity(path)?)?)
}

fn hash_u64(values: &[u64]) -> String {
    let mut hash = Sha256Hasher::default();
    for v in values {
        hash.update(&v.to_le_bytes());
    }
    hex(&hash.finalize())
}

pub(super) struct Configuration {
    pub document: Value,
}

/// 引数、入力の識別情報、全訓練集合の前処理と観測数を`config.json`へ記録する。
/// `settings`は更新の軌跡を決める条件であり、superbatch数、検証間隔、時間と実行資源を除く。
/// 教師の尺度は分類順に値、尺度比、推定境界への到達状況と出所を保存する。
/// λが0の分類はnull、出所は`"base"`、`"calibrate"`または較正ファイルのパスと内容である。
/// 入力パスは正規化し、入力と前処理は再開時に欄ごとに照合する。
/// 間引きは前処理に適用せず、指定時だけ抽出後の件数と検査和を追加する。
/// `resumes`は新規実行では空の配列とし、再開条件を[`append_resume`]で追記する。
pub(super) fn configuration<'a>(
    args: &Args,
    workers: usize,
    dataset: impl Into<super::store::DatasetInfo<'a>>,
    observations: &Observations,
    calibration: &crate::data::TeacherCalibration,
    model: &Model,
) -> Result<Configuration> {
    let dataset = dataset.into();
    let mut settings = serde_json::to_value(args)?;
    let object = settings
        .as_object_mut()
        .ok_or(Error::Invalid("arguments are not an object"))?;
    // 予算と実行資源は更新の軌跡を変えないので、再開時の照合から除く。
    for key in [
        "output_dir",
        "resume",
        "data",
        "teacher_cp_per_logit",
        "base",
        "superbatches",
        "time_limit_hours",
        "threads",
        "validation_superbatches",
        "bucket_positions",
        "scratch_dir",
    ] {
        object.remove(key);
    }
    let resolved_threads = workers;
    object.insert(
        "optimizer".into(),
        serde_json::to_value(args.training.optimizer())?,
    );
    object.insert(
        "initial_standard_deviation".into(),
        json!(initial_standard_deviation()),
    );
    let files = dataset
        .paths
        .into_iter()
        .map(file_record)
        .collect::<Result<Vec<_>>>()?;
    let inputs = json!({"files": files, "base": file_record(&args.base)?});
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
    let mut preprocessing = json!({
        "teacher_classes": dataset.classes, "teacher_cp_per_logit": calibration.teacher_cp_per_logit,
        "teacher_lambdas": dataset.lambdas,
        "teacher_cp_per_logit_source": calibration.source,
        "teacher_score_multiplier": calibration.teacher_score_multiplier,
        "teacher_cp_per_logit_at_search_bound": calibration.at_search_bound, "cp_per_logit": model.shape().cp_per_logit(),
        "training_positions": dataset.training,
        "validation_positions": dataset.validation,
        "training_indices_sha256": dataset.training_hash,
        "validation_indices_sha256": dataset.validation_hash,
        "observations": {"linear_rows": count(model.masks().canonical()),
            "feature_rows": count(model.masks().first_layer()), "virtual_rows": count(model.masks().virtual_rows())},
        "canonical_counts_sha256": hash_u64(observations.canonical_counts()),
        "input_counts_sha256": hash_u64(observations.input_counts()),
    });
    if let Some(subsample) = dataset.subsample {
        preprocessing["train_subsample"] = serde_json::to_value(subsample)?;
    }
    Ok(Configuration {
        document: json!({"device": args.device, "arguments": args, "resolved_threads": resolved_threads, "settings": settings,
        "inputs": inputs, "preprocessing": preprocessing,
        "resumes": []}),
    })
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

/// 確定したsuperbatchまで連番を検査し、以後の行と改行のない末尾を捨てる。
/// ログの追記後に`latest.npz`を保存し、その保存を確定の時点とする。
/// 更新0回の検証から最後に確定した検証までのうち、最後の検証のsuperbatch数と損失を返す。
pub(super) fn reconcile_log(path: &Path, superbatch: u64) -> Result<(u64, f64)> {
    let bytes = std::fs::read(path)?;
    let mut keep = 0;
    let mut last_loss = None;
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
            last_loss = Some((record.superbatch, loss));
        }
        if expected == 0 && last_loss.is_none() {
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
            return last_loss
                .ok_or_else(|| Error::Invalid("training log has no validation").into());
        }
    }
    Err(Error::Invalid("training log is missing a completed superbatch").into())
}

/// 予算を保存済みの進捗まで短縮した場合に、最後の行の検証を補完する。
/// 確定した重みと訓練記録は保ち、検証の統計と累積秒数だけを更新する。
pub(super) fn complete_final_validation(
    path: &Path,
    validation: &Validation,
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
    record.set_validation(validation, seconds);
    record.elapsed_seconds = elapsed;
    let mut bytes = prefix.as_bytes().to_vec();
    bytes.push(b'\n');
    serde_json::to_writer(&mut bytes, &record)?;
    bytes.push(b'\n');
    Ok(atomic_write(path, &bytes)?)
}
