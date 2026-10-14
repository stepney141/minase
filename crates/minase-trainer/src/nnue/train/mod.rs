//! CUDAまたはCPUによるNNUEの学習と、全検証集合の評価、保存と再開。
//! CUDAでは供給スレッドがワーカー数ぶんのバッチを並列に作り、順列の順に3面へ書く。
//! 各更新の結果は次の更新を投入してから回収し、CPUでは単一スレッドとビット一致する並列参照計算を使う。
//! 各エポックは`epoch_permutation(train, seed, epoch)`で始め、更新、snapshot、全検証、最良値更新、ログ、latest.npz、改善時のbest.npzの順に進む。
//! `latest.npz`をエポックの保存確定点とし、時間切れで未完了のエポックは採用しない。
//!
//! `--output-dir`には次の6ファイルを保存する。
//!
//! | ファイル | 内容 |
//! | --- | --- |
//! | `config.json` | 引数、実効ワーカー数、照合用の設定と入力、前処理の結果、デバイス名、再開履歴を保存する。欄は`record::configuration`に定める。 |
//! | `training.jsonl` | エポック0の検証と、検証を終えた各エポックの[`EpochRecord`]を1行ずつ追記する。 |
//! | `latest.npz` | 最後に検証を終えたモデル、AdamWの状態、更新回数、最良エポック、累積時間、照合用設定を[`Checkpoint`]で保存する。 |
//! | `best.npz` | 最良エポックのモデルとその時点のAdamWの状態を[`Checkpoint`]で保存する。 |
//! | `nnue.bin` | 最良モデルを量子化したMNUE版5を保存する。 |
//! | `summary.json` | 停止理由、最良エポック、量子化検査を[`Summary`]で保存する。 |
//!
//! 新規実行の出力ディレクトリは存在しないか空でなければならず、空でなければ`--resume`の使用を求めるエラーで終了する。
//! 停止条件はエポック開始前と終了時に、累積時間、改善のないエポック数、完了エポック数の順に判定する。
//! 累積時間には前処理、エポック0の検証、各エポックの学習と検証、再開前の実行を含む。
//! 訓練中はバッチ投入前に期限を判定し、期限を過ぎたエポックの更新は採用しない。
//! 検証では各バッチの前後に期限を判定し、期限を過ぎたエポックの最良値更新、ログ追記、チェックポイント保存を行わない。
//! 最良エポックは1以上から選び、[`crate::nn::update_best`]に従って検証損失が厳密に小さい場合だけ更新する。

use crate::nn::{atomic_write, patience_reached};
mod args;
mod backend;
mod base;
mod pipeline;
mod record;
#[cfg(test)]
mod tests;

use crate::{
    data::{Dataset, epoch_permutation},
    nnue::{
        data::{Observations, build_batches, build_targets},
        export::{self, QuantizationReport},
        model::{Checkpoint, Masks, Model, Parameter, Progress, Shape, load_initial},
        reference::{AdamW, RemovalReference, Validation},
    },
};
pub use args::Args;
use backend::Backend;
pub use record::{EpochRecord, LinearMovement, Saturation};
use record::{
    append_epoch, append_resume, configuration, linear_movement, reconcile_log,
    validate_statistics, write_json,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    fmt,
    time::{Duration, Instant},
};

/// 学習を継続できない引数、保存状態、または終了時の検査結果。
#[derive(Debug)]
pub enum Error {
    /// 引数または保存状態が契約に違反する。
    Invalid(&'static str),
    /// 1エポックも検証を完了せず時間の上限に達した。
    NoCompletedEpoch,
    /// 量子化した推論の平均絶対誤差がK/40を超えた。
    QuantizationError,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => f.write_str(reason),
            Self::NoCompletedEpoch => f.write_str("time limit reached with 0 completed training epochs; 予算の上限で終了し、収束は未確認"),
            Self::QuantizationError => f.write_str("quantization_error: mean absolute error exceeds K/40"),
        }
    }
}
impl std::error::Error for Error {}
/// 原因の型を保持する、学習の結果。
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// 学習を停止した理由。診断値だけでは停止しない。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// 指定されたエポック数を完了した。
    MaxEpochs,
    /// 指定された連続エポック数にわたって改善しなかった。
    EarlyStopping,
    /// 再開前を含む時間の上限に達した。
    TimeLimit,
}

/// 終了理由、最良エポック、量子化検査を`summary.json`へ保存する。
///
/// `status = "candidate"`は量子化検査の合格で、コマンドの終了コードは0である。
/// 平均絶対誤差がK/40を超えると`"quantization_error"`となり、`nnue.bin`と`best.npz`を残して終了コード1を返す。
/// このとき`reason`は`"quantization_error: mean absolute error exceeds K/40"`である。
/// 1エポックも検証を終えず時間切れになった場合と、最良重みの書き出しに失敗した場合は`"error"`で終了コード1となる。
/// 前者の`reason`は`"time limit reached with 0 completed training epochs; 予算の上限で終了し、収束は未確認"`である。
/// `stopped_by = "time_limit"`なら`convergence`は`"予算の上限で終了し、収束は未確認"`、それ以外は`null`である。
/// 学習ループ中のエラーで保存する簡略な報告は[`run`]に定める。
#[derive(Debug, Serialize)]
pub struct Summary {
    /// candidate、quantization_error、またはerror。
    pub status: &'static str,
    /// 検査失敗または学習未完了の理由。
    pub reason: Option<String>,
    /// 学習の停止理由。
    pub stopped_by: StopReason,
    /// 時間切れの場合の収束に関する注記。
    pub convergence: Option<&'static str>,
    /// 検証を終えた最終エポック。
    pub last_epoch: u64,
    /// 1以上の最良エポック。未完了ならnull。
    pub best_epoch: Option<u64>,
    /// 最良エポックの検証損失。
    pub best_validation_loss: Option<f64>,
    /// 初期モデルの検証損失。初期検証中の時間切れならnull。
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

fn update_best(progress: &mut Progress, epoch: u64, validation_loss: f64) -> Result<bool> {
    if !validation_loss.is_finite() || validation_loss < 0.0 {
        return Err(Error::Invalid("non-finite or negative validation loss").into());
    }
    if epoch == 0 {
        return Ok(false);
    }
    if epoch <= progress.epoch {
        return Err(Error::Invalid("epoch must advance").into());
    }
    progress.epoch = epoch;
    Ok(crate::nn::update_best(
        &mut progress.best,
        epoch,
        validation_loss,
    )?)
}

/// エポック開始前と終了時の停止理由を優先順に決める。
/// 累積秒数が`time_limit_hours × 3600`以上なら`time_limit`、最良エポックから
/// `patience`エポック以上経過したら`early_stopping`、完了エポック数が`epochs`以上なら`max_epochs`を返す。
fn stop_reason(progress: &Progress, args: &Args) -> Option<StopReason> {
    if progress.elapsed_seconds >= args.time_limit_hours * 3600.0 {
        Some(StopReason::TimeLimit)
    } else if patience_reached(progress.best, progress.epoch, args.patience) {
        Some(StopReason::EarlyStopping)
    } else if progress.epoch >= args.epochs {
        Some(StopReason::MaxEpochs)
    } else {
        None
    }
}

struct Session {
    args: Args,
    workers: usize,
    dataset: Dataset,
    base: base::Base,
    teacher_ks: Vec<f64>,
    checkpoint: Checkpoint,
    backend: Backend,
    started: Instant,
    previous_elapsed: f64,
    initial_validation_loss: Option<f64>,
    gpu_used_bytes: Option<usize>,
    gpu_total_bytes: Option<usize>,
}

impl Session {
    /// 初期値と前処理を再計算し、新規学習または保存済みエポックからの再開を準備する。
    /// `--init-pst`の2端点を[`validate_initial`]で`--base`のMNPTと照合し、初期化したモデルの
    /// [`Model::expanded_linear`]も同じ検査に通す。不一致なら学習を始めない。
    /// 全MNSDの規則セット名は基準MNPTと一致し、訓練集合と検証集合はどちらも空であってはならない。
    /// 出力用K、47個の探索用駒価値、規則セット名は[`base::Base`]としてMNPTから引き継ぎ、Kを[`Shape`]へ渡す。
    /// 初期値の鏡映対称性は[`load_initial`]と[`Model::initialize`]で検査する。
    /// 再開にはconfig.jsonのresumes配列の存在と、settings、inputs、preprocessingの一致を要求する。
    /// settingsからは出力先、再開指定、入力パス、epochs、patience、time_limit_hours、threadsを除く。
    /// 入力の識別情報は正規化したパスとともにinputsへ記録し、欄ごとに照合する。
    /// 予算を除く理由は`docs/lessons/resume-hash-excludes-budgets.md`にある。
    /// 復元した形状、マスク、初期値、係数、シードは再計算値と一致し、更新回数は`epoch×ceil(N_train/batch)`、epochは指定上限以下でなければならない。
    /// 記録されたdevice_nameも一致を要求する。
    /// best.npzの書き込み前に中断した場合は、最新エポックが最良であるときだけlatest.npzから復元できる。
    fn new(args: Args) -> Result<Self> {
        let started = Instant::now();
        args.validate()?;
        let workers = args.workers()?;
        let base = base::read(&args.base)?;
        let initial = load_initial(&args.init_pst)?;
        validate_initial(&initial, &base.linear)?;

        // このセッションもDatasetを破棄するまで入力ファイルへ書き込まない。
        let dataset = Dataset::open(
            &args.data,
            args.lookahead()?,
            std::num::NonZeroUsize::new(workers)
                .ok_or(crate::nn::Error::Invalid("worker count must be positive"))?,
        )?
        .with_lambda(args.training.lambda)?;
        if dataset.training_indices().is_empty() || dataset.validation_indices().is_empty() {
            return Err(
                Error::Invalid("training and validation sets must both be nonempty").into(),
            );
        }
        if dataset
            .headers()
            .any(|header| header.rule_set() != base.rule_set)
        {
            return Err(Error::Invalid("MNSD rule set differs from baseline MNPT").into());
        }
        let teacher_ks = dataset.estimate_teacher_ks()?;
        let observations = Observations::compute(&dataset, args.feature_set)?;
        let model = Model::initialize(
            Shape::new(args.feature_set, args.width, base.k)?,
            &initial,
            Masks::from_observations(args.feature_set, &observations)?,
            args.seed,
            0.05,
        )?;
        validate_initial(&model.expanded_linear(), &base.linear)?;
        let mut config =
            configuration(&args, workers, &dataset, &observations, &teacher_ks, &model)?;
        let mut checkpoint = Checkpoint {
            removal_penalty: args.training.removal_penalty,
            lambda: args.training.lambda,
            optimizer: AdamW::new(model.shape(), args.training.optimizer())?,
            model,
            progress: Progress {
                epoch: 0,
                best: None,
                elapsed_seconds: 0.0,
                seed: args.seed,
                config: crate::nn::checkpoint::Configuration::from_document(&config.document)?,
            },
        };
        let mut initial_validation_loss = None;
        let saved_config = if let Some(resume) = &args.resume {
            if resume.canonicalize()? != args.output_dir.canonicalize()? {
                return Err(Error::Invalid(
                    "--resume and --output-dir must identify the same directory",
                )
                .into());
            }
            let saved: Value = serde_json::from_slice(&std::fs::read(resume.join("config.json"))?)?;
            if saved.get("resumes").and_then(Value::as_array).is_none() {
                return Err(Error::Invalid("configuration resumes must be an array").into());
            }
            crate::nn::checkpoint::Configuration::from_document(&saved)?.check(
                &crate::nn::checkpoint::Configuration::from_document(&config.document)?,
            )?;
            let restored = Checkpoint::read(resume.join("latest.npz"))?;
            restored.check_config(&crate::nn::checkpoint::Configuration::from_document(
                &config.document,
            )?)?;
            if restored.model.shape() != checkpoint.model.shape()
                || restored.model.masks() != checkpoint.model.masks()
                || restored.model.initial_linear() != checkpoint.model.initial_linear()
                || restored.model.initial_virtual() != checkpoint.model.initial_virtual()
                || restored.optimizer.config() != args.training.optimizer()
                || restored.removal_penalty != args.training.removal_penalty
                || restored.lambda != args.training.lambda
                || restored.progress.seed != args.seed
                || restored.progress.epoch > args.epochs
                || restored.optimizer.step()
                    != restored.progress.epoch
                        * dataset.training_indices().len().div_ceil(args.batch) as u64
            {
                return Err(Error::Invalid(
                    "checkpoint state differs from recomputed initialization or progress",
                )
                .into());
            }
            checkpoint = restored;
            initial_validation_loss = Some(reconcile_log(
                &resume.join("training.jsonl"),
                checkpoint.progress.epoch,
            )?);
            Some(saved)
        } else {
            if args.output_dir.exists() && std::fs::read_dir(&args.output_dir)?.next().is_some() {
                return Err(Error::Invalid(
                    "output directory is not empty; use --resume to continue a run",
                )
                .into());
            }
            None
        };
        let backend = Backend::new(
            args.device,
            &checkpoint.model,
            &checkpoint.optimizer,
            &RemovalReference::v0(),
            args.batch,
            args.training.removal_penalty,
            workers,
        )?;
        let name = backend.name();
        if let Some(saved) = &saved_config {
            if saved.get("device_name") != Some(&json!(name)) {
                return Err(
                    Error::Invalid("resume device differs from the recorded device").into(),
                );
            }
        } else {
            std::fs::create_dir_all(&args.output_dir)?;
            config.document["device_name"] = json!(name);
            let memory = backend.memory()?;
            config.document["gpu_memory_used_bytes"] = json!(memory.map(|(used, _)| used));
            config.document["gpu_memory_total_bytes"] = json!(memory.map(|(_, total)| total));
            config.document["preprocessing_seconds"] = json!(started.elapsed().as_secs_f64());
            write_json(&args.output_dir.join("config.json"), &config.document)?;
        }
        let previous_elapsed = checkpoint.progress.elapsed_seconds;
        let memory = backend.memory()?;
        let mut session = Self {
            args,
            workers,
            dataset,
            base,
            teacher_ks,
            checkpoint,
            backend,
            started,
            previous_elapsed,
            initial_validation_loss,
            gpu_used_bytes: memory.map(|(used, _)| used),
            gpu_total_bytes: memory.map(|(_, total)| total),
        };
        if session.checkpoint.progress.best.is_some() {
            recover_best_checkpoint(&session.args.output_dir, &session.checkpoint)?;
        }
        if let Some(mut saved) = saved_config {
            append_resume(
                &session.args.output_dir.join("config.json"),
                &mut saved,
                &session.args,
                session.workers,
                session.checkpoint.progress.epoch,
            )?;
        }
        if session.initial_validation_loss.is_none() && !session.expired() {
            let validation_started = Instant::now();
            let model = session.checkpoint.model.clone();
            if let Some(validation) = session.validate(&model)? {
                session.initial_validation_loss = Some(validation.loss());
                session.checkpoint.progress.elapsed_seconds = session.elapsed();
                let record = session.epoch_record(
                    &validation,
                    None,
                    validation_started.elapsed().as_secs_f64(),
                );
                append_epoch(&session.log_path(), &record)?;
                session.save_latest()?;
            }
        }
        Ok(session)
    }

    fn elapsed(&self) -> f64 {
        self.previous_elapsed + self.started.elapsed().as_secs_f64()
    }
    fn expired(&self) -> bool {
        self.elapsed() >= self.args.time_limit_hours * 3600.0
    }
    fn deadline(&self) -> Instant {
        self.started
            + Duration::from_secs_f64(
                (self.args.time_limit_hours * 3600.0 - self.previous_elapsed).max(0.0),
            )
    }
    fn log_path(&self) -> std::path::PathBuf {
        self.args.output_dir.join("training.jsonl")
    }
    fn save_latest(&self) -> Result<()> {
        Ok(atomic_write(
            &self.args.output_dir.join("latest.npz"),
            &self.checkpoint.encode()?,
        )?)
    }
    fn memory(&mut self) -> Result<()> {
        if let Some((used, total)) = self.backend.memory()? {
            self.gpu_used_bytes = self.gpu_used_bytes.max(Some(used));
            self.gpu_total_bytes = Some(total);
        }
        Ok(())
    }

    fn validate(&mut self, model: &Model) -> Result<Option<Validation>> {
        if self.expired() {
            return Ok(None);
        }
        let mut evaluator =
            self.backend
                .evaluator(model, self.args.batch, self.args.training.removal_penalty)?;
        self.memory()?;
        validate_batches(
            self.dataset.validation_indices().chunks(self.args.batch),
            |indices| {
                let batch =
                    build_batches(&self.dataset, self.args.feature_set, &[indices])?.remove(0);
                let values =
                    build_targets(&batch, &self.teacher_ks, self.dataset.teacher_lambdas())?;
                evaluator.validate(&batch, &values)
            },
            || self.expired(),
        )
    }

    fn advance(&mut self) -> Result<bool> {
        self.checkpoint.progress.elapsed_seconds = self.elapsed();
        if stop_reason(&self.checkpoint.progress, &self.args).is_some() {
            return Ok(false);
        }
        self.memory()?;
        let epoch = self.checkpoint.progress.epoch + 1;
        let training_started = Instant::now();
        let indices = epoch_permutation(self.dataset.training_indices(), self.args.seed, epoch);
        let deadline = self.deadline();
        let training = pipeline::train_epoch(
            pipeline::Supply {
                dataset: &self.dataset,
                feature_set: self.args.feature_set,
                indices: &indices,
                teacher_ks: &self.teacher_ks,
                batch: self.args.batch,
                schedule: self.args.lr_schedule,
                steps_per_epoch: self
                    .dataset
                    .training_indices()
                    .len()
                    .div_ceil(self.args.batch) as u64,
                workers: self.workers,
                deadline,
            },
            &mut self.backend,
        )?;
        let Some(training) = training else {
            return Ok(false);
        };
        self.gpu_used_bytes = self.gpu_used_bytes.max(training.gpu_used_bytes);
        self.gpu_total_bytes = training.gpu_total_bytes;
        let training_seconds = training_started.elapsed().as_secs_f64();
        let (parameters, optimizer) = self.backend.snapshot()?;
        parameters.validate()?;
        let mut candidate = self.checkpoint.model.clone();
        for p in Parameter::ALL {
            candidate.parameter_mut(p).copy_from_slice(&parameters[p]);
        }
        let validation_started = Instant::now();
        let Some(validation) = self.validate(&candidate)? else {
            return Ok(false);
        };
        let validation_seconds = validation_started.elapsed().as_secs_f64();
        self.checkpoint.model = candidate;
        self.checkpoint.optimizer = optimizer;
        let improved = update_best(&mut self.checkpoint.progress, epoch, validation.loss())?;
        self.checkpoint.progress.elapsed_seconds = self.elapsed();
        let record = self.epoch_record(
            &validation,
            Some((&training, training_seconds)),
            validation_seconds,
        );
        append_epoch(&self.log_path(), &record)?;
        // latestがコミット点。新しいbestを公開する前に保存し、中断時にはlatestからbestを復元する。
        self.save_latest()?;
        if improved {
            atomic_write(
                &self.args.output_dir.join("best.npz"),
                &self.checkpoint.encode()?,
            )?;
        }
        eprintln!(
            "epoch {epoch}: train_loss={:.9} removal_loss={:.9} validation_loss={:.9}",
            training.bce / training.positions as f64,
            training.auxiliary / training.positions as f64,
            validation.loss()
        );
        Ok(true)
    }

    fn epoch_record(
        &self,
        validation: &Validation,
        training: Option<(&pipeline::Training, f64)>,
        validation_seconds: f64,
    ) -> EpochRecord {
        let saturation = validation.saturation();
        let train_loss = training.map(|(t, _)| t.bce / t.positions as f64);
        let removal_loss = training.map(|(t, _)| t.auxiliary / t.positions as f64);
        EpochRecord {
            device: self.args.device,
            epoch: self.checkpoint.progress.epoch,
            train_loss,
            removal_loss,
            total_loss: train_loss.zip(removal_loss).map(|(a, b)| a + b),
            validation_loss: validation.loss(),
            sign_accuracy: validation.sign_accuracy(),
            samples: validation.positions,
            decisive_samples: validation.decisive,
            nonlinear_std_cp: validation.nonlinear_std_cp(),
            saturation: Saturation {
                acc_le_zero: saturation[0],
                acc_ge_one: saturation[1],
            },
            linear_movement: linear_movement(&self.checkpoint.model),
            training_seconds: training.map(|(_, seconds)| seconds),
            validation_seconds,
            training_positions_per_second: training
                .map(|(t, seconds)| t.positions as f64 / seconds),
            update_seconds: training.map(|(t, _)| t.compute_seconds / t.steps as f64),
            gpu_memory_used_bytes: self.gpu_used_bytes,
            gpu_memory_total_bytes: self.gpu_total_bytes,
            elapsed_seconds: self.checkpoint.progress.elapsed_seconds,
            steps: training.map_or(0, |(t, _)| t.steps),
            total_steps: self.checkpoint.optimizer.step(),
            lr_multiplier_first: training.and_then(|(t, _)| t.first_multiplier),
            lr_multiplier_last: training.and_then(|(t, _)| t.last_multiplier),
        }
    }

    fn best_checkpoint(&self) -> Result<Checkpoint> {
        let best = Checkpoint::read(self.args.output_dir.join("best.npz"))?;
        best.check_config(&self.checkpoint.progress.config)?;
        if best.progress.best != self.checkpoint.progress.best
            || best
                .progress
                .best
                .is_none_or(|entry| entry.epoch != best.progress.epoch)
        {
            return Err(Error::Invalid("best checkpoint disagrees with latest checkpoint").into());
        }
        Ok(best)
    }

    fn finish(mut self) -> Result<Summary> {
        self.checkpoint.progress.elapsed_seconds = self.elapsed();
        let reason = stop_reason(&self.checkpoint.progress, &self.args).ok_or(Error::Invalid(
            "training stopped without a stopping condition",
        ))?;
        // 時間切れの未完了エポックは採用せず、費やした時間だけを保存する。
        if self.initial_validation_loss.is_some() {
            self.save_latest()?;
        }
        let best = self.checkpoint.progress.best;
        let mut summary = Summary {
            status: "error",
            reason: None,
            stopped_by: reason,
            convergence: (reason == StopReason::TimeLimit)
                .then_some("予算の上限で終了し、収束は未確認"),
            last_epoch: self.checkpoint.progress.epoch,
            best_epoch: best.map(|v| v.epoch),
            best_validation_loss: best.map(|v| v.validation_loss),
            initial_validation_loss: self.initial_validation_loss,
            improved_over_epoch_zero: best
                .zip(self.initial_validation_loss)
                .map(|(v, initial)| v.validation_loss < initial),
            total_steps: self.checkpoint.optimizer.step(),
            elapsed_seconds: self.checkpoint.progress.elapsed_seconds,
            quantization: None,
        };
        if best.is_none() {
            summary.reason = Some(Error::NoCompletedEpoch.to_string());
            write_json(&self.args.output_dir.join("summary.json"), &summary)?;
            return Err(Error::NoCompletedEpoch.into());
        }
        let checkpoint = self.best_checkpoint()?;
        let quantization = match export::write_best(
            &self.dataset,
            &checkpoint,
            &self.args.output_dir,
            &self.base.piece_values,
            &self.base.rule_set,
            self.args.validation_sample,
        ) {
            Ok(report) => report,
            Err(error) => {
                summary.reason = Some(error.to_string());
                write_json(&self.args.output_dir.join("summary.json"), &summary)?;
                return Err(error.into());
            }
        };
        let passed = quantization.passed;
        summary.status = if passed {
            "candidate"
        } else {
            "quantization_error"
        };
        summary.quantization = Some(quantization);
        if !passed {
            summary.reason = Some(Error::QuantizationError.to_string());
        }
        write_json(&self.args.output_dir.join("summary.json"), &summary)?;
        if !passed {
            return Err(Error::QuantizationError.into());
        }
        Ok(summary)
    }
}

/// 全検証バッチを集計し、各バッチの前後に期限を検査する。
/// 最終バッチの後も検査し、期限を過ぎたら`None`を返す。
/// 呼び手はそのエポックの最良値更新、ログ追記、チェックポイント保存を行わない。
fn validate_batches<T>(
    batches: impl IntoIterator<Item = T>,
    mut evaluate: impl FnMut(T) -> Result<Validation>,
    expired: impl Fn() -> bool,
) -> Result<Option<Validation>> {
    let mut total: Option<Validation> = None;
    for batch in batches {
        if expired() {
            return Ok(None);
        }
        let statistics = evaluate(batch)?;
        validate_statistics(&statistics)?;
        if let Some(total) = &mut total {
            total.loss_sum += statistics.loss_sum;
            total.correct += statistics.correct;
            total.decisive += statistics.decisive;
            total.nonlinear_sum += statistics.nonlinear_sum;
            total.nonlinear_square_sum += statistics.nonlinear_square_sum;
            total.lower += statistics.lower;
            total.upper += statistics.upper;
            total.positions += statistics.positions;
            total.activations += statistics.activations;
        } else {
            total = Some(statistics);
        }
        // 最終バッチの後も期限を確認し、検証完了が期限を越えたエポックを採用しない。
        if expired() {
            return Ok(None);
        }
    }
    let total = total.ok_or(Error::Invalid("empty validation set"))?;
    validate_statistics(&total)?;
    Ok(Some(total))
}

/// 実数PSTの全13,680特徴×2端点が基準MNPTへ再量子化できることを検査する。
/// `--init-pst`の`middlegame`と`endgame`の各値vについて、`round_ties_even(v × 8)`が
/// 対応するMNPTのi16値と一致しなければならない。MNPTの単位は1/8センチポーンである。
/// 配列長の不一致と非有限値も拒否し、初期化後の[`Model::expanded_linear`]にも同じ検査を行う。
fn validate_initial(initial: &[[f32; 2]], baseline: &[[i16; 2]]) -> Result<()> {
    if initial.len() != baseline.len()
        || initial
            .iter()
            .flatten()
            .zip(baseline.iter().flatten())
            .any(|(&value, &integer)| {
                !value.is_finite()
                    || (f64::from(value) * 8.0).round_ties_even() != f64::from(integer)
            })
    {
        return Err(
            Error::Invalid("--init-pst does not requantize to the baseline MNPT PST").into(),
        );
    }
    Ok(())
}

/// 確定済みの`latest.npz`に対応する`best.npz`を復旧する。
/// 保存順は[`reconcile_log`]の契約に従う。
/// `best.npz`が最良エポックと一致しなければ、最新の完了エポックが最良であり、
/// `best.npz`がないか、その完了エポックが最良より古いときだけ`latest.npz`から書き直す。
/// 既存の`best.npz`の復号失敗、設定の不一致、最良以上のエポックの不整合は復旧せず拒否する。
fn recover_best_checkpoint(directory: &std::path::Path, latest: &Checkpoint) -> Result<()> {
    let best = latest
        .progress
        .best
        .ok_or(Error::Invalid("checkpoint has no best epoch"))?;
    let path = directory.join("best.npz");
    let saved = if path.exists() {
        Some(Checkpoint::read(&path)?)
    } else {
        None
    };
    if let Some(saved) = &saved {
        saved.check_config(&latest.progress.config)?;
        if saved.progress.best == Some(best) && saved.progress.epoch == best.epoch {
            return Ok(());
        }
    }
    if latest.progress.epoch != best.epoch
        || saved
            .as_ref()
            .is_some_and(|saved| saved.progress.epoch >= best.epoch)
    {
        return Err(Error::Invalid("best checkpoint disagrees with latest checkpoint").into());
    }
    eprintln!(
        "recovering best checkpoint at epoch {} from committed latest checkpoint",
        best.epoch
    );
    Ok(atomic_write(&path, &latest.encode()?)?)
}

/// 指定デバイスで初期検証、学習、保存、最良重みの書き出しを順に行う。
///
/// 成功時と最終検査の失敗時の報告、および終了コードは[`Summary`]に従う。
/// 学習ループ中のエラーでは、この型の代わりに`status = "error"`、`reason`、`last_epoch`、
/// `elapsed_seconds`の4欄だけを`summary.json`へ書き、コマンドは終了コード1で終わる。
/// 終了時は時間切れで未完了のエポックを採用せず、初期検証を完了していれば費やした時間を`latest.npz`へ保存する。
/// 最良エポックがあれば`best.npz`から`nnue.bin`を書き出す。
/// 再開後も同じシードとエポック番号から[`epoch_permutation`]で順列を再生成する。
pub fn run(args: Args) -> Result<Summary> {
    let mut session = Session::new(args)?;
    loop {
        match session.advance() {
            Ok(true) => {}
            Ok(false) => break,
            Err(error) => {
                write_json(
                    &session.args.output_dir.join("summary.json"),
                    &json!({"status":"error", "reason":error.to_string(),
                    "last_epoch": session.checkpoint.progress.epoch, "elapsed_seconds": session.elapsed()}),
                )?;
                return Err(error);
            }
        }
    }
    session.finish()
}
