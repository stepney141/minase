use super::{
    Args, Distribution, GradientNorm, Result, ZeroCorrectionReport,
    export::{CorrectionDistribution, QuantizationReport},
};
use crate::{
    data::Dataset,
    nn::{Sha256Hasher, atomic_write, hex},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{fs::OpenOptions, io::Write, path::Path};

/// FMの1エポックの記録。
/// 損失は局面数で重み付けした平均で、端数バッチも含める。
/// `validation_loss`は全検証集合の二値交差エントロピーで、正則化項を含めない。
/// `regularization_loss = lambda_res * mean(Φ²)`は係数を含み、`total_loss = train_loss + regularization_loss`である。
/// `phi`はロジット単位の[`Distribution`]である。
/// エポック0では訓練の損失、訓練秒数、速度、更新秒数が`null`、`steps`が0である。
#[derive(Debug, Serialize)]
pub struct EpochRecord {
    /// CPUで実行したことを記録する。
    pub device: &'static str,
    /// 完了したエポック。0は学習前の検証。
    pub epoch: u64,
    /// 訓練集合の二値交差エントロピーの平均。
    pub train_loss: Option<f64>,
    /// 係数を掛けた正則化項の平均。
    pub regularization_loss: Option<f64>,
    /// 全検証局面のFM補正の分布。
    pub phi: Distribution,
    /// BCEと正則化項の和。
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

/// FM学習の`config.json`を、[`pst::record::configuration`](../../pst/record/fn.configuration.html)と同じ構成で作る。
/// 入力の識別は[`crate::identity`]に従い、`settings.lambda`に指定した混合比を保存する。
/// `settings.model = "fm_rank32"`とし、`settings.optimizer`は
/// `{lr, weight_decay, betas: [0.9, 0.999], epsilon: 1e-8}`で、`weight_decay`には`--weight-decay`を使う。
/// `preprocessing`に、訓練集合とその全鏡映で数えた全13,680特徴の観測回数`feature_observations`を特徴番号順で加える。
/// `unobserved_features`は観測回数0の特徴数である。
/// 尺度の出所は`"calibrate"`または正規化した較正ファイルのパスと全内容で、`preprocessing`で照合する。
/// `teacher_score_multiplier`は勝率尺度と教師の勝率尺度の比、`teacher_cp_per_logit_at_search_bound`は推定の境界到達状況である。
/// 両配列とも分類順で、λが0の分類は`null`とする。引数の`teacher_cp_per_logit`は`settings`から除く。
pub(super) fn configuration(
    args: &Args,
    workers: usize,
    dataset: &Dataset,
    calibration: &crate::data::TeacherCalibration,
    cp_per_logit: f32,
    counts: &[u64],
) -> Result<Configuration> {
    let mut settings = serde_json::to_value(args)?;
    let object = settings
        .as_object_mut()
        .ok_or("arguments are not an object")?;
    // 予算と並列度だけは再開で変更できる。ファイルは検査和と正規化したパスで照合する。
    for key in [
        "output_dir",
        "resume",
        "data",
        "teacher_cp_per_logit",
        "init",
        "epochs",
        "patience",
        "threads",
    ] {
        object.remove(key);
    }
    object.insert("model".into(), json!("fm_rank32"));
    object.insert(
        "optimizer".into(),
        json!({"lr":args.lr, "weight_decay":args.weight_decay, "betas":[0.9,0.999], "epsilon":1e-8}),
    );
    let files = dataset
        .paths()
        .map(file_record)
        .collect::<Result<Vec<_>>>()?;
    let inputs = json!({"files":files, "init":file_record(&args.init)?});
    let preprocessing = json!({"feature_observations":counts, "unobserved_features":counts.iter().filter(|&&n| n == 0).count(), "teacher_classes":dataset.teacher_classes(), "teacher_cp_per_logit":calibration.teacher_cp_per_logit,
        "teacher_lambdas":dataset.teacher_lambdas(), "teacher_cp_per_logit_source":calibration.source,
        "teacher_score_multiplier":calibration.teacher_score_multiplier,
        "teacher_cp_per_logit_at_search_bound":calibration.at_search_bound, "cp_per_logit":cp_per_logit,
        "training_positions":dataset.training_indices().len(), "validation_positions":dataset.validation_indices().len(),
        "training_indices_sha256":hash_u64(dataset.training_indices()), "validation_indices_sha256":hash_u64(dataset.validation_indices())});
    Ok(Configuration {
        document: json!({"device":"cpu", "arguments":args, "resolved_threads":workers, "settings":settings,
            "inputs":inputs, "preprocessing":preprocessing, "resumes":[]}),
    })
}

pub(crate) fn file_record(path: &Path) -> Result<Value> {
    Ok(serde_json::to_value(crate::identity::file_identity(path)?)?)
}

pub(crate) fn hash_u64(values: &[u64]) -> String {
    let mut hash = Sha256Hasher::default();
    for v in values {
        hash.update(&v.to_le_bytes());
    }
    hex(&hash.finalize())
}

pub(crate) struct Configuration {
    pub document: Value,
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
            return Err("training log does not match checkpoint epochs".into());
        }
        if record.epoch == 0 {
            initial = Some(record.validation_loss);
        }
        keep += line.len();
        expected += 1;
    }
    if expected != epoch + 1 {
        return Err("training log is missing a completed epoch".into());
    }
    if keep != bytes.len() {
        eprintln!("discarding training log entries after checkpoint epoch {epoch}");
        atomic_write(path, &bytes[..keep])?;
    }
    initial.ok_or("training log has no epoch zero".into())
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

/// 終了理由、最良エポック、量子化検査を`summary.json`へ保存する。
/// `status = "candidate"`なら終了コード0、`"quantization_error"`または`"error"`なら終了コード1である。
/// 量子化検査に合格し、`quarter_zero_check.identically_zero`が真なら`"excluded_zero_correction"`で終了コード0となり、
/// `reason = "integer quarter FM correction is identically zero"`を記録する。
/// 量子化検査の合否にかかわらず、`fm.bin`、`fm-float.npz`、`quantization`、
/// `quarter_zero_check`、`correction_cp`、`initial_position_score`を残す。
/// エポック途中の失敗では`{status: "error", reason, last_epoch, gradients, total_steps, elapsed_seconds}`だけを保存する。
/// `initial_position_score`は、書き出した`fm.bin`のPST評価に縮小前の整数FM補正を加え、±28,999へ切り詰めた手番側視点のセンチポーンである。
/// `phi`は最良エポックのモデルで全検証集合を評価し直した分布である。
#[derive(Debug, Serialize)]
pub struct Summary {
    /// candidate、excluded_zero_correction、quantization_error、またはerror。
    pub status: &'static str,
    /// 検査失敗または学習未完了の理由。
    pub reason: Option<String>,
    /// 学習の停止理由。
    pub stopped_by: StopReason,
    /// 収束に関する注記。FMではnull。
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
    /// 最初の5更新の勾配ノルム。
    pub gradients: Vec<GradientNorm>,
    /// 1/4補正が全局面で0になる十分条件。
    pub quarter_zero_check: Option<ZeroCorrectionReport>,
    /// 整数補正の分布、センチポーン。
    pub correction_cp: Option<CorrectionDistribution>,
    /// 最良エポックの全検証集合のPhi分布。
    pub phi: Option<Distribution>,
    /// 量子化誤差の検査結果。
    pub quantization: Option<QuantizationReport>,
    /// 書き出した重みによる初期局面の整数評価。手番側視点、センチポーン。
    pub initial_position_score: Option<i32>,
}
