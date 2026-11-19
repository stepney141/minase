use super::{Error, Result};
use cudarc::{
    driver::{CudaContext, CudaStream, DeviceRepr, PinnedHostSlice, ValidAsZeroBits},
    nvrtc::{CompileOptions, compile_ptx_with_opts},
};
use std::sync::Arc;

/// CUDA文脈と、計算・転送それぞれの非同期ストリーム。
#[derive(Clone, Debug)]
pub struct Device {
    pub(in crate::nnue) context: Arc<CudaContext>,
    pub(in crate::nnue) compute: Arc<CudaStream>,
    pub(in crate::nnue) transfer: Arc<CudaStream>,
    name: String,
    capability: (i32, i32),
}
impl Device {
    /// 指定番号のGPUを開く。ドライバ不在と初期化失敗はエラーで返す。
    pub fn new(ordinal: usize) -> Result<Self> {
        if ordinal > i32::MAX as usize {
            return Err(Error::Invalid("device ordinal"));
        }
        // SAFETY: cudarcが列挙するCUDA共有ライブラリをOSのローダーで調べるだけで、
        // GPUポインタには触れない。実行環境のCUDAライブラリを信頼する。
        if !unsafe { cudarc::driver::sys::is_culib_present() } {
            return Err(Error::DriverUnavailable);
        }
        let context = CudaContext::new(ordinal).map_err(Error::DeviceUnavailable)?;
        let name = context.name()?;
        let capability = context.compute_capability()?;
        let compute = context.new_stream()?;
        let transfer = context.new_stream()?;
        Ok(Self {
            context,
            compute,
            transfer,
            name,
            capability,
        })
    }
    /// CUDAが報告するデバイス名。
    pub fn name(&self) -> &str {
        &self.name
    }
    /// 計算能力の主番号と副番号。
    pub fn compute_capability(&self) -> (i32, i32) {
        self.capability
    }
    /// CUDAが報告する空き容量と総容量。単位はバイトで、デバイス全体の値。
    pub fn memory_info(&self) -> Result<(usize, usize)> {
        Ok(self.context.mem_get_info()?)
    }
}

pub(in crate::nnue) fn pinned_zeros<T: DeviceRepr + ValidAsZeroBits>(
    device: &Device,
    len: usize,
) -> Result<PinnedHostSlice<T>> {
    let bytes = len
        .checked_mul(std::mem::size_of::<T>())
        .ok_or(Error::Invalid("pinned allocation overflow"))?;
    if bytes == 0 || bytes >= isize::MAX as usize {
        return Err(Error::Invalid("pinned allocation size"));
    }
    // SAFETY: 長さとバイト数を検査済み。フラグ0で確保した未初期化領域を、
    // Rustの参照を作る前に全域ゼロ初期化する。Tはゼロ表現が有効で、所有者はここだけ。
    let mut result = unsafe { device.context.alloc_pinned_with_flags::<T>(len, 0)? };
    let ptr = result.as_mut_ptr()?;
    // SAFETY: ptrは上で確保したlen要素の整列済み領域。まだ転送・参照はなく、
    // T: ValidAsZeroBitsによりゼロの全ビット表現は有効である。
    unsafe { ptr.write_bytes(0, len) };
    Ok(result)
}

/// NVRTCへ`--gpu-architecture=sm_{主}{副}`とfmad=false、prec_div=true、prec_sqrt=true、ftz=falseを渡す。
/// 積和の融合を禁じ、CPU参照と同じ演算単位で丸める。
fn options(capability: (i32, i32)) -> CompileOptions {
    CompileOptions {
        // archフィールドは&'static strなので、動的な指定にはoptionsを使う。
        options: vec![format!(
            "--gpu-architecture=sm_{}{}",
            capability.0, capability.1
        )],
        fmad: Some(false),
        prec_div: Some(true),
        prec_sqrt: Some(true),
        ftz: Some(false),
        name: Some("minase-trainer.cu".into()),
        ..Default::default()
    }
}

// 構造ごとのソースにも同じNVRTC設定と不在時のエラーを適用する。
pub(in crate::nnue) fn compile_sources(
    sources: &[&str],
    capability: (i32, i32),
) -> Result<cudarc::nvrtc::Ptx> {
    // SAFETY: 実行環境のNVRTC共有ライブラリの存在だけを検査する。
    if !unsafe { cudarc::nvrtc::sys::is_culib_present() } {
        return Err(Error::CompilerUnavailable);
    }
    Ok(compile_ptx_with_opts(
        sources.join("\n"),
        options(capability),
    )?)
}
