use super::{
    engine::{BackwardBuffers, ForwardBuffers, Targets, memory_bytes},
    *,
};
use crate::nnue::{
    data::FeatureSet,
    gpu::{BatchTicket, HostRing, device::pinned_zeros},
    simple::{AdamW, OptimizerConfig, Scale, optimizer::Coefficients},
};
use cudarc::driver::{CudaEvent, PinnedHostSlice, PushKernelArg, sys::CUevent_flags};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct Readback {
    sums: PinnedHostSlice<f64>,
    status: PinnedHostSlice<u32>,
    events: [CudaEvent; 4],
}
impl Readback {
    fn new(device: &Device) -> Result<Self> {
        Ok(Self {
            sums: pinned_zeros(device, STATS)?,
            status: pinned_zeros(device, 1)?,
            events: [
                device
                    .context
                    .new_event(Some(CUevent_flags::CU_EVENT_DEFAULT))?,
                device
                    .context
                    .new_event(Some(CUevent_flags::CU_EVENT_DEFAULT))?,
                device
                    .context
                    .new_event(Some(CUevent_flags::CU_EVENT_DEFAULT))?,
                device
                    .context
                    .new_event(Some(CUevent_flags::CU_EVENT_DEFAULT))?,
            ],
        })
    }
}
pub(crate) struct StepMetrics {
    pub loss: f64,
    pub step: u64,
    pub forward_ms: f32,
    pub backward_ms: f32,
    pub optimizer_ms: f32,
}
#[must_use = "collect the result after submitting the next update"]
pub(crate) struct StepResult {
    readback: Arc<Readback>,
    failed: Arc<AtomicBool>,
    n: usize,
    step: u64,
}
impl StepResult {
    pub fn collect(self) -> Result<StepMetrics> {
        let result = (|| {
            let sums = self.readback.sums.as_slice()?;
            if self.readback.status.as_slice()?[0] != 0 || sums.iter().any(|v| !v.is_finite()) {
                return Err(
                    InfrastructureError::NonFinite("Simple computation or AdamW state").into(),
                );
            }
            Ok(StepMetrics {
                loss: sums[0] / self.n as f64,
                step: self.step,
                forward_ms: self.readback.events[0].elapsed_ms(&self.readback.events[1])?,
                backward_ms: self.readback.events[1].elapsed_ms(&self.readback.events[2])?,
                optimizer_ms: self.readback.events[2].elapsed_ms(&self.readback.events[3])?,
            })
        })();
        if result.is_err() {
            self.failed.store(true, Ordering::Relaxed);
        }
        result
    }
}

/// Resident Simple parameters, AdamW, and two asynchronous result slots.
/// Input and target copies use the transfer stream; kernels and result copies
/// use the compute stream. A result owns its pinned slot until collected.
pub(crate) struct Step {
    device: Device,
    kernels: Kernels,
    weights: CudaSlice<f32>,
    moment: CudaSlice<f32>,
    variance: CudaSlice<f32>,
    forward: ForwardBuffers,
    backward: BackwardBuffers,
    targets: [Targets; 2],
    readbacks: [Arc<Readback>; 2],
    next: usize,
    count: u64,
    config: OptimizerConfig,
    scale: Scale,
    capacity: usize,
    width: usize,
    failed: Arc<AtomicBool>,
}
impl Step {
    pub fn new(
        device: Device,
        weights: &Parameters,
        optimizer: &AdamW,
        scale: Scale,
        n: usize,
    ) -> Result<Self> {
        capacity(n)?;
        weights.validate()?;
        optimizer.validate()?;
        if weights.width() != optimizer.m.width() {
            return Err(invalid("optimizer width mismatch"));
        }
        Ok(Self {
            kernels: Kernels::new(&device)?,
            weights: upload(&device, weights)?,
            moment: upload(&device, &optimizer.m)?,
            variance: upload(&device, &optimizer.v)?,
            forward: ForwardBuffers::new(&device, n, weights.width())?,
            backward: BackwardBuffers::new(&device, n, weights.width())?,
            targets: [Targets::new(&device, n)?, Targets::new(&device, n)?],
            readbacks: [
                Arc::new(Readback::new(&device)?),
                Arc::new(Readback::new(&device)?),
            ],
            next: 0,
            count: optimizer.step,
            config: optimizer.config,
            scale,
            capacity: n,
            width: weights.width(),
            failed: Arc::new(AtomicBool::new(false)),
            device,
        })
    }
    pub fn device(&self) -> &Device {
        &self.device
    }
    pub fn step(&self) -> u64 {
        self.count
    }
    /// Includes the caller-owned HostRing's two device input slots.
    pub fn device_bytes(&self) -> Result<usize> {
        memory_bytes(self.width, self.capacity, true)
    }
    pub fn enqueue(
        &mut self,
        ring: &mut HostRing,
        ticket: &BatchTicket,
        values: &[f32],
        multiplier: f64,
    ) -> Result<StepResult> {
        if self.failed.load(Ordering::Relaxed) {
            return Err(invalid("Simple step has failed"));
        }
        ring.validate_streams(&self.device, FeatureSet::HalfKpVm)?;
        let n = ring.input(ticket)?.len;
        targets(self.capacity, n, values)?;
        let count = self
            .count
            .checked_add(1)
            .ok_or_else(|| invalid("AdamW step overflow"))?;
        let groups = [
            Coefficients::new(count, self.config.ft, multiplier)?,
            Coefficients::new(count, self.config.head, multiplier)?,
        ];
        let readback = Arc::get_mut(&mut self.readbacks[self.next])
            .ok_or_else(|| invalid("collect the oldest Simple result before slot reuse"))?;
        let submitted = (|| -> Result<()> {
            self.targets[self.next].upload(&self.device, values)?;
            let input = ring.input(ticket)?;
            readback.events[0].record(&self.device.compute)?;
            self.forward.launch(
                &self.device,
                &self.kernels,
                &self.weights,
                input,
                &self.targets[self.next].values,
                self.scale,
            )?;
            readback.events[1].record(&self.device.compute)?;
            self.backward.launch(
                &self.device,
                &self.kernels,
                (&self.weights, self.scale),
                input,
                &self.targets[self.next].values,
                &mut self.forward,
            )?;
            readback.events[2].record(&self.device.compute)?;
            ring.finish(ticket.clone())?;
            for p in Parameter::ALL {
                let coeff =
                    &groups[usize::from(!matches!(p, Parameter::FtWeights | Parameter::FtVirtual))];
                let offset = offset(p, self.width) as u32;
                let length = p.count(self.width) as u32;
                let (limit, bounded) = match p.limit() {
                    Some(limit) => (limit, 1_u32),
                    None => (0.0, 0),
                };
                let mut args = self.device.compute.launch_builder(&self.kernels.adamw);
                args.arg(&mut self.weights)
                    .arg(&self.backward.gradients)
                    .arg(&mut self.moment)
                    .arg(&mut self.variance)
                    .arg(&offset)
                    .arg(&length)
                    .arg(&limit)
                    .arg(&bounded)
                    .arg(&coeff.correction1)
                    .arg(&coeff.correction2)
                    .arg(&coeff.lr)
                    .arg(&coeff.decay)
                    .arg(&mut self.forward.nonfinite);
                // SAFETY: offset/count select only this parameter's active region;
                // all four arrays have parameter_count(width) elements. Kernel arguments
                // match simple_adamw and launches serialize all writes on compute.
                unsafe {
                    args.launch(grid(p.count(self.width).div_ceil(THREADS), 1)?)?;
                }
            }
            readback.events[3].record(&self.device.compute)?;
            // The status flag is sticky. A later submission cannot erase an earlier
            // nonfinite result, including when its result ticket has been dropped.
            self.device
                .compute
                .memcpy_dtoh(&self.forward.sums, &mut readback.sums)?;
            self.device
                .compute
                .memcpy_dtoh(&self.forward.nonfinite, &mut readback.status)?;
            Ok(())
        })();
        if let Err(e) = submitted {
            self.failed.store(true, Ordering::Relaxed);
            return Err(e);
        }
        self.count = count;
        let result = StepResult {
            readback: self.readbacks[self.next].clone(),
            failed: self.failed.clone(),
            n,
            step: count,
        };
        self.next ^= 1;
        Ok(result)
    }
    /// Validation reuses resident weights/forward buffers without changing AdamW.
    pub fn validate(
        &mut self,
        ring: &mut HostRing,
        batch: &crate::nnue::data::BatchSlot,
        values: &[f32],
    ) -> Result<crate::nnue::train::backend::Validation> {
        if self.failed.load(Ordering::Relaxed) {
            return Err(invalid("Simple step has failed"));
        }
        ring.validate_streams(&self.device, FeatureSet::HalfKpVm)?;
        targets(self.capacity, batch.len(), values)?;
        self.targets[self.next].upload(&self.device, values)?;
        let ticket = ring.upload(batch)?;
        self.forward.launch(
            &self.device,
            &self.kernels,
            &self.weights,
            ring.input(&ticket)?,
            &self.targets[self.next].values,
            self.scale,
        )?;
        ring.finish(ticket)?;
        self.forward.validation(&self.device, batch.len())
    }
    /// 最後に投入した更新の勾配を、CPUとの一致検査用に回収する。
    pub fn gradients(&self) -> Result<Parameters> {
        check_status(&self.device, &self.forward.nonfinite)?;
        download(&self.device, &self.backward.gradients, self.width)
    }
    /// 更新完了と数値状態を検査し、重みだけをホストへ取り出す。
    pub fn weights(&self) -> Result<Parameters> {
        if self.failed.load(Ordering::Relaxed) {
            return Err(invalid("Simple step has failed"));
        }
        // This also waits for all queued updates, independently of result collection.
        check_status(&self.device, &self.forward.nonfinite)?;
        download(&self.device, &self.weights, self.width)
    }
    pub fn snapshot(&self) -> Result<(Parameters, AdamW)> {
        let weights = self.weights()?;
        let optimizer = AdamW {
            m: download(&self.device, &self.moment, self.width)?,
            v: download(&self.device, &self.variance, self.width)?,
            step: self.count,
            config: self.config,
        };
        optimizer.validate()?;
        Ok((weights, optimizer))
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_adam_groups_reject_underflow_overflow_and_nonpositive_rates() {
        let config = OptimizerConfig::default();
        for multiplier in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::from_bits(1), 1e-60] {
            assert!(Coefficients::new(1, config.ft, multiplier).is_err());
        }
        let mut overflow = config;
        overflow.ft.lr = f64::MAX;
        assert!(Coefficients::new(1, overflow.ft, 2.0).is_err());
        overflow.ft.lr = 1e40;
        assert!(Coefficients::new(1, overflow.ft, 1.0).is_err());
        overflow.ft.lr = 1.0;
        overflow.ft.weight_decay = f64::MAX;
        assert!(Coefficients::new(1, overflow.ft, 1.0).is_err());
        let coeff = Coefficients::new(1, config.ft, 0.7).unwrap();
        assert_eq!((coeff.lr, coeff.decay), (0.0007, 1.0));
    }
}
