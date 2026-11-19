use super::*;
use crate::nnue::{
    data::{BatchSlot, FEATURE_COLUMNS, FeatureSet, max_segment_count},
    gpu::{HostRing, device::pinned_zeros, ring::DeviceInput},
    layerstack::{Forward, Scale},
    reference::Validation,
};
use cudarc::driver::{PinnedHostSlice, PushKernelArg};

// These lengths are used both by allocation and memory accounting.
struct ForwardLengths {
    values: usize,
    partials: usize,
}
impl ForwardLengths {
    fn new(n: usize) -> Self {
        Self {
            values: n * FORWARD,
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
    sparse: usize,
}
impl BackwardLengths {
    fn new(n: usize) -> Self {
        Self {
            values: n * BACKWARD,
            head: n.div_ceil(THREADS) * HEAD,
            sparse: max_segment_count(ROWS, n * 2 * FEATURE_COLUMNS) * (H + 1),
        }
    }
    fn bytes(&self) -> usize {
        4 * (self.values + parameter_count()) + 8 * (self.head + self.sparse)
    }
}

/// Exact device-array payload, excluding CUDA context/module/allocator overhead
/// and pinned host memory. Includes the two input faces of HostRing.
pub(crate) fn memory_bytes(n: usize, training: bool) -> Result<usize> {
    capacity(n)?;
    let ring = crate::nnue::gpu::ring::device_bytes(FeatureSet::P, n)?;
    let forward = ForwardLengths::new(n).bytes();
    Ok(ring
        + forward
        + if training {
            12 * parameter_count() + BackwardLengths::new(n).bytes() + 8 * n
        } else {
            4 * parameter_count() + 4 * n
        })
}

pub(super) struct ForwardBuffers {
    pub values: CudaSlice<f32>,
    partials: CudaSlice<f64>,
    pub sums: CudaSlice<f64>,
    pub nonfinite: CudaSlice<u32>,
}
impl ForwardBuffers {
    pub fn new(device: &Device, n: usize) -> Result<Self> {
        let l = ForwardLengths::new(n);
        Ok(Self {
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
        let (h, rows, columns, p, n) = (
            H as u32,
            ROWS as u32,
            FEATURE_COLUMNS as u32,
            1_u32,
            input.len as u32,
        );
        let mut args = device.compute.launch_builder(&k.forward);
        args.arg(&input.features)
            .arg(weights)
            .arg(&mut self.values)
            .arg(&h)
            .arg(&rows)
            .arg(&columns)
            .arg(&p)
            .arg(&mut self.nonfinite);
        // SAFETY: validated P input has n*2*145 entries; values has capacity*FORWARD,
        // weights has the fixed layout. One block owns each position, on compute.
        unsafe {
            args.launch(grid(input.len, 1)?)?;
        }
        let ratio = scale.ratio();
        let output_scale = scale.output();
        let mut args = device.compute.launch_builder(&k.statistics);
        args.arg(&self.values)
            .arg(targets)
            .arg(&input.outcomes)
            .arg(&mut self.partials)
            .arg(&n)
            .arg(&h)
            .arg(&ratio)
            .arg(&output_scale)
            .arg(&mut self.nonfinite);
        // SAFETY: all inputs cover n positions; each block writes one distinct
        // partial, within ceil(capacity/128)*14. All 128 lanes reach barriers.
        unsafe {
            args.launch(grid(input.len.div_ceil(THREADS), STATS)?)?;
        }
        let blocks = n.div_ceil(128);
        let mut args = device.compute.launch_builder(&k.statistics_reduce);
        args.arg(&self.partials).arg(&mut self.sums).arg(&blocks);
        // SAFETY: 14 blocks own 14 sums and read only initialized partials.
        unsafe {
            args.launch(grid(STATS, 1)?)?;
        }
        Ok(())
    }
    fn validation(&self, device: &Device, n: usize) -> Result<(Validation, u64, u64)> {
        check_status(device, &self.nonfinite)?;
        let sums = device.compute.clone_dtoh(&self.sums)?;
        if sums.iter().any(|v| !v.is_finite()) {
            return Err(InfrastructureError::NonFinite("LayerStack validation").into());
        }
        let common: [f64; 12] = sums[..12].try_into().expect("fixed statistics length");
        Ok((
            Validation::from_sums(&common, n, H),
            sums[12] as u64,
            sums[13] as u64,
        ))
    }
    pub fn read(&self, device: &Device, n: usize) -> Result<Evaluation> {
        let (validation, square_lower, square_upper) = self.validation(device, n)?;
        let values = device
            .compute
            .clone_dtoh(&self.values.slice(..n * FORWARD))?;
        let forward = values
            .as_chunks::<FORWARD>()
            .0
            .iter()
            .map(|v| {
                let v: Vec<_> = v.iter().copied().map(f64::from).collect();
                Forward {
                    a: std::array::from_fn(|view| std::array::from_fn(|j| v[view * H + j])),
                    p: std::array::from_fn(|j| v[2 * H + j]),
                    y: std::array::from_fn(|j| v[3 * H + j]),
                    z: std::array::from_fn(|j| v[3 * H + 16 + j]),
                    u: std::array::from_fn(|j| v[3 * H + 46 + j]),
                    n: v[3 * H + 78],
                    psqt: [v[3 * H + 79], v[3 * H + 80]],
                    output: v[3 * H + 81],
                }
            })
            .collect();
        Ok(Evaluation {
            forward,
            validation,
            square_lower,
            square_upper,
        })
    }
}
pub(crate) struct Evaluation {
    pub forward: Vec<Forward>,
    pub validation: Validation,
    pub square_lower: u64,
    pub square_upper: u64,
}

pub(super) struct BackwardBuffers {
    values: CudaSlice<f32>,
    head: CudaSlice<f64>,
    sparse: CudaSlice<f64>,
    pub gradients: CudaSlice<f32>,
}
impl BackwardBuffers {
    pub fn new(device: &Device, n: usize) -> Result<Self> {
        let l = BackwardLengths::new(n);
        Ok(Self {
            values: device.compute.alloc_zeros(l.values)?,
            head: device.compute.alloc_zeros(l.head)?,
            sparse: device.compute.alloc_zeros(l.sparse)?,
            gradients: device.compute.alloc_zeros(parameter_count())?,
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
        let (h, rows, n, columns, p) =
            (H as u32, ROWS as u32, input.len as u32, HEAD as u32, 1_u32);
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
            .arg(&ratio)
            .arg(&mut f.nonfinite);
        // SAFETY: a block owns BACKWARD values for a validated position; every
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
        // SAFETY: grid indexes ceil(n/128)*HEAD initialized partials, with 128 lanes.
        unsafe {
            args.launch(grid(blocks as usize, HEAD)?)?;
        }
        let mut args = device.compute.launch_builder(&k.head_reduce);
        args.arg(&self.head)
            .arg(&mut self.gradients)
            .arg(&blocks)
            .arg(&h)
            .arg(&rows)
            .arg(&columns)
            .arg(&mut f.nonfinite);
        // SAFETY: each column owns distinct gradient elements, including shared L1.
        unsafe {
            args.launch(grid(HEAD, 1)?)?;
        }
        if input.first_segments > 0 {
            let mut args = device.compute.launch_builder(&k.sparse_partials);
            args.arg(&input.first_segment_offsets)
                .arg(&input.first_entries)
                .arg(&self.values)
                .arg(&mut self.sparse)
                .arg(&h)
                .arg(&p);
            // SAFETY: BatchSlot builds bounded occurrence words and intervals;
            // allocation uses the same max_segment_count bound as the input ring.
            unsafe {
                args.launch(grid(input.first_segments, 1)?)?;
            }
        }
        let mut args = device.compute.launch_builder(&k.sparse_reduce);
        args.arg(&input.first_segment_starts)
            .arg(&self.sparse)
            .arg(&mut self.gradients)
            .arg(&h)
            .arg(&rows)
            .arg(&mut f.nonfinite);
        // SAFETY: each block owns a row, empty rows read no partials, and the
        // padding row reads no index bounds and writes zero.
        unsafe {
            args.launch(grid(ROWS, 1)?)?;
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
}
impl Evaluator {
    pub fn new(device: Device, weights: &Parameters, scale: Scale, n: usize) -> Result<Self> {
        capacity(n)?;
        Ok(Self {
            kernels: Kernels::new(&device)?,
            weights: upload(&device, weights)?,
            buffers: ForwardBuffers::new(&device, n)?,
            targets: Targets::new(&device, n)?,
            ring: HostRing::new(&device, FeatureSet::P, n)?,
            device,
            scale,
            capacity: n,
        })
    }
    fn prepare(&mut self, batch: &BatchSlot, values: &[f32]) -> Result<()> {
        targets(self.capacity, batch.len(), values)?;
        if batch.feature_set() != FeatureSet::P {
            return Err(invalid("LayerStack requires P input"));
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
        self.prepare(batch, values)?;
        self.buffers.read(&self.device, batch.len())
    }
    /// Transfer only the reduced validation statistics, without backward arrays.
    pub fn validate(
        &mut self,
        batch: &BatchSlot,
        values: &[f32],
    ) -> Result<(Validation, u64, u64)> {
        self.prepare(batch, values)?;
        self.buffers.validation(&self.device, batch.len())
    }
    pub fn device_bytes(&self) -> Result<usize> {
        memory_bytes(self.capacity, false)
    }
}
