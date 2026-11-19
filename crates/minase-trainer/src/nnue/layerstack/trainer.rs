//! LayerStackの学習状態とデバイスを所有し、共通ループへ接続する。
use super::{
    gpu::{
        engine::Evaluator,
        step::{Step, StepResult},
    },
    *,
};
use crate::nnue::{
    data::{BatchSlot, FeatureSet},
    gpu::{Device, HostRing},
    options::DeviceKind,
    train::{
        self, Args, Progress, TrainerBackend,
        backend::{Buffer, Queue, StepMetrics, Validation},
    },
};
use minase_core::{Color, Position};
use serde_json::{Value, json};
use std::{path::Path, time::Instant};

enum Execution {
    Cpu {
        weights: Box<Parameters>,
        optimizer: Box<AdamW>,
    },
    Cuda(Box<Step>),
}
pub(crate) struct Trainer {
    execution: Execution,
    settings: Settings,
    pending: Option<(StepResult, usize)>,
    evaluator: Option<Evaluator>,
    initial_psqt: Vec<f32>,
    capacity: usize,
}
impl Trainer {
    pub fn new(args: &Args, progress: Progress) -> train::Result<(Self, Progress)> {
        let settings = settings(args)?;
        let checkpoint = Checkpoint {
            parameters: Parameters::initialize(args.seed, settings.scale),
            optimizer: AdamW::new(settings.optimizer)?,
            settings,
            progress,
        };
        Self::from_checkpoint(args, checkpoint)
    }
    pub fn resume(args: &Args, expected: Progress) -> train::Result<(Self, Progress)> {
        let path = args
            .resume
            .as_ref()
            .ok_or(train::Error::Invalid("missing resume directory"))?;
        let checkpoint = Checkpoint::read(path.join("latest.npz"))?;
        checkpoint.check_settings(&settings(args)?)?;
        checkpoint.progress.config.check(&expected.config)?;
        Self::from_checkpoint(args, checkpoint)
    }
    fn from_checkpoint(args: &Args, checkpoint: Checkpoint) -> train::Result<(Self, Progress)> {
        let initial_psqt =
            Parameters::initialize(args.seed, checkpoint.settings.scale)[Parameter::Psqt].to_vec();
        let execution = match args.device {
            DeviceKind::Cpu => Execution::Cpu {
                weights: Box::new(checkpoint.parameters),
                optimizer: Box::new(checkpoint.optimizer),
            },
            DeviceKind::Cuda => Execution::Cuda(Box::new(Step::new(
                Device::new(0)?,
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
                evaluator: None,
                initial_psqt,
                capacity: args.batch,
            },
            checkpoint.progress,
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
fn settings(args: &Args) -> train::Result<Settings> {
    Ok(Settings {
        scale: Scale::new(args.teacher_cp_per_logit)?,
        rules: args.parsed_rules()?,
        lambda: args.training.lambda,
        seed: args.seed,
        optimizer: args.training.optimizer(),
    })
}
/// LayerStackの初期化または再開を終え、共通の学習ループへ渡す。
pub fn run(args: Args) -> train::Result<train::Summary> {
    let settings = settings(&args)?;
    let architecture = json!({"kind": "LayerStack", "feature_set": FeatureSet::P.spec().arch_name(), "H": H,
        "K_t": settings.scale.teacher(), "m": settings.scale.divisor(), "K_o": settings.scale.output(),
        "rules": settings.rules.to_string(), "optimizer": settings.optimizer,
        "initialization": {"ft_distribution": "uniform", "ft_half_width": (1.0 / FeatureSet::P.spec().dimensions() as f64).sqrt(), "dense_distribution": "uniform", "dense_half_width": 0.01, "biases": "zero", "psqt": "signed_v0_material_over_K_o"}});
    train::run_with(args, architecture, |args, progress| {
        if args.resume.is_some() {
            Trainer::resume(args, progress)
        } else {
            Trainer::new(args, progress)
        }
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
        FeatureSet::P
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
                let mut ring = HostRing::new(step.device(), FeatureSet::P, capacity)?;
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
        self.evaluator = None;
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
        let (v, lower, upper) = match &self.execution {
            Execution::Cpu { weights, .. } => {
                validation(weights, self.settings.scale, batch, targets)?
            }
            Execution::Cuda(step) => {
                if self.evaluator.is_none() {
                    let weights = step.weights()?;
                    self.evaluator = Some(Evaluator::new(
                        step.device().clone(),
                        &weights,
                        self.settings.scale,
                        self.capacity,
                    )?);
                }
                self.evaluator
                    .as_mut()
                    .expect("created evaluator")
                    .validate(batch, targets)?
            }
        };
        let mut sums = std::collections::BTreeMap::new();
        for (key, value) in [
            "loss",
            "correct",
            "decisive",
            "nonlinear_sum",
            "nonlinear_square_sum",
            "ft_lower",
            "ft_upper",
            "l1_lower",
            "l1_upper",
            "l2_lower",
            "l2_upper",
            "pairwise_positive",
        ]
        .into_iter()
        .zip(v.sums())
        {
            sums.insert(key.to_owned(), value);
        }
        sums.insert("square_lower".into(), lower as f64);
        sums.insert("square_upper".into(), upper as f64);
        Ok(Validation {
            loss_sum: v.loss_sum,
            positions: v.positions,
            sums,
        })
    }
    fn statistics(&mut self, validation: Option<&Validation>) -> train::Result<Value> {
        self.flush_pending_loss()?;
        let weights = self.weights()?;
        let delta: Vec<_> = weights[Parameter::Psqt][..ROWS - 1]
            .iter()
            .zip(&self.initial_psqt)
            .map(|(&a, &b)| (f64::from(a) - f64::from(b)).abs() * self.settings.scale.output())
            .collect();
        let mut result = json!({"psqt_movement": {"mean_absolute_cp": delta.iter().sum::<f64>() / delta.len() as f64, "max_absolute_cp": delta.iter().copied().fold(0.0, f64::max)}});
        if let Some(v) = validation {
            let n = v.positions as f64;
            let get = |key: &str| -> train::Result<f64> {
                v.sums.get(key).copied().ok_or_else(|| {
                    train::Error::Invalid("missing LayerStack validation statistic").into()
                })
            };
            let decisive = get("decisive")?;
            result["samples"] = json!(v.positions);
            result["sign_accuracy"] = json!(if decisive == 0.0 {
                None
            } else {
                Some(get("correct")? / decisive)
            });
            result["nonlinear_std_cp"] = json!(
                (get("nonlinear_square_sum")? / n - (get("nonlinear_sum")? / n).powi(2))
                    .max(0.0)
                    .sqrt()
            );
            for (name, low, high, width) in [
                ("ft_saturation", "ft_lower", "ft_upper", 2 * H),
                ("l1_linear_saturation", "l1_lower", "l1_upper", 15),
                ("l1_square_saturation", "square_lower", "square_upper", 15),
                ("l2_saturation", "l2_lower", "l2_upper", 32),
            ] {
                result[name] = json!({"lower": get(low)?/(n*width as f64), "upper": get(high)?/(n*width as f64)});
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
    weights: &Parameters,
    scale: Scale,
    batch: &BatchSlot,
    targets: &[f32],
) -> Result<(crate::nnue::reference::Validation, u64, u64)> {
    let features = Features::from_batch(batch)?;
    if targets.len() != features.len()
        || targets
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(Error::Invalid("validation targets"));
    }
    weights.validate()?;
    let mut sums = [0.0; 12];
    let (mut lower, mut upper) = (0, 0);
    for (i, (features, target)) in features.iter().zip(targets).enumerate() {
        let f = reference::forward_validated(weights, features);
        let x = scale.ratio() * f.output;
        let difference = 1.0 / (1.0 + (-x).exp()) - f64::from(*target);
        sums[0] += difference * difference;
        if batch.outcomes()[i] != 1 {
            sums[1] += f64::from((x > 0.0) == (batch.outcomes()[i] == 2));
            sums[2] += 1.0;
        }
        let cp = f.n * scale.output();
        sums[3] += cp;
        sums[4] += cp * cp;
        for &a in f.a.iter().flatten() {
            sums[5] += f64::from(a <= 0.0);
            sums[6] += f64::from(a >= 1.0);
        }
        for &y in &f.y[..15] {
            sums[7] += f64::from(y <= 0.0);
            sums[8] += f64::from(y >= 1.0);
            let square = (model::ALPHA as f32) * (y as f32) * (y as f32);
            lower += u64::from(square <= 0.0);
            upper += u64::from(square >= 1.0);
        }
        for &u in &f.u {
            sums[9] += f64::from(u <= 0.0);
            sums[10] += f64::from(u >= 1.0);
        }
        for view in &f.a {
            for j in 0..H / 2 {
                sums[11] += f64::from(view[j] > 0.0 && view[j + H / 2] > 0.0);
            }
        }
    }
    Ok((
        crate::nnue::reference::Validation::from_sums(&sums, batch.len(), H),
        lower,
        upper,
    ))
}

fn collect((result, positions): (StepResult, usize)) -> train::Result<StepMetrics> {
    let metrics = result.collect()?;
    Ok(StepMetrics {
        loss_sum: metrics.loss * positions as f64,
        positions,
        compute_seconds: f64::from(metrics.forward_ms + metrics.backward_ms + metrics.optimizer_ms)
            / 1000.0,
    })
}
