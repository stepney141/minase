use super::{Args, Error, Result, export::QuantizationReport};
use crate::{
    data::Dataset,
    nn::{Sha256Hasher, atomic_write, hex},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{fs::OpenOptions, io::Write, path::Path};

/// PSTの1エポックの記録。
/// 損失はすべて局面数で重み付けした平均で、端数バッチも含める。
/// `train_loss`の教師は混合後の教師確率、`validation_loss`は全検証集合の二値交差エントロピーで、除去損失を含めない。
/// `removal_loss`は[`super::Losses::removal`]と同じロジット単位の値である。
/// `--removal-penalty 0`では除去の参照を作らず、除去損失も計算しないため、訓練後の`removal_loss`は0になる。
/// エポック0では`train_loss`、`removal_loss`、`total_loss`、`training_seconds`、
/// `training_positions_per_second`、`update_seconds`が`null`、`steps`が0である。
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

/// `config.json`に保存する引数、設定、入力、前処理結果を作る。
///
/// 最上位の欄は`device = "cpu"`、新規実行時の[`Args`]を直列化した`arguments`、
/// 実効ワーカー数`resolved_threads`、`settings`、`inputs`、`preprocessing`、`resumes`である。
/// 新規実行では呼び手が、開始から設定を書き出すまでの秒数`preprocessing_seconds`を加える。
/// `settings`は[`Args`]から`output_dir`、`resume`、`data`、`init`、`epochs`、`patience`、`threads`を除き、
/// `model = "mirrored_pst"`と`optimizer = {lr, weight_decay: 0.0, betas: [0.9, 0.999], epsilon: 1e-8}`を加えた値で、再開で変更できない。
///
/// `inputs.files`は入力順のMNSD識別情報で、各要素は正規化した`path`、`byte_length`、`header`を持つ。
/// `header`の各欄は[`crate::identity`]に従う。`inputs.init`はMNPTの`path`とヘッダ内の`checksum`である。
///
/// `preprocessing.teacher_classes`は分類順の[`crate::data::TeacherClass`]の配列である。
/// 検査和は32個の数値の配列、`lookahead`は`{gamma, plies}`または`null`、
/// `lambda`は分類へ適用した値であり、実戦対局では0である。`settings.lambda`は利用者が指定した値である。
/// `teacher_cp_per_logit`は分類順の教師の勝率尺度（1ロジットあたりのcp）でλが0の分類は`null`、`teacher_lambdas`は分類順の実効λである。
/// `cp_per_logit`は初期MNPTから引き継いだ勝率尺度である。
/// `training_positions`と`validation_positions`は局面数、`training_indices_sha256`と
/// `validation_indices_sha256`は入力を連結した0始まりの通算番号をリトルエンディアンu64列にしたSHA-256である。
///
/// `resumes`は新規実行では空で、再開のたびに`{resumed_at_unix_seconds, epochs, patience, workers, epoch}`を追記する。
/// 時刻は整数のUnix秒、`workers`は実効ワーカー数、`epoch`は再開時点の完了済みエポック番号である。
/// 尺度の出所は`"calibrate"`または正規化した較正ファイルのパスと全内容で、`preprocessing`で照合する。
/// `teacher_score_multiplier`は勝率尺度と教師の勝率尺度の比、`teacher_cp_per_logit_at_search_bound`は推定の境界到達状況である。
/// 両配列とも分類順で、λが0の分類は`null`とする。引数の`teacher_cp_per_logit`は`settings`から除く。
pub(super) fn configuration(
    args: &Args,
    workers: usize,
    dataset: &Dataset,
    calibration: &crate::data::TeacherCalibration,
    cp_per_logit: f32,
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
        "teacher_cp_per_logit",
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
    let files = dataset
        .paths()
        .map(file_record)
        .collect::<Result<Vec<_>>>()?;
    let inputs = json!({"files":files, "init":file_record(&args.init)?});
    let preprocessing = json!({"teacher_classes":dataset.teacher_classes(), "teacher_cp_per_logit":calibration.teacher_cp_per_logit,
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

/// 終了理由、最良エポック、量子化検査を`summary.json`へ保存する。
/// `status = "candidate"`なら終了コード0、`"quantization_error"`または`"error"`なら終了コード1である。
/// 量子化検査が不合格でも出力した重みと報告を残し、`quantization`と`initial_position_score`を記録する。
/// 最良重みの書き出しに失敗した場合は`reason`を加えた同じ形で保存し、この2欄は`null`になる。
/// エポックの途中の失敗では、この型の代わりに`{status: "error", reason, last_epoch, elapsed_seconds}`だけを保存する。
/// `initial_position_score`はFMの次元0のMNPT版3を[`minase::eval::pst::evaluate_pst`]で評価した値で、手番側視点のセンチポーンである。
/// この関数の評価は±28,999へ切り詰められ、報告時には追加の切り詰めを行わない。
/// `convergence`は常に`null`である。
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
    /// 書き出した重みによる初期局面の整数評価。手番側視点、センチポーン。
    pub initial_position_score: Option<i32>,
}
