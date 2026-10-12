use super::Args;
use crate::{
    data::Dataset,
    train::{
        Error, Result,
        record::{Configuration, digest, file_record, hash_u64, hex},
    },
};
use serde::Serialize;
use serde_json::json;
use std::path::Path;

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
