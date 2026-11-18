use super::*;
use crate::nnue::{
    data::{BatchSlot, FEATURE_COLUMNS, FeatureSet},
    gpu::{HostRing, device::pinned_zeros, ring::DeviceInput},
    simple::{Forward, Scale},
    train::backend::Validation,
};
use cudarc::driver::{PinnedHostSlice, PushKernelArg};

// These lengths are used both by allocation and memory accounting.
struct ForwardLengths {
    values: usize,
    partials: usize,
}
impl ForwardLengths {
    fn new(n: usize, h: usize) -> Self {
        Self {
            values: n * forward_len(h),
            partials: n.div_ceil(THREADS) * STATS,
        }
    }
    fn bytes(&self) -> usize {
        4 * self.values + 8 * (self.partials + STATS) + 4
    }
}
struct BackwardLengths {
    values: usize,
    head: usize,
}
impl BackwardLengths {
    fn new(n: usize, h: usize) -> Self {
        Self {
            values: n * backward_len(h),
            head: n.div_ceil(THREADS) * head_len(h),
        }
    }
    fn bytes(&self) -> usize {
        4 * self.values + 8 * self.head
    }
}

/// Exact device-array payload, excluding CUDA context/module/allocator overhead
/// and pinned host memory. Includes the two input faces of HostRing.
#[derive(Debug, serde::Serialize)]
pub(crate) struct Memory {
    pub parameters: usize,
    pub moments: usize,
    pub gradients: usize,
    pub input_ring: usize,
    pub forward: usize,
    pub backward: usize,
    pub targets: usize,
}
impl Memory {
    pub fn total(&self) -> usize {
        self.parameters
            + self.moments
            + self.gradients
            + self.input_ring
            + self.forward
            + self.backward
            + self.targets
    }
}
pub(crate) fn memory(h: usize, n: usize, training: bool) -> Result<Memory> {
    capacity(n)?;
    super::super::model::check_width(h)?;
    let parameters = 4 * parameter_count(h);
    Ok(Memory {
        parameters,
        moments: if training { 2 * parameters } else { 0 },
        gradients: if training { parameters } else { 0 },
        input_ring: crate::nnue::gpu::ring::device_bytes(FeatureSet::HalfKpVm, n)?,
        forward: ForwardLengths::new(n, h).bytes(),
        backward: if training {
            BackwardLengths::new(n, h).bytes()
        } else {
            0
        },
        targets: if training { 8 * n } else { 4 * n },
    })
}
pub(crate) fn memory_bytes(h: usize, n: usize, training: bool) -> Result<usize> {
    Ok(memory(h, n, training)?.total())
}

pub(super) struct ForwardBuffers {
    width: usize,
    pub values: CudaSlice<f32>,
    partials: CudaSlice<f64>,
    pub sums: CudaSlice<f64>,
    pub nonfinite: CudaSlice<u32>,
}
impl ForwardBuffers {
    pub fn new(device: &Device, n: usize, h: usize) -> Result<Self> {
        let l = ForwardLengths::new(n, h);
        Ok(Self {
            width: h,
            values: device.compute.alloc_zeros(l.values)?,
            partials: device.compute.alloc_zeros(l.partials)?,
            sums: device.compute.alloc_zeros(STATS)?,
            nonfinite: device.compute.alloc_zeros(1)?,
        })
    }
    pub fn launch(
        &mut self,
        device: &Device,
        k: &Kernels,
        weights: &CudaSlice<f32>,
        input: &DeviceInput,
        targets: &CudaSlice<f32>,
        scale: Scale,
    ) -> Result<()> {
        let (h, rows, base, columns, n) = (
            self.width as u32,
            SPEC.dimensions() as u32,
            SPEC.piece_inputs() as u32,
            FEATURE_COLUMNS as u32,
            input.len as u32,
        );
        let mut args = device.compute.launch_builder(&k.forward);
        args.arg(&input.features)
            .arg(weights)
            .arg(&mut self.values)
            .arg(&h)
            .arg(&rows)
            .arg(&base)
            .arg(&columns)
            .arg(&mut self.nonfinite);
        // SAFETY: validated HalfKP_vm input has n*2*145 entries; values has capacity*forward_len(width),
        // weights has the fixed layout. One block owns each position, on compute.
        unsafe {
            args.launch(grid(input.len, 1)?)?;
        }
        let ratio = scale.ratio();
        let mut args = device.compute.launch_builder(&k.statistics);
        args.arg(&self.values)
            .arg(targets)
            .arg(&mut self.partials)
            .arg(&n)
            .arg(&h)
            .arg(&ratio)
            .arg(&mut self.nonfinite);
        // SAFETY: all inputs cover n positions; each block writes one distinct
        // partial, within ceil(capacity/128)*7. All 128 lanes reach barriers.
        unsafe {
            args.launch(grid(input.len.div_ceil(THREADS), STATS)?)?;
        }
        let blocks = n.div_ceil(128);
        let mut args = device.compute.launch_builder(&k.statistics_reduce);
        args.arg(&self.partials).arg(&mut self.sums).arg(&blocks);
        // SAFETY: 7 blocks own 7 sums and read only initialized partials.
        unsafe {
            args.launch(grid(STATS, 1)?)?;
        }
        Ok(())
    }
    pub fn validation(&self, device: &Device, n: usize) -> Result<Validation> {
        check_status(device, &self.nonfinite)?;
        let sums = device.compute.clone_dtoh(&self.sums)?;
        if sums.iter().any(|v| !v.is_finite()) {
            return Err(InfrastructureError::NonFinite("Simple validation").into());
        }
        Ok(Validation {
            loss_sum: sums[0],
            positions: n,
            sums: [
                "ft_lower", "ft_upper", "l1_lower", "l1_upper", "l2_lower", "l2_upper",
            ]
            .into_iter()
            .zip(&sums[1..])
            .map(|(k, &v)| (k.to_owned(), v))
            .collect(),
        })
    }
    pub fn read(&self, device: &Device, n: usize, validation: Validation) -> Result<Evaluation> {
        let h = self.width;
        let values = device
            .compute
            .clone_dtoh(&self.values.slice(..n * forward_len(h)))?;
        let forward = values
            .chunks_exact(forward_len(h))
            .map(|v| {
                let array = |start| std::array::from_fn(|j| f64::from(v[start + j]));
                Forward {
                    a: std::array::from_fn(|view| {
                        v[view * h..(view + 1) * h]
                            .iter()
                            .copied()
                            .map(f64::from)
                            .collect()
                    }),
                    x: v[2 * h..4 * h].iter().copied().map(f64::from).collect(),
                    y: array(4 * h),
                    h1: array(4 * h + 32),
                    u: array(4 * h + 64),
                    h2: array(4 * h + 96),
                    output: f64::from(v[4 * h + 128]),
                }
            })
            .collect();
        Ok(Evaluation {
            forward,
            validation,
        })
    }
}
pub(crate) struct Evaluation {
    pub forward: Vec<Forward>,
    pub validation: Validation,
}

pub(super) struct BackwardBuffers {
    width: usize,
    values: CudaSlice<f32>,
    head: CudaSlice<f64>,
    pub gradients: CudaSlice<f32>,
}
impl BackwardBuffers {
    pub fn new(device: &Device, n: usize, h: usize) -> Result<Self> {
        let l = BackwardLengths::new(n, h);
        Ok(Self {
            width: h,
            values: device.compute.alloc_zeros(l.values)?,
            head: device.compute.alloc_zeros(l.head)?,
            gradients: device.compute.alloc_zeros(parameter_count(h))?,
        })
    }
    pub fn launch(
        &mut self,
        device: &Device,
        k: &Kernels,
        model: (&CudaSlice<f32>, Scale),
        input: &DeviceInput,
        targets: &CudaSlice<f32>,
        f: &mut ForwardBuffers,
    ) -> Result<()> {
        let (h, rows, n, columns, base) = (
            self.width as u32,
            SPEC.dimensions() as u32,
            input.len as u32,
            head_len(self.width) as u32,
            SPEC.piece_inputs() as u32,
        );
        let (weights, scale) = model;
        let ratio = scale.ratio();
        let mut args = device.compute.launch_builder(&k.backward);
        args.arg(weights)
            .arg(&f.values)
            .arg(targets)
            .arg(&mut self.values)
            .arg(&n)
            .arg(&h)
            .arg(&rows)
            .arg(&base)
            .arg(&ratio)
            .arg(&mut f.nonfinite);
        // SAFETY: a block owns backward_len(width) values for a validated position; every
        // input covers n positions and the kernel matches the fixed parameter layout.
        unsafe {
            args.launch(grid(input.len, 1)?)?;
        }
        let blocks = n.div_ceil(128);
        let mut args = device.compute.launch_builder(&k.head_partials);
        args.arg(&f.values)
            .arg(&self.values)
            .arg(&mut self.head)
            .arg(&n)
            .arg(&h)
            .arg(&columns);
        // SAFETY: each block owns up to 32 columns of one 128-position partial;
        // padded columns are masked and all 128 threads reach the barriers.
        unsafe {
            args.launch(grid(head_len(self.width).div_ceil(32), blocks as usize)?)?;
        }
        let mut args = device.compute.launch_builder(&k.head_reduce);
        args.arg(&self.head)
            .arg(&mut self.gradients)
            .arg(&blocks)
            .arg(&h)
            .arg(&rows)
            .arg(&base)
            .arg(&columns)
            .arg(&mut f.nonfinite);
        // SAFETY: each column owns distinct gradient elements, in the contiguous dense parameter region.
        unsafe {
            args.launch(grid(head_len(self.width).div_ceil(32), 1)?)?;
        }
        // AdamW reads every real row, including rows absent from this batch.
        device
            .compute
            .memset_zeros(&mut self.gradients.slice_mut(..SPEC.dimensions() * self.width))?;
        if input.first_segments != 0 {
            let feature_columns = FEATURE_COLUMNS as u32;
            let mut args = device.compute.launch_builder(&k.sparse_reduce);
            args.arg(&input.first_segment_starts)
                .arg(&input.first_segment_offsets)
                .arg(&input.first_entries)
                .arg(&input.features)
                .arg(&self.values)
                .arg(&mut self.gradients)
                .arg(&h)
                .arg(&feature_columns)
                .arg(&mut f.nonfinite);
            // SAFETY: each existing segment identifies a nonempty row via its
            // first occurrence and the validated HalfKP_vm feature array;
            // only its first segment writes that row. The validated inverse index
            // bounds all reads. The real region was zeroed on this same stream.
            unsafe {
                args.launch(grid(input.first_segments, 1)?)?;
            }
        }
        let mut args = device.compute.launch_builder(&k.virtual_reduce);
        args.arg(&mut self.gradients)
            .arg(&h)
            .arg(&rows)
            .arg(&base)
            .arg(&mut f.nonfinite);
        // SAFETY: real gradients are initialized on the same stream, and each
        // virtual row is written by one block in its disjoint parameter region.
        unsafe {
            args.launch(grid(SPEC.piece_inputs(), 1)?)?;
        }
        Ok(())
    }
}

pub(super) struct Targets {
    host: PinnedHostSlice<f32>,
    pub values: CudaSlice<f32>,
}
impl Targets {
    pub fn new(device: &Device, n: usize) -> Result<Self> {
        Ok(Self {
            host: pinned_zeros(device, n)?,
            values: device.transfer.alloc_zeros(n)?,
        })
    }
    pub fn upload(&mut self, device: &Device, targets: &[f32]) -> Result<()> {
        self.host.as_mut_slice()?[..targets.len()].copy_from_slice(targets);
        device.transfer.memcpy_htod(&self.host, &mut self.values)?;
        let done = device.transfer.record_event(None)?;
        device.compute.wait(&done)?;
        Ok(())
    }
}

/// Validation-only evaluator; never allocates gradients or backward buffers.
pub(crate) struct Evaluator {
    device: Device,
    kernels: Kernels,
    weights: CudaSlice<f32>,
    buffers: ForwardBuffers,
    targets: Targets,
    ring: HostRing,
    scale: Scale,
    capacity: usize,
    width: usize,
}
impl Evaluator {
    pub fn new(device: Device, weights: &Parameters, scale: Scale, n: usize) -> Result<Self> {
        capacity(n)?;
        Ok(Self {
            kernels: Kernels::new(&device)?,
            weights: upload(&device, weights)?,
            buffers: ForwardBuffers::new(&device, n, weights.width())?,
            targets: Targets::new(&device, n)?,
            ring: HostRing::new(&device, FeatureSet::HalfKpVm, n)?,
            device,
            scale,
            capacity: n,
            width: weights.width(),
        })
    }
    fn prepare(&mut self, batch: &BatchSlot, values: &[f32]) -> Result<()> {
        targets(self.capacity, batch.len(), values)?;
        if batch.feature_set() != FeatureSet::HalfKpVm {
            return Err(invalid("Simple requires HalfKP_vm input"));
        }
        self.targets.upload(&self.device, values)?;
        let ticket = self.ring.upload(batch)?;
        self.buffers.launch(
            &self.device,
            &self.kernels,
            &self.weights,
            self.ring.input(&ticket)?,
            &self.targets.values,
            self.scale,
        )?;
        self.ring.finish(ticket)?;
        Ok(())
    }
    /// Read intermediate values only for diagnostics/equivalence checks.
    pub fn evaluate(&mut self, batch: &BatchSlot, values: &[f32]) -> Result<Evaluation> {
        let validation = self.validate(batch, values)?;
        self.buffers.read(&self.device, batch.len(), validation)
    }
    /// Transfer only the reduced validation statistics, without backward arrays.
    pub fn validate(&mut self, batch: &BatchSlot, values: &[f32]) -> Result<Validation> {
        self.prepare(batch, values)?;
        self.buffers.validation(&self.device, batch.len())
    }
    pub fn device_bytes(&self) -> Result<usize> {
        memory_bytes(self.width, self.capacity, false)
    }
}
