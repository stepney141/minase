//! CUDAまたはCPUで、指定したsuperbatch数だけNNUEを学習する。
//!
//! 1 superbatchは`--batches-per-superbatch`回の更新であり、教師の1周とは独立した単位である。
//! 間引き後の訓練局面数をM、`--batch`の局面数をmとすると、1周の更新回数は`U = ceil(M/m)`となる。
//! 周ごとに訓練集合の全体を一様に並べ替える。
//! superbatchの境界は周の途中に来てよく、同じ周の順列を境界を越えて保持する。
//! 周の最後の短いバッチも1更新と数え、次の周の局面で埋めない。
//! 学習率の倍率は、`--lr-schedule`、完了した更新回数、`--batches-per-superbatch`だけで決まる。
//!
//! 新規実行では学習前に全検証集合を評価し、更新0回の行を`training.jsonl`に記録する。
//! 予定のsuperbatch数をN、`--validation-superbatches`の間隔をVとすると、k番目のsuperbatchの終了時に、kがVで割り切れるか`k = N`なら全検証集合を評価する。
//! 検証しなかったsuperbatchの検証損失は`null`とし、検証損失は停止や重みの選択には用いない。
//! superbatchの終了時は計算結果を回収して重みと最適化器の状態を取得し、必要な検証、ログの追記、`latest.npz`の保存の順に進む。
//! この保存を確定の時点とし、先読みしただけのバッチと、検証を含めて時間内に完了しなかったsuperbatchの更新は採用しない。
//!
//! | ファイル | 内容 |
//! | --- | --- |
//! | config.json | 引数、照合用の設定と入力、前処理、デバイス名、再開履歴。 |
//! | training.jsonl | 学習前の検証と、完了したsuperbatchごとの[`SuperbatchRecord`]。 |
//! | latest.npz | 確定した重み、最適化器の状態、superbatch数、更新数、累積時間。 |
//! | nnue.bin | 予定の更新を終えたlatest.npzを量子化したtatara形式。 |
//! | summary.json | 予定と完了のsuperbatch数と更新数、停止理由、検証と量子化の結果。 |
//!
//! 新規実行の出力先は存在しないか空でなければならない。
//! 累積時間には前処理と検証、再開前の実行を含む。
//! 時間の上限で予定の更新を完了できなければ、`summary.json`の状態を`incomplete`とし、CLIは終了コード1を返す。
//! この場合は`nnue.bin`を書かず、確定済みの`latest.npz`があれば残して再開に用いる。
//! 学習前の検証も完了していなければ、チェックポイントは作らない。

mod args;
pub(crate) mod backend;
mod pipeline;
mod record;
mod store;
mod subsample;
use crate::{
    data::{Dataset, epoch_permutation},
    nnue::data::{BatchSlot, FeatureSet, build_batches, build_targets},
};
pub use args::Args;
pub(crate) use backend::{TrainerBackend, Validation};
pub use record::SuperbatchRecord;
use record::{append_resume, append_superbatch, configuration, reconcile_log, write_json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fmt,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use store::Store;

/// 学習ループの確定済み進捗。構造固有の配列を含まない。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    /// 保存を確定したsuperbatch数。
    pub completed_superbatches: u64,
    /// 保存を確定した更新数。
    pub completed_steps: u64,
    /// 再開前を含む累積秒数。
    pub elapsed_seconds: f64,
    /// 再開時に照合する設定、入力と前処理。
    pub config: crate::nn::checkpoint::Configuration,
}
impl Progress {
    pub(crate) fn validate(&self) -> Result<()> {
        if !self.elapsed_seconds.is_finite() || self.elapsed_seconds < 0.0 {
            return Err(Error::Invalid("invalid checkpoint elapsed time").into());
        }
        Ok(())
    }
}

/// 学習を継続できない引数または停止条件。
#[derive(Debug)]
pub enum Error {
    /// 引数または保存状態が契約に違反する。
    Invalid(&'static str),
    /// 予定した更新回数に達する前に時間上限へ達した。
    Incomplete,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => f.write_str(reason),
            Self::Incomplete => f.write_str(
                "time limit reached before completing the planned training; resume to continue",
            ),
        }
    }
}
impl std::error::Error for Error {}
/// 原因の型を保持する学習の処理結果。
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// 学習を停止した理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// 予定のsuperbatch数を完了した。
    Completed,
    /// 再開前を含む時間の上限に達した。
    TimeLimit,
}

impl fmt::Display for StopReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Completed => "completed",
            Self::TimeLimit => "time_limit",
        })
    }
}

/// 予定と確定済みの進捗、および最終の検査結果。
/// candidateだけが終了コード0であり、それ以外は1で終了する。
#[derive(Debug, Serialize)]
pub struct Summary {
    /// candidate、incomplete、errorのいずれか。
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
    pub quantization: Option<Value>,
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

struct Session<B> {
    args: Args,
    workers: usize,
    dataset: Arc<Store>,
    selection: subsample::Selection,
    teacher_cp_per_logit: Vec<f64>,
    progress: Progress,
    backend: B,
    started: Instant,
    previous_elapsed: f64,
    last_validation: Option<(u64, f64)>,
    gpu_used_bytes: Option<usize>,
    gpu_total_bytes: Option<usize>,
}

impl<B: TrainerBackend> Session<B> {
    fn new(
        args: Args,
        architecture: Value,
        initialize: impl FnOnce(&Args, Progress) -> Result<(B, Progress)>,
    ) -> Result<Self> {
        let started = Instant::now();
        let resumed_at = SystemTime::now();
        args.validate()?;
        let workers = args.workers()?;
        let mut dataset = Store::open(&args, workers)?;
        if dataset.training_count() == 0 || dataset.validation_count() == 0 {
            return Err(
                Error::Invalid("training and validation sets must both be nonempty").into(),
            );
        }
        dataset.check_rules(args.parsed_rules()?)?;
        let selection = subsample::Selection::new(&mut dataset, args.train_subsample)?;
        let mut dataset_info = store::DatasetInfo::from(&dataset);
        dataset_info.subsample = selection.summary.as_ref();
        let teacher_cp_per_logit = vec![args.teacher_cp_per_logit; dataset_info.classes.len()];
        let mut config = configuration(&args, workers, dataset_info, architecture)?;
        let expected = crate::nn::checkpoint::Configuration::from_document(&config)?;
        let initial_progress = Progress {
            completed_superbatches: 0,
            completed_steps: 0,
            elapsed_seconds: 0.0,
            config: expected.clone(),
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
            crate::nn::checkpoint::Configuration::from_document(&saved)?.check(&expected)?;
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
        let (backend, progress) = initialize(&args, initial_progress)?;
        progress.validate()?;
        progress.config.check(&expected)?;
        if progress.completed_superbatches > args.superbatches.get()
            || progress
                .completed_superbatches
                .checked_mul(args.batches_per_superbatch.get())
                != Some(progress.completed_steps)
            || backend.step() != progress.completed_steps
        {
            return Err(
                Error::Invalid("checkpoint progress differs from requested updates").into(),
            );
        }
        let name = backend.name();
        if let Some(saved) = &saved_config {
            if saved.get("device_name") != Some(&json!(name)) {
                return Err(
                    Error::Invalid("resume device differs from the recorded device").into(),
                );
            }
        } else {
            std::fs::create_dir_all(&args.output_dir)?;
            config["device_name"] = json!(name);
            let memory = backend.memory()?;
            config["gpu_memory_used_bytes"] = json!(memory.map(|(used, _)| used));
            config["gpu_memory_total_bytes"] = json!(memory.map(|(_, total)| total));
            config["preprocessing_seconds"] = json!(started.elapsed().as_secs_f64());
            write_json(&args.output_dir.join("config.json"), &config)?;
        }
        if let Some(resume) = &args.resume {
            last_validation = Some(reconcile_log(
                &resume.join("training.jsonl"),
                progress.completed_superbatches,
            )?);
        }
        let previous_elapsed = progress.elapsed_seconds;
        let memory = backend.memory()?;
        let mut session = Self {
            args,
            workers,
            dataset: Arc::new(dataset),
            selection,
            teacher_cp_per_logit,
            progress,
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
                session.progress.completed_superbatches,
                resumed_at,
            )?;
        }
        if session.last_validation.is_none() && !session.expired() {
            let validation_started = Instant::now();
            if let Some(validation) = session.validate()? {
                session.last_validation = Some((0, validation.loss()));
                session.progress.elapsed_seconds = session.elapsed();
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
    fn save_latest(&mut self) -> Result<()> {
        self.backend.flush_pending_loss()?;
        self.backend
            .save_checkpoint(&self.args.output_dir.join("latest.npz"), &self.progress)
    }
    fn memory(&mut self) -> Result<()> {
        if let Some((used, total)) = self.backend.memory()? {
            self.gpu_used_bytes = self.gpu_used_bytes.max(Some(used));
            self.gpu_total_bytes = Some(total);
        }
        Ok(())
    }

    fn validate(&mut self) -> Result<Option<Validation>> {
        self.backend.flush_pending_loss()?;
        let request = StoreValidation {
            store: &self.dataset,
            feature_set: self.backend.feature_set(),
            teacher_cp_per_logit: &self.teacher_cp_per_logit,
            batch: self.args.batch,
            deadline: self.deadline(),
        };
        let validation = request.evaluate(
            &mut self.backend,
            &mut self.gpu_used_bytes,
            &mut self.gpu_total_bytes,
        )?;
        self.memory()?;
        Ok(validation)
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
        self.progress.elapsed_seconds = self.elapsed();
        if stop_reason(&self.progress, &self.args).is_some() {
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
                feature_set: self.backend.feature_set(),
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
        self.backend.flush_pending_loss()?;
        let mut candidate = self.progress.clone();
        candidate.completed_superbatches += 1;
        candidate.completed_steps = self.backend.step();
        let training_seconds = training_started.elapsed().as_secs_f64();
        let k = candidate.completed_superbatches;
        let validation = if k.is_multiple_of(self.args.validation_superbatches.get())
            || k == self.args.superbatches.get()
        {
            let started = Instant::now();
            let Some(validation) = self.validate()? else {
                return Ok(false);
            };
            Some((validation, started.elapsed().as_secs_f64()))
        } else {
            None
        };
        if self.expired() {
            return Ok(false);
        }
        candidate.elapsed_seconds = self.elapsed();
        let previous = std::mem::replace(&mut self.progress, candidate);
        let saved = (|| {
            let record = self.superbatch_record(
                validation.as_ref().map(|(v, seconds)| (v, *seconds)),
                Some((&training, training_seconds)),
            )?;
            append_superbatch(&self.log_path(), &record)?;
            self.save_latest()
        })();
        if let Err(error) = saved {
            self.progress = previous;
            return Err(error);
        }
        if let Some((validation, _)) = &validation {
            self.last_validation = Some((k, validation.loss()));
        }
        let validation_loss = match &validation {
            Some((validation, _)) => format!("{:.9}", validation.loss()),
            None => "-".to_owned(),
        };
        eprintln!(
            "superbatch {k}: updates={} train_loss={:.9} validation_loss={validation_loss}",
            self.progress.completed_steps,
            training.loss / training.positions as f64,
        );
        Ok(true)
    }

    fn superbatch_record(
        &mut self,
        validation: Option<(&Validation, f64)>,
        training: Option<(&pipeline::Training, f64)>,
    ) -> Result<SuperbatchRecord> {
        Ok(SuperbatchRecord {
            device: self.args.device,
            superbatch: self.progress.completed_superbatches,
            epochs: self.progress.completed_steps as f64 / self.steps_per_epoch() as f64,
            train_loss: training.map(|(t, _)| t.loss / t.positions as f64),
            validation_loss: validation.map(|(v, _)| v.loss()),
            statistics: self.backend.statistics(validation.map(|(v, _)| v))?,
            validation_seconds: validation.map(|(_, seconds)| seconds),
            training_seconds: training.map(|(_, seconds)| seconds),
            training_positions_per_second: training
                .map(|(t, seconds)| t.positions as f64 / seconds),
            update_seconds: training.map(|(t, _)| t.compute_seconds / t.steps as f64),
            gpu_memory_used_bytes: self.gpu_used_bytes,
            gpu_memory_total_bytes: self.gpu_total_bytes,
            elapsed_seconds: self.progress.elapsed_seconds,
            steps: training.map_or(0, |(t, _)| t.steps),
            total_steps: self.progress.completed_steps,
            lr_multiplier_first: training.and_then(|(t, _)| t.first_multiplier),
            lr_multiplier_last: training.and_then(|(t, _)| t.last_multiplier),
        })
    }

    fn finish(mut self) -> Result<Summary> {
        self.backend.flush_pending_loss()?;
        self.progress.elapsed_seconds = self.elapsed();
        let mut stopped_by = stop_reason(&self.progress, &self.args).ok_or(Error::Invalid(
            "training stopped without a stopping condition",
        ))?;
        let completed = self.progress.completed_superbatches;
        // Nを保存済みkまで短縮した再開では、kの検証がまだ行われていないことがある。
        if stopped_by == StopReason::Completed
            && self.last_validation.map(|(k, _)| k) != Some(completed)
        {
            let started = Instant::now();
            if let Some(validation) = self.validate()? {
                record::complete_final_validation(
                    &self.log_path(),
                    &validation,
                    self.backend.statistics(Some(&validation))?,
                    started.elapsed().as_secs_f64(),
                    self.elapsed(),
                )?;
                self.last_validation = Some((completed, validation.loss()));
            } else {
                stopped_by = StopReason::TimeLimit;
            }
        }
        self.progress.elapsed_seconds = self.elapsed();
        // 未確定の更新は採用せず、費やした時間だけを保存する。
        if self.last_validation.is_some() {
            self.save_latest()?;
        }
        let mut summary = Summary {
            status: "error",
            reason: None,
            stopped_by,
            planned_superbatches: self.args.superbatches.get(),
            completed_superbatches: self.progress.completed_superbatches,
            planned_steps: self.args.planned_steps()?,
            completed_steps: self.progress.completed_steps,
            epochs: self.progress.completed_steps as f64 / self.steps_per_epoch() as f64,
            final_validation_loss: self.last_validation.map(|(_, loss)| loss),
            elapsed_seconds: self.progress.elapsed_seconds,
            quantization: None,
        };
        if stopped_by == StopReason::TimeLimit {
            summary.status = "incomplete";
            eprintln!(
                "superbatches: {completed}; updates: {}; stopped_by: {stopped_by}; status: incomplete",
                self.progress.completed_steps
            );
            summary.reason = Some(Error::Incomplete.to_string());
            write_json(&self.args.output_dir.join("summary.json"), &summary)?;
            return Err(Error::Incomplete.into());
        }
        let samples = self
            .dataset
            .validation_sample(self.args.validation_sample, self.args.seed)?;
        match self
            .backend
            .export(&self.args.output_dir.join("nnue.bin"), &samples)
        {
            Ok(report) => {
                summary.status = "candidate";
                summary.quantization = Some(report);
            }
            Err(error) => {
                summary.reason = Some(error.to_string());
                write_json(&self.args.output_dir.join("summary.json"), &summary)?;
                return Err(error);
            }
        }
        write_json(&self.args.output_dir.join("summary.json"), &summary)?;
        Ok(summary)
    }
}

/// 供給元に応じて検証バッチを読み、共通の順序で集計する。
struct StoreValidation<'a> {
    store: &'a Store,
    feature_set: FeatureSet,
    teacher_cp_per_logit: &'a [f64],
    batch: usize,
    deadline: Instant,
}
impl StoreValidation<'_> {
    /// 各バッチの前後で期限を確認し、時間切れなら未完了として返す。
    fn evaluate(
        &self,
        backend: &mut impl TrainerBackend,
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
            .evaluate(backend, gpu_used_bytes, gpu_total_bytes),
            Store::Spill(store) => {
                if Instant::now() >= self.deadline {
                    return Ok(None);
                }
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
                        backend.validate_step(&batch, &values)
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
    feature_set: FeatureSet,
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
        backend: &mut impl TrainerBackend,
        gpu_used_bytes: &mut Option<usize>,
        gpu_total_bytes: &mut Option<usize>,
    ) -> Result<Option<Validation>> {
        if Instant::now() >= self.deadline {
            return Ok(None);
        }
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
                backend.validate_step(&batch, &values)
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
        statistics.validate()?;
        if let Some(total) = &mut total {
            total.append(statistics);
        } else {
            total = Some(statistics);
        }
        // 最終バッチの後も期限を確認し、検証完了が期限を越えたsuperbatchを採用しない。
        if expired() {
            return Ok(None);
        }
    }
    let total = total.ok_or(Error::Invalid("empty validation set"))?;
    total.validate()?;
    Ok(Some(total))
}

/// 指定デバイスで初期検証、予定回数の更新、最終の重みの書き出しを行う。
/// 学習ループ中のエラーでは、最後に確定したsuperbatch数と更新回数を報告する。
pub(crate) fn run_with<B: TrainerBackend>(
    args: Args,
    architecture: Value,
    initialize: impl FnOnce(&Args, Progress) -> Result<(B, Progress)>,
) -> Result<Summary> {
    let mut session = Session::new(args, architecture, initialize)?;
    if let Err(error) = session.train() {
        write_json(
            &session.args.output_dir.join("summary.json"),
            &json!({
                "status": "error", "reason": error.to_string(),
                "completed_superbatches": session.progress.completed_superbatches,
                "completed_steps": session.progress.completed_steps, "elapsed_seconds": session.elapsed(),
            }),
        )?;
        return Err(error);
    }
    session.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation_aggregates_weighted_losses_and_checks_the_final_deadline() {
        let total = validate_batches(
            [2, 1],
            |n| {
                Ok(Validation {
                    loss_sum: n as f64 * 0.25,
                    positions: n,
                    sums: Default::default(),
                })
            },
            || false,
        )
        .unwrap()
        .unwrap();
        assert_eq!(total.positions, 3);
        assert_eq!(total.loss(), 0.25);
        let expired = std::cell::Cell::new(false);
        assert!(
            validate_batches(
                [1],
                |_| {
                    expired.set(true);
                    Ok(Validation {
                        loss_sum: 0.0,
                        positions: 1,
                        sums: Default::default(),
                    })
                },
                || expired.get()
            )
            .unwrap()
            .is_none()
        );
        assert!(
            validate_batches(
                [1],
                |_| Ok(Validation {
                    loss_sum: f64::NAN,
                    positions: 1,
                    sums: Default::default()
                }),
                || false
            )
            .is_err()
        );
    }
}
