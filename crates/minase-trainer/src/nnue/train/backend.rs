use super::{Error, Result};
use crate::nnue::{
    data::{BatchSlot, InputFeatureSet},
    gpu::{self, Device, HostBuffer, HostRing},
    model::{Model, Parameters},
    options::DeviceKind,
    reference::{self, AdamW, Validation},
};
use std::time::Instant;

/// デバイス固有の状態を所有し、学習ループに共通の操作を提供する。
/// CUDAは常にデバイス0を使い、利用できなくてもCPUへ切り替えない。
/// CPUの検証は単一スレッドの前進と`validation_forward`で行い、CUDAの検証は前進と検証のカーネルだけを使う。
/// 理由は`docs/adr/write-trainer-kernels-in-cuda-cpp-via-nvrtc.md`にある。
pub(super) enum Backend {
    Cuda(Box<gpu::Step>),
    Cpu(Box<Cpu>),
}

pub(super) struct Cpu {
    model: Model,
    optimizer: AdamW,
    threads: usize,
}

/// 供給スレッドが循環利用する3面。CPUでは構築済みバッチを所有する。
pub(super) enum Buffer {
    Cuda(HostBuffer),
    Cpu(Option<BatchSlot>),
}

impl Buffer {
    pub fn fill(&mut self, batch: BatchSlot) -> gpu::Result<()> {
        match self {
            Self::Cuda(host) => host.fill(&batch),
            Self::Cpu(slot) => {
                *slot = Some(batch);
                Ok(())
            }
        }
    }
}

pub(super) enum Queue {
    Cuda(Box<HostRing>),
    Cpu,
}

pub(super) enum StepResult {
    Cuda(gpu::StepResult),
    Cpu(gpu::StepMetrics),
}

impl StepResult {
    pub fn collect(self) -> gpu::Result<gpu::StepMetrics> {
        match self {
            Self::Cuda(result) => result.collect(),
            Self::Cpu(metrics) => Ok(metrics),
        }
    }
}

pub(super) enum Evaluator<'a> {
    Cuda(Box<gpu::Evaluator>),
    Cpu { model: &'a Model, weights: Vec<f32> },
}

impl Evaluator<'_> {
    pub fn validate(&mut self, batch: &BatchSlot, targets: &[f32]) -> Result<Validation> {
        match self {
            Self::Cuda(evaluator) => Ok(evaluator.validate(batch, targets)?),
            Self::Cpu { model, weights } => {
                let forward = reference::forward(model, batch, weights)?;
                Ok(reference::validation_forward(
                    model, batch, &forward, targets,
                )?)
            }
        }
    }
}

impl Backend {
    pub fn new(
        kind: DeviceKind,
        model: &Model,
        optimizer: &AdamW,
        capacity: usize,
        threads: usize,
    ) -> Result<Self> {
        Ok(match kind {
            DeviceKind::Cuda => Self::Cuda(Box::new(gpu::Step::new(
                Device::new(0)?,
                model,
                optimizer,
                capacity,
            )?)),
            DeviceKind::Cpu => Self::Cpu(Box::new(Cpu {
                model: model.clone(),
                optimizer: optimizer.clone(),
                threads,
            })),
        })
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Cuda(step) => step.device().name(),
            Self::Cpu(_) => "CPU",
        }
    }

    /// 完了済み更新回数。CUDAでは計算ストリームへ投入済みの更新を含む。
    pub fn step(&self) -> u64 {
        match self {
            Self::Cuda(step) => step.step(),
            Self::Cpu(cpu) => cpu.optimizer.step(),
        }
    }

    pub fn memory(&self) -> Result<Option<(usize, usize)>> {
        match self {
            Self::Cuda(step) => {
                let (free, total) = step.device().memory_info()?;
                Ok(Some((total - free, total)))
            }
            Self::Cpu(_) => Ok(None),
        }
    }

    pub fn queue(
        &self,
        feature_set: InputFeatureSet,
        capacity: usize,
    ) -> Result<(Queue, [Buffer; 3])> {
        match self {
            Self::Cuda(step) => {
                let mut ring = HostRing::new(step.device(), feature_set, capacity)?;
                let buffers = ring.take_host_buffers()?.map(Buffer::Cuda);
                Ok((Queue::Cuda(Box::new(ring)), buffers))
            }
            Self::Cpu(_) => Ok((Queue::Cpu, std::array::from_fn(|_| Buffer::Cpu(None)))),
        }
    }

    pub fn enqueue(
        &mut self,
        queue: &mut Queue,
        buffer: &mut Buffer,
        targets: &[f32],
        multiplier: f64,
    ) -> Result<StepResult> {
        match (self, queue, buffer) {
            (Self::Cuda(step), Queue::Cuda(ring), Buffer::Cuda(host)) => {
                let ticket = ring.upload_prepared(host)?;
                Ok(StepResult::Cuda(
                    step.enqueue(ring, &ticket, targets, multiplier)?,
                ))
            }
            (Self::Cpu(cpu), Queue::Cpu, Buffer::Cpu(slot)) => {
                let Cpu {
                    model,
                    optimizer,
                    threads,
                } = cpu.as_mut();
                let batch = slot.take().ok_or(Error::Invalid("missing CPU batch"))?;
                let started = Instant::now();
                let gradient =
                    reference::loss_and_gradients_parallel(model, &batch, targets, *threads)?;
                optimizer.update_parallel(model, &gradient.parameters, *threads, multiplier)?;
                Ok(StepResult::Cpu(gpu::StepMetrics {
                    losses: gradient.losses,
                    compute_ms: started.elapsed().as_secs_f32() * 1000.0,
                    target_transfer_ms: 0.0,
                    target_transfer_bytes: 0,
                    step: optimizer.step(),
                }))
            }
            _ => Err(Error::Invalid("batch buffers differ from the selected device").into()),
        }
    }

    pub fn snapshot(&self) -> Result<(Parameters, AdamW)> {
        match self {
            Self::Cuda(step) => Ok(step.snapshot()?),
            Self::Cpu(cpu) => Ok((cpu.model.parameters().clone(), cpu.optimizer.clone())),
        }
    }

    pub fn evaluator<'a>(&self, model: &'a Model, capacity: usize) -> Result<Evaluator<'a>> {
        match self {
            Self::Cuda(step) => Ok(Evaluator::Cuda(Box::new(gpu::Evaluator::new(
                step.device().clone(),
                model,
                capacity,
            )?))),
            Self::Cpu(_) => Ok(Evaluator::Cpu {
                model,
                weights: reference::fold(model)?,
            }),
        }
    }
}
