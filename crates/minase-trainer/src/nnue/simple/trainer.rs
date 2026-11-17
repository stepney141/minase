//! SimpleのCPU学習状態を共通ループへ接続する。
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

pub(crate) struct Trainer {
    checkpoint: Checkpoint,
}
impl Trainer {
    pub(crate) fn create(
        args: &Args,
        progress: Progress,
        width: usize,
    ) -> train::Result<(Self, Progress)> {
        if args.device == DeviceKind::Cuda {
            return Err(Error::CudaUnavailable.into());
        }
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
        Ok((Self { checkpoint }, progress))
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
    if args.device == DeviceKind::Cuda {
        return Err(Error::CudaUnavailable.into());
    }
    let s = settings(&args, width)?;
    let architecture = json!({"kind":"Simple","feature_set":SPEC.arch_name(),"H":width,"K_t":s.scale.teacher(),"m":s.scale.divisor(),"K_o":s.scale.output(),"rules":s.rules.to_string(),"optimizer":s.optimizer,"factorized":SPEC.factorized(),"initialization":{"distribution":"uniform","half_width":0.01,"virtual":"zero"}});
    train::run_with(args, architecture, |args, progress| {
        Trainer::create(args, progress, width)
    })
}
impl TrainerBackend for Trainer {
    fn name(&self) -> &str {
        "CPU"
    }
    fn feature_set(&self) -> FeatureSet {
        SPEC.feature_set()
    }
    fn step(&self) -> u64 {
        self.checkpoint.optimizer.step
    }
    fn memory(&self) -> train::Result<Option<(usize, usize)>> {
        Ok(None)
    }
    fn queue(&self, _capacity: usize) -> train::Result<(Queue, [Buffer; 3])> {
        Ok((Queue::Cpu, std::array::from_fn(|_| Buffer::Cpu(None))))
    }
    fn train_step(
        &mut self,
        queue: &mut Queue,
        buffer: &mut Buffer,
        targets: &[f32],
        multiplier: f64,
    ) -> train::Result<Option<StepMetrics>> {
        let (Queue::Cpu, Buffer::Cpu(slot)) = (queue, buffer) else {
            return Err(train::Error::Invalid("Simple CPU batch buffers required").into());
        };
        let batch = slot
            .take()
            .ok_or(train::Error::Invalid("missing CPU batch"))?;
        let started = Instant::now();
        let cp = &mut self.checkpoint;
        let (loss, gradients) = backward(
            &cp.parameters,
            cp.settings.scale,
            &Features::from_batch(&batch)?,
            targets,
        )?;
        cp.optimizer
            .update(&mut cp.parameters, &gradients, multiplier)?;
        Ok(Some(StepMetrics {
            loss_sum: loss * batch.len() as f64,
            positions: batch.len(),
            compute_seconds: started.elapsed().as_secs_f64(),
        }))
    }
    fn flush_pending_loss(&mut self) -> train::Result<Option<StepMetrics>> {
        Ok(None)
    }
    fn validate_step(&mut self, batch: &BatchSlot, targets: &[f32]) -> train::Result<Validation> {
        let w = &self.checkpoint.parameters;
        let scale = self.checkpoint.settings.scale;
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
    fn statistics(&mut self, validation: Option<&Validation>) -> train::Result<Value> {
        let mut result = json!({});
        if let Some(v) = validation {
            result["samples"] = json!(v.positions);
            for (key, width) in [
                ("ft", 2 * self.checkpoint.parameters.width()),
                ("l1", 32),
                ("l2", 32),
            ] {
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
        if progress.completed_steps == self.step() {
            self.checkpoint.progress = progress.clone();
            self.checkpoint.write(path)?;
        } else {
            let mut saved = Checkpoint::read(path)?;
            if saved.optimizer.step != progress.completed_steps {
                return Err(train::Error::Invalid("committed checkpoint step mismatch").into());
            }
            saved.progress = progress.clone();
            saved.write(path)?;
        }
        Ok(())
    }
    fn export(&mut self, path: &Path, samples: &[(Position, Color)]) -> train::Result<Value> {
        let borrowed: Vec<_> = samples.iter().map(|(p, c)| (p, *c)).collect();
        let cp = &self.checkpoint;
        let report = quantization_report(&cp.parameters, cp.settings.scale, &borrowed)?;
        crate::nn::atomic_write(path, &export(&cp.parameters, cp.settings.scale)?)?;
        Ok(serde_json::to_value(report)?)
    }
}
