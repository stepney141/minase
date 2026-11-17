//! Simple CUDA kernels use 128 threads, fixed reductions, and resident W/V arrays.
//! GPU tests: cargo test -p minase-trainer simple::gpu -- --ignored --test-threads=1

pub(crate) mod engine;
pub(crate) mod step;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod training_tests;

use super::{Parameter, Parameters, SPEC};
use crate::nnue::gpu::{Device, Error as InfrastructureError};
use cudarc::driver::{CudaFunction, CudaSlice, LaunchConfig};

#[derive(Debug)]
pub(crate) enum Error {
    Infrastructure(InfrastructureError),
    Model(super::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Infrastructure(e) => e.fmt(f),
            Self::Model(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Infrastructure(e) => Some(e),
            Self::Model(e) => Some(e),
        }
    }
}
impl From<InfrastructureError> for Error {
    fn from(e: InfrastructureError) -> Self {
        Self::Infrastructure(e)
    }
}
impl From<cudarc::driver::DriverError> for Error {
    fn from(e: cudarc::driver::DriverError) -> Self {
        Self::Infrastructure(e.into())
    }
}
impl From<super::Error> for Error {
    fn from(e: super::Error) -> Self {
        Self::Model(e)
    }
}
type Result<T> = std::result::Result<T, Error>;
fn invalid(message: &'static str) -> Error {
    InfrastructureError::Invalid(message).into()
}

const THREADS: usize = 128;
fn forward_len(h: usize) -> usize {
    4 * h + 129
}
fn backward_len(h: usize) -> usize {
    2 * h + 65
}
fn head_len(h: usize) -> usize {
    65 * h + 1121
}
const STATS: usize = 7;
const SOURCES: [&str; 3] = [
    include_str!("../../../../kernels/simple/forward.cu"),
    include_str!("../../../../kernels/simple/backward.cu"),
    include_str!("../../../../kernels/simple/optimizer.cu"),
];
fn parameter_count(h: usize) -> usize {
    Parameter::ALL.into_iter().map(|p| p.count(h)).sum()
}
fn offset(p: Parameter, h: usize) -> usize {
    Parameter::ALL[..p as usize]
        .iter()
        .map(|p| p.count(h))
        .sum()
}
fn upload(device: &Device, parameters: &Parameters) -> Result<CudaSlice<f32>> {
    parameters.validate()?;
    let h = parameters.width();
    let mut values = device.compute.alloc_zeros(parameter_count(h))?;
    for p in Parameter::ALL {
        device.compute.memcpy_htod(
            &parameters[p],
            &mut values.slice_mut(offset(p, h)..offset(p, h) + p.count(h)),
        )?;
    }
    Ok(values)
}
fn download(device: &Device, values: &CudaSlice<f32>, h: usize) -> Result<Parameters> {
    let mut result = Parameters::zeros(h)?;
    for p in Parameter::ALL {
        device.compute.memcpy_dtoh(
            &values.slice(offset(p, h)..offset(p, h) + p.count(h)),
            &mut result[p],
        )?;
    }
    result.validate()?;
    Ok(result)
}
fn grid(x: usize, y: usize) -> Result<LaunchConfig> {
    if x == 0 || x > i32::MAX as usize || y == 0 || y > 65535 {
        return Err(invalid("Simple launch grid"));
    }
    Ok(LaunchConfig {
        grid_dim: (x as u32, y as u32, 1),
        block_dim: (128, 1, 1),
        shared_mem_bytes: 0,
    })
}
fn capacity(capacity: usize) -> Result<()> {
    if !(1..=crate::nnue::data::MAX_BATCH_SIZE).contains(&capacity) {
        return Err(invalid("Simple batch capacity"));
    }
    Ok(())
}
fn targets(capacity: usize, n: usize, targets: &[f32]) -> Result<()> {
    if n == 0
        || n > capacity
        || targets.len() != n
        || targets
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(invalid("Simple targets or batch size"));
    }
    Ok(())
}
fn check_status(device: &Device, flag: &CudaSlice<u32>) -> Result<()> {
    if device.compute.clone_dtoh(flag)?[0] != 0 {
        return Err(InfrastructureError::NonFinite("Simple computation or AdamW state").into());
    }
    Ok(())
}
struct Kernels {
    forward: CudaFunction,
    statistics: CudaFunction,
    statistics_reduce: CudaFunction,
    backward: CudaFunction,
    head_partials: CudaFunction,
    head_reduce: CudaFunction,
    virtual_reduce: CudaFunction,
    sparse_reduce: CudaFunction,
    adamw: CudaFunction,
}
impl Kernels {
    fn new(device: &Device) -> Result<Self> {
        let ptx = crate::nnue::gpu::device::compile_sources(&SOURCES, device.compute_capability())?;
        let m = device.context.load_module(ptx)?;
        Ok(Self {
            forward: m.load_function("simple_forward")?,
            statistics: m.load_function("simple_statistics")?,
            statistics_reduce: m.load_function("simple_statistics_reduce")?,
            backward: m.load_function("simple_backward")?,
            head_partials: m.load_function("simple_head_partials")?,
            head_reduce: m.load_function("simple_head_reduce")?,
            virtual_reduce: m.load_function("simple_virtual_reduce")?,
            sparse_reduce: m.load_function("simple_sparse_reduce")?,
            adamw: m.load_function("simple_adamw")?,
        })
    }
}
