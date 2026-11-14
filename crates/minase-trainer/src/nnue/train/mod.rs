//! CUDAまたはCPUで、指定したsuperbatch数だけNNUEを学習する。
//! 各周の順列は1回だけ作り、周の最後の短いバッチも1更新と数える。
//! superbatchの終了時は結果回収、snapshot、必要な検証、ログ、latest.npzの順に進む。
//! latest.npzの保存を確定の時点とし、時間切れのsuperbatchは採用しない。
//!
//! | ファイル | 内容 |
//! | --- | --- |
//! | config.json | 引数、照合用の設定と入力、前処理、デバイス名、再開履歴。 |
//! | training.jsonl | 学習前の検証と、完了したsuperbatchごとの[`SuperbatchRecord`]。 |
//! | latest.npz | 確定した重み、最適化器の状態、superbatch数、更新数、累積時間。 |
//! | nnue.bin | 予定の更新を終えたlatest.npzを量子化したMNUE版5。 |
//! | summary.json | 予定と完了のsuperbatch数と更新数、停止理由、検証と量子化の結果。 |
//!
//! 新規実行の出力先は存在しないか空でなければならない。
//! 累積時間には前処理と検証、再開前の実行を含む。
//! 検証損失は記録だけに用い、停止と重みの選択には用いない。

use crate::nn::atomic_write;
mod args;
mod backend;
mod base;
mod pipeline;
mod record;
mod spill_calibration;
mod store;
mod subsample;
use store::Store;
#[cfg(test)]
mod tests;

use crate::{
    data::{Dataset, epoch_permutation},
    nnue::{
        data::{BatchSlot, InputFeatureSet, build_batches, build_targets},
        export::{self, QuantizationReport, RemovalSignReport},
        model::{Checkpoint, Masks, Model, Parameter, Progress, Shape},
        reference::{AdamW, Validation},
    },
};
pub use args::Args;
use backend::Backend;
use minase::eval::features::FEATURE_COUNT;
pub use record::{LinearMovement, Saturation, SuperbatchRecord};
use record::{
    append_resume, append_superbatch, configuration, linear_movement, reconcile_log,
    validate_statistics, write_json,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    fmt,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

/// 学習を継続できない引数、保存状態、または終了時の検査結果。
#[derive(Debug)]
pub enum Error {
    /// 引数または保存状態が契約に違反する。
    Invalid(&'static str),
    /// 予定した更新回数に達する前に時間の上限に達した。
    Incomplete,
    /// 勝率尺度（1ロジットあたりのcp）を`cp_per_logit`とする。
    /// 量子化した推論の平均絶対誤差が`cp_per_logit / 40`を超えた。
    QuantizationError,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => f.write_str(reason),
            Self::Incomplete => f.write_str(
                "time limit reached before completing the planned training; resume to continue",
            ),
            Self::QuantizationError => {
                f.write_str("quantization_error: mean absolute error exceeds cp_per_logit / 40")
            }
        }
    }
}
impl std::error::Error for Error {}
/// 原因の型を保持する、学習の結果。
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// 第1層の正規分布の標準偏差を、基底特徴数Dに対してsqrt(1/(3D))とする。
/// HalfKPも乱数を置く仮想行の基底特徴数を使い、P型と初期値を揃える。
fn initial_standard_deviation() -> f32 {
    (1.0 / (3.0 * FEATURE_COUNT as f32)).sqrt()
}

/// 学習を停止した理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// 予定のsuperbatch数を完了した。
    Completed,
    /// 再開前を含む時間の上限に達した。
    TimeLimit,
}

/// 予定と確定済みの進捗、および最終の検査結果。
/// candidateだけが終了コード0であり、それ以外は1で終了する。
/// incompleteではネットを書き出さず、最後の確定済みチェックポイントを残す。
#[derive(Debug, Serialize)]
pub struct Summary {
    /// candidate、quantization_error、incomplete、errorのいずれか。
    pub status: &'static str,
    /// 失敗または未完了の理由。
    pub reason: Option<String>,
    /// 停止の理由。
    pub stopped_by: StopReason,
    /// 予定したsuperbatch数。
    pub planned_superbatches: u64,
    /// 確定したsuperbatch数。
    pub completed_superbatches: u64,
    /// 予定した更新回数。
    pub planned_steps: u64,
    /// 確定した更新回数。
    pub completed_steps: u64,
    /// 更新回数を1周の更新回数で割った周回数。
    pub epochs: f64,
    /// 最後に確定した検証損失。初期検証も未完了ならnull。
    pub final_validation_loss: Option<f64>,
    /// 再開前を含む累積秒数。
    pub elapsed_seconds: f64,
    /// 量子化誤差の検査結果。
    pub quantization: Option<QuantizationReport>,
    /// 量子化検査と同じ標本の駒除去差分の符号反転。
    pub removal_sign: Option<RemovalSignReport>,
}

/// 完了を優先し、予定に達していない場合だけ時間の上限を判定する。
fn stop_reason(progress: &Progress, args: &Args) -> Option<StopReason> {
    if progress.completed_superbatches == args.superbatches.get() {
        Some(StopReason::Completed)
    } else if progress.elapsed_seconds >= args.time_limit_hours * 3600.0 {
        Some(StopReason::TimeLimit)
    } else {
        None
    }
}

/// 同じ周の並べ替えをsuperbatchの境界を越えて保持する。
enum Epoch<'a> {
    Memory(Vec<u64>),
    Spill(crate::data::EpochReader<'a>),
}

struct Session {
    args: Args,
    workers: usize,
    dataset: Arc<Store>,
    selection: subsample::Selection,
    base: base::Base,
    teacher_cp_per_logit: Vec<f64>,
    checkpoint: Checkpoint,
    backend: Backend,
    started: Instant,
    previous_elapsed: f64,
    last_validation: Option<(u64, f64)>,
    gpu_used_bytes: Option<usize>,
    gpu_total_bytes: Option<usize>,
}

impl Session {
    /// 初期値と前処理を再計算し、照合を通った保存状態から再開する。
    /// 更新数は完了したsuperbatch数と1 superbatchの更新数の積に一致しなければならない。
    /// 予算と実行資源を除いた設定、入力、前処理、デバイス名は既存の照合で検査する。
    fn new(args: Args) -> Result<Self> {
        let started = Instant::now();
        let resumed_at = SystemTime::now();
        args.validate()?;
        let workers = args.workers()?;
        let base = base::read(&args.base)?;
        let initial = base.initial_linear();

        let mut dataset = Store::open(&args, workers)?;
        if dataset.training_count() == 0 || dataset.validation_count() == 0 {
            return Err(
                Error::Invalid("training and validation sets must both be nonempty").into(),
            );
        }
        dataset.check_rules(&base.rule_set)?;
        let calibration =
            dataset.calibration(args.teacher_cp_per_logit.as_ref(), base.cp_per_logit)?;
        let teacher_cp_per_logit = calibration.target_values();
        let observations = dataset.observations(args.feature_set)?;
        let model = Model::initialize(
            Shape::new(args.feature_set, args.width, base.cp_per_logit, args.head)?,
            &initial,
            Masks::from_observations(args.feature_set, &observations)?,
            args.seed,
            initial_standard_deviation(),
        )?;
        let selection = subsample::Selection::new(&mut dataset, args.train_subsample)?;
        let mut dataset_info = store::DatasetInfo::from(&dataset);
        dataset_info.subsample = selection.summary.as_ref();
        let mut config = configuration(
            &args,
            workers,
            dataset_info,
            &observations,
            &calibration,
            &model,
        )?;
        let mut checkpoint = Checkpoint {
            lambda: args.training.lambda,
            optimizer: AdamW::new(model.shape(), args.training.optimizer())?,
            model,
            progress: Progress {
                completed_superbatches: 0,
                elapsed_seconds: 0.0,
                seed: args.seed,
                config: crate::nn::checkpoint::Configuration::from_document(&config.document)?,
            },
        };
        let mut last_validation = None;
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
            let restored = Checkpoint::read(resume.join("latest.npz"))?;
            crate::nn::checkpoint::Configuration::from_document(&saved)?.check(
                &crate::nn::checkpoint::Configuration::from_document(&config.document)?,
            )?;
            restored.check_config(&crate::nn::checkpoint::Configuration::from_document(
                &config.document,
            )?)?;
            if restored.model.shape() != checkpoint.model.shape()
                || restored.model.masks() != checkpoint.model.masks()
                || restored.model.initial_linear() != checkpoint.model.initial_linear()
                || restored.model.initial_virtual() != checkpoint.model.initial_virtual()
                || restored.optimizer.config() != args.training.optimizer()
                || restored.lambda != args.training.lambda
                || restored.progress.seed != args.seed
                || restored.progress.completed_superbatches > args.superbatches.get()
                || restored
                    .progress
                    .completed_superbatches
                    .checked_mul(args.batches_per_superbatch.get())
                    != Some(restored.optimizer.step())
            {
                return Err(Error::Invalid(
                    "checkpoint state differs from recomputed initialization or progress",
                )
                .into());
            }
            checkpoint = restored;
            last_validation = Some(reconcile_log(
                &resume.join("training.jsonl"),
                checkpoint.progress.completed_superbatches,
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
            args.batch,
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
            dataset: Arc::new(dataset),
            selection,
            base,
            teacher_cp_per_logit,
            checkpoint,
            backend,
            started,
            previous_elapsed,
            last_validation,
            gpu_used_bytes: memory.map(|(used, _)| used),
            gpu_total_bytes: memory.map(|(_, total)| total),
        };
        if let Some(mut saved) = saved_config {
            for name in ["nnue.bin", "summary.json"] {
                match std::fs::remove_file(session.args.output_dir.join(name)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            append_resume(
                &session.args.output_dir.join("config.json"),
                &mut saved,
                &session.args,
                session.workers,
                session.checkpoint.progress.completed_superbatches,
                resumed_at,
            )?;
        }
        if session.last_validation.is_none() && !session.expired() {
            let validation_started = Instant::now();
            let model = session.checkpoint.model.clone();
            if let Some(validation) = session.validate(&model)? {
                session.last_validation = Some((0, validation.loss()));
                session.checkpoint.progress.elapsed_seconds = session.elapsed();
                let record = session.superbatch_record(
                    Some((&validation, validation_started.elapsed().as_secs_f64())),
                    None,
                )?;
                append_superbatch(&session.log_path(), &record)?;
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
        StoreValidation {
            store: &self.dataset,
            feature_set: self.args.feature_set,
            teacher_cp_per_logit: &self.teacher_cp_per_logit,
            batch: self.args.batch,
            deadline: self.deadline(),
        }
        .evaluate(
            &self.backend,
            model,
            &mut self.gpu_used_bytes,
            &mut self.gpu_total_bytes,
        )
    }

    /// 各周の読み手を維持し、superbatchごとに確定する。
    fn train(&mut self) -> Result<()> {
        let dataset = Arc::clone(&self.dataset);
        let mut epoch = None;
        while self.advance(&dataset, &mut epoch)? {}
        Ok(())
    }

    fn steps_per_epoch(&self) -> u64 {
        self.selection.positions.div_ceil(self.args.batch as u64)
    }

    fn advance<'a>(&mut self, dataset: &'a Store, epoch: &mut Option<Epoch<'a>>) -> Result<bool> {
        self.checkpoint.progress.elapsed_seconds = self.elapsed();
        if stop_reason(&self.checkpoint.progress, &self.args).is_some() {
            return Ok(false);
        }
        self.memory()?;
        let training_started = Instant::now();
        let deadline = self.deadline();
        let steps_per_epoch = self.steps_per_epoch();
        let mut remaining = self.args.batches_per_superbatch.get();
        let mut training = pipeline::Training::default();
        while remaining > 0 {
            if self.expired() {
                return Ok(false);
            }
            let offset = self.backend.step() % steps_per_epoch;
            let steps = remaining.min(steps_per_epoch - offset);
            if epoch.is_none() {
                let number = self.backend.step() / steps_per_epoch + 1;
                *epoch = Some(match dataset {
                    Store::Memory(dataset) => Epoch::Memory(epoch_permutation(
                        self.selection.indices(dataset),
                        self.args.seed,
                        number,
                    )),
                    Store::Spill(store) => {
                        let Some(mut reader) = store.epoch(self.args.seed, number, deadline)?
                        else {
                            return Ok(false);
                        };
                        // 再開時だけ同じ順列を作り直し、確定した更新の局面を読み飛ばす。
                        for _ in 0..offset {
                            if self.expired() {
                                return Ok(false);
                            }
                            if reader.next_chunk(self.args.batch)?.is_none() {
                                return Err(Error::Invalid(
                                    "checkpoint cursor exceeds training epoch",
                                )
                                .into());
                            }
                        }
                        Epoch::Spill(reader)
                    }
                });
            }
            let options = pipeline::BatchOptions {
                feature_set: self.args.feature_set,
                batch: self.args.batch,
                schedule: self.args.lr_schedule,
                batches_per_superbatch: self.args.batches_per_superbatch,
                steps,
                deadline,
            };
            let part = match (dataset, epoch.as_mut()) {
                (Store::Memory(dataset), Some(Epoch::Memory(indices))) => {
                    let start = usize::try_from(offset * self.args.batch as u64)?;
                    let end = if offset + steps == steps_per_epoch {
                        indices.len()
                    } else {
                        usize::try_from((offset + steps) * self.args.batch as u64)?
                    };
                    pipeline::train_batches(
                        pipeline::Supply {
                            dataset,
                            indices: &indices[start..end],
                            teacher_cp_per_logit: &self.teacher_cp_per_logit,
                            workers: self.workers,
                            options,
                        },
                        &mut self.backend,
                    )?
                }
                (Store::Spill(store), Some(Epoch::Spill(reader))) => pipeline::train_spill_batches(
                    store,
                    reader,
                    self.workers,
                    &self.teacher_cp_per_logit,
                    options,
                    &mut self.backend,
                )?,
                _ => return Err(Error::Invalid("training cursor does not match its store").into()),
            };
            let Some(part) = part else {
                return Ok(false);
            };
            training.append(part)?;
            remaining -= steps;
            if offset + steps == steps_per_epoch {
                *epoch = None;
            }
        }
        self.gpu_used_bytes = self.gpu_used_bytes.max(training.gpu_used_bytes);
        self.gpu_total_bytes = training.gpu_total_bytes;
        let (parameters, optimizer) = self.backend.snapshot()?;
        parameters.validate()?;
        let mut candidate = Checkpoint {
            model: self.checkpoint.model.clone(),
            optimizer,
            lambda: self.checkpoint.lambda,
            progress: self.checkpoint.progress.clone(),
        };
        for parameter in Parameter::ALL {
            candidate
                .model
                .parameter_mut(parameter)
                .copy_from_slice(&parameters[parameter]);
        }
        candidate.progress.completed_superbatches += 1;
        let training_seconds = training_started.elapsed().as_secs_f64();
        let k = candidate.progress.completed_superbatches;
        let validation = if k.is_multiple_of(self.args.validation_superbatches.get())
            || k == self.args.superbatches.get()
        {
            let started = Instant::now();
            let Some(validation) = self.validate(&candidate.model)? else {
                return Ok(false);
            };
            Some((validation, started.elapsed().as_secs_f64()))
        } else {
            None
        };
        if self.expired() {
            return Ok(false);
        }
        candidate.progress.elapsed_seconds = self.elapsed();
        let previous = std::mem::replace(&mut self.checkpoint, candidate);
        let saved = (|| {
            let record = self.superbatch_record(
                validation.as_ref().map(|(v, seconds)| (v, *seconds)),
                Some((&training, training_seconds)),
            )?;
            append_superbatch(&self.log_path(), &record)?;
            self.save_latest()
        })();
        if let Err(error) = saved {
            self.checkpoint = previous;
            return Err(error);
        }
        if let Some((validation, _)) = &validation {
            self.last_validation = Some((k, validation.loss()));
        }
        eprintln!(
            "superbatch {k}: updates={} train_loss={:.9} validation_loss={:?}",
            self.checkpoint.optimizer.step(),
            training.loss / training.positions as f64,
            validation.as_ref().map(|(v, _)| v.loss())
        );
        Ok(true)
    }

    fn superbatch_record(
        &self,
        validation: Option<(&Validation, f64)>,
        training: Option<(&pipeline::Training, f64)>,
    ) -> Result<SuperbatchRecord> {
        let mut record = SuperbatchRecord {
            device: self.args.device,
            superbatch: self.checkpoint.progress.completed_superbatches,
            epochs: self.checkpoint.optimizer.step() as f64 / self.steps_per_epoch() as f64,
            train_loss: training.map(|(t, _)| t.loss / t.positions as f64),
            validation_loss: None,
            sign_accuracy: None,
            samples: None,
            decisive_samples: None,
            nonlinear_std_cp: None,
            saturation: None,
            hidden1_saturation: None,
            hidden2_saturation: None,
            pairwise_both_positive: None,
            validation_seconds: None,
            first_layer_at_limit: self.checkpoint.model.first_layer_at_limit()?,
            linear_movement: linear_movement(&self.checkpoint.model),
            training_seconds: training.map(|(_, seconds)| seconds),
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
        };
        if let Some((validation, seconds)) = validation {
            record.set_validation(validation, seconds);
        }
        Ok(record)
    }

    fn finish(mut self) -> Result<Summary> {
        self.checkpoint.progress.elapsed_seconds = self.elapsed();
        let mut stopped_by = stop_reason(&self.checkpoint.progress, &self.args).ok_or(
            Error::Invalid("training stopped without a stopping condition"),
        )?;
        let completed = self.checkpoint.progress.completed_superbatches;
        // Nを保存済みkまで短縮した再開では、kの検証がまだ行われていないことがある。
        if stopped_by == StopReason::Completed
            && self.last_validation.map(|(k, _)| k) != Some(completed)
        {
            let started = Instant::now();
            let model = self.checkpoint.model.clone();
            if let Some(validation) = self.validate(&model)? {
                record::complete_final_validation(
                    &self.log_path(),
                    &validation,
                    started.elapsed().as_secs_f64(),
                    self.elapsed(),
                )?;
                self.last_validation = Some((completed, validation.loss()));
            } else {
                stopped_by = StopReason::TimeLimit;
            }
        }
        self.checkpoint.progress.elapsed_seconds = self.elapsed();
        // 未確定の更新は採用せず、費やした時間だけを保存する。
        if self.last_validation.is_some() {
            self.save_latest()?;
        }
        let mut summary = Summary {
            status: "error",
            reason: None,
            stopped_by,
            planned_superbatches: self.args.superbatches.get(),
            completed_superbatches: self.checkpoint.progress.completed_superbatches,
            planned_steps: self.args.planned_steps()?,
            completed_steps: self.checkpoint.optimizer.step(),
            epochs: self.checkpoint.optimizer.step() as f64 / self.steps_per_epoch() as f64,
            final_validation_loss: self.last_validation.map(|(_, loss)| loss),
            elapsed_seconds: self.checkpoint.progress.elapsed_seconds,
            quantization: None,
            removal_sign: None,
        };
        if stopped_by == StopReason::TimeLimit {
            summary.status = "incomplete";
            eprintln!(
                "superbatches: {completed}; updates: {}; status: incomplete",
                self.checkpoint.optimizer.step()
            );
            summary.reason = Some(Error::Incomplete.to_string());
            write_json(&self.args.output_dir.join("summary.json"), &summary)?;
            return Err(Error::Incomplete.into());
        }
        let checkpoint = Checkpoint::read(self.args.output_dir.join("latest.npz"))?;
        let source = match self.dataset.as_ref() {
            Store::Memory(dataset) => export::ValidationSource::Memory(dataset),
            Store::Spill(store) => export::ValidationSource::Spill(store),
        };
        let exported = export::write_completed(
            source,
            &checkpoint,
            &self.args.output_dir,
            &self.base.piece_values,
            &self.base.rule_set,
            self.args.validation_sample,
        );
        let (quantization, removal_sign) = match exported {
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
        summary.removal_sign = Some(removal_sign);
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

/// 供給元に応じて検証バッチを読み、共通の順序で集計する。
struct StoreValidation<'a> {
    store: &'a Store,
    feature_set: InputFeatureSet,
    teacher_cp_per_logit: &'a [f64],
    batch: usize,
    deadline: Instant,
}
impl StoreValidation<'_> {
    /// 各バッチの前後で期限を確認し、時間切れなら未完了として返す。
    fn evaluate(
        &self,
        backend: &Backend,
        model: &Model,
        gpu_used_bytes: &mut Option<usize>,
        gpu_total_bytes: &mut Option<usize>,
    ) -> Result<Option<Validation>> {
        match self.store {
            Store::Memory(dataset) => ValidationSet {
                dataset,
                feature_set: self.feature_set,
                indices: dataset.validation_indices(),
                teacher_cp_per_logit: self.teacher_cp_per_logit,
                batch: self.batch,
                deadline: self.deadline,
            }
            .evaluate(backend, model, gpu_used_bytes, gpu_total_bytes),
            Store::Spill(store) => {
                if Instant::now() >= self.deadline {
                    return Ok(None);
                }
                let mut evaluator = backend.evaluator(model, self.batch)?;
                if let Some((used, total)) = backend.memory()? {
                    *gpu_used_bytes = (*gpu_used_bytes).max(Some(used));
                    *gpu_total_bytes = Some(total);
                }
                validate_batches(
                    store.validation_chunks(self.batch),
                    |records| {
                        let batch = BatchSlot::from_spill_records(self.feature_set, &records?)?;
                        let values = build_targets(
                            &batch,
                            self.teacher_cp_per_logit,
                            store.teacher_lambdas(),
                        )?;
                        evaluator.validate(&batch, &values)
                    },
                    || Instant::now() >= self.deadline,
                )
            }
        }
    }
}

/// 全検証集合への借用と評価条件。
struct ValidationSet<'a> {
    dataset: &'a Dataset,
    feature_set: InputFeatureSet,
    indices: &'a [u64],
    teacher_cp_per_logit: &'a [f64],
    batch: usize,
    deadline: Instant,
}

impl ValidationSet<'_> {
    /// 学習状態を共有借用して評価器を作り、各検証バッチの前後に同じ期限を検査する。
    /// 時間切れなら`None`を返し、呼び手は検証結果を記録しない。
    fn evaluate(
        &self,
        backend: &Backend,
        model: &Model,
        gpu_used_bytes: &mut Option<usize>,
        gpu_total_bytes: &mut Option<usize>,
    ) -> Result<Option<Validation>> {
        if Instant::now() >= self.deadline {
            return Ok(None);
        }
        let mut evaluator = backend.evaluator(model, self.batch)?;
        if let Some((used, total)) = backend.memory()? {
            *gpu_used_bytes = (*gpu_used_bytes).max(Some(used));
            *gpu_total_bytes = Some(total);
        }
        validate_batches(
            self.indices.chunks(self.batch),
            |indices| {
                let batch = build_batches(self.dataset, self.feature_set, &[indices])?.remove(0);
                let values = build_targets(
                    &batch,
                    self.teacher_cp_per_logit,
                    self.dataset.teacher_lambdas(),
                )?;
                evaluator.validate(&batch, &values)
            },
            || Instant::now() >= self.deadline,
        )
    }
}

/// 全検証バッチを集計し、各バッチの前後に期限を検査する。
/// 最終バッチの後も検査し、期限を過ぎたら`None`を返す。
/// 呼び手はそのsuperbatchのログ追記とチェックポイント保存を行わない。
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
            total.hidden1_lower += statistics.hidden1_lower;
            total.hidden2_lower += statistics.hidden2_lower;
            total.pairwise_positive += statistics.pairwise_positive;
            total.pairwise_products += statistics.pairwise_products;
            total.hidden1_upper += statistics.hidden1_upper;
            total.hidden2_upper += statistics.hidden2_upper;
            total.hidden1_activations += statistics.hidden1_activations;
            total.hidden2_activations += statistics.hidden2_activations;
        } else {
            total = Some(statistics);
        }
        // 最終バッチの後も期限を確認し、検証完了が期限を越えたsuperbatchを採用しない。
        if expired() {
            return Ok(None);
        }
    }
    let total = total.ok_or(Error::Invalid("empty validation set"))?;
    validate_statistics(&total)?;
    Ok(Some(total))
}

/// 指定デバイスで初期検証、予定回数の更新、最終の重みの書き出しを行う。
/// 学習ループ中のエラーでは、最後に確定したsuperbatch数と更新回数を報告する。
pub fn run(args: Args) -> Result<Summary> {
    let mut session = Session::new(args)?;
    if let Err(error) = session.train() {
        write_json(
            &session.args.output_dir.join("summary.json"),
            &json!({
                "status": "error", "reason": error.to_string(),
                "completed_superbatches": session.checkpoint.progress.completed_superbatches,
                "completed_steps": session.checkpoint.optimizer.step(), "elapsed_seconds": session.elapsed(),
            }),
        )?;
        return Err(error);
    }
    session.finish()
}
