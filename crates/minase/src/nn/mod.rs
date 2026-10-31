//! モンテカルロ木探索（MCTS）が使うネットの入力平面、方策のラベル、および評価器。
//!
//! AlphaZero型探索のMCTS、自己対局生成器、およびPythonの学習器が共有するネットの入出力の契約を定める。
//! 入力は159面、方策のラベルは249面×144升で、価値は手番側の勝率1つである。
//! 推論には評価器の抽象[`Evaluator`]、試験用の[`UniformEvaluator`]、およびONNX Runtimeによる[`OnnxEvaluator`]を用意する。
//! プロジェクト内では`minase_core`と駒の状態番号を得るための`crate::eval::features`にだけ依存し、`search`を参照しない。
//!
//! 設計判断は`docs/adr/`の次の7本に記録している。
//!
//! - `encode-zero-network-inputs-as-159-planes-with-pseudo-control-from-the-side-to-move.md`
//! - `encode-the-zero-policy-as-249-planes-per-from-square.md`
//! - `predict-a-single-win-probability-in-the-zero-network.md`
//! - `run-zero-network-inference-through-ort-load-dynamic.md`
//! - `select-zero-networks-by-startup-argument-and-embed-only-the-judged-one.md`
//! - `key-tensorrt-engine-caches-by-network-checksum-gpu-and-max-batch.md`
//! - `start-the-zero-network-as-a-10-block-128-channel-resnet-with-global-pooling-and-a-32-channel-value-head.md`
//!
//! 出力の照合の手順は`docs/guides/alphazero-network-check.md`にある。

use core::fmt;
use minase_core::{BOARD_SQUARE_COUNT, Color, Square};
use std::collections::TryReserveError;
use std::io;
use std::path::PathBuf;

pub mod control;
pub mod encoding;
pub mod evaluator;
pub mod onnx;
pub mod policy;
pub mod tensorrt_cache;

pub use encoding::{INPUT_PLANES, INPUT_SIZE, encode};
pub use evaluator::{Evaluator, Query, UniformEvaluator};
pub use onnx::{ExecutionProvider, Network, OnnxEvaluator};
pub use policy::{POLICY_PLANES, POLICY_SIZE, policy_label, softmax};
use tensorrt_cache::GpuQueryError;

/// ネットの読み込み、推論の準備、入力の符号化、または評価に失敗した理由。
#[derive(Debug)]
pub enum NnError {
    /// 159面の入力では表せない成り権の保留がある。
    PromotionDeferred,
    /// `ORT_DYLIB_PATH`が未設定または空である。
    RuntimeLibraryPathUnset,
    /// ONNX Runtimeのライブラリのパスを解決または検査できなかった。
    RuntimeLibraryPath {
        /// 指定されたパス。
        path: PathBuf,
        /// 元の入出力エラー。
        source: io::Error,
    },
    /// ONNX Runtimeのライブラリのパスが通常のファイルではない。
    RuntimeLibraryNotFile {
        /// 指定されたパス。
        path: PathBuf,
    },
    /// ONNX Runtimeの共有ライブラリを読み込めなかった。
    RuntimeInitialization(ort::LoadDynamicError),
    /// 別の呼び出し元がONNX Runtimeをすでに設定していた。
    RuntimeAlreadyInitialized,
    /// ONNX Runtimeの初期化中のパニックにより排他ロックが汚染された。
    RuntimeInitializationPoisoned,
    /// ネットのファイルを読み込めなかった。
    ReadNetwork {
        /// 読み込もうとしたパス。
        path: PathBuf,
        /// 元の入出力エラー。
        source: io::Error,
    },
    /// TensorRTのキャッシュ用ディレクトリを作成できなかった。
    CreateCacheDir {
        /// 作成しようとしたパス。
        path: PathBuf,
        /// 元の入出力エラー。
        source: io::Error,
    },
    /// TensorRTへ渡すキャッシュのパスをUTF-8で表せない。
    CachePathNotUtf8 {
        /// 指定されたパス。
        path: PathBuf,
    },
    /// GPUの情報を取得できなかった。
    GpuQuery(GpuQueryError),
    /// ONNX Runtimeが返したエラー。
    Runtime(ort::Error),
    /// ネットの入出力の名前、要素型、個数、または形が契約と異なる。
    ModelSignature {
        /// 期待する入出力の名前、要素型、および形。`-1`は動的なバッチ次元。
        expected: &'static str,
        /// 実際の入力の名前と型。テンソルの型は形も含む。
        inputs: Vec<(String, ort::value::ValueType)>,
        /// 実際の出力の名前と型。テンソルの型は形も含む。
        outputs: Vec<(String, ort::value::ValueType)>,
    },
    /// 最大バッチから入力バッファの要素数を表現できない。
    InvalidMaxBatch {
        /// 指定された最大バッチ。
        max: usize,
    },
    /// 入力バッファを確保できなかった。
    AllocateInput {
        /// 指定された最大バッチ。
        max: usize,
        /// 元の確保エラー。
        source: TryReserveError,
    },
    /// 依頼数が構築時に指定した最大バッチを超えている。
    BatchTooLarge {
        /// 依頼数。
        len: usize,
        /// 最大バッチ。
        max: usize,
    },
    /// 推論結果に必要な名前の出力がない。
    MissingOutput {
        /// 必要な出力の名前。
        name: &'static str,
    },
    /// 推論結果のテンソルの形が依頼したバッチと一致しない。
    OutputShape {
        /// 出力の名前。
        name: &'static str,
        /// 期待する形。
        expected: [usize; 2],
        /// 実際の形。
        actual: Vec<i64>,
    },
    /// 依頼した合法手のlogitまたは価値に非有限値がある。
    NonFiniteOutput {
        /// バッチ内の依頼の添字。
        query: usize,
    },
    /// 価値が勝率の範囲0〜1を外れている。
    ValueOutOfRange {
        /// バッチ内の依頼の添字。
        query: usize,
        /// 返された価値。
        value: f32,
    },
}

impl fmt::Display for NnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PromotionDeferred => {
                write!(
                    formatter,
                    "deferred promotion rights cannot be encoded in {INPUT_PLANES} planes"
                )
            }
            Self::RuntimeLibraryPathUnset => {
                formatter.write_str("ORT_DYLIB_PATH is unset or empty")
            }
            Self::RuntimeLibraryPath { path, source } => write!(
                formatter,
                "cannot resolve runtime library {}: {source}",
                path.display()
            ),
            Self::RuntimeLibraryNotFile { path } => {
                write!(
                    formatter,
                    "runtime library is not a file: {}",
                    path.display()
                )
            }
            Self::RuntimeInitialization(source) => {
                write!(formatter, "cannot initialize ONNX Runtime: {source}")
            }
            Self::RuntimeAlreadyInitialized => {
                formatter.write_str("ONNX Runtime was already configured by another caller")
            }
            Self::RuntimeInitializationPoisoned => {
                formatter.write_str("ONNX Runtime initialization lock is poisoned")
            }
            Self::ReadNetwork { path, source } => {
                write!(
                    formatter,
                    "cannot read network {}: {source}",
                    path.display()
                )
            }
            Self::CreateCacheDir { path, source } => write!(
                formatter,
                "cannot create cache directory {}: {source}",
                path.display()
            ),
            Self::CachePathNotUtf8 { path } => {
                write!(formatter, "cache path is not UTF-8: {}", path.display())
            }
            Self::GpuQuery(source) => write!(formatter, "cannot query GPU: {source}"),
            Self::Runtime(source) => write!(formatter, "ONNX Runtime error: {source}"),
            Self::ModelSignature {
                expected,
                inputs,
                outputs,
            } => write!(
                formatter,
                "invalid model signature: expected {expected}; got inputs {inputs:?}, outputs {outputs:?}"
            ),
            Self::InvalidMaxBatch { max } => write!(formatter, "maximum batch is too large: {max}"),
            Self::AllocateInput { max, source } => {
                write!(formatter, "cannot allocate input for batch {max}: {source}")
            }
            Self::BatchTooLarge { len, max } => {
                write!(formatter, "batch size {len} exceeds maximum {max}")
            }
            Self::MissingOutput { name } => write!(formatter, "missing output: {name}"),
            Self::OutputShape {
                name,
                expected,
                actual,
            } => write!(
                formatter,
                "invalid {name} shape: expected {expected:?}, got {actual:?}"
            ),
            Self::NonFiniteOutput { query } => {
                write!(formatter, "non-finite output for query {query}")
            }
            Self::ValueOutOfRange { query, value } => {
                write!(
                    formatter,
                    "value {value} is outside [0, 1] for query {query}"
                )
            }
        }
    }
}

impl std::error::Error for NnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RuntimeLibraryPath { source, .. }
            | Self::ReadNetwork { source, .. }
            | Self::CreateCacheDir { source, .. } => Some(source),
            Self::RuntimeInitialization(source) => Some(source),
            Self::GpuQuery(source) => Some(source),
            Self::Runtime(source) => Some(source),
            Self::AllocateInput { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<ort::Error> for NnError {
    fn from(source: ort::Error) -> Self {
        Self::Runtime(source)
    }
}

impl From<GpuQueryError> for NnError {
    fn from(source: GpuQueryError) -> Self {
        Self::GpuQuery(source)
    }
}

/// 後手視点では盤を180度回転し、手番側から見た升番号を返す。
fn relative_square(square: Square, side: Color) -> usize {
    match side {
        Color::Black => square.dense_index(),
        Color::White => BOARD_SQUARE_COUNT - 1 - square.dense_index(),
    }
}

#[cfg(test)]
mod test_util {
    use minase_core::rng::XorShift64;
    use minase_core::{Color, MoveGenerator, Position, Rules, Square};
    use std::num::{NonZeroU64, NonZeroUsize};

    pub(super) fn rotate(square: Square) -> Square {
        Square::new(11 - square.file(), 11 - square.rank()).unwrap()
    }

    /// 3局を最大500手ずつ進め、王駒の消失または合法手なしで打ち切る。
    pub(super) fn random_positions() -> Vec<Position> {
        let rules = Rules::ENGINE_DEFAULT.moves;
        let generator = MoveGenerator::new(rules);
        let mut positions = Vec::new();
        let mut moves = Vec::new();
        for seed in [1, 71, 2026] {
            let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
            let mut position = Position::initial();
            for _ in 0..500 {
                positions.push(position.clone());
                if Color::ALL
                    .iter()
                    .any(|&side| position.royal_pieces(side).is_empty())
                {
                    break;
                }
                moves.clear();
                generator.generate_moves(&position, &mut moves);
                let Some(count) = NonZeroUsize::new(moves.len()) else {
                    break;
                };
                position.make_move_unchecked(moves[rng.index(count)], rules);
            }
        }
        positions
    }
}
