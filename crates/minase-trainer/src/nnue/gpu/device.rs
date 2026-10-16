use super::{Error, Result};
use cudarc::{
    driver::{CudaContext, CudaFunction, CudaStream, DeviceRepr, PinnedHostSlice, ValidAsZeroBits},
    nvrtc::{CompileOptions, compile_ptx_with_opts},
};
use std::sync::Arc;

/// CUDA文脈と、計算・転送それぞれの非同期ストリーム。
#[derive(Clone, Debug)]
pub struct Device {
    pub(super) context: Arc<CudaContext>,
    pub(super) compute: Arc<CudaStream>,
    pub(super) transfer: Arc<CudaStream>,
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

pub(super) fn pinned_zeros<T: DeviceRepr + ValidAsZeroBits>(
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

/// forward、head、backward、reduce、optimizerの順にCUDAソースを連結する。
/// 後のファイルが前の`feature_weight`、`screlu`、`phase`、`logit_value`を参照するため、この順序を保つ。
pub(super) fn source() -> String {
    [
        include_str!("../../../kernels/forward.cu"),
        include_str!("../../../kernels/head.cu"),
        include_str!("../../../kernels/backward.cu"),
        include_str!("../../../kernels/reduce.cu"),
        include_str!("../../../kernels/optimizer.cu"),
    ]
    .join("\n")
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

pub(super) struct Kernels {
    pub acc: CudaFunction,
    pub linear: CudaFunction,
    pub head: CudaFunction,
    pub first_backward_partials: CudaFunction,
    pub first_backward_reduce: CudaFunction,
    pub linear_backward_partials: CudaFunction,
    pub linear_backward_reduce: CudaFunction,
    pub virtual_reduce: CudaFunction,
    pub gradients: CudaFunction,
    pub reduce: CudaFunction,
    pub validation: CudaFunction,
    pub optimizer_rows: CudaFunction,
    pub optimizer_virtual: CudaFunction,
    pub optimizer_first: CudaFunction,
    pub optimizer_head: CudaFunction,
}
impl Kernels {
    pub fn compile(device: &Device) -> Result<Self> {
        // SAFETY: NVRTC共有ライブラリの存在検査だけを行い、未検証のポインタを渡さない。
        // Device::newと同じく、実行環境のCUDAライブラリを信頼する。
        if !unsafe { cudarc::nvrtc::sys::is_culib_present() } {
            return Err(Error::CompilerUnavailable);
        }
        let ptx = compile_ptx_with_opts(source(), options(device.capability))?;
        let module = device.context.load_module(ptx)?;
        Ok(Self {
            acc: module.load_function("forward_acc")?,
            linear: module.load_function("forward_linear")?,
            head: module.load_function("output_backward")?,
            first_backward_partials: module.load_function("first_layer_backward_partials")?,
            first_backward_reduce: module.load_function("first_layer_backward_reduce")?,
            linear_backward_partials: module.load_function("linear_backward_partials")?,
            linear_backward_reduce: module.load_function("linear_backward_reduce")?,
            virtual_reduce: module.load_function("reduce_virtual")?,
            gradients: module.load_function("gradient_partials")?,
            reduce: module.load_function("reduce_partials")?,
            validation: module.load_function("validation_head")?,
            optimizer_rows: module.load_function("optimizer_rows")?,
            optimizer_virtual: module.load_function("optimizer_virtual")?,
            optimizer_first: module.load_function("optimizer_first")?,
            optimizer_head: module.load_function("optimizer_head")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // CUDAの行コメントとブロックコメントを除き、文字列・文字リテラルは保つ。
    fn without_comments(source: &str) -> String {
        let mut chars = source.chars().peekable();
        let mut code = String::new();
        while let Some(c) = chars.next() {
            match (c, chars.peek().copied()) {
                ('/', Some('/')) => {
                    chars.next();
                    for c in chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                    code.push('\n');
                }
                ('/', Some('*')) => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '*' && chars.peek() == Some(&'/') {
                            chars.next();
                            break;
                        }
                    }
                    code.push(' ');
                }
                ('"' | '\'', _) => {
                    code.push(c);
                    while let Some(next) = chars.next() {
                        code.push(next);
                        if next == c {
                            break;
                        }
                        if next == '\\'
                            && let Some(escaped) = chars.next()
                        {
                            code.push(escaped);
                        }
                    }
                }
                _ => code.push(c),
            }
        }
        code
    }

    #[test]
    fn embedded_sources_and_compiler_options_keep_runtime_dimensions() {
        let src = source();
        for name in [
            "forward_acc",
            "forward_linear",
            "output_backward",
            "first_layer_backward_partials",
            "first_layer_backward_reduce",
            "linear_backward_partials",
            "linear_backward_reduce",
            "reduce_virtual",
            "gradient_partials",
            "reduce_partials",
            "validation_head",
            "optimizer_rows",
            "optimizer_virtual",
            "optimizer_first",
            "optimizer_head",
        ] {
            assert_eq!(
                src.matches(&format!("extern \"C\" __global__ void {name}("))
                    .count(),
                1
            );
        }
        let code = without_comments(&src);
        assert!(!code.contains("atomicAdd"));
        assert!(!code.contains("template"));
        let opts = options((8, 6));
        assert_eq!(opts.options, ["--gpu-architecture=sm_86"]);
        assert_eq!(opts.fmad, Some(false));
        assert_eq!(opts.prec_div, Some(true));
        assert_eq!(opts.prec_sqrt, Some(true));
        assert_eq!(opts.ftz, Some(false));
    }

    #[test]
    fn rust_launch_argument_order_matches_cuda_signatures() {
        let cuda = source();
        let rust = [include_str!("engine.rs"), include_str!("optimizer.rs")].join("\n");
        for (kernel, field) in [
            ("forward_acc", "acc"),
            ("forward_linear", "linear"),
            ("output_backward", "head"),
            ("first_layer_backward_partials", "first_backward_partials"),
            ("first_layer_backward_reduce", "first_backward_reduce"),
            ("linear_backward_partials", "linear_backward_partials"),
            ("linear_backward_reduce", "linear_backward_reduce"),
            ("reduce_virtual", "virtual_reduce"),
            ("gradient_partials", "gradients"),
            ("reduce_partials", "reduce"),
            ("validation_head", "validation"),
            ("optimizer_rows", "optimizer_rows"),
            ("optimizer_virtual", "optimizer_virtual"),
            ("optimizer_first", "optimizer_first"),
            ("optimizer_head", "optimizer_head"),
        ] {
            let signature = cuda
                .split(&format!("void {kernel}("))
                .nth(1)
                .unwrap()
                .split(')')
                .next()
                .unwrap();
            let names: Vec<_> = signature
                .split(',')
                .map(|p| p.split_whitespace().last().unwrap())
                .collect();
            let launch = rust
                .split(&format!("launch_builder(&k.{field})"))
                .nth(1)
                .unwrap()
                .split("unsafe")
                .next()
                .unwrap();
            let args: Vec<_> = launch
                .split(".arg(")
                .skip(1)
                .map(|s| {
                    let expression = s
                        .split(')')
                        .next()
                        .unwrap()
                        .trim_start_matches('&')
                        .trim_start_matches("mut ");
                    let name = expression.rsplit('.').next().unwrap();
                    match (kernel, name) {
                        (_, "n" | "n32") => "B",
                        (_, "h") if expression == "h" => "H",
                        (_, "scale") => "K",
                        (_, "piece_counts") => "counts",
                        (_, "output_bias") => "bias",
                        (_, "gradient_partials" | "validation_partials") => "partials",
                        ("forward_linear", "linear_features") => "features",
                        ("forward_linear", "linear") => "weights",
                        ("reduce_partials", "columns") => "values",
                        (_, "weight_decay") => "decay",
                        _ => name,
                    }
                })
                .collect();
            assert_eq!(args, names, "{kernel}: Rust/CUDA argument order");
        }
    }
}
