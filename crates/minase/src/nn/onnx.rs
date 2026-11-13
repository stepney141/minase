//! ONNX Runtimeを実行時に読み込み、ネットをメモリから構築してバッチ推論する。

use std::env;
use std::fs;
use std::io::{self, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use half::f16;
use ort::environment::Environment;
use ort::ep;
use ort::logging::LogLevel;
use ort::memory::{AllocationDevice, AllocatorType, MemoryInfo, MemoryType};
use ort::session::{IoBinding, Session, SessionOutputs};
use ort::value::{PrimitiveTensorElementType, TensorElementType, TensorRef, ValueType};
use sha2::{Digest, Sha256};

use super::policy_map::PolicyMap;
use super::tensorrt_cache::{TensorRtCacheKey, query_gpus, validate_cuda_visible_devices};
use super::{Evaluator, INPUT_PLANES, INPUT_SIZE, NnError, Query, encode, policy_label};

/// ONNX形式のネットのバイト列とSHA-256。
///
/// 検査和はTensorRTのエンジンのキャッシュの鍵と、自己対局生成器が教師データ（MNSD）の
/// ヘッダに書く「教師の重みの検査和」に使う。推論にも同じバイト列を渡す。
pub struct Network {
    bytes: Vec<u8>,
    sha256: [u8; 32],
}

impl Network {
    /// ONNXのファイルを読み、同じバイト列から検査和を求める。
    ///
    /// # Errors
    /// ファイルを読めなければ[`NnError::ReadNetwork`]を返す。
    pub fn read(path: &Path) -> Result<Self, NnError> {
        let bytes = fs::read(path).map_err(|source| NnError::ReadNetwork {
            path: path.to_owned(),
            source,
        })?;
        Ok(Self::from_bytes(bytes))
    }

    /// 埋め込みなどで与えられたバイト列からネットを作る。
    ///
    /// ONNXの妥当性と入出力の契約は[`OnnxEvaluator::new`]で検査する。
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let sha256 = Sha256::digest(&bytes).into();
        Self { bytes, sha256 }
    }

    /// ONNXのバイト列のSHA-256を返す。
    pub fn sha256(&self) -> &[u8; 32] {
        &self.sha256
    }

    /// セッションの構築に渡すONNXのバイト列を返す。
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// 推論に使用する実行プロバイダ。初期化に失敗しても別のプロバイダへ切り替えない。
pub enum ExecutionProvider {
    /// TensorRTのFP16。`cache_root`の下にエンジンのキャッシュを置く。
    TensorRt {
        /// 設定ごとのキャッシュ用ディレクトリを置く親ディレクトリ。
        cache_root: PathBuf,
        /// 推論に使うCUDAの機器番号。0から始まる。
        device_id: usize,
    },
    /// CUDAによる推論。
    Cuda {
        /// 推論に使うCUDAの機器番号。0から始まる。
        device_id: usize,
    },
    /// CPUによる推論。
    Cpu,
}

impl ExecutionProvider {
    /// USIオプション`NnBackend`と同じ実行プロバイダ名を返す。
    pub fn name(&self) -> &'static str {
        match self {
            Self::TensorRt { .. } => "tensorrt",
            Self::Cuda { .. } => "cuda",
            Self::Cpu => "cpu",
        }
    }
}

/// プロセスに1つだけ作るONNX Runtimeの環境。作成を直列化し、作った環境はプロセスの終了まで保持して解放しない。
///
/// ortは、環境を1回作ったプロセスでは2回目の作成を拒む。推論器を破棄して作り直す呼び出し元があるので、
/// 環境は推論器の寿命に結び付けず、ここに保持する。
/// 終了処理で環境を解放すると、ONNX RuntimeのCUDAの実行プロバイダが静的デストラクタで解放した登録表を
/// もう一度手放してヒープを壊すので、終了時にも解放しない。
/// 理由は`docs/adr/hold-the-onnx-runtime-environment-until-process-exit-through-ort-rc15.md`にある。
/// 作成に失敗したときは空のままにし、次の呼び出しで再試行できるようにする。
static RUNTIME: Mutex<Option<Environment>> = Mutex::new(None);

/// 保持している環境を返す。まだなければ、`ORT_DYLIB_PATH`の共有ライブラリを読み込んで環境を作る。
///
/// 作った環境は、返す前に[`RUNTIME`]へ入れる。呼び出し元の後続の処理が失敗しても、環境は失われない。
fn initialize_runtime() -> Result<Environment, NnError> {
    let mut runtime = RUNTIME
        .lock()
        .map_err(|_| NnError::RuntimeInitializationPoisoned)?;
    if let Some(environment) = runtime.as_ref() {
        return Ok(environment.clone());
    }
    let path = env::var_os("ORT_DYLIB_PATH")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(NnError::RuntimeLibraryPathUnset)?;
    let metadata = fs::metadata(&path).map_err(|source| NnError::RuntimeLibraryPath {
        path: path.clone(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(NnError::RuntimeLibraryNotFile { path });
    }
    // ortは相対パスを実行ファイルの位置からも探す。絶対パスで指定したファイルに限定する。
    let path = std::path::absolute(&path)
        .map_err(|source| NnError::RuntimeLibraryPath { path, source })?;
    let environment = ort::init_from(path)
        .map_err(NnError::RuntimeInitialization)?
        .with_logger(Arc::new(|level, _category, _id, location, message| {
            if level >= LogLevel::Warning {
                // FFIから呼ばれるロガーなので、標準エラーの書き込み失敗でもパニックしない。
                let _ = writeln!(io::stderr().lock(), "[{level:?}] {location}: {message}");
            }
        }))
        .build()?;
    Ok(runtime.insert(environment).clone())
}

/// ONNX Runtimeのセッションと、最大バッチ分を一度だけ確保した入力バッファを持つ評価器。
///
/// TensorRTとCUDAでは、1つの[`IoBinding`]を保持し、出力をCUDAのピン留めメモリで受け取る。
/// 出力の結び付けを保持するとONNX Runtime 1.31.1が2回目の呼び出しで同じ領域への転送を
/// 拒むため、呼び出しごとに出力の結び付けを消して結び付け直す。
/// 入力は通常のホストメモリに置き、CPUでは`session.run`で出力を受け取る。
pub struct OnnxEvaluator {
    session: Session,
    // CUDAのピン留めメモリの情報を保持し、出力を結び付け直すたびに再利用する。
    output_binding: Option<(IoBinding, MemoryInfo<'static>)>,
    policy_map: PolicyMap,
    input: Vec<u8>,
    max_batch: NonZeroUsize,
}

impl OnnxEvaluator {
    /// 明示された実行プロバイダでネットをメモリから読み、入出力の契約を検査する。
    ///
    /// `ORT_DYLIB_PATH`が示すファイルからONNX Runtimeをプロセスで1回だけ初期化する。
    /// TensorRTとCUDAではCPUへの部分的な退避も禁じる。
    /// TensorRTとCUDAでは`CUDA_VISIBLE_DEVICES`が設定されていれば、空値でも起動を拒む。
    /// 入力はu8の`input: [動的バッチ, 159, 12, 12]`、出力はf16の
    /// `policy: [動的バッチ, 正の固定幅]`とf32の`value: [動的バッチ, 1]`でなければならない。
    /// メタデータ`policy_labels`には、詰める前のラベルを10進整数のカンマ区切りで指定する。
    /// 表は厳密な昇順で、各ラベルは35,856未満、要素数は方策の固定幅と一致しなければならない。
    ///
    /// # Errors
    /// ライブラリ、実行プロバイダ、キャッシュ、バッファ確保、ネットの入出力、またはラベルの表に
    /// 問題があれば[`NnError`]を返す。ほかのプロバイダやネットへの切り替えは行わない。
    pub fn new(
        network: &Network,
        provider: &ExecutionProvider,
        max_batch: NonZeroUsize,
    ) -> Result<Self, NnError> {
        if !matches!(provider, ExecutionProvider::Cpu) {
            validate_cuda_visible_devices(env::var_os("CUDA_VISIBLE_DEVICES").as_deref())?;
        }
        let input = allocate_input(max_batch)?;
        let environment = initialize_runtime()?;
        let mut builder = Session::builder(&environment)?
            .with_log_level(LogLevel::Warning)
            .map_err(ort::Error::from)?;
        if !matches!(provider, ExecutionProvider::Cpu) {
            builder = builder
                .with_disable_cpu_fallback()
                .map_err(ort::Error::from)?;
        }
        let (provider, output_device_id) = match provider {
            ExecutionProvider::TensorRt {
                cache_root,
                device_id,
            } => {
                let device_id = ort_device_id(*device_id)?;
                let key = TensorRtCacheKey::new(*network.sha256(), query_gpus()?.gpu, max_batch);
                let path = cache_root.join(key.directory_name());
                let cache_path = path
                    .to_str()
                    .ok_or_else(|| NnError::CachePathNotUtf8 { path: path.clone() })?;
                fs::create_dir_all(&path).map_err(|source| NnError::CreateCacheDir {
                    path: path.clone(),
                    source,
                })?;
                let profile = format!("input:{}x{INPUT_PLANES}x12x12", max_batch.get());
                (
                    ep::TensorRT::default()
                        .with_device_id(device_id)
                        .with_profile_min_shapes(format!("input:1x{INPUT_PLANES}x12x12"))
                        .with_profile_opt_shapes(&profile)
                        .with_profile_max_shapes(&profile)
                        .with_fp16(true)
                        .with_engine_cache(true)
                        .with_engine_cache_path(cache_path)
                        .with_timing_cache(false)
                        .build(),
                    Some(device_id),
                )
            }
            ExecutionProvider::Cuda { device_id } => {
                let device_id = ort_device_id(*device_id)?;
                (
                    ep::CUDA::default().with_device_id(device_id).build(),
                    Some(device_id),
                )
            }
            ExecutionProvider::Cpu => (ep::CPU::default().build(), None),
        };
        let session = builder
            .with_execution_providers([provider.error_on_failure()])
            .map_err(ort::Error::from)?
            .commit_from_memory(network.bytes())?;
        let policy_width = validate_signature(
            session
                .inputs()
                .iter()
                .map(|port| (port.name().to_owned(), port.dtype().clone()))
                .collect(),
            session
                .outputs()
                .iter()
                .map(|port| (port.name().to_owned(), port.dtype().clone()))
                .collect(),
        )?;
        let labels = session.metadata()?.custom("policy_labels");
        let policy_map = PolicyMap::from_metadata(labels.as_deref(), policy_width)?;
        let output_binding = match output_device_id {
            Some(device_id) => Some((
                session.create_binding()?,
                MemoryInfo::new(
                    AllocationDevice::CUDA_PINNED,
                    device_id,
                    AllocatorType::Device,
                    MemoryType::CPUOutput,
                )?,
            )),
            None => None,
        };
        Ok(Self {
            session,
            output_binding,
            policy_map,
            input,
            max_batch,
        })
    }
}

fn ort_device_id(device_id: usize) -> Result<i32, NnError> {
    i32::try_from(device_id).map_err(|_| NnError::InvalidDeviceId { device_id })
}

fn allocate_input(max_batch: NonZeroUsize) -> Result<Vec<u8>, NnError> {
    let max = max_batch.get();
    let len = max
        .checked_mul(INPUT_SIZE)
        .ok_or(NnError::InvalidMaxBatch { max })?;
    let mut input = Vec::new();
    input
        .try_reserve_exact(len)
        .map_err(|source| NnError::AllocateInput { max, source })?;
    input.resize(len, 0);
    Ok(input)
}

fn validate_signature(
    inputs: Vec<(String, ValueType)>,
    outputs: Vec<(String, ValueType)>,
) -> Result<usize, NnError> {
    let matches = |ports: &[(String, ValueType)], name: &str, element_type, expected: &[i64]| {
        ports.iter().any(|(actual_name, dtype)| {
            actual_name == name
                && matches!(
                    dtype,
                    ValueType::Tensor { ty, shape, .. }
                        if *ty == element_type && &shape[..] == expected
                )
        })
    };
    let policy_width = outputs.iter().find_map(|(name, dtype)| {
        if name == "policy"
            && let ValueType::Tensor {
                ty: TensorElementType::Float16,
                shape,
                ..
            } = dtype
            && let [-1, width] = &shape[..]
        {
            usize::try_from(*width).ok().filter(|&width| width > 0)
        } else {
            None
        }
    });
    if inputs.len() == 1
        && outputs.len() == 2
        && matches(
            &inputs,
            "input",
            TensorElementType::Uint8,
            &[-1, INPUT_PLANES as i64, 12, 12],
        )
        && matches(&outputs, "value", TensorElementType::Float32, &[-1, 1])
        && let Some(width) = policy_width
    {
        return Ok(width);
    }
    Err(NnError::ModelSignature {
        expected: "input u8 [-1, 159, 12, 12]; policy f16 [-1, width] (fixed width > 0), value f32 [-1, 1]",
        inputs,
        outputs,
    })
}

fn output_tensor<'a, T: PrimitiveTensorElementType>(
    outputs: &'a SessionOutputs<'_>,
    name: &'static str,
    expected: [usize; 2],
) -> Result<&'a [T], NnError> {
    let output = outputs.get(name).ok_or(NnError::MissingOutput { name })?;
    let (shape, values) = output.try_extract_tensor::<T>()?;
    if **shape != expected.map(|dimension| dimension as i64) {
        return Err(NnError::OutputShape {
            name,
            expected,
            actual: shape.to_vec(),
        });
    }
    Ok(values)
}

impl Evaluator for OnnxEvaluator {
    fn evaluate(
        &mut self,
        queries: &[Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        logits.clear();
        values.clear();
        let n = queries.len();
        if n == 0 {
            return Ok(());
        }
        if n > self.max_batch.get() {
            return Err(NnError::BatchTooLarge {
                len: n,
                max: self.max_batch.get(),
            });
        }
        let input = &mut self.input[..n * INPUT_SIZE];
        for (query, buffer) in queries.iter().zip(input.as_chunks_mut::<INPUT_SIZE>().0) {
            encode(query.position, buffer);
        }
        let tensor = TensorRef::from_array_view(([n, INPUT_PLANES, 12, 12], &*input))?;
        let outputs = if let Some((binding, memory)) = &mut self.output_binding {
            binding.bind_input("input", &tensor)?;
            binding.clear_outputs();
            binding.bind_output_to_device("policy", memory)?;
            binding.bind_output_to_device("value", memory)?;
            binding.synchronize_inputs()?;
            let outputs = self.session.run_binding(binding)?;
            binding.synchronize_outputs()?;
            outputs
        } else {
            self.session.run(ort::inputs!["input" => tensor])?
        };
        let policy = output_tensor::<f16>(&outputs, "policy", [n, self.policy_map.width()])?;
        let value = output_tensor::<f32>(&outputs, "value", [n, 1])?;
        collect_outputs(queries, &self.policy_map, policy, value, logits, values)
    }
}

/// 形を検査済みの出力から、依頼された合法手のlogitだけをf32へ変換し、価値とともに検査して取り出す。
fn collect_outputs(
    queries: &[Query<'_>],
    policy_map: &PolicyMap,
    policy: &[f16],
    value: &[f32],
    logits: &mut Vec<f32>,
    values: &mut Vec<f32>,
) -> Result<(), NnError> {
    for (index, query) in queries.iter().enumerate() {
        let value = value[index];
        if !value.is_finite() {
            return Err(NnError::NonFiniteOutput { query: index });
        }
        if !(0.0..=1.0).contains(&value) {
            return Err(NnError::ValueOutOfRange {
                query: index,
                value,
            });
        }
        for &mv in query.moves {
            let label = policy_label(mv, query.position.side_to_move());
            let packed = policy_map
                .packed_index(label)
                .ok_or(NnError::MissingPolicyLabel {
                    query: index,
                    label,
                })?;
            let logit = policy[index * policy_map.width() + packed].to_f32();
            if !logit.is_finite() {
                return Err(NnError::NonFiniteOutput { query: index });
            }
            logits.push(logit);
        }
        values.push(value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase_core::test_util::{position, sq};
    use minase_core::{Color, Move, PieceKind};
    use ort::value::{Shape, SymbolicDimensions};

    // ortの契約: CUDAの機器番号は非負のi32とし、表せない番号は型付きエラーにする。
    #[test]
    fn device_ids_must_fit_ort_signed_integers() {
        for (input, expected) in [(0, 0), (3, 3), (i32::MAX as usize, i32::MAX)] {
            assert_eq!(ort_device_id(input).unwrap(), expected);
        }
        for invalid in [i32::MAX as usize + 1, usize::MAX] {
            assert!(matches!(
                ort_device_id(invalid),
                Err(NnError::InvalidDeviceId { device_id }) if device_id == invalid
            ));
        }
    }

    // [`Network`]の契約: バイト列を保持し、SHA-256の既知の値を返す。
    #[test]
    fn network_hash_matches_sha256_test_vector() {
        let network = Network::from_bytes(b"abc".to_vec());
        assert_eq!(network.bytes(), b"abc");
        assert_eq!(
            *network.sha256(),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad,
            ]
        );
    }

    // [`Network::read`]の契約: 存在しないファイルではパスと元の入出力エラーを保持する。
    #[test]
    fn missing_network_preserves_read_error() {
        let path = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/Cargo.toml/missing.onnx"
        ));
        let Err(NnError::ReadNetwork {
            path: actual,
            source,
        }) = Network::read(path)
        else {
            panic!("expected a network read error");
        };
        assert_eq!(actual, path);
        assert!(matches!(
            source.kind(),
            io::ErrorKind::NotADirectory | io::ErrorKind::NotFound
        ));
    }

    fn tensor_port(name: &str, shape: &[i64], ty: TensorElementType) -> (String, ValueType) {
        (
            name.to_owned(),
            ValueType::Tensor {
                ty,
                shape: Shape::new(shape.iter().copied()),
                dimension_symbols: SymbolicDimensions::empty(shape.len()),
            },
        )
    }

    // [`OnnxEvaluator::new`]の契約: 名前・入力u8・方策f16・価値f32・動的バッチ・固定幅を検査する。
    #[test]
    fn model_signature_requires_uint8_input_fp16_policy_fp32_value_and_dynamic_batches() {
        let input = tensor_port("input", &[-1, 159, 12, 12], TensorElementType::Uint8);
        let outputs = vec![
            tensor_port("value", &[-1, 1], TensorElementType::Float32),
            tensor_port("policy", &[-1, 3], TensorElementType::Float16),
        ];
        for width in [1, 3, 35856] {
            assert_eq!(
                validate_signature(
                    vec![input.clone()],
                    vec![
                        outputs[0].clone(),
                        tensor_port("policy", &[-1, width], TensorElementType::Float16),
                    ],
                )
                .unwrap(),
                width as usize
            );
        }
        for inputs in [
            vec![],
            vec![input.clone(), input.clone()],
            vec![tensor_port(
                "other",
                &[-1, 159, 12, 12],
                TensorElementType::Uint8,
            )],
            vec![tensor_port(
                "input",
                &[7, 159, 12, 12],
                TensorElementType::Uint8,
            )],
            vec![tensor_port(
                "input",
                &[-1, -1, 12, 12],
                TensorElementType::Uint8,
            )],
            vec![tensor_port(
                "input",
                &[-1, 159, 12, 12],
                TensorElementType::Float16,
            )],
            vec![tensor_port(
                "input",
                &[-1, 159, 12, 12],
                TensorElementType::Float32,
            )],
        ] {
            assert!(matches!(
                validate_signature(inputs, outputs.clone()),
                Err(NnError::ModelSignature { expected, .. })
                    if expected == "input u8 [-1, 159, 12, 12]; policy f16 [-1, width] (fixed width > 0), value f32 [-1, 1]"
            ));
        }
        for invalid in [
            tensor_port("other", &[-1, 3], TensorElementType::Float16),
            tensor_port("policy", &[-1, 0], TensorElementType::Float16),
            tensor_port("policy", &[-1, -1], TensorElementType::Float16),
            tensor_port("policy", &[-1, -2], TensorElementType::Float16),
            tensor_port("policy", &[1, 3], TensorElementType::Float16),
            tensor_port("policy", &[-1], TensorElementType::Float16),
            tensor_port("policy", &[-1, 3, 1], TensorElementType::Float16),
            tensor_port("policy", &[-1, 3], TensorElementType::Float32),
        ] {
            assert!(matches!(
                validate_signature(vec![input.clone()], vec![outputs[0].clone(), invalid]),
                Err(NnError::ModelSignature { .. })
            ));
        }
        for invalid in [
            tensor_port("other", &[-1, 1], TensorElementType::Float32),
            tensor_port("value", &[-1, 2], TensorElementType::Float32),
            tensor_port("value", &[1, 1], TensorElementType::Float32),
            tensor_port("value", &[-1, 1], TensorElementType::Float16),
        ] {
            assert!(matches!(
                validate_signature(vec![input.clone()], vec![invalid, outputs[1].clone()]),
                Err(NnError::ModelSignature { .. })
            ));
        }
        assert!(matches!(
            validate_signature(vec![input], vec![outputs[0].clone()]),
            Err(NnError::ModelSignature { .. })
        ));
    }

    // [`Evaluator`]の契約: 依頼順・着手順に集め、未使用のlogitを検査せず、勝率0と1を受理する。
    #[test]
    fn gathers_only_requested_logits_in_query_order() {
        let black = position(Color::Black, &[(sq(5, 5), Color::Black, PieceKind::Rook)]);
        let white = position(Color::White, &[(sq(5, 5), Color::White, PieceKind::Rook)]);
        let moves = [
            Move {
                from: sq(5, 5),
                mid: None,
                to: sq(6, 5),
                promote: false,
            },
            Move {
                from: sq(5, 5),
                mid: None,
                to: sq(5, 6),
                promote: false,
            },
        ];
        let queries = [
            Query {
                position: &black,
                moves: &moves,
            },
            Query {
                position: &white,
                moves: &[],
            },
            Query {
                position: &white,
                moves: &moves[..1],
            },
        ];
        // 方策ラベルの仕様表: 南へ1歩は面0、東へ1歩は面22、後手から見た西へ1歩は面66。
        // 相対移動元は先手65、後手78なので、元のラベルは65、3233、9582である。
        let policy_map = PolicyMap::from_metadata(Some("0,65,3233,9582,35855"), 5).unwrap();
        let mut policy = vec![f16::NAN; 3 * 5];
        policy[1] = f16::from_f32(2.0);
        policy[2] = f16::from_f32(-3.0);
        policy[2 * 5 + 3] = f16::from_f32(4.0);
        let mut logits = Vec::new();
        let mut values = Vec::new();
        collect_outputs(
            &queries,
            &policy_map,
            &policy,
            &[0.0, 0.5, 1.0],
            &mut logits,
            &mut values,
        )
        .unwrap();
        assert_eq!(logits, [-3.0, 2.0, 4.0]);
        assert_eq!(values, [0.0, 0.5, 1.0]);
    }

    // メタデータの契約: 合法手が表にないときは、依頼番号と元のラベルを持つエラーを返す。
    #[test]
    fn rejects_requested_labels_missing_from_the_policy_map() {
        let position = position(Color::Black, &[(sq(5, 5), Color::Black, PieceKind::Rook)]);
        let moves = [Move {
            from: sq(5, 5),
            mid: None,
            to: sq(5, 6),
            promote: false,
        }];
        let queries = [
            Query {
                position: &position,
                moves: &[],
            },
            Query {
                position: &position,
                moves: &moves,
            },
        ];
        // 南へ1歩は面0、移動元の相対升は65。表の64と66の間に目的のラベルはない。
        let policy_map = PolicyMap::from_metadata(Some("64,66"), 2).unwrap();
        assert!(matches!(
            collect_outputs(
                &queries,
                &policy_map,
                &[f16::ZERO; 4],
                &[0.5, 0.5],
                &mut Vec::new(),
                &mut Vec::new(),
            ),
            Err(NnError::MissingPolicyLabel {
                query: 1,
                label: 65
            })
        ));
    }

    // [`Evaluator`]の契約: 合法手の非有限logit、非有限の価値、および範囲外の価値を区別して拒む。
    #[test]
    fn rejects_nonfinite_outputs_and_out_of_range_values() {
        let position = position(Color::Black, &[(sq(5, 5), Color::Black, PieceKind::Rook)]);
        let moves = [Move {
            from: sq(5, 5),
            mid: None,
            to: sq(5, 6),
            promote: false,
        }];
        let queries = [Query {
            position: &position,
            moves: &moves,
        }];
        // 南へ1歩の面0と相対升65から得るラベルを、幅1の出力に対応付ける。
        let policy_map = PolicyMap::from_metadata(Some("65"), 1).unwrap();
        let mut policy = [f16::ZERO];
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            policy[0] = f16::from_f32(invalid);
            assert!(matches!(
                collect_outputs(
                    &queries,
                    &policy_map,
                    &policy,
                    &[0.5],
                    &mut Vec::new(),
                    &mut Vec::new()
                ),
                Err(NnError::NonFiniteOutput { query: 0 })
            ));
            policy[0] = f16::ZERO;
            assert!(matches!(
                collect_outputs(
                    &queries,
                    &policy_map,
                    &policy,
                    &[invalid],
                    &mut Vec::new(),
                    &mut Vec::new()
                ),
                Err(NnError::NonFiniteOutput { query: 0 })
            ));
        }
        for invalid in [-0.01, 1.01] {
            assert!(matches!(
                collect_outputs(&queries, &policy_map, &policy, &[invalid], &mut Vec::new(), &mut Vec::new()),
                Err(NnError::ValueOutOfRange { query: 0, value }) if value == invalid
            ));
        }
    }

    // [`OnnxEvaluator::new`]の契約: 過大な外部指定でバッファ要素数が桁あふれしてもパニックしない。
    #[test]
    fn rejects_unrepresentable_input_buffer() {
        assert!(matches!(
            allocate_input(NonZeroUsize::new(usize::MAX).unwrap()),
            Err(NnError::InvalidMaxBatch { max: usize::MAX })
        ));
    }
}
