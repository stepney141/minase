//! SimpleのCPU/CUDA学習状態を共通ループへ接続する。
use super::*;
use crate::nnue::{
    data::BatchSlot,
    options::DeviceKind,
    train::{
        self, Args, Progress, TrainerBackend,
        backend::{Buffer, Queue, StepMetrics, Validation},
    },
};
use minase::eval::nnue::input::FeatureSet;
use minase_core::{Color, Position};
use serde_json::{Value, json};
use std::{path::Path, time::Instant};

enum Execution {
    Cpu {
        weights: Box<Parameters>,
        optimizer: Box<AdamW>,
    },
    Cuda(Box<gpu::step::Step>),
}
pub(crate) struct Trainer {
    execution: Execution,
    settings: Settings,
    pending: Option<(gpu::step::StepResult, usize)>,
    validation_ring: Option<crate::nnue::gpu::HostRing>,
    capacity: usize,
}
impl Trainer {
    pub(crate) fn create(
        args: &Args,
        progress: Progress,
        width: usize,
    ) -> train::Result<(Self, Progress)> {
        let settings = settings(args, width)?;
        let checkpoint = if let Some(path) = &args.resume {
            let checkpoint = Checkpoint::read(path.join("latest.npz"))?;
            checkpoint.check_settings(&settings)?;
            checkpoint.progress.config.check(&progress.config)?;
            checkpoint
        } else {
            Checkpoint {
                parameters: Parameters::initialize(args.seed, width)?,
                optimizer: AdamW::new(settings.optimizer, width)?,
                settings,
                progress,
            }
        };
        let progress = checkpoint.progress.clone();
        let execution = match args.device {
            DeviceKind::Cpu => Execution::Cpu {
                weights: Box::new(checkpoint.parameters),
                optimizer: Box::new(checkpoint.optimizer),
            },
            DeviceKind::Cuda => Execution::Cuda(Box::new(gpu::step::Step::new(
                crate::nnue::gpu::Device::new(0)?,
                &checkpoint.parameters,
                &checkpoint.optimizer,
                checkpoint.settings.scale,
                args.batch,
            )?)),
        };
        Ok((
            Self {
                execution,
                settings: checkpoint.settings,
                pending: None,
                validation_ring: None,
                capacity: args.batch,
            },
            progress,
        ))
    }
    fn weights(&self) -> train::Result<Parameters> {
        Ok(match &self.execution {
            Execution::Cpu { weights, .. } => (**weights).clone(),
            Execution::Cuda(step) => step.weights()?,
        })
    }
    fn snapshot(&self) -> train::Result<(Parameters, AdamW)> {
        Ok(match &self.execution {
            Execution::Cpu { weights, optimizer } => ((**weights).clone(), (**optimizer).clone()),
            Execution::Cuda(step) => step.snapshot()?,
        })
    }
}

fn settings(args: &Args, width: usize) -> train::Result<Settings> {
    let groups = args.training.optimizer();
    Ok(Settings {
        width,
        scale: Scale::new(args.teacher_cp_per_logit)?,
        rules: args.parsed_rules()?,
        lambda: args.training.lambda,
        seed: args.seed,
        optimizer: OptimizerConfig {
            ft: groups.ft,
            head: groups.head,
        },
    })
}
/// CLIではH=256を固定して共通の学習ループを実行する。
pub fn run(args: Args) -> train::Result<train::Summary> {
    run_width(args, H)
}
pub(crate) fn run_width(args: Args, width: usize) -> train::Result<train::Summary> {
    let s = settings(&args, width)?;
    let architecture = json!({"kind":"Simple","feature_set":SPEC.arch_name(),"H":width,"K_t":s.scale.teacher(),"m":s.scale.divisor(),"K_o":s.scale.output(),"rules":s.rules.to_string(),"optimizer":s.optimizer,"factorized":SPEC.factorized(),"initialization":{"distribution":"uniform","half_width":0.01,"virtual":"zero"}});
    train::run_with(args, architecture, |args, progress| {
        Trainer::create(args, progress, width)
    })
}
impl TrainerBackend for Trainer {
    fn name(&self) -> &str {
        match &self.execution {
            Execution::Cpu { .. } => "CPU",
            Execution::Cuda(step) => step.device().name(),
        }
    }
    fn feature_set(&self) -> FeatureSet {
        FeatureSet::HalfKpVm
    }
    fn step(&self) -> u64 {
        match &self.execution {
            Execution::Cpu { optimizer, .. } => optimizer.step,
            Execution::Cuda(step) => step.step(),
        }
    }
    fn memory(&self) -> train::Result<Option<(usize, usize)>> {
        match &self.execution {
            Execution::Cpu { .. } => Ok(None),
            Execution::Cuda(step) => {
                let (free, total) = step.device().memory_info()?;
                Ok(Some((total - free, total)))
            }
        }
    }
    fn queue(&self, capacity: usize) -> train::Result<(Queue, [Buffer; 3])> {
        match &self.execution {
            Execution::Cpu { .. } => Ok((Queue::Cpu, std::array::from_fn(|_| Buffer::Cpu(None)))),
            Execution::Cuda(step) => {
                let mut ring =
                    crate::nnue::gpu::HostRing::new(step.device(), FeatureSet::HalfKpVm, capacity)?;
                let buffers = ring.take_host_buffers()?.map(Buffer::Cuda);
                Ok((Queue::Cuda(Box::new(ring)), buffers))
            }
        }
    }
    fn train_step(
        &mut self,
        queue: &mut Queue,
        buffer: &mut Buffer,
        targets: &[f32],
        multiplier: f64,
    ) -> train::Result<Option<StepMetrics>> {
        self.validation_ring = None;
        match (&mut self.execution, queue, buffer) {
            (Execution::Cpu { weights, optimizer }, Queue::Cpu, Buffer::Cpu(slot)) => {
                let batch = slot
                    .take()
                    .ok_or(train::Error::Invalid("missing CPU batch"))?;
                let started = Instant::now();
                let (loss, gradients) = backward(
                    weights,
                    self.settings.scale,
                    &Features::from_batch(&batch)?,
                    targets,
                )?;
                optimizer.update(weights, &gradients, multiplier)?;
                Ok(Some(StepMetrics {
                    loss_sum: loss * batch.len() as f64,
                    positions: batch.len(),
                    compute_seconds: started.elapsed().as_secs_f64(),
                }))
            }
            (Execution::Cuda(step), Queue::Cuda(ring), Buffer::Cuda(host)) => {
                let ticket = ring.upload_prepared(host)?;
                let result = step.enqueue(ring, &ticket, targets, multiplier)?;
                self.pending
                    .replace((result, targets.len()))
                    .map(collect)
                    .transpose()
            }
            _ => Err(train::Error::Invalid("batch buffers differ from selected device").into()),
        }
    }
    fn flush_pending_loss(&mut self) -> train::Result<Option<StepMetrics>> {
        self.pending.take().map(collect).transpose()
    }

    fn validate_step(&mut self, batch: &BatchSlot, targets: &[f32]) -> train::Result<Validation> {
        self.flush_pending_loss()?;
        match &mut self.execution {
            Execution::Cpu { weights, .. } => {
                validation(weights, self.settings.scale, batch, targets)
            }
            Execution::Cuda(step) => {
                if self.validation_ring.is_none() {
                    self.validation_ring = Some(crate::nnue::gpu::HostRing::new(
                        step.device(),
                        FeatureSet::HalfKpVm,
                        self.capacity,
                    )?);
                }
                Ok(step.validate(
                    self.validation_ring
                        .as_mut()
                        .expect("created validation ring"),
                    batch,
                    targets,
                )?)
            }
        }
    }
    fn statistics(&mut self, validation: Option<&Validation>) -> train::Result<Value> {
        // Validation has ended; release its input ring before the next training queue.
        self.validation_ring = None;
        let mut result = json!({});
        if let Some(v) = validation {
            result["samples"] = json!(v.positions);
            for (key, width) in [("ft", 2 * self.settings.width), ("l1", 32), ("l2", 32)] {
                let get = |suffix| -> train::Result<f64> {
                    Ok(*v
                        .sums
                        .get(&format!("{key}_{suffix}"))
                        .ok_or(train::Error::Invalid("missing Simple validation statistic"))?
                        / (v.positions * width) as f64)
                };
                result[format!("{key}_saturation")] =
                    json!({"lower":get("lower")?,"upper":get("upper")?});
            }
        }
        Ok(result)
    }
    fn save_checkpoint(&mut self, path: &Path, progress: &Progress) -> train::Result<()> {
        self.flush_pending_loss()?;
        let checkpoint = if progress.completed_steps == self.step() {
            let (parameters, optimizer) = self.snapshot()?;
            Checkpoint {
                parameters,
                optimizer,
                settings: self.settings.clone(),
                progress: progress.clone(),
            }
        } else {
            // 時間切れの未確定更新は保存せず、確定済みの状態へ累積時間だけを記録する。
            let mut saved = Checkpoint::read(path)?;
            if saved.optimizer.step != progress.completed_steps {
                return Err(train::Error::Invalid("committed checkpoint step mismatch").into());
            }
            saved.progress = progress.clone();
            saved
        };
        Ok(checkpoint.write(path)?)
    }
    fn export(
        &mut self,
        path: &Path,
        samples: &[(Position, Color)],
        checkpoint: Option<(&Path, u64)>,
    ) -> train::Result<Value> {
        self.flush_pending_loss()?;
        let weights = if let Some((source, superbatch)) = checkpoint {
            let saved = Checkpoint::read(source)?;
            saved.check_settings(&self.settings)?;
            if saved.progress.completed_superbatches != superbatch {
                return Err(train::Error::Invalid(
                    "best validation checkpoint does not match the training log",
                )
                .into());
            }
            saved.parameters
        } else {
            self.weights()?
        };
        let borrowed: Vec<_> = samples.iter().map(|(p, c)| (p, *c)).collect();
        let report = quantization_report(&weights, self.settings.scale, &borrowed)?;
        crate::nn::atomic_write(path, &super::export(&weights, self.settings.scale)?)?;
        Ok(serde_json::to_value(report)?)
    }
}

pub(crate) fn validation(
    w: &Parameters,
    scale: Scale,
    batch: &BatchSlot,
    targets: &[f32],
) -> train::Result<Validation> {
    let features = Features::from_batch(batch)?;
    let loss = loss(w, scale, &features, targets)?;
    let mut v = Validation {
        loss_sum: loss * batch.len() as f64,
        positions: batch.len(),
        ..Validation::default()
    };
    for features in &features {
        let f = reference::forward_validated(w, features);
        for (key, values) in [
            ("ft", f.a.iter().flatten().copied().collect::<Vec<_>>()),
            ("l1", f.y.to_vec()),
            ("l2", f.u.to_vec()),
        ] {
            for (suffix, count) in [
                ("lower", values.iter().filter(|&&x| x <= 0.0).count()),
                ("upper", values.iter().filter(|&&x| x >= 1.0).count()),
            ] {
                *v.sums.entry(format!("{key}_{suffix}")).or_insert(0.0) += count as f64;
            }
        }
    }
    Ok(v)
}

fn collect((result, positions): (gpu::step::StepResult, usize)) -> train::Result<StepMetrics> {
    let metrics = result.collect()?;
    Ok(StepMetrics {
        loss_sum: metrics.loss * positions as f64,
        positions,
        compute_seconds: f64::from(metrics.forward_ms + metrics.backward_ms + metrics.optimizer_ms)
            / 1000.0,
    })
}
