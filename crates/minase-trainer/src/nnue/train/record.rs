use super::{Args, Error, Result};
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

/// エポックの経過秒数から途中検証の秒数を除き、局面数から学習速度を求める。
pub(super) fn training_timing(
    positions: usize,
    elapsed_seconds: f64,
    interval_validation_seconds: f64,
) -> TrainingTiming {
    let training_seconds = elapsed_seconds - interval_validation_seconds;
    TrainingTiming {
        training_seconds,
        interval_validation_seconds,
        training_positions_per_second: positions as f64 / training_seconds,
    }
}

pub(super) struct TrainingTiming {
    pub training_seconds: f64,
    pub interval_validation_seconds: f64,
    pub training_positions_per_second: f64,
}

/// 線形重みの1端点について、現在値と初期値の差の絶対値をセンチポーンで集計する。
/// 埋め草を除く正準行6,840行の平均が`mean_absolute_cp`、最大が`max_absolute_cp`である。
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Movement {
    mean_absolute_cp: f64,
    max_absolute_cp: f64,
}

/// 完了した1エポックの記録。学習前のエポック0では訓練の欄はnull。
/// `training.jsonl`へ1エポック1行で保存し、最初と最後の更新で使った倍率を
/// `lr_multiplier_first`と`lr_multiplier_last`に記録する。エポック0では両欄ともnull。
#[derive(Debug, Serialize, Deserialize)]
pub struct EpochRecord {
    /// 学習を実行したデバイス。
    pub device: DeviceKind,
    /// 完了したエポック番号。0は学習前の検証。
    pub epoch: u64,
    /// 訓練集合の局面数で重み付けしたBCE。
    pub train_loss: Option<f64>,
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
    /// 隠れ層hの0以下・1以上の割合。隠れ層がない場合はnull。
    /// Saturationのacc_le_zeroとacc_ge_oneに、ここではhの割合を記録する。
    pub hidden_saturation: Option<Saturation>,
    /// 初期線形重みからの絶対移動量、センチポーン。
    pub linear_movement: LinearMovement,
    /// 当該エポックの学習秒数。順列作成と供給、転送を含み、途中検証の秒数は除く。
    pub training_seconds: Option<f64>,
    /// 当該エポックの終了時の検証秒数。
    pub validation_seconds: f64,
    /// 当該エポックの途中検証に要した秒数の合計。パラメータの回収とログ保存を含む。
    /// 途中検証がなければ0であり、エポック0でも0である。
    pub interval_validation_seconds: f64,
    /// 訓練集合の局面数を、途中検証を除いた学習秒数で割った値。
    pub training_positions_per_second: Option<f64>,
    /// 1更新の平均秒数。CUDAではイベント、CPUではホストの経過時間を用いる。
    pub update_seconds: Option<f64>,
    /// CUDAが報告するデバイス全体の使用量（総量−空き容量）の、このプロセス内での最大値。単位はバイト。
    /// 他のプロセスの確保も含む。
    /// バックエンド確保直後、各検証の評価器確保後、各エポックの訓練開始前と入力面確保後に測る。
    /// `--device cpu`では`null`である。
    pub gpu_memory_used_bytes: Option<usize>,
    /// CUDAが報告するデバイス全体の総メモリ、バイト。
    /// 測定時点は[`Self::gpu_memory_used_bytes`]と同じで、直近の総量を保存する。
    /// `--device cpu`では`null`である。
    pub gpu_memory_total_bytes: Option<usize>,
    /// 再開前を含む累積秒数。
    pub elapsed_seconds: f64,
    /// このエポックの更新回数。
    pub steps: u64,
    /// 全エポックの更新回数。
    pub total_steps: u64,
    /// このエポックの最初の更新で学習率に掛けた倍率。エポック0ではnull。
    pub lr_multiplier_first: Option<f64>,
    /// このエポックの最後の更新で学習率に掛けた倍率。エポック0ではnull。
    pub lr_multiplier_last: Option<f64>,
}

/// エポック途中の全検証集合の評価を`validation-interval.jsonl`へ1行ずつ保存する。
/// エポック境界では記録せず、最良モデルの選択やチェックポイントの更新には使わない。
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct IntervalRecord {
    /// 完了した更新回数の累計。[`EpochRecord::total_steps`]と同じ数え方である。
    pub step: u64,
    /// 進行中のエポック番号。1以上である。
    pub epoch: u64,
    /// エポック内の進捗率。1エポックの更新回数をUとして`(step − (epoch − 1) × U) / U`。
    pub epoch_fraction: f64,
    /// 全検証集合のBCE。[`EpochRecord::validation_loss`]と同じ定義である。
    pub validation_loss: f64,
    /// 引き分けを除いた符号一致率。分母が0ならnull。
    pub sign_accuracy: Option<f64>,
    /// 検証局面数。
    pub samples: usize,
    /// 引き分けでない検証局面数。
    pub decisive_samples: u64,
}

/// 累算器が0以下と1以上であった要素の割合。
#[derive(Debug, Serialize, Deserialize)]
pub struct Saturation {
    /// 0以下の割合。
    pub acc_le_zero: f64,
    /// 1以上の割合。
    pub acc_ge_one: f64,
}

/// [`EpochRecord::linear_movement`]へ保存する線形重みの移動量。
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
        || ![v.loss_sum, v.nonlinear_sum, v.nonlinear_square_sum]
            .iter()
            .all(|v| v.is_finite())
        || v.loss_sum < 0.0
        || v.nonlinear_square_sum < 0.0
        || v.correct > v.decisive
        || v.decisive > v.positions as u64
        || v.hidden_lower + v.hidden_upper > v.hidden_activations as u64
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

/// 整形JSONとして保存する`config.json`の設定、入力、前処理結果を作る。
///
/// 最上位の`device`は`"cuda"`または`"cpu"`、`arguments`は新規実行時の全引数[`Args`]、
/// `resolved_threads`は実効ワーカー数である。
/// `settings`は引数から`output_dir`、`resume`、`data`、`init_pst`、`base`、`epochs`、
/// `patience`、`time_limit_hours`、`threads`、`validation_interval`、`bucket_positions`、`scratch_dir`を除き、`optimizer`と`initial_standard_deviation = 0.05`を加える。
/// `optimizer`は線形、第1層、出力部の3群の`lr`と`weight_decay`を持つ。
/// `lr_schedule`は`arguments`と`settings`の両方に、`kind`と形ごとの引数を持つオブジェクトとして保存する。
/// 学習の軌跡を変える設定なので、再開時の照合に含める。
///
/// 教師と最適化の係数は[`crate::nnue::options::TrainingOptions`]に従う。
/// `settings.lambda`は利用者が指定した混合比であり、実戦対局の実効λは0である。
/// `inputs.files`は入力順のMNSDの`path`、`byte_length`、`header`であり、各欄は[`crate::identity`]に従う。
/// `inputs.init_pst`はNPZの`{path, byte_length}`、`inputs.base`はMNPTの`{path, checksum}`であり、パスは正規化済みである。
/// `preprocessing`は`teacher_classes`、`teacher_cp_per_logit`、`teacher_lambdas`、`teacher_cp_per_logit_source`、
/// `cp_per_logit`、`training_positions`、`validation_positions`を持つ。
/// 分類の定義は[`crate::data::TeacherClass`]に従い、λが0の分類の教師の勝率尺度（1ロジットあたりのcp）は`null`である。
/// `training_indices_sha256`と`validation_indices_sha256`は通算番号をリトルエンディアンu64列にしたSHA-256である。
/// `observations`は`linear_rows`、`feature_rows`、`virtual_rows`の各`{observed, unobserved}`を持ち、埋め草行を除く。
/// `canonical_counts_sha256`と`input_counts_sha256`は、それぞれの行の観測回数をリトルエンディアンu64列にしたSHA-256である。
///
/// `resumes`は再開履歴の配列で、新規実行では空とし、[`append_resume`]で追記する。
/// 新規実行時だけ、呼び手が`device_name`、`gpu_memory_used_bytes`、`gpu_memory_total_bytes`、`preprocessing_seconds`を加える。
/// `device_name`はCUDAのデバイス名または`"CPU"`、メモリはバックエンド確保後の値でCPUでは`null`である。
/// 再開時は`settings`、`inputs`、`preprocessing`、`device_name`の一致を要求する。
/// 変更できる予算と並列度は`epochs`、`patience`、`time_limit_hours`、`threads`、`validation_interval`である。
/// `validation_interval`は学習の軌跡を変えない診断用の予算なので、再開時に変更できる。
/// `buckets`は順列を決めるので照合する。`bucket_positions`と`scratch_dir`は再開時に変更できる。
/// 尺度の出所は`"calibrate"`または正規化した較正ファイルのパスと全内容で、`preprocessing`で照合する。
/// `teacher_score_multiplier`は勝率尺度と教師の勝率尺度の比、`teacher_cp_per_logit_at_search_bound`は推定の境界到達状況である。
/// 両配列とも分類順で、λが0の分類は`null`とする。引数の`teacher_cp_per_logit`は`settings`から除く。
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
    // 保存先と再開操作は数値計算の設定ではない。入力パスは別途正規化して照合する。
    // エポック数、改善なしの許容数、時間の上限、ワーカー数、途中検証の間隔は予算と並列度であり、
    // 学習の軌跡を変えないので、再開時に変えてよい（比較に含めない）。
    for key in [
        "output_dir",
        "resume",
        "data",
        "teacher_cp_per_logit",
        "init_pst",
        "base",
        "epochs",
        "patience",
        "time_limit_hours",
        "threads",
        "validation_interval",
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
    object.insert("initial_standard_deviation".into(), json!(0.05));
    let files = dataset
        .paths
        .into_iter()
        .map(file_record)
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
    Ok(Configuration {
        document: json!({"device": args.device, "arguments": args, "resolved_threads": resolved_threads, "settings": settings,
        "inputs": inputs, "preprocessing": preprocessing,
        "resumes": []}),
    })
}

/// 再開のたびに、呼び出し元から受け取った時刻を`config.json`の`resumes`へ追記して書き直す。
/// `resumes`は配列として存在しなければならず、欠落や型の不一致は拒否する。
/// 要素は整数のUnix秒`resumed_at_unix_seconds`、`epochs`、`patience`、`time_limit_hours`、
/// 実効ワーカー数`workers`、途中検証の間隔`validation_interval`、再開時点の完了エポック番号`epoch`、
/// bucketの上限`bucket_positions`と作業ディレクトリ`scratch_dir`を持つ。
pub(super) fn append_resume(
    path: &Path,
    config: &mut Value,
    args: &Args,
    workers: usize,
    epoch: u64,
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
        "epochs": args.epochs,
        "patience": args.patience,
        "time_limit_hours": args.time_limit_hours,
        "workers": workers,
        "validation_interval": args.validation_interval,
        "epoch": epoch,
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

pub(super) fn append_epoch(path: &Path, record: &EpochRecord) -> Result<()> {
    append_record(path, record)
}

/// 完了した途中検証を1行追記し、`sync_all`で永続化する。
/// 呼び手は間隔が正で、エポック境界以外の検証を期限内に完了したときだけ呼ぶ。
pub(super) fn append_interval(path: &Path, record: &IntervalRecord) -> Result<()> {
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

/// `latest.npz`で確定したエポックまでのログを検査し、未確定の末尾を切り捨てる。
/// 各エポックは`training.jsonl`への追記、`latest.npz`、改善時の`best.npz`の順に保存し、`latest.npz`の確定を保存完了とする。
/// ログを先頭から読み、エポック0から`epoch`までが連番で揃い、検証損失が有限であることを要求する。
/// その後の行と改行で終わらない末尾は切り捨て、未確定のエポックは再実行する。
/// エポック0を含む必要な行が欠けていれば拒否し、成功時はエポック0の検証損失を返す。
/// 最良チェックポイントの復旧は[`super::recover_best_checkpoint`]に従う。
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

/// 途中検証のログを、`latest.npz`で確定した更新回数`step`以下の行だけに切り詰める。
/// 改行で完結した全行の更新回数が厳密に増加することを要求し、不一致なら書き換えずに拒否する。
/// チェックポイントより後の行と、改行で終わらない末尾を捨てる。ファイルがなければ何もしない。
/// 途中検証の間隔を変更した場合や無効にした場合も、再開前に同じ処理を行う。
pub(super) fn reconcile_interval_log(path: &Path, step: u64) -> Result<()> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let mut keep = 0;
    let mut previous = None;
    for line in bytes.split_inclusive(|&b| b == b'\n') {
        if line.last() != Some(&b'\n') {
            break;
        }
        let record: IntervalRecord = serde_json::from_slice(line)?;
        if previous.is_some_and(|previous| record.step <= previous) {
            return Err(Error::Invalid("interval validation steps must strictly increase").into());
        }
        previous = Some(record.step);
        if record.step <= step {
            keep += line.len();
        }
    }
    if keep != bytes.len() {
        eprintln!("discarding interval validation records after checkpoint step {step}");
        atomic_write(path, &bytes[..keep])?;
    }
    Ok(())
}
