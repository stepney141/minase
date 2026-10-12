use super::{Args, Distribution, GradientNorm, QuantizationReport, Result};
use crate::{
    data::{Dataset, replay::Replay},
    fm::record::{Configuration, digest, file_record, hash_u64, hex},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::Path;

/// 同時学習の1エポックの記録。学習前の損失と速度はnull。
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

pub(super) fn configuration(
    args: &Args,
    workers: usize,
    dataset: &Dataset,
    ks: &[f64],
    k: f32,
    replay: Option<&Replay>,
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
        "replay_steps",
    ] {
        object.remove(key);
    }
    object.insert("model".into(), json!("joint_mirrored_pst_fm_rank32"));
    object.insert(
        "replay".into(),
        replay.map_or(
            Value::Null,
            |replay| json!({"path": replay.directory(), "replay_json_sha256": replay.sha256()}),
        ),
    );
    if let Some(replay) = replay {
        object.insert(
            "replay_initial".into(),
            file_record(&replay.directory().join("initial.npz"))?,
        );
    }
    object.insert(
        "optimizer".into(),
        json!({"pst_lr":args.pst_lr, "pst_weight_decay":0, "fm_lr":args.fm_lr, "fm_weight_decay":args.fm_weight_decay, "betas":[0.9,0.999], "epsilon":1e-8}),
    );
    let files = dataset.paths().zip(dataset.headers()).map(|(path, header)| {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(".provenance.json");
        Ok(json!({"data":file_record(path)?, "provenance":file_record(Path::new(&sidecar))?, "header_sha256":hex(&digest(&header.encode()))}))
    }).collect::<Result<Vec<_>>>()?;
    let inputs = json!({"files":files, "base":file_record(&args.base)?, "pst_init":file_record(&args.pst_init)?, "fm_init":file_record(&args.fm_init)?, "fm_reference":file_record(&args.fm_reference)?});
    let preprocessing = json!({"feature_observations":counts, "mask_mismatches":mask_mismatches, "removal_reference":"v0", "unobserved_features":counts.iter().filter(|&&n| n == 0).count(), "teacher_classes":dataset.teacher_classes(), "teacher_ks":ks,
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

/// 学習を停止した理由。診断値だけでは停止しない。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// 一時的な照合用に指定した更新回数へ到達した。
    ReplaySteps,
    /// 指定されたエポック数を完了した。
    MaxEpochs,
    /// 指定された連続エポック数にわたって改善しなかった。
    EarlyStopping,
}

/// 終了理由、最良エポック、量子化検査をsummary.jsonへ保存する。
#[derive(Debug, Serialize)]
pub struct Summary {
    /// candidate、replay_steps、quantization_error、またはerror。
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
