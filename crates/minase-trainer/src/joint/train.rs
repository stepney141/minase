//! PSTとFMを同時に学習し、NPZで保存する。
//! 新規出力先、入力規則名、再開ディレクトリ、設定とハッシュ、エポックと更新回数、保存順と最良モデルの復旧は[`pst::train`](../../pst/train/index.html)と同じ契約である。
//! 規則名を照合する基準は`--base`であり、再開時はK、両群の学習率、FMの重み減衰、シード、観測マスクの一致を要求する。
//! 開始時に初期値を量子化して検査し、平均誤差が2センチポーンを超えれば`quantization_error`を記録して停止する。
//! 最初の5更新の勾配ノルムを記録し、PST、埋め込み、aのどれかが5回とも0なら停止する。

use super::{
    Adam, Args, Batch, Checkpoint, Distribution, EpochRecord, GradientNorm, Progress, Result,
    StopReason, Summary, compute, export, initialize, record::configuration,
};
use crate::{
    data::{Dataset, epoch_permutation},
    fm::{
        feature_observations, mirror_bit,
        record::{append_epoch, reconcile_log, write_json},
    },
    nn::{atomic_write, patience_reached, pipeline, update_best, workers},
};
use minase::eval::pst::Pst;
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

struct Session {
    args: Args,
    workers: usize,
    dataset: Dataset,
    base: Pst,
    ks: Vec<f64>,
    checkpoint: Checkpoint,
    started: Instant,
    previous_elapsed: f64,
    initial_validation_loss: f64,
    initial_quantization: Option<super::QuantizationReport>,
}

impl Session {
    fn new(args: Args) -> Result<Self> {
        let started = Instant::now();
        args.validate()?;
        let workers = workers::workers(args.threads)?;
        let base = Pst::decode(&std::fs::read(&args.base)?)?;
        // このセッションもDatasetを破棄するまで入力を書き換えない。
        let dataset = Dataset::open(
            &args.data,
            args.lookahead()?,
            std::num::NonZeroUsize::new(workers)
                .ok_or(crate::nn::Error::Invalid("worker count must be positive"))?,
        )?
        .with_lambda(args.lambda)?;
        if dataset.training_indices().is_empty() || dataset.validation_indices().is_empty() {
            return Err("training and validation sets must both be nonempty".into());
        }
        if dataset
            .headers()
            .any(|header| header.rule_set() != base.rule_set())
        {
            return Err("MNSD rule set differs from initial MNPT".into());
        }
        let counts = feature_observations(&dataset)?;
        let mask: Vec<_> = counts.iter().map(|&n| n > 0).collect();
        let reference = Pst::decode(&std::fs::read(&args.fm_reference)?)?;
        let (model, mask_mismatches) =
            initialize(&base, &args.pst_init, &args.fm_init, &reference, mask)?;
        let initial_model = model.clone();
        let ks = dataset.estimate_teacher_ks()?;
        let mut config = configuration(
            &args,
            workers,
            &dataset,
            &ks,
            base.k(),
            (&counts, mask_mismatches),
        )?;
        let mut checkpoint = Checkpoint {
            gradients: Vec::new(),
            optimizer: Adam::new(args.pst_lr, args.fm_lr, args.fm_weight_decay)?,
            model,
            progress: Progress {
                epoch: 0,
                best: None,
                elapsed_seconds: 0.0,
                seed: args.seed,
                config: crate::nn::checkpoint::Configuration::from_document(&config.document)?,
            },
        };
        let mut initial_loss = None;
        if let Some(resume) = &args.resume {
            if resume.canonicalize()? != args.output_dir.canonicalize()? {
                return Err("--resume and --output-dir must identify the same directory".into());
            }
            let mut saved: Value =
                serde_json::from_slice(&std::fs::read(resume.join("config.json"))?)?;
            crate::nn::checkpoint::Configuration::from_document(&saved)?.check(
                &crate::nn::checkpoint::Configuration::from_document(&config.document)?,
            )?;
            let restored = Checkpoint::read(resume.join("latest.npz"))?;
            restored.progress.check_config(
                &crate::nn::checkpoint::Configuration::from_document(&config.document)?,
            )?;
            if restored.model.pst.k().to_bits() != base.k().to_bits()
                || restored.optimizer.pst.lr() != args.pst_lr
                || restored.optimizer.fm.group.lr != args.fm_lr
                || restored.optimizer.fm.group.weight_decay != args.fm_weight_decay
                || restored.model.fm.mask() != checkpoint.model.fm.mask()
                || restored.progress.seed != args.seed
                || restored.progress.epoch > args.epochs
                || restored.optimizer.step()
                    != restored.progress.epoch
                        * dataset.training_indices().len().div_ceil(args.batch) as u64
            {
                return Err("checkpoint state differs from configuration or progress".into());
            }
            checkpoint = restored;
            if checkpoint.progress.best.is_some() {
                recover_best(&args.output_dir, &checkpoint)?;
            }
            initial_loss = Some(reconcile_log(
                &resume.join("training.jsonl"),
                checkpoint.progress.epoch,
            )?);
            saved.get_mut("resumes").and_then(Value::as_array_mut).ok_or("configuration resumes must be an array")?.push(json!({
                "resumed_at_unix_seconds":SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                "epochs":args.epochs, "patience":args.patience, "workers":workers, "epoch":checkpoint.progress.epoch}));
            write_json(&args.output_dir.join("config.json"), &saved)?;
        } else {
            if args.output_dir.exists() && std::fs::read_dir(&args.output_dir)?.next().is_some() {
                return Err("output directory is not empty; use --resume to continue a run".into());
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
            checkpoint,
            started,
            previous_elapsed,
            initial_validation_loss: 0.0,
            initial_quantization: None,
        };
        let initial_report = export::quantization_report(
            &session.dataset,
            &initial_model,
            &session.base,
            session.args.validation_sample,
            session.args.seed,
        );
        match initial_report {
            Ok(report) if report.passed => session.initial_quantization = Some(report),
            Ok(report) => {
                write_json(
                    &session.args.output_dir.join("summary.json"),
                    &json!({"status":"quantization_error", "reason":"initial quantization mean absolute error exceeds 2 cp", "initial_quantization":report, "total_steps":session.checkpoint.optimizer.step()}),
                )?;
                return Err("initial quantization mean absolute error exceeds 2 cp".into());
            }
            Err(error) => {
                write_json(
                    &session.args.output_dir.join("summary.json"),
                    &json!({"status":"error", "reason":error.to_string(), "total_steps":session.checkpoint.optimizer.step()}),
                )?;
                return Err(error);
            }
        }
        if let Some(loss) = initial_loss {
            session.initial_validation_loss = loss;
        } else {
            let start = Instant::now();
            let (loss, phi) = session.validate()?;
            session.initial_validation_loss = loss;
            session.checkpoint.progress.elapsed_seconds = session.elapsed();
            let record = session.epoch_record(loss, phi, None, start.elapsed().as_secs_f64());
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
            &self.args.output_dir.join("latest.npz"),
            &self.checkpoint.encode()?,
        )?)
    }

    fn validate(&self) -> Result<(f64, Distribution)> {
        let mut total = 0.0;
        let mut phi_sum = 0.0;
        let mut squares = 0.0;
        let mut maximum = 0.0_f64;
        let chunks: Vec<_> = self
            .dataset
            .validation_indices()
            .chunks(self.args.batch)
            .collect();
        pipeline::ordered::<_, Box<dyn std::error::Error + Send + Sync>>(
            chunks.len(),
            self.workers,
            |index| {
                let batch = Batch::from_indices(
                    &self.dataset,
                    chunks[index],
                    &vec![0; chunks[index].len()],
                    &self.ks,
                )?;
                compute::evaluate(&self.checkpoint.model, &batch)
            },
            |_, values| {
                for (bce, phi) in values {
                    total += bce;
                    phi_sum += phi;
                    squares += phi * phi;
                    maximum = maximum.max(phi.abs());
                }
                Ok(())
            },
        )?;
        let samples = self.dataset.validation_indices().len();
        let mean = phi_sum / samples as f64;
        let loss = total / samples as f64;
        if [loss, mean, squares, maximum]
            .iter()
            .any(|v| !v.is_finite())
        {
            return Err("non-finite FM validation".into());
        }
        Ok((
            loss,
            Distribution {
                mean,
                std: (squares / samples as f64 - mean * mean).max(0.0).sqrt(),
                max_absolute: maximum,
                samples,
                unit: "logit",
                std_cp: (squares / samples as f64 - mean * mean).max(0.0).sqrt()
                    * f64::from(self.base.k()),
            },
        ))
    }

    fn train_epoch(&mut self, indices: &[u64], mirrors: &[u8]) -> Result<Training> {
        let started = Instant::now();
        let mut training = Training::default();
        let chunks: Vec<_> = indices.chunks(self.args.batch).collect();
        let dataset = &self.dataset;
        let ks = &self.ks;
        let batch_size = self.args.batch;
        pipeline::ordered::<_, Box<dyn std::error::Error + Send + Sync>>(
            chunks.len(),
            self.workers,
            |index| {
                let start = index * batch_size;
                Batch::from_indices(
                    dataset,
                    chunks[index],
                    &mirrors[start..start + chunks[index].len()],
                    ks,
                )
            },
            |_, batch| {
                let update = Instant::now();
                let gradients = compute::gradients(
                    &self.checkpoint.model,
                    &batch,
                    self.args.lambda_res,
                    self.args.removal_penalty,
                    self.workers,
                )?;
                if self.checkpoint.optimizer.step() < 5 {
                    self.checkpoint.gradients.push(GradientNorm::new(
                        self.checkpoint.optimizer.step() + 1,
                        &gradients,
                    ));
                    if self.checkpoint.gradients.len() == 5 {
                        for group in ["pst", "embedding", "a"] {
                            if self.checkpoint.gradients.iter().all(|g| match group {
                                "pst" => g.pst == 0.0,
                                "embedding" => g.embedding == 0.0,
                                _ => g.a == 0.0,
                            }) {
                                return Err(format!(
                                    "{group} loss gradient stayed zero for 5 steps"
                                )
                                .into());
                            }
                        }
                    }
                }
                self.checkpoint.optimizer.update(
                    &mut self.checkpoint.model,
                    &gradients,
                    self.workers,
                )?;
                training.update_seconds += update.elapsed().as_secs_f64();
                training.bce += gradients.losses.bce * batch.len() as f64;
                training.removal += gradients.losses.removal * batch.len() as f64;
                training.phi_squared += gradients.losses.phi_squared_mean * batch.len() as f64;
                training.total += gradients.losses.total * batch.len() as f64;
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
        let mirrors: Vec<_> = indices
            .iter()
            .map(|&index| mirror_bit(self.args.seed, epoch, index))
            .collect();
        let training = self.train_epoch(&indices, &mirrors)?;
        let started = Instant::now();
        let (loss, phi) = self.validate()?;
        let seconds = started.elapsed().as_secs_f64();
        let improved = update_best(&mut self.checkpoint.progress.best, epoch, loss)?;
        self.checkpoint.progress.epoch = epoch;
        self.checkpoint.progress.elapsed_seconds = self.elapsed();
        let record = self.epoch_record(loss, phi, Some(&training), seconds);
        append_epoch(&self.args.output_dir.join("training.jsonl"), &record)?;
        // latestを永続化した後、best公開直前の中断は再開時に復旧する。
        self.save_latest()?;
        if improved {
            atomic_write(
                &self.args.output_dir.join("best.npz"),
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
        phi: Distribution,
        training: Option<&Training>,
        validation_seconds: f64,
    ) -> EpochRecord {
        let train_loss = training.map(|t| t.bce / t.positions as f64);
        let removal_loss = training.map(|t| t.removal / t.positions as f64);
        let phi_squared_mean = training.map(|t| t.phi_squared / t.positions as f64);
        EpochRecord {
            device: "cpu",
            epoch: self.checkpoint.progress.epoch,
            train_loss,
            removal_loss,
            phi_squared_mean,
            phi,
            total_loss: training.map(|t| t.total / t.positions as f64),
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
        .ok_or("training stopped without a stopping condition")?;
        self.checkpoint.progress.elapsed_seconds = self.elapsed();
        self.save_latest()?;
        let best = self
            .checkpoint
            .progress
            .best
            .ok_or("training has no best epoch")?;
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
            gradients: self.checkpoint.gradients.clone(),
            initial_quantization: self.initial_quantization.clone(),
            initial_position_score: None,
            phi: None,
            quantization: None,
        };
        let result = (|| {
            let checkpoint = read_best(&self.args.output_dir, &self.checkpoint)?;
            // 最良モデルのPhi分布を、全検証集合で再計算する。
            let latest = std::mem::replace(&mut self.checkpoint, checkpoint);
            summary.phi = Some(self.validate()?.1);
            let result = export::write_best(
                &self.dataset,
                &self.checkpoint.model,
                &self.base,
                &self.args.output_dir,
                self.args.validation_sample,
                self.args.seed,
            );
            self.checkpoint = latest;
            result
        })();
        let report = match result {
            Ok(report) => report,
            Err(error) => {
                summary.reason = Some(error.to_string());
                write_json(&self.args.output_dir.join("summary.json"), &summary)?;
                return Err(error);
            }
        };
        let (quantization, initial_score) = report;
        let passed = quantization.passed;
        summary.status = if passed {
            "candidate"
        } else {
            "quantization_error"
        };
        summary.quantization = Some(quantization);
        summary.initial_position_score = Some(initial_score);
        let failure: Box<dyn std::error::Error + Send + Sync> =
            "quantization_error: mean absolute error exceeds 2 centipawns".into();
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
    phi_squared: f64,
    total: f64,
    positions: usize,
    steps: u64,
    seconds: f64,
    update_seconds: f64,
}

fn read_best(directory: &Path, latest: &Checkpoint) -> Result<Checkpoint> {
    let saved = Checkpoint::read(directory.join("best.npz"))?;
    saved.progress.check_config(&latest.progress.config)?;
    if saved.progress.best != latest.progress.best
        || saved
            .progress
            .best
            .is_none_or(|best| best.epoch != saved.progress.epoch)
    {
        return Err("best checkpoint disagrees with latest checkpoint".into());
    }
    Ok(saved)
}

fn recover_best(directory: &Path, latest: &Checkpoint) -> Result<()> {
    let best = latest.progress.best.ok_or("checkpoint has no best epoch")?;
    let path = directory.join("best.npz");
    if path.exists() {
        let saved = Checkpoint::read(&path)?;
        saved.progress.check_config(&latest.progress.config)?;
        if saved.progress.best == Some(best) && saved.progress.epoch == best.epoch {
            return Ok(());
        }
        if saved.progress.epoch >= best.epoch {
            return Err("best checkpoint disagrees with latest checkpoint".into());
        }
    }
    if latest.progress.epoch != best.epoch {
        return Err("best checkpoint is missing or inconsistent".into());
    }
    Ok(atomic_write(&path, &latest.encode()?)?)
}

/// 初期検証、CPU同時学習、再開と最良モデルの書き出しを行う。
pub fn train(args: Args) -> Result<Summary> {
    let mut session = Session::new(args)?;
    loop {
        match session.advance() {
            Ok(true) => {}
            Ok(false) => break,
            Err(error) => {
                write_json(
                    &session.args.output_dir.join("summary.json"),
                    &json!({"status":"error", "reason":error.to_string(), "last_epoch":session.checkpoint.progress.epoch, "gradients":session.checkpoint.gradients, "total_steps":session.checkpoint.optimizer.step(), "elapsed_seconds":session.elapsed()}),
                )?;
                return Err(error);
            }
        }
    }
    session.finish()
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
