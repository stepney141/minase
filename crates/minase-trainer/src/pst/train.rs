use super::{
    Adam, Args, Batch, Checkpoint, EpochRecord, Error, Model, Progress, RemovalReference,
    RemovalReferenceKind, Result, StopReason, Summary,
    base::{self, Base},
    compute, export,
    record::{append_epoch, configuration, reconcile_log, write_json},
};
use crate::{
    data::{Dataset, build_targets_from_arrays, epoch_permutation},
    nn::{atomic_write, patience_reached, pipeline, update_best, workers},
};
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

fn initial_model(base: &Base) -> Result<Model> {
    let initial: Vec<_> = base
        .linear
        .iter()
        .map(|row| row.map(|v| f32::from(v) / 8.0))
        .collect();
    Model::initialize(&initial, base.k)
}

struct Session {
    args: Args,
    workers: usize,
    dataset: Dataset,
    base: Base,
    ks: Vec<f64>,
    reference: Option<RemovalReference>,
    checkpoint: Checkpoint,
    started: Instant,
    previous_elapsed: f64,
    initial_validation_loss: f64,
}

impl Session {
    fn new(args: Args) -> Result<Self> {
        let started = Instant::now();
        args.validate()?;
        let workers = workers::workers(args.threads)?;
        let base = base::read(&args.init)?;
        let model = initial_model(&base)?;
        let reference = if args.removal_penalty == 0.0 {
            None
        } else {
            Some(match args.removal_reference {
                RemovalReferenceKind::V0 => RemovalReference::v0(),
                RemovalReferenceKind::Init => RemovalReference::new(&base.linear)?,
            })
        };
        // SAFETY: --dataは実行中に変更・切り詰めないというCLIの契約に従う。
        // このセッションもDatasetを破棄するまで入力を書き換えない。
        let dataset =
            unsafe { Dataset::open(&args.data, args.lookahead()?, args.lambda_override)? };
        if dataset.training_indices().is_empty() || dataset.validation_indices().is_empty() {
            return Err(Error::Invalid(
                "training and validation sets must both be nonempty",
            ));
        }
        if dataset
            .headers()
            .any(|header| header.rule_set() != base.rule_set)
        {
            return Err(Error::Invalid("MNSD rule set differs from initial MNPT"));
        }
        let ks = dataset.estimate_teacher_ks()?;
        let mut config = configuration(&args, workers, &dataset, &ks, base.k)?;
        let mut checkpoint = Checkpoint {
            optimizer: Adam::new(args.lr)?,
            model,
            progress: Progress {
                epoch: 0,
                best: None,
                elapsed_seconds: 0.0,
                seed: args.seed,
                config_hash: config.config_hash,
                input_hash: config.input_hash,
            },
        };
        let mut initial_loss = None;
        if let Some(resume) = &args.resume {
            if resume.canonicalize()? != args.output_dir.canonicalize()? {
                return Err(Error::Invalid(
                    "--resume and --output-dir must identify the same directory",
                ));
            }
            let mut saved: Value =
                serde_json::from_slice(&std::fs::read(resume.join("config.json"))?)?;
            let comparable: Value = serde_json::from_slice(&serde_json::to_vec(&config.document)?)?;
            for field in [
                "settings",
                "inputs",
                "preprocessing",
                "config_sha256",
                "input_sha256",
            ] {
                if saved.get(field) != comparable.get(field) {
                    return Err(Error::Invalid(
                        "resume configuration or input hash mismatch",
                    ));
                }
            }
            let restored = Checkpoint::read(resume.join("latest.bin"))?;
            restored
                .progress
                .check_hashes(&config.config_hash, &config.input_hash)?;
            if restored.model.k().to_bits() != base.k.to_bits()
                || restored.optimizer.lr() != args.lr
                || restored.progress.seed != args.seed
                || restored.progress.epoch > args.epochs
                || restored.optimizer.step()
                    != restored.progress.epoch
                        * dataset.training_indices().len().div_ceil(args.batch) as u64
            {
                return Err(Error::Invalid(
                    "checkpoint state differs from configuration or progress",
                ));
            }
            checkpoint = restored;
            if checkpoint.progress.best.is_some() {
                recover_best(&args.output_dir, &checkpoint)?;
            }
            initial_loss = Some(reconcile_log(
                &resume.join("training.jsonl"),
                checkpoint.progress.epoch,
            )?);
            saved.get_mut("resumes").and_then(Value::as_array_mut).ok_or(Error::Invalid("configuration resumes must be an array"))?.push(json!({
                "resumed_at_unix_seconds":SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                "epochs":args.epochs, "patience":args.patience, "workers":workers, "epoch":checkpoint.progress.epoch}));
            write_json(&args.output_dir.join("config.json"), &saved)?;
        } else {
            if args.output_dir.exists() && std::fs::read_dir(&args.output_dir)?.next().is_some() {
                return Err(Error::Invalid(
                    "output directory is not empty; use --resume to continue a run",
                ));
            }
            std::fs::create_dir_all(&args.output_dir)?;
            config.document["preprocessing_seconds"] = json!(started.elapsed().as_secs_f64());
            write_json(&args.output_dir.join("config.json"), &config.document)?;
        }
        let previous_elapsed = checkpoint.progress.elapsed_seconds;
        let mut session = Self {
            args,
            workers,
            dataset,
            base,
            ks,
            reference,
            checkpoint,
            started,
            previous_elapsed,
            initial_validation_loss: 0.0,
        };
        if let Some(loss) = initial_loss {
            session.initial_validation_loss = loss;
        } else {
            let start = Instant::now();
            let loss = session.validate()?;
            session.initial_validation_loss = loss;
            session.checkpoint.progress.elapsed_seconds = session.elapsed();
            let record = session.epoch_record(loss, None, start.elapsed().as_secs_f64());
            append_epoch(&session.args.output_dir.join("training.jsonl"), &record)?;
            session.save_latest()?;
        }
        Ok(session)
    }

    fn elapsed(&self) -> f64 {
        self.previous_elapsed + self.started.elapsed().as_secs_f64()
    }
    fn save_latest(&self) -> Result<()> {
        Ok(atomic_write(
            &self.args.output_dir.join("latest.bin"),
            &self.checkpoint.encode()?,
        )?)
    }

    fn validate(&self) -> Result<f64> {
        let mut total = 0.0;
        let chunks: Vec<_> = self
            .dataset
            .validation_indices()
            .chunks(self.args.batch)
            .collect();
        pipeline::ordered::<_, Error>(
            chunks.len(),
            self.workers,
            |index| {
                let batch = Batch::from_indices(&self.dataset, chunks[index])?;
                let targets = targets(&batch, &self.ks, self.dataset.teacher_lambdas())?;
                compute::validation_sum(&self.checkpoint.model, &batch, &targets)
            },
            |_, loss| {
                total += loss;
                Ok(())
            },
        )?;
        let loss = total / self.dataset.validation_indices().len() as f64;
        if !loss.is_finite() || loss < 0.0 {
            return Err(Error::Invalid("non-finite or negative validation loss"));
        }
        Ok(loss)
    }

    fn train_epoch(&mut self, indices: &[u64]) -> Result<Training> {
        let started = Instant::now();
        let mut training = Training::default();
        let chunks: Vec<_> = indices.chunks(self.args.batch).collect();
        let dataset = &self.dataset;
        let ks = &self.ks;
        pipeline::ordered::<_, Error>(
            chunks.len(),
            self.workers,
            |index| {
                let batch = Batch::from_indices(dataset, chunks[index])?;
                let targets = targets(&batch, ks, dataset.teacher_lambdas())?;
                Ok((batch, targets))
            },
            |_, (batch, targets)| {
                let update = Instant::now();
                let gradients = compute::gradients(
                    &self.checkpoint.model,
                    &batch,
                    &targets,
                    self.reference.as_ref(),
                    self.args.removal_penalty,
                    self.workers,
                )?;
                self.checkpoint
                    .optimizer
                    .update(&mut self.checkpoint.model, &gradients.weights)?;
                training.update_seconds += update.elapsed().as_secs_f64();
                training.bce += gradients.losses.bce * batch.len() as f64;
                training.removal += gradients.losses.removal * batch.len() as f64;
                training.positions += batch.len();
                training.steps += 1;
                Ok(())
            },
        )?;
        training.seconds = started.elapsed().as_secs_f64();
        Ok(training)
    }

    fn advance(&mut self) -> Result<bool> {
        if epoch_stop_reason(
            &self.checkpoint.progress,
            self.args.epochs,
            self.args.patience,
        )
        .is_some()
        {
            return Ok(false);
        }
        let epoch = self.checkpoint.progress.epoch + 1;
        let indices = epoch_permutation(self.dataset.training_indices(), self.args.seed, epoch);
        let training = self.train_epoch(&indices)?;
        let started = Instant::now();
        let loss = self.validate()?;
        let seconds = started.elapsed().as_secs_f64();
        let improved = update_best(&mut self.checkpoint.progress.best, epoch, loss)?;
        self.checkpoint.progress.epoch = epoch;
        self.checkpoint.progress.elapsed_seconds = self.elapsed();
        let record = self.epoch_record(loss, Some(&training), seconds);
        append_epoch(&self.args.output_dir.join("training.jsonl"), &record)?;
        // latestを永続化した後、best公開直前の中断は再開時に復旧する。
        self.save_latest()?;
        if improved {
            atomic_write(
                &self.args.output_dir.join("best.bin"),
                &self.checkpoint.encode()?,
            )?;
        }
        eprintln!(
            "epoch {epoch}: train_loss={:.9} removal_loss={:.9} validation_loss={loss:.9}",
            training.bce / training.positions as f64,
            training.removal / training.positions as f64
        );
        Ok(true)
    }

    fn epoch_record(
        &self,
        loss: f64,
        training: Option<&Training>,
        validation_seconds: f64,
    ) -> EpochRecord {
        let train_loss = training.map(|t| t.bce / t.positions as f64);
        let removal_loss = training.map(|t| t.removal / t.positions as f64);
        EpochRecord {
            device: "cpu",
            epoch: self.checkpoint.progress.epoch,
            train_loss,
            removal_loss,
            total_loss: train_loss
                .zip(removal_loss)
                .map(|(bce, removal)| bce + self.args.removal_penalty * removal),
            validation_loss: loss,
            samples: self.dataset.validation_indices().len(),
            training_seconds: training.map(|t| t.seconds),
            validation_seconds,
            training_positions_per_second: training.map(|t| t.positions as f64 / t.seconds),
            update_seconds: training.map(|t| t.update_seconds / t.steps as f64),
            elapsed_seconds: self.checkpoint.progress.elapsed_seconds,
            steps: training.map_or(0, |t| t.steps),
            total_steps: self.checkpoint.optimizer.step(),
        }
    }

    fn finish(mut self) -> Result<Summary> {
        let stopped_by = epoch_stop_reason(
            &self.checkpoint.progress,
            self.args.epochs,
            self.args.patience,
        )
        .ok_or(Error::Invalid(
            "training stopped without a stopping condition",
        ))?;
        self.checkpoint.progress.elapsed_seconds = self.elapsed();
        self.save_latest()?;
        let best = self
            .checkpoint
            .progress
            .best
            .ok_or(Error::Invalid("training has no best epoch"))?;
        let mut summary = Summary {
            status: "error",
            reason: None,
            stopped_by,
            convergence: None,
            last_epoch: self.checkpoint.progress.epoch,
            best_epoch: Some(best.epoch),
            best_validation_loss: Some(best.validation_loss),
            initial_validation_loss: Some(self.initial_validation_loss),
            improved_over_epoch_zero: Some(best.validation_loss < self.initial_validation_loss),
            total_steps: self.checkpoint.optimizer.step(),
            elapsed_seconds: self.checkpoint.progress.elapsed_seconds,
            quantization: None,
            initial_position_score: None,
        };
        let result = (|| {
            let checkpoint = read_best(&self.args.output_dir, &self.checkpoint)?;
            export::write_best(
                &self.dataset,
                &checkpoint,
                &self.args.output_dir,
                &self.base,
                self.args.validation_sample,
            )
        })();
        let (report, initial_score) = match result {
            Ok(report) => report,
            Err(error) => {
                summary.reason = Some(error.to_string());
                write_json(&self.args.output_dir.join("summary.json"), &summary)?;
                return Err(error);
            }
        };
        let passed = report.passed;
        summary.status = if passed {
            "candidate"
        } else {
            "quantization_error"
        };
        summary.quantization = Some(report);
        summary.initial_position_score = Some(initial_score);
        let failure =
            Error::Invalid("quantization_error: mean absolute error exceeds 2 centipawns");
        if !passed {
            summary.reason = Some(failure.to_string());
        }
        write_json(&self.args.output_dir.join("summary.json"), &summary)?;
        if !passed {
            return Err(failure);
        }
        Ok(summary)
    }
}

#[derive(Default)]
struct Training {
    bce: f64,
    removal: f64,
    positions: usize,
    steps: u64,
    seconds: f64,
    update_seconds: f64,
}

fn read_best(directory: &Path, latest: &Checkpoint) -> Result<Checkpoint> {
    let saved = Checkpoint::read(directory.join("best.bin"))?;
    saved
        .progress
        .check_hashes(&latest.progress.config_hash, &latest.progress.input_hash)?;
    if saved.progress.best != latest.progress.best
        || saved
            .progress
            .best
            .is_none_or(|best| best.epoch != saved.progress.epoch)
    {
        return Err(Error::Invalid(
            "best checkpoint disagrees with latest checkpoint",
        ));
    }
    Ok(saved)
}

fn recover_best(directory: &Path, latest: &Checkpoint) -> Result<()> {
    let best = latest
        .progress
        .best
        .ok_or(Error::Invalid("checkpoint has no best epoch"))?;
    let path = directory.join("best.bin");
    if path.exists() {
        let saved = Checkpoint::read(&path)?;
        saved
            .progress
            .check_hashes(&latest.progress.config_hash, &latest.progress.input_hash)?;
        if saved.progress.best == Some(best) && saved.progress.epoch == best.epoch {
            return Ok(());
        }
        if saved.progress.epoch >= best.epoch {
            return Err(Error::Invalid(
                "best checkpoint disagrees with latest checkpoint",
            ));
        }
    }
    if latest.progress.epoch != best.epoch {
        return Err(Error::Invalid("best checkpoint is missing or inconsistent"));
    }
    Ok(atomic_write(&path, &latest.encode()?)?)
}

/// 初期検証、CPU学習、チェックポイント保存、最良PSTの書き出しを行う。
pub fn run(args: Args) -> Result<Summary> {
    let mut session = Session::new(args)?;
    loop {
        match session.advance() {
            Ok(true) => {}
            Ok(false) => break,
            Err(error) => {
                write_json(
                    &session.args.output_dir.join("summary.json"),
                    &json!({"status":"error", "reason":error.to_string(), "last_epoch":session.checkpoint.progress.epoch, "elapsed_seconds":session.elapsed()}),
                )?;
                return Err(error);
            }
        }
    }
    session.finish()
}

fn targets(batch: &Batch, ks: &[f64], lambdas: &[f64]) -> Result<Vec<f32>> {
    Ok(build_targets_from_arrays(
        batch.teacher_scores(),
        batch.teacher_classes(),
        batch.outcomes(),
        ks,
        lambdas,
    )?)
}

fn epoch_stop_reason(progress: &Progress, epochs: u64, patience: u64) -> Option<StopReason> {
    if patience_reached(progress.best, progress.epoch, patience) {
        Some(StopReason::EarlyStopping)
    } else if progress.epoch >= epochs {
        Some(StopReason::MaxEpochs)
    } else {
        None
    }
}
