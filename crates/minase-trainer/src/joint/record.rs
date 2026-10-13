use super::{Args, Distribution, GradientNorm, QuantizationReport, Result};
use crate::{
    data::Dataset,
    fm::record::{Configuration, file_record, hash_u64},
    nn::{hex, sha256},
};
use serde::Serialize;
use serde_json::json;
use std::path::Path;

/// 同時学習の1エポックの記録。
/// 損失は局面数で重み付けした平均で、端数バッチも含める。
/// `removal_loss`は係数適用前の駒除去損失をKで割ったロジット単位の値で、参照は常にv0の駒価値である。
/// `--removal-penalty 0`でも計算して記録し、損失全体と勾配へ掛ける係数だけが0になる。
/// `phi_squared_mean`は係数適用前の`mean(Φ²)`である。
/// `total_loss = train_loss + lambda_res * phi_squared_mean + removal_penalty * removal_loss`とする。
/// `validation_loss`は全検証集合の二値交差エントロピーで、除去損失と正則化項を含めない。
/// エポック0では訓練の損失、`phi_squared_mean`、訓練秒数、速度、更新秒数が`null`、`steps`が0である。
#[derive(Debug, Serialize)]
pub struct EpochRecord {
    /// CPUで実行したことを記録する。
    pub device: &'static str,
    /// 完了したエポック。0は学習前の検証。
    pub epoch: u64,
    /// 訓練集合の二値交差エントロピーの平均。
    pub train_loss: Option<f64>,
    /// 係数を掛ける前の除去損失。
    pub removal_loss: Option<f64>,
    /// Phiの二乗平均。
    pub phi_squared_mean: Option<f64>,
    /// 全検証局面のFM補正の分布。
    pub phi: Distribution,
    /// BCE、正則化項、係数適用後の除去損失の和。
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

/// 同時学習の`config.json`を、[`pst::record::configuration`](../../pst/record/fn.configuration.html)と同じ構成で作る。
/// `settings`からは`init`に代えて`base`、`pst_init`、`fm_init`、`fm_reference`を除く。
/// `settings.model = "joint_mirrored_pst_fm_rank32"`とし、`settings.optimizer`は
/// `{pst_lr, pst_weight_decay: 0, fm_lr, fm_weight_decay, betas: [0.9, 0.999], epsilon: 1e-8}`である。
/// `inputs`は`files`のほかに`base`、`pst_init`、`fm_init`、`fm_reference`の各`{path, sha256}`を持ち、`init`を持たない。
/// `preprocessing`には[`fm::record::configuration`](../../fm/record/fn.configuration.html)と同じ`feature_observations`と`unobserved_features`を加える。
/// 加えて、`--fm-init`のマスクと数え直したマスクの不一致数`mask_mismatches`、および`removal_reference = "v0"`を保存する。
pub(super) fn configuration(
    args: &Args,
    workers: usize,
    dataset: &Dataset,
    ks: &[f64],
    k: f32,
    observations: (&[u64], usize),
) -> Result<Configuration> {
    let (counts, mask_mismatches) = observations;
    let mut settings = serde_json::to_value(args)?;
    let object = settings
        .as_object_mut()
        .ok_or("arguments are not an object")?;
    // 予算と並列度だけは再開で変更できる。ファイルは検査和と正規化したパスで照合する。
    for key in [
        "output_dir",
        "resume",
        "data",
        "base",
        "pst_init",
        "fm_init",
        "fm_reference",
        "epochs",
        "patience",
        "threads",
    ] {
        object.remove(key);
    }
    object.insert("model".into(), json!("joint_mirrored_pst_fm_rank32"));
    object.insert(
        "optimizer".into(),
        json!({"pst_lr":args.pst_lr, "pst_weight_decay":0, "fm_lr":args.fm_lr, "fm_weight_decay":args.fm_weight_decay, "betas":[0.9,0.999], "epsilon":1e-8}),
    );
    let files = dataset.paths().zip(dataset.headers()).map(|(path, header)| {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(".provenance.json");
        Ok(json!({"data":file_record(path)?, "provenance":file_record(Path::new(&sidecar))?, "header_sha256":hex(&sha256(&header.encode()))}))
    }).collect::<Result<Vec<_>>>()?;
    let inputs = json!({"files":files, "base":file_record(&args.base)?, "pst_init":file_record(&args.pst_init)?, "fm_init":file_record(&args.fm_init)?, "fm_reference":file_record(&args.fm_reference)?});
    let preprocessing = json!({"feature_observations":counts, "mask_mismatches":mask_mismatches, "removal_reference":"v0", "unobserved_features":counts.iter().filter(|&&n| n == 0).count(), "teacher_classes":dataset.teacher_classes(), "teacher_ks":ks,
        "teacher_lambdas":dataset.teacher_lambdas(), "teacher_k_source":"training", "output_k":k,
        "training_positions":dataset.training_indices().len(), "validation_positions":dataset.validation_indices().len(),
        "training_indices_sha256":hash_u64(dataset.training_indices()), "validation_indices_sha256":hash_u64(dataset.validation_indices())});
    let config_hash = sha256(&serde_json::to_vec(&settings)?);
    let input_hash = sha256(&serde_json::to_vec(
        &json!({"inputs":inputs,"preprocessing":preprocessing}),
    )?);
    Ok(Configuration {
        config_hash,
        input_hash,
        document: json!({"device":"cpu", "arguments":args, "resolved_threads":workers, "settings":settings,
            "inputs":inputs, "preprocessing":preprocessing, "resumes":[], "config_sha256":hex(&config_hash), "input_sha256":hex(&input_hash)}),
    })
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
///
/// 学習前の量子化検査が不合格なら学習せずに
/// `{status: "quantization_error", reason: "initial quantization mean absolute error exceeds 2 cp", initial_quantization, total_steps}`を保存する。
/// その検査自体が失敗した場合は`{status: "error", reason, total_steps}`を保存する。
/// エポック途中の失敗では`{status: "error", reason, last_epoch, gradients, total_steps, elapsed_seconds}`を保存する。
/// 最終の量子化検査が不合格でも重みと`initial_position_score`は残す。
/// `initial_position_score`は、書き出したMNPT版3のPST評価に整数FM補正を加え、±28,999へ切り詰めた手番側視点のセンチポーンである。
#[derive(Debug, Serialize)]
pub struct Summary {
    /// candidate、quantization_error、またはerror。
    pub status: &'static str,
    /// 検査失敗または学習未完了の理由。
    pub reason: Option<String>,
    /// 学習の停止理由。
    pub stopped_by: StopReason,
    /// 収束に関する注記。同時学習ではnull。
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
    /// 学習前の量子化検査。
    pub initial_quantization: Option<QuantizationReport>,
    /// 書き出したネットの初期局面の整数評価。
    pub initial_position_score: Option<i32>,
    /// 最良エポックの全検証集合のPhi分布。
    pub phi: Option<Distribution>,
    /// 量子化誤差の検査結果。
    pub quantization: Option<QuantizationReport>,
}
