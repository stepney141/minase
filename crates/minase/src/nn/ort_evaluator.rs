//! ONNX Runtimeによる[`Evaluator`]の実装。
//!
//! ONNX Runtimeの共有ライブラリは、ビルド時ではなくセッションの作成時に環境変数`ORT_DYLIB_PATH`から
//! 読み込む。ネットは`crates/minase/nets/zero.onnx`を実行ファイルへ埋め込み、メモリからセッションを作る
//! （docs/plans/alphazero.md「推論はortのload-dynamicで行う」「推論層」）。埋め込み以外のネットは、
//! 呼び出し側が[`read_model`]でファイルから読んで同じ形で渡す。
//! 選んだ実行プロバイダを初期化できない場合は、別の実行プロバイダへ切り替えずにエラーを返す。

use core::fmt;
use core::num::NonZeroUsize;
use core::str::FromStr;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock, PoisonError};

use minase_core::{BOARD_FILES, BOARD_RANKS};
use ort::ep::{self, ExecutionProviderDispatch};
use ort::session::{Session, SessionOutputs};
use ort::value::TensorRef;
use sha2::{Digest, Sha256};

use super::INPUT_PLANES;
use super::evaluator::{Evaluator, EvaluatorError};

/// 実行ファイルに埋め込んだネット（ONNX）。
pub const NETWORK: &[u8] = include_bytes!("../../nets/zero.onnx");

/// ネットの入力の名前。形状は`[N, 96, 12, 12]`（tools/train/zero/README.md「ONNXへの出力」）。
const INPUT_NAME: &str = "input";

/// 方策logitの出力の名前。形状は`[N, 35856]`。
const POLICY_NAME: &str = "policy";

/// 勝率の出力の名前。形状は`[N, 1]`。
const VALUE_NAME: &str = "value";

/// 推論を任せる実行プロバイダ。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NnBackend {
    /// TensorRT（FP16）。
    TensorRt,
    /// CUDA。
    Cuda,
    /// CPU。
    Cpu,
}

impl NnBackend {
    /// USIオプション`NnBackend`の値として受理する名前を宣言の順に並べた表。
    pub const NAMES: [&'static str; 3] = ["tensorrt", "cuda", "cpu"];

    /// USIオプションの値として使う名前を返す。
    pub const fn name(self) -> &'static str {
        match self {
            Self::TensorRt => Self::NAMES[0],
            Self::Cuda => Self::NAMES[1],
            Self::Cpu => Self::NAMES[2],
        }
    }
}

impl fmt::Display for NnBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// 実行プロバイダの名前として解釈できない文字列。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UnknownNnBackend(String);

impl fmt::Display for UnknownNnBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unknown NnBackend '{}' (expected {})",
            self.0,
            NnBackend::NAMES.join(", ")
        )
    }
}

impl std::error::Error for UnknownNnBackend {}

impl FromStr for NnBackend {
    type Err = UnknownNnBackend;

    /// 名前を大文字小文字を区別せずに解釈する。USIのcombo値の慣例に合わせる。
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        [Self::TensorRt, Self::Cuda, Self::Cpu]
            .into_iter()
            .find(|backend| backend.name().eq_ignore_ascii_case(input))
            .ok_or_else(|| UnknownNnBackend(input.to_owned()))
    }
}

/// セッションの作成に使う設定。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SessionConfig {
    /// 実行プロバイダ。
    pub backend: NnBackend,
    /// TensorRTのエンジンのキャッシュを置く親ディレクトリ。TensorRTでだけ使う。
    pub cache_dir: PathBuf,
    /// 1回の推論にまとめる局面数の上限。TensorRTの最適化プロファイルの最大値と基準値にする。
    pub max_batch: NonZeroUsize,
}

/// セッションを作れない理由。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OrtError {
    /// 環境変数`ORT_DYLIB_PATH`が設定されていない。
    MissingDylibPath,
    /// ONNX Runtimeの共有ライブラリを読み込めない。
    Load(String),
    /// TensorRTのエンジンのキャッシュを置くディレクトリが指定されていない。
    MissingCacheDir,
    /// GPUの機種名、メモリ容量、または計算能力を取得できない。
    GpuInfo(String),
    /// TensorRTの版を特定できない。
    TensorRtVersion(String),
    /// エンジンのキャッシュのディレクトリを作れない。
    CacheDir(String),
    /// セッションを作れない。実行プロバイダの初期化の失敗を含む。
    Session(String),
    /// ネットの入出力の名前が想定と異なる。
    Interface(String),
    /// ネットのファイルを読めない、またはファイルが空である。
    Model(String),
}

impl fmt::Display for OrtError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDylibPath => formatter.write_str(
                "ORT_DYLIB_PATH must name the ONNX Runtime shared library to run the network",
            ),
            Self::Load(reason) => write!(formatter, "cannot load ONNX Runtime: {reason}"),
            Self::MissingCacheDir => formatter.write_str(
                "NnCacheDir is empty; set it, XDG_CACHE_HOME, or HOME to cache TensorRT engines",
            ),
            Self::GpuInfo(reason) => write!(formatter, "cannot query the GPU: {reason}"),
            Self::TensorRtVersion(reason) => {
                write!(formatter, "cannot determine the TensorRT version: {reason}")
            }
            Self::CacheDir(reason) => {
                write!(
                    formatter,
                    "cannot create the TensorRT engine cache: {reason}"
                )
            }
            Self::Session(reason) => write!(
                formatter,
                "cannot create the ONNX Runtime session: {reason}"
            ),
            Self::Interface(reason) => write!(formatter, "unexpected network interface: {reason}"),
            Self::Model(reason) => write!(formatter, "cannot read the network: {reason}"),
        }
    }
}

impl std::error::Error for OrtError {}

/// 埋め込み以外のネット（ONNX）をファイルから読む。
///
/// USIオプション`NnModelPath`と生成器の`--nn-model`が使う（docs/plans/alphazero.md「推論層」）。
/// 中身の形式はセッションの作成時にONNX Runtimeが検査する。
///
/// # Errors
///
/// ファイルを読めない場合と、ファイルが空の場合はエラーを返す。
pub fn read_model(path: &Path) -> Result<Vec<u8>, OrtError> {
    let model =
        fs::read(path).map_err(|error| OrtError::Model(format!("{}: {error}", path.display())))?;
    if model.is_empty() {
        return Err(OrtError::Model(format!("{} is empty", path.display())));
    }
    Ok(model)
}

/// TensorRTのエンジンが依存するGPUの属性。値は`nvidia-smi`の表示のまま持つ。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GpuInfo {
    /// 機種名。
    pub name: String,
    /// メモリ容量。
    pub memory_total: String,
    /// 計算能力（compute capability）。
    pub compute_capability: String,
}

/// TensorRTのエンジンのキャッシュの鍵を返す。
///
/// ネットの検査和、GPUの機種名、メモリ容量、計算能力、およびTensorRTの版を、各要素の長さを前置して
/// SHA-256にかけた値の16進表記である（docs/plans/alphazero.md「推論層」）。
/// 長さを前置するので、要素の境界をずらした組が同じ鍵になることはない。
pub fn cache_key(network_sha256: &[u8], gpu: &GpuInfo, tensorrt_version: &str) -> String {
    let mut hasher = Sha256::new();
    for field in [
        network_sha256,
        gpu.name.as_bytes(),
        gpu.memory_total.as_bytes(),
        gpu.compute_capability.as_bytes(),
        tensorrt_version.as_bytes(),
    ] {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// プロセスで1回だけ行う、ONNX Runtimeの共有ライブラリの読み込みと環境の設定の結果。
static RUNTIME: OnceLock<Result<(), OrtError>> = OnceLock::new();

/// セッションの作成を直列化する錠。TensorRTの実行プロバイダは、複数のスレッドが同時にセッションを作ると
/// プラグインの演算子の領域`trt.plugins`を重ねて登録して失敗し得るからである。
static SESSION_CREATION: Mutex<()> = Mutex::new(());

/// ONNX Runtimeのセッションで推論する評価器。
pub struct OrtEvaluator {
    /// 推論のセッション。
    session: Session,
}

impl OrtEvaluator {
    /// ONNX Runtimeを読み込み、ネット`model`から指定の実行プロバイダのセッションを作る。
    ///
    /// 共有ライブラリの読み込みと環境の設定はプロセスで最初の呼び出しだけが行い、その結果を以後の呼び出しでも返す。
    /// セッションの作成はプロセス全体で1つずつ行う。
    ///
    /// TensorRTでは、エンジンのキャッシュを`cache_dir`の下の[`cache_key`]のディレクトリに置き
    /// （鍵のネットの検査和は、埋め込みのネットではなく`model`のSHA-256とする）、
    /// FP16を有効にして、バッチの大きさ1から`max_batch`までの最適化プロファイルを作る。
    /// 推論のスレッドは1つとする（docs/plans/alphazero.md「棋力測定」のCPUのスレッドTに合わせる）。
    ///
    /// # Errors
    ///
    /// ONNX Runtimeを読み込めない場合、TensorRTのキャッシュの鍵を作れない場合、選んだ実行プロバイダを
    /// 初期化できない場合、およびネットの入出力の名前が想定と異なる場合はエラーを返す。
    pub fn new(model: &[u8], config: &SessionConfig) -> Result<Self, OrtError> {
        RUNTIME.get_or_init(load_runtime).clone()?;
        let provider = execution_provider(model, config)?;
        let session_error = |error: ort::Error<_>| OrtError::Session(error.to_string());
        let creation = SESSION_CREATION
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let session = Session::builder()
            .map_err(|error| OrtError::Session(error.to_string()))?
            .with_intra_threads(1)
            .map_err(session_error)?
            .with_execution_providers([provider])
            .map_err(session_error)?
            .commit_from_memory(model)
            .map_err(|error| OrtError::Session(error.to_string()))?;
        drop(creation);
        check_interface(&session)?;
        Ok(Self { session })
    }
}

/// 環境変数`ORT_DYLIB_PATH`の共有ライブラリを読み込み、ONNX Runtimeの環境を作って、プロセスの終了まで保持する。
///
/// ortは、実行ファイルの`.fini_array`に置いた関数で、大域に保持する環境への参照を終了時に手放して環境を解放する。
/// その時点では、`dlopen`で読み込んだONNX RuntimeとTensorRTのライブラリのC++の静的デストラクタが
/// `__cxa_atexit`の登録順の逆に先に走り得る。後始末を終えたライブラリの上で環境を解放すると、glibcが
/// `corrupted double-linked list`で異常終了する（終了コード134）。そこで、環境への参照を1つ
/// [`std::mem::forget`]で意図的に漏らし、終了時に手放される参照が最後の参照にならないようにする。
/// 環境は解放されないまま、OSがプロセスとともに回収する。CPUとCUDAの実行プロバイダではこの異常終了は
/// 起きないが、プロバイダによらず同じ扱いにする。
fn load_runtime() -> Result<(), OrtError> {
    let dylib = env::var_os("ORT_DYLIB_PATH")
        .filter(|path| !path.is_empty())
        .ok_or(OrtError::MissingDylibPath)?;
    ort::init_from(&dylib)
        .map_err(|error| OrtError::Load(error.to_string()))?
        .commit();
    let environment = ort::environment::Environment::current()
        .map_err(|error| OrtError::Load(error.to_string()))?;
    std::mem::forget(environment);
    Ok(())
}

/// 設定の実行プロバイダを、初期化できなければエラーとする形で作る。
fn execution_provider(
    model: &[u8],
    config: &SessionConfig,
) -> Result<ExecutionProviderDispatch, OrtError> {
    let provider = match config.backend {
        NnBackend::TensorRt => {
            if config.cache_dir.as_os_str().is_empty() {
                return Err(OrtError::MissingCacheDir);
            }
            let network_sha256 = Sha256::digest(model);
            let key = cache_key(&network_sha256, &query_gpu()?, &tensorrt_version()?);
            let cache = config.cache_dir.join(key);
            fs::create_dir_all(&cache)
                .map_err(|error| OrtError::CacheDir(format!("{}: {error}", cache.display())))?;
            let shape = |batch: usize| {
                format!("{INPUT_NAME}:{batch}x{INPUT_PLANES}x{BOARD_RANKS}x{BOARD_FILES}")
            };
            let max = shape(config.max_batch.get());
            ep::TensorRT::default()
                .with_fp16(true)
                .with_engine_cache(true)
                .with_engine_cache_path(cache.display())
                .with_profile_min_shapes(shape(1))
                .with_profile_opt_shapes(&max)
                .with_profile_max_shapes(&max)
                .build()
        }
        NnBackend::Cuda => ep::CUDA::default().build(),
        NnBackend::Cpu => ep::CPU::default().build(),
    };
    Ok(provider.error_on_failure())
}

/// セッションの入出力の名前が[`INPUT_NAME`]、[`POLICY_NAME`]、[`VALUE_NAME`]であることを確かめる。
fn check_interface(session: &Session) -> Result<(), OrtError> {
    let names = |outlets: &[ort::value::Outlet]| {
        outlets
            .iter()
            .map(|outlet| outlet.name().to_owned())
            .collect::<Vec<_>>()
    };
    let (inputs, outputs) = (names(session.inputs()), names(session.outputs()));
    if inputs != [INPUT_NAME]
        || outputs.len() != 2
        || !outputs.iter().any(|name| name == POLICY_NAME)
        || !outputs.iter().any(|name| name == VALUE_NAME)
    {
        return Err(OrtError::Interface(format!(
            "inputs {inputs:?} and outputs {outputs:?} (expected [\"{INPUT_NAME}\"] and \
             [\"{POLICY_NAME}\", \"{VALUE_NAME}\"])"
        )));
    }
    Ok(())
}

/// ONNX Runtimeが使うGPU（`CUDA_VISIBLE_DEVICES`の先頭、未設定なら番号0）の属性を`nvidia-smi`で調べる。
fn query_gpu() -> Result<GpuInfo, OrtError> {
    let device = env::var("CUDA_VISIBLE_DEVICES")
        .ok()
        .and_then(|devices| devices.split(',').next().map(str::trim).map(str::to_owned))
        .unwrap_or_else(|| "0".to_owned());
    if device.is_empty() {
        return Err(OrtError::GpuInfo(
            "CUDA_VISIBLE_DEVICES hides every GPU".to_owned(),
        ));
    }
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total,compute_cap",
            "--format=csv,noheader",
            "-i",
            &device,
        ])
        .output()
        .map_err(|error| OrtError::GpuInfo(format!("cannot run nvidia-smi: {error}")))?;
    if !output.status.success() {
        return Err(OrtError::GpuInfo(format!(
            "nvidia-smi exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    parse_gpu_info(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| OrtError::GpuInfo("unexpected nvidia-smi output".to_owned()))
}

/// `nvidia-smi --query-gpu=name,memory.total,compute_cap --format=csv,noheader`の1行を読む。
fn parse_gpu_info(output: &str) -> Option<GpuInfo> {
    let mut lines = output.lines().filter(|line| !line.trim().is_empty());
    let line = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    let mut fields = line.rsplitn(3, ',').map(str::trim);
    let compute_capability = fields.next()?;
    let memory_total = fields.next()?;
    let name = fields.next()?;
    [name, memory_total, compute_capability]
        .iter()
        .all(|field| !field.is_empty())
        .then(|| GpuInfo {
            name: name.to_owned(),
            memory_total: memory_total.to_owned(),
            compute_capability: compute_capability.to_owned(),
        })
}

/// 動的リンカの検索パスにあるTensorRTの版を返す。
///
/// `LD_LIBRARY_PATH`の各ディレクトリ、次に`ldconfig -p`の一覧から`libnvinfer.so.<主版>`を持つ
/// ディレクトリを探し、そのディレクトリにある`libnvinfer*.so.<主版>.<副版>.<修正版>`の版を返す。
/// aptで入れたTensorRTは`libnvinfer.so`自体が完全な版の名前を持ち、pipで入れたTensorRTは
/// 同じディレクトリの`libnvinfer_builder_resource*.so`が完全な版の名前を持つ。
fn tensorrt_version() -> Result<String, OrtError> {
    let from_env = env::var_os("LD_LIBRARY_PATH")
        .map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .find(|directory| directory_has_nvinfer(directory));
    let directory = match from_env {
        Some(directory) => directory,
        None => ldconfig_nvinfer_directory()?,
    };
    let mut versions = fs::read_dir(&directory)
        .map_err(|error| OrtError::TensorRtVersion(format!("{}: {error}", directory.display())))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| full_version(&entry.file_name()))
        .collect::<Vec<_>>();
    versions.sort_unstable();
    versions.dedup();
    match versions.as_slice() {
        [version] => Ok(version.clone()),
        [] => Err(OrtError::TensorRtVersion(format!(
            "{} has no libnvinfer library named with a full version",
            directory.display()
        ))),
        _ => Err(OrtError::TensorRtVersion(format!(
            "{} has libnvinfer libraries of versions {}",
            directory.display(),
            versions.join(", ")
        ))),
    }
}

/// ディレクトリが`libnvinfer.so.<版>`を持つかを返す。
fn directory_has_nvinfer(directory: &Path) -> bool {
    fs::read_dir(directory).is_ok_and(|entries| {
        entries.filter_map(|entry| entry.ok()).any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("libnvinfer.so."))
        })
    })
}

/// `ldconfig -p`の一覧から`libnvinfer.so.<版>`のあるディレクトリを返す。
fn ldconfig_nvinfer_directory() -> Result<PathBuf, OrtError> {
    let output = Command::new("ldconfig")
        .arg("-p")
        .output()
        .map_err(|error| OrtError::TensorRtVersion(format!("cannot run ldconfig: {error}")))?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.trim_start().starts_with("libnvinfer.so."))
        .find_map(|line| {
            let (_, path) = line.rsplit_once(" => ")?;
            Path::new(path.trim()).parent().map(Path::to_path_buf)
        })
        .ok_or_else(|| {
            OrtError::TensorRtVersion(
                "libnvinfer is in neither LD_LIBRARY_PATH nor the ldconfig cache".to_owned(),
            )
        })
}

/// `libnvinfer*.so.<主版>.<副版>.<修正版>`の形の名前から版を返す。
fn full_version(file_name: &OsStr) -> Option<String> {
    let name = file_name.to_str()?;
    if !name.starts_with("libnvinfer") {
        return None;
    }
    let (_, version) = name.split_once(".so.")?;
    let parts = version.split('.').collect::<Vec<_>>();
    (parts.len() >= 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())))
    .then(|| version.to_owned())
}

impl Evaluator for OrtEvaluator {
    fn evaluate(
        &mut self,
        inputs: &[f32],
        batch: usize,
        out_policy: &mut [f32],
        out_value: &mut [f32],
    ) -> Result<(), EvaluatorError> {
        let failed = |error: ort::Error| EvaluatorError::Inference(error.to_string());
        let shape = [
            batch,
            INPUT_PLANES,
            usize::from(BOARD_RANKS),
            usize::from(BOARD_FILES),
        ];
        let input = TensorRef::from_array_view((shape, inputs)).map_err(failed)?;
        let outputs = self
            .session
            .run(ort::inputs![INPUT_NAME => input])
            .map_err(failed)?;
        copy_output(&outputs, POLICY_NAME, out_policy)?;
        copy_output(&outputs, VALUE_NAME, out_value)
    }
}

/// 出力`name`を、要素数が`out`と一致することを確かめて写す。
fn copy_output(
    outputs: &SessionOutputs<'_>,
    name: &str,
    out: &mut [f32],
) -> Result<(), EvaluatorError> {
    let output = outputs
        .get(name)
        .ok_or_else(|| EvaluatorError::Inference(format!("the network has no output '{name}'")))?;
    let (_, values) = output
        .try_extract_tensor::<f32>()
        .map_err(|error| EvaluatorError::Inference(error.to_string()))?;
    if values.len() != out.len() {
        return Err(EvaluatorError::Inference(format!(
            "output '{name}' has {} values (expected {})",
            values.len(),
            out.len()
        )));
    }
    out.copy_from_slice(values);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu() -> GpuInfo {
        GpuInfo {
            name: "NVIDIA GeForce RTX 3060 Ti".to_owned(),
            memory_total: "8192 MiB".to_owned(),
            compute_capability: "8.6".to_owned(),
        }
    }

    // alphazero.md「検証」: キャッシュの鍵は、ネットの検査和、GPUの機種名、メモリ容量、計算能力、
    // またはTensorRTの版のどれか1つだけが異なる場合にも別の鍵になる。
    #[test]
    fn cache_key_changes_with_each_component() {
        let network = [1_u8; 32];
        let base = cache_key(&network, &gpu(), "10.14.1");
        assert_eq!(base, cache_key(&network, &gpu(), "10.14.1"));
        assert_eq!(base.len(), 64);
        let variants = [
            cache_key(&[2_u8; 32], &gpu(), "10.14.1"),
            cache_key(
                &network,
                &GpuInfo {
                    name: "NVIDIA A100-SXM4-80GB".to_owned(),
                    ..gpu()
                },
                "10.14.1",
            ),
            cache_key(
                &network,
                &GpuInfo {
                    memory_total: "16384 MiB".to_owned(),
                    ..gpu()
                },
                "10.14.1",
            ),
            cache_key(
                &network,
                &GpuInfo {
                    compute_capability: "8.9".to_owned(),
                    ..gpu()
                },
                "10.14.1",
            ),
            cache_key(&network, &gpu(), "10.15.0"),
        ];
        for (index, variant) in variants.iter().enumerate() {
            assert_ne!(*variant, base, "component {index}");
        }
    }

    // 同節: 要素の境界をずらしただけの組は同じ鍵にならない。
    #[test]
    fn cache_key_separates_component_boundaries() {
        let network = [1_u8; 32];
        let shifted = GpuInfo {
            name: "ab".to_owned(),
            memory_total: "c".to_owned(),
            compute_capability: "8.6".to_owned(),
        };
        let other = GpuInfo {
            name: "a".to_owned(),
            memory_total: "bc".to_owned(),
            compute_capability: "8.6".to_owned(),
        };
        assert_ne!(
            cache_key(&network, &shifted, "10"),
            cache_key(&network, &other, "10")
        );
    }

    #[test]
    fn gpu_info_is_parsed_from_one_csv_line() {
        assert_eq!(
            parse_gpu_info("NVIDIA GeForce RTX 3060 Ti, 8192 MiB, 8.6\n"),
            Some(gpu())
        );
        assert_eq!(parse_gpu_info(""), None);
        assert_eq!(parse_gpu_info("RTX, 8192 MiB\n"), None);
        assert_eq!(
            parse_gpu_info("A, 1 MiB, 8.6\nB, 1 MiB, 8.6\n"),
            None,
            "more than one GPU"
        );
    }

    #[test]
    fn full_versions_are_read_from_nvinfer_library_names() {
        for (name, expected) in [
            ("libnvinfer.so.10.14.1", Some("10.14.1")),
            (
                "libnvinfer_builder_resource_sm86.so.10.14.1",
                Some("10.14.1"),
            ),
            ("libnvinfer.so.10", None),
            ("libnvinfer.so.10.14", None),
            ("libnvonnxparser.so.10.14.1", None),
            ("libnvinfer.so.10.x.1", None),
        ] {
            assert_eq!(
                full_version(OsStr::new(name)).as_deref(),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn backend_names_round_trip() {
        for backend in [NnBackend::TensorRt, NnBackend::Cuda, NnBackend::Cpu] {
            assert_eq!(backend.name().parse(), Ok(backend));
            assert_eq!(backend.to_string(), backend.name());
        }
        assert_eq!("TensorRT".parse(), Ok(NnBackend::TensorRt));
        assert_eq!(
            "gpu".parse::<NnBackend>(),
            Err(UnknownNnBackend("gpu".to_owned()))
        );
    }

    // 共通の前提: 外部入力でパニックしない。ORT_DYLIB_PATHがなければ専用のエラーを返す。
    // 環境変数は他の試験と共有するので、設定されている環境ではこの試験を省く。
    #[test]
    fn missing_dylib_path_is_an_error() {
        if env::var_os("ORT_DYLIB_PATH").is_some() {
            return;
        }
        let config = SessionConfig {
            backend: NnBackend::Cpu,
            cache_dir: PathBuf::new(),
            max_batch: NonZeroUsize::new(1).unwrap(),
        };
        assert!(matches!(
            OrtEvaluator::new(NETWORK, &config),
            Err(OrtError::MissingDylibPath)
        ));
    }
}
