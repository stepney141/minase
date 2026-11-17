use super::{commands::BenchArgs, gpu::step::Step, *};
use crate::{
    data::{Dataset, epoch_permutation},
    nnue::{
        data::{BatchSlot, InputFeatureSet, MAX_BATCH_SIZE, build_batches, build_targets},
        gpu::{Device, HostRing},
        options::{self, DeviceKind},
        train,
    },
};
use serde_json::{Value, json};
use std::{num::NonZeroUsize, time::Instant};

// Benchmarking retains one device implementation and supplies bounded groups of batches.
enum Execution {
    Cpu {
        weights: Box<Parameters>,
        optimizer: Box<AdamW>,
        scale: Scale,
    },
    Cuda {
        step: Box<Step>,
        ring: Box<HostRing>,
    },
}
#[derive(Default)]
struct Timing {
    forward: f64,
    backward: f64,
    optimizer: f64,
    transfer: f64,
    loss: f64,
}
impl Execution {
    fn update(&mut self, batch: &BatchSlot, targets: &[f32]) -> train::Result<Timing> {
        match self {
            Self::Cpu {
                weights,
                optimizer,
                scale,
            } => {
                let features = Features::from_batch(batch)?;
                let started = Instant::now();
                let (loss, gradient) = backward(weights, *scale, &features, targets)?;
                let backward = started.elapsed().as_secs_f64() * 1000.0;
                let started = Instant::now();
                optimizer.update(weights, &gradient, 1.0)?;
                Ok(Timing {
                    backward,
                    optimizer: started.elapsed().as_secs_f64() * 1000.0,
                    loss,
                    ..Timing::default()
                })
            }
            Self::Cuda { step, ring } => {
                let ticket = ring.upload(batch)?;
                let transfer = ring.transfer_timing(&ticket)?;
                let metrics = step.enqueue(ring, &ticket, targets, 1.0)?.collect()?;
                Ok(Timing {
                    forward: f64::from(metrics.forward_ms),
                    backward: f64::from(metrics.backward_ms),
                    optimizer: f64::from(metrics.optimizer_ms),
                    transfer: f64::from(transfer.collect()?),
                    loss: metrics.loss,
                })
            }
        }
    }
}

pub(super) fn run(args: BenchArgs) -> train::Result<Value> {
    if args.steps < 2 || !(1..=MAX_BATCH_SIZE).contains(&args.check.batch) {
        return Err(
            train::Error::Invalid("bench requires --steps >= 2 and a valid --batch").into(),
        );
    }
    args.check.training.validate()?;
    let scale = Scale::new(args.check.teacher_cp_per_logit)?;
    let threads = options::workers(args.threads)?;
    let dataset = Dataset::open(
        &args.data,
        None,
        NonZeroUsize::new(threads).expect("validated workers"),
    )?
    .with_lambda(args.check.training.lambda)?;
    if dataset.training_indices().len() < args.check.batch {
        return Err(
            train::Error::Invalid("bench requires at least one full training batch").into(),
        );
    }
    let scales = vec![scale.teacher(); dataset.teacher_classes().len()];
    let weights = Parameters::initialize(args.check.seed, scale);
    let optimizer = AdamW::new(args.check.training.optimizer())?;
    let (mut execution, name, transfer_bytes, device_array_bytes, memory) = match args.device {
        DeviceKind::Cpu => (
            Execution::Cpu {
                weights: Box::new(weights),
                optimizer: Box::new(optimizer),
                scale,
            },
            "CPU".to_owned(),
            0,
            None,
            None,
        ),
        DeviceKind::Cuda => {
            let device = Device::new(0)?;
            let step = Box::new(Step::new(
                device.clone(),
                &weights,
                &optimizer,
                scale,
                args.check.batch,
            )?);
            let ring = Box::new(HostRing::new(
                &device,
                InputFeatureSet::P,
                args.check.batch,
            )?);
            let bytes = step.device_bytes()?;
            let (free, total) = device.memory_info()?;
            let transfer_bytes = ring.transfer_bytes();
            (
                Execution::Cuda { step, ring },
                device.name().to_owned(),
                transfer_bytes,
                Some(bytes),
                Some((total - free, total)),
            )
        }
    };
    let warmup = build_batches(
        &dataset,
        InputFeatureSet::P,
        &[&dataset.training_indices()[..args.check.batch]],
    )?
    .remove(0);
    execution.update(
        &warmup,
        &build_targets(&warmup, &scales, dataset.teacher_lambdas())?,
    )?;
    let mut total = Timing::default();
    let (mut epoch, mut cursor, mut measured) = (1, 0, 0);
    let mut indices = epoch_permutation(dataset.training_indices(), args.check.seed, epoch);
    let started = Instant::now();
    let mut supply_seconds = 0.0;
    while measured < args.steps {
        let mut group = Vec::new();
        for _ in 0..threads.min(args.steps - measured) {
            if cursor + args.check.batch > indices.len() {
                epoch += 1;
                cursor = 0;
                indices = epoch_permutation(dataset.training_indices(), args.check.seed, epoch);
            }
            group.push(indices[cursor..cursor + args.check.batch].to_vec());
            cursor += args.check.batch;
        }
        let supply = Instant::now();
        let ranges: Vec<_> = group.iter().map(Vec::as_slice).collect();
        let batches = build_batches(&dataset, InputFeatureSet::P, &ranges)?;
        let batches = batches
            .into_iter()
            .map(|batch| {
                let targets = build_targets(&batch, &scales, dataset.teacher_lambdas())?;
                Ok((batch, targets))
            })
            .collect::<crate::nnue::data::Result<Vec<_>>>()?;
        supply_seconds += supply.elapsed().as_secs_f64();
        for (batch, targets) in batches {
            let t = execution.update(&batch, &targets)?;
            total.forward += t.forward;
            total.backward += t.backward;
            total.optimizer += t.optimizer;
            total.transfer += t.transfer;
            total.loss += t.loss;
            measured += 1;
        }
    }
    let n = args.steps as f64;
    Ok(
        json!({"device":name,"threads":threads,"steps":args.steps,"batch":args.check.batch,
        "compute_ms":(total.forward+total.backward+total.optimizer)/n,
        "input_transfer_ms":total.transfer/n,"input_transfer_bytes":transfer_bytes,
        "device_array_bytes":device_array_bytes,"gpu_memory_used_bytes":memory.map(|(used,_)|used),"gpu_memory_total_bytes":memory.map(|(_,total)|total),
        "worker_positions_per_second":n*args.check.batch as f64/supply_seconds,"supply_seconds":supply_seconds,"elapsed_seconds":started.elapsed().as_secs_f64(),"loss":total.loss/n,
        "profile": args.profile.then(||json!({"forward_ms":total.forward/n,"backward_ms":total.backward/n,"optimizer_ms":total.optimizer/n,"cpu_backward_includes_forward":args.device==DeviceKind::Cpu}))}),
    )
}
