//! ONNX Runtimeを実行時に読み込み、ネットをメモリから構築してバッチ推論する。

use std::env;
use std::fs;
use std::io::{self, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ort::ep;
use ort::logging::LogLevel;
use ort::session::{Session, SessionOutputs};
use ort::value::{TensorElementType, TensorRef, ValueType};
use sha2::{Digest, Sha256};

use super::tensorrt_cache::{TensorRtCacheKey, query_gpu};
use super::{
    Evaluator, INPUT_PLANES, INPUT_SIZE, NnError, POLICY_SIZE, Query, encode, policy_label,
};

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
    },
    /// CUDAによる推論。
    Cuda,
    /// CPUによる推論。
    Cpu,
}

impl ExecutionProvider {
    /// USIオプション`NnBackend`と同じ実行プロバイダ名を返す。
    pub fn name(&self) -> &'static str {
        match self {
            Self::TensorRt { .. } => "tensorrt",
            Self::Cuda => "cuda",
            Self::Cpu => "cpu",
        }
    }
}

/// 設定の確定だけを直列化し、失敗した読み込みは次の呼び出しで再試行できるようにする。
static RUNTIME_INITIALIZED: Mutex<bool> = Mutex::new(false);

fn initialize_runtime() -> Result<(), NnError> {
    let mut initialized = RUNTIME_INITIALIZED
        .lock()
        .map_err(|_| NnError::RuntimeInitializationPoisoned)?;
    if *initialized {
        return Ok(());
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
    let committed = ort::init_from(path)
        .map_err(NnError::RuntimeInitialization)?
        .with_logger(Arc::new(|level, _category, _id, location, message| {
            if level >= LogLevel::Warning {
                // FFIから呼ばれるロガーなので、標準エラーの書き込み失敗でもパニックしない。
                let _ = writeln!(io::stderr().lock(), "[{level:?}] {location}: {message}");
            }
        }))
        .commit();
    if !committed {
        return Err(NnError::RuntimeAlreadyInitialized);
    }
    *initialized = true;
    Ok(())
}

/// ONNX Runtimeのセッションと、最大バッチ分を一度だけ確保した入力バッファを持つ評価器。
///
/// 出力は`session.run`でONNX Runtimeに確保させる。`IoBinding`による出力の束縛は使わない。
pub struct OnnxEvaluator {
    session: Session,
    input: Vec<f32>,
    max_batch: NonZeroUsize,
}

impl OnnxEvaluator {
    /// 明示された実行プロバイダでネットをメモリから読み、入出力の契約を検査する。
    ///
    /// `ORT_DYLIB_PATH`が示すファイルからONNX Runtimeをプロセスで1回だけ初期化する。
    /// TensorRTとCUDAではCPUへの部分的な退避も禁じる。
    /// 入力はf32の`input: [動的バッチ, 159, 12, 12]`、出力はf32の
    /// `policy: [動的バッチ, 35856]`と`value: [動的バッチ, 1]`でなければならない。
    ///
    /// # Errors
    /// ライブラリ、実行プロバイダ、キャッシュ、バッファ確保、またはネットの入出力に
    /// 問題があれば[`NnError`]を返す。ほかのプロバイダやネットへの切り替えは行わない。
    pub fn new(
        network: &Network,
        provider: &ExecutionProvider,
        max_batch: NonZeroUsize,
    ) -> Result<Self, NnError> {
        let input = allocate_input(max_batch)?;
        initialize_runtime()?;
        let mut builder = Session::builder()?
            .with_log_level(LogLevel::Warning)
            .map_err(ort::Error::from)?;
        if !matches!(provider, ExecutionProvider::Cpu) {
            builder = builder
                .with_disable_cpu_fallback()
                .map_err(ort::Error::from)?;
        }
        let provider = match provider {
            ExecutionProvider::TensorRt { cache_root } => {
                let key = TensorRtCacheKey::new(*network.sha256(), query_gpu()?, max_batch);
                let path = cache_root.join(key.directory_name());
                let cache_path = path
                    .to_str()
                    .ok_or_else(|| NnError::CachePathNotUtf8 { path: path.clone() })?;
                fs::create_dir_all(&path).map_err(|source| NnError::CreateCacheDir {
                    path: path.clone(),
                    source,
                })?;
                let profile = format!("input:{}x{INPUT_PLANES}x12x12", max_batch.get());
                ep::TensorRT::default()
                    .with_profile_min_shapes(format!("input:1x{INPUT_PLANES}x12x12"))
                    .with_profile_opt_shapes(&profile)
                    .with_profile_max_shapes(&profile)
                    .with_fp16(true)
                    .with_engine_cache(true)
                    .with_engine_cache_path(cache_path)
                    .with_timing_cache(false)
                    .build()
            }
            ExecutionProvider::Cuda => ep::CUDA::default().build(),
            ExecutionProvider::Cpu => ep::CPU::default().build(),
        };
        let session = builder
            .with_execution_providers([provider.error_on_failure()])
            .map_err(ort::Error::from)?
            .commit_from_memory(network.bytes())?;
        validate_signature(
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
        Ok(Self {
            session,
            input,
            max_batch,
        })
    }
}

fn allocate_input(max_batch: NonZeroUsize) -> Result<Vec<f32>, NnError> {
    let max = max_batch.get();
    let len = max
        .checked_mul(INPUT_SIZE)
        .ok_or(NnError::InvalidMaxBatch { max })?;
    let mut input = Vec::new();
    input
        .try_reserve_exact(len)
        .map_err(|source| NnError::AllocateInput { max, source })?;
    input.resize(len, 0.0);
    Ok(input)
}

fn validate_signature(
    inputs: Vec<(String, ValueType)>,
    outputs: Vec<(String, ValueType)>,
) -> Result<(), NnError> {
    let matches = |ports: &[(String, ValueType)], name: &str, expected: &[i64]| {
        ports.iter().any(|(actual_name, dtype)| {
            actual_name == name
                && matches!(
                    dtype,
                    ValueType::Tensor { ty: TensorElementType::Float32, shape, .. }
                        if &shape[..] == expected
                )
        })
    };
    if inputs.len() != 1
        || outputs.len() != 2
        || !matches(&inputs, "input", &[-1, INPUT_PLANES as i64, 12, 12])
        || !matches(&outputs, "policy", &[-1, 35856])
        || !matches(&outputs, "value", &[-1, 1])
    {
        return Err(NnError::ModelSignature {
            expected: "input f32 [-1, 159, 12, 12]; policy f32 [-1, 35856], value f32 [-1, 1]",
            inputs,
            outputs,
        });
    }
    Ok(())
}

fn output_tensor<'a>(
    outputs: &'a SessionOutputs<'_>,
    name: &'static str,
    expected: [usize; 2],
) -> Result<&'a [f32], NnError> {
    let output = outputs.get(name).ok_or(NnError::MissingOutput { name })?;
    let (shape, values) = output.try_extract_tensor::<f32>()?;
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
            encode(query.position, buffer)?;
        }
        let tensor = TensorRef::from_array_view(([n, INPUT_PLANES, 12, 12], &*input))?;
        let outputs = self.session.run(ort::inputs!["input" => tensor])?;
        let policy = output_tensor(&outputs, "policy", [n, POLICY_SIZE])?;
        let value = output_tensor(&outputs, "value", [n, 1])?;
        collect_outputs(queries, policy, value, logits, values)
    }
}

/// 形を検査済みの出力から、依頼された合法手のlogitと価値だけを検査して取り出す。
fn collect_outputs(
    queries: &[Query<'_>],
    policy: &[f32],
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
            let logit = policy[index * POLICY_SIZE + label];
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

    // [`OnnxEvaluator::new`]の契約: 出力の順序によらず、名前・f32・動的バッチ・固定幅を検査する。
    #[test]
    fn model_signature_requires_named_float_tensors_with_dynamic_batches() {
        let input = tensor_port(
            "input",
            &[-1, INPUT_PLANES as i64, 12, 12],
            TensorElementType::Float32,
        );
        let outputs = vec![
            tensor_port("value", &[-1, 1], TensorElementType::Float32),
            tensor_port("policy", &[-1, 35856], TensorElementType::Float32),
        ];
        validate_signature(vec![input.clone()], outputs.clone()).unwrap();
        for inputs in [
            vec![],
            vec![input.clone(), input.clone()],
            vec![tensor_port(
                "other",
                &[-1, INPUT_PLANES as i64, 12, 12],
                TensorElementType::Float32,
            )],
            vec![tensor_port(
                "input",
                &[7, INPUT_PLANES as i64, 12, 12],
                TensorElementType::Float32,
            )],
            vec![tensor_port(
                "input",
                &[-1, -1, 12, 12],
                TensorElementType::Float32,
            )],
            vec![tensor_port(
                "input",
                &[-1, INPUT_PLANES as i64, 12, 12],
                TensorElementType::Float16,
            )],
        ] {
            assert!(matches!(
                validate_signature(inputs, outputs.clone()),
                Err(NnError::ModelSignature { .. })
            ));
        }
        for invalid in [
            tensor_port("other", &[-1, 35856], TensorElementType::Float32),
            tensor_port("policy", &[-1, 35855], TensorElementType::Float32),
            tensor_port("policy", &[1, 35856], TensorElementType::Float32),
            tensor_port("policy", &[-1, 35856], TensorElementType::Float16),
        ] {
            assert!(matches!(
                validate_signature(vec![input.clone()], vec![outputs[0].clone(), invalid]),
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
                to: sq(5, 6),
                promote: false,
            },
            Move {
                from: sq(5, 5),
                mid: None,
                to: sq(6, 5),
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
                moves: &moves[1..],
            },
        ];
        // 方策ラベルの仕様表: 南へ1歩は面0、東へ1歩は面22、後手から見た西へ1歩は面66。
        let mut policy = vec![f32::NAN; 3 * POLICY_SIZE];
        policy[65] = 2.0;
        policy[22 * 144 + 65] = -3.0;
        policy[2 * POLICY_SIZE + 66 * 144 + 78] = 4.0;
        let mut logits = Vec::new();
        let mut values = Vec::new();
        collect_outputs(
            &queries,
            &policy,
            &[0.0, 0.5, 1.0],
            &mut logits,
            &mut values,
        )
        .unwrap();
        assert_eq!(logits, [2.0, -3.0, 4.0]);
        assert_eq!(values, [0.0, 0.5, 1.0]);
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
        let mut policy = vec![0.0; POLICY_SIZE];
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            policy[65] = invalid;
            assert!(matches!(
                collect_outputs(&queries, &policy, &[0.5], &mut Vec::new(), &mut Vec::new()),
                Err(NnError::NonFiniteOutput { query: 0 })
            ));
            policy[65] = 0.0;
            assert!(matches!(
                collect_outputs(
                    &queries,
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
                collect_outputs(&queries, &policy, &[invalid], &mut Vec::new(), &mut Vec::new()),
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
