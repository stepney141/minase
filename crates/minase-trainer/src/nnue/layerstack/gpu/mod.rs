//! P型LayerStackのCUDA学習。幅と特徴集合を起動引数で渡す。
//! 検証器は順伝播だけの配列を持ち、学習器は128局面ごとの勾配部分和を持つ。
//! GPU試験は `cargo test -p minase-trainer layerstack::gpu -- --ignored --test-threads=1`。

pub(crate) mod engine;
pub(crate) mod step;
#[cfg(test)]
mod tests;

use super::{H, Parameter, Parameters, ROWS};
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
const FORWARD: usize = 3 * H + 82;
const BACKWARD: usize = 2 * H + 49;
const HEAD: usize = H / 2 + 16 * H + 16 + 960 + 32 + 32 + 1;
const STATS: usize = 14;
const SOURCES: [&str; 3] = [
    include_str!("../../../../kernels/layerstack/forward.cu"),
    include_str!("../../../../kernels/layerstack/backward.cu"),
    include_str!("../../../../kernels/layerstack/optimizer.cu"),
];
fn parameter_count() -> usize {
    Parameter::ALL.into_iter().map(Parameter::count).sum()
}
fn offset(p: Parameter) -> usize {
    Parameter::ALL[..p as usize].iter().map(|p| p.count()).sum()
}
fn upload(device: &Device, parameters: &Parameters) -> Result<CudaSlice<f32>> {
    parameters.validate()?;
    let values: Vec<_> = Parameter::ALL
        .into_iter()
        .flat_map(|p| parameters[p].iter().copied())
        .collect();
    Ok(device.compute.clone_htod(&values)?)
}
fn download(device: &Device, values: &CudaSlice<f32>) -> Result<Parameters> {
    let values = device.compute.clone_dtoh(values)?;
    let mut result = Parameters::zeros();
    for p in Parameter::ALL {
        result[p].copy_from_slice(&values[offset(p)..offset(p) + p.count()]);
    }
    result.validate()?;
    Ok(result)
}
fn grid(x: usize, y: usize) -> Result<LaunchConfig> {
    if x == 0 || x > i32::MAX as usize || y == 0 || y > 65535 {
        return Err(invalid("LayerStack launch grid"));
    }
    Ok(LaunchConfig {
        grid_dim: (x as u32, y as u32, 1),
        block_dim: (128, 1, 1),
        shared_mem_bytes: 0,
    })
}
fn capacity(capacity: usize) -> Result<()> {
    if !(1..=crate::nnue::data::MAX_BATCH_SIZE).contains(&capacity) {
        return Err(invalid("LayerStack batch capacity"));
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
        return Err(invalid("LayerStack targets or batch size"));
    }
    Ok(())
}
fn check_status(device: &Device, flag: &CudaSlice<u32>) -> Result<()> {
    if device.compute.clone_dtoh(flag)?[0] != 0 {
        return Err(InfrastructureError::NonFinite("LayerStack computation or AdamW state").into());
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
    sparse_partials: CudaFunction,
    sparse_reduce: CudaFunction,
    adamw: CudaFunction,
}
impl Kernels {
    fn new(device: &Device) -> Result<Self> {
        let ptx = crate::nnue::gpu::device::compile_sources(&SOURCES, device.compute_capability())?;
        let m = device.context.load_module(ptx)?;
        Ok(Self {
            forward: m.load_function("ls_forward")?,
            statistics: m.load_function("ls_statistics")?,
            statistics_reduce: m.load_function("ls_statistics_reduce")?,
            backward: m.load_function("ls_backward")?,
            head_partials: m.load_function("ls_head_partials")?,
            head_reduce: m.load_function("ls_head_reduce")?,
            sparse_partials: m.load_function("ls_sparse_partials")?,
            sparse_reduce: m.load_function("ls_sparse_reduce")?,
            adamw: m.load_function("ls_adamw")?,
        })
    }
}
