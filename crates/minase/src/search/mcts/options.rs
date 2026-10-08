//! MCTSが宣言するUSIオプションと、推論のセッションの作成。
//!
//! オプションの集合は docs/plans/alphazero.md「共存の構造」と「推論層」に従う。`Threads`と`USI_Hash`は
//! ハーネスが既定値を照会するのでαβ探索と同じ名前で宣言し、`USI_Hash`は探索木の容量として解釈する。

use core::mem::size_of;
use core::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::env;
use std::path::PathBuf;
use std::sync::OnceLock;

use minase_core::mv::Move;

use crate::nn::ort_evaluator::{NETWORK, NnBackend, OrtEvaluator, SessionConfig};
use crate::search::DEFAULT_TT_SIZE_MB;
use crate::search::searcher::{OptionKind, OptionSpec};

use super::engine::{Config, DEFAULT_GUMBEL_SIMULATIONS, Engine, MAX_CONSIDERED_ACTIONS};
use super::select::Selection;
use super::tree::{Edge, Node};

/// オプション`USI_Hash`で設定できる容量の上限(MiB)。αβ探索の置換表と同じ値とする。
const MAX_HASH_SIZE_MB: usize = 65_536;

/// オプション`Threads`の上限。初版のMCTSは1スレッドで探索するので、宣言の範囲のうち1だけを受理する。
const MAX_THREADS: usize = 256;

/// オプション`NnBatchSize`の既定値。
const DEFAULT_BATCH_SIZE: NonZeroUsize = NonZeroUsize::new(64).unwrap();

/// オプション`NnBatchSize`の上限。入力と出力のバッチの領域は1局面あたり約200 KBなので、
/// 上限でも約200 MBに収まる。
const MAX_BATCH_SIZE: usize = 1024;

/// 1節点あたりの子の数の見込み。2026年10月3日の教師データの平均合法手数73.6
/// （docs/measurements/alphazero-feasibility.md「第2項」）を切り上げた値。
const EXPECTED_EDGES_PER_NODE: usize = 74;

/// 探索木の1節点の概算バイト数。節点の本体と、見込みの数の辺からなる。
const NODE_BYTES: usize =
    size_of::<Node<Move>>() + EXPECTED_EDGES_PER_NODE * size_of::<Edge<Move>>();

/// 対局で使う探索の乱数のシード。PUCTは乱数を使わないので値は探索に影響しない。
const PLAY_SEED: NonZeroU64 = NonZeroU64::new(1).unwrap();

/// `USI_Hash`の容量(MiB)を探索木の節点数の上限へ換算する。
///
/// 初版はNNキャッシュを持たないので、容量の全体を探索木に充てる。
fn max_nodes(hash_mb: usize) -> NonZeroU32 {
    let nodes = (hash_mb << 20) / NODE_BYTES;
    NonZeroU32::new(u32::try_from(nodes).unwrap_or(u32::MAX)).unwrap_or(NonZeroU32::MIN)
}

/// `NnCacheDir`の既定値を返す。`$XDG_CACHE_HOME/minase/tensorrt`、未設定なら
/// `$HOME/.cache/minase/tensorrt`とし、どちらもなければ空とする。
pub(super) fn default_cache_dir() -> PathBuf {
    PathBuf::from(default_cache_dir_text())
}

/// [`default_cache_dir`]の文字列。オプションの宣言に使うので、プロセスで1回だけ求める。
fn default_cache_dir_text() -> &'static str {
    static DEFAULT: OnceLock<String> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        let non_empty = |name| {
            env::var(name)
                .ok()
                .filter(|value: &String| !value.is_empty())
        };
        non_empty("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| non_empty("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .map(|cache| cache.join("minase").join("tensorrt"))
            .and_then(|path| path.to_str().map(str::to_owned))
            .unwrap_or_default()
    })
}

impl Engine {
    /// 対局用の既定の設定で、推論器を持たない状態を作る。
    ///
    /// 選択規則はPUCT（docs/plans/alphazero.md「対局ではPUCTを使う」）、探索木の容量は`USI_Hash`の
    /// 既定値とする。推論のセッションは[`Engine::prepare`]が作る。
    pub fn for_play(win_rate_scale: f32) -> Self {
        Self::without_evaluator(Config {
            selection: Selection::Puct,
            batch_size: DEFAULT_BATCH_SIZE,
            gumbel_simulations: DEFAULT_GUMBEL_SIMULATIONS,
            max_considered: MAX_CONSIDERED_ACTIONS,
            seed: PLAY_SEED,
            win_rate_scale,
            max_nodes: max_nodes(DEFAULT_TT_SIZE_MB),
        })
    }

    /// 宣言するオプションを宣言の順に返す。
    pub fn options(&self) -> &'static [OptionSpec] {
        static OPTIONS: OnceLock<[OptionSpec; 5]> = OnceLock::new();
        OPTIONS.get_or_init(|| {
            [
                OptionSpec {
                    name: "USI_Hash",
                    kind: OptionKind::Spin {
                        default: DEFAULT_TT_SIZE_MB as i64,
                        min: 1,
                        max: MAX_HASH_SIZE_MB as i64,
                    },
                },
                OptionSpec {
                    name: "Threads",
                    kind: OptionKind::Spin {
                        default: 1,
                        min: 1,
                        max: MAX_THREADS as i64,
                    },
                },
                OptionSpec {
                    name: "NnBackend",
                    kind: OptionKind::Combo {
                        default: NnBackend::TensorRt.name(),
                        vars: &NnBackend::NAMES,
                    },
                },
                OptionSpec {
                    name: "NnCacheDir",
                    kind: OptionKind::String {
                        default: default_cache_dir_text(),
                    },
                },
                OptionSpec {
                    name: "NnBatchSize",
                    kind: OptionKind::Spin {
                        default: DEFAULT_BATCH_SIZE.get() as i64,
                        min: 1,
                        max: MAX_BATCH_SIZE as i64,
                    },
                },
            ]
        })
    }

    /// 名前が宣言したオプション（大文字小文字を区別しない）なら値を設定する。
    ///
    /// 未知の名前には`None`を返す。値が不正な場合は状態を変えずに理由を`Some(Err)`で返す。
    /// `NnBackend`、`NnCacheDir`、または`NnBatchSize`を変えると推論のセッションを捨て、
    /// 次の[`Engine::prepare`]で作り直す。
    pub fn set_option(&mut self, name: &str, value: Option<&str>) -> Option<Result<(), String>> {
        let result = if name.eq_ignore_ascii_case("USI_Hash") {
            parse_range("USI_Hash", value, MAX_HASH_SIZE_MB)
                .map(|size_mb| self.set_hash_mb(size_mb.get()))
        } else if name.eq_ignore_ascii_case("Threads") {
            parse_range("Threads", value, MAX_THREADS).and_then(|threads| self.set_threads(threads))
        } else if name.eq_ignore_ascii_case("NnBackend") {
            value
                .ok_or_else(|| "NnBackend requires a value".to_owned())
                .and_then(|value| value.parse().map_err(|error| format!("{error}")))
                .map(|backend| {
                    if self.backend != backend {
                        self.backend = backend;
                        self.evaluator = None;
                    }
                })
        } else if name.eq_ignore_ascii_case("NnCacheDir") {
            // USIは空の文字列を`<empty>`と書く。
            let cache_dir = PathBuf::from(value.filter(|&value| value != "<empty>").unwrap_or(""));
            if self.cache_dir != cache_dir {
                self.cache_dir = cache_dir;
                self.evaluator = None;
            }
            Ok(())
        } else if name.eq_ignore_ascii_case("NnBatchSize") {
            parse_range("NnBatchSize", value, MAX_BATCH_SIZE).map(|batch_size| {
                if self.config.batch_size != batch_size {
                    self.config.batch_size = batch_size;
                    self.evaluator = None;
                }
            })
        } else {
            return None;
        };
        Some(result)
    }

    /// `USI_Hash`の容量(MiB)から探索木の節点数の上限を設定し、再利用する探索木を捨てる。
    pub fn set_hash_mb(&mut self, size_mb: usize) {
        self.config.max_nodes = max_nodes(size_mb);
        self.new_game();
    }

    /// ワーカー数を設定する。初版のMCTSは1スレッドで探索するので、1だけを受理する。
    ///
    /// # Errors
    ///
    /// 1以外を指定した場合は理由を返す。
    pub fn set_threads(&mut self, threads: NonZeroUsize) -> Result<(), String> {
        if threads.get() == 1 {
            Ok(())
        } else {
            Err(format!(
                "Threads must be 1 because the MCTS search runs on one thread (got {threads})"
            ))
        }
    }

    /// 推論器がなければ、オプションの設定で推論のセッションを作る。
    ///
    /// TensorRTではエンジンの作成に数分かかることがあるので、USIでは`isready`の応答の前に呼ぶ
    /// （docs/plans/alphazero.md「推論層」）。
    ///
    /// # Errors
    ///
    /// セッションを作れない場合は理由を返し、状態を変えない。
    pub fn prepare(&mut self) -> Result<(), String> {
        if self.evaluator.is_none() {
            let config = SessionConfig {
                backend: self.backend,
                cache_dir: self.cache_dir.clone(),
                max_batch: self.config.batch_size,
            };
            let evaluator =
                OrtEvaluator::new(NETWORK, &config).map_err(|error| error.to_string())?;
            self.evaluator = Some(Box::new(evaluator));
        }
        Ok(())
    }
}

/// `1..=max`の整数の値を解析する。
fn parse_range(name: &str, value: Option<&str>, max: usize) -> Result<NonZeroUsize, String> {
    let value = value.ok_or_else(|| format!("{name} requires a value"))?;
    value
        .parse::<usize>()
        .ok()
        .and_then(NonZeroUsize::new)
        .filter(|parsed| parsed.get() <= max)
        .ok_or_else(|| format!("{name} must be an integer from 1 to {max}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nn::evaluator::UniformEvaluator;

    fn engine() -> Engine {
        Engine::new(Box::new(UniformEvaluator), Engine::for_play(1.0).config)
    }

    // alphazero.md「共存の構造」: MCTSもUSI_HashとThreadsを宣言し、続けて推論の設定を宣言する。
    #[test]
    fn options_declare_hash_threads_and_network_settings() {
        let names: Vec<_> = engine()
            .options()
            .iter()
            .map(|option| option.name)
            .collect();
        assert_eq!(
            names,
            [
                "USI_Hash",
                "Threads",
                "NnBackend",
                "NnCacheDir",
                "NnBatchSize"
            ]
        );
        assert_eq!(
            engine().options()[2].kind,
            OptionKind::Combo {
                default: "tensorrt",
                vars: &["tensorrt", "cuda", "cpu"],
            }
        );
    }

    // 指示書: Threadsは1だけを受理し、範囲外と1以外は理由を返す。
    #[test]
    fn threads_accepts_only_one() {
        let mut engine = engine();
        assert_eq!(engine.set_option("threads", Some("1")), Some(Ok(())));
        for value in ["2", "256"] {
            assert!(matches!(
                engine.set_option("Threads", Some(value)),
                Some(Err(_))
            ));
        }
        assert_eq!(
            engine.set_option("Threads", Some("0")),
            Some(Err("Threads must be an integer from 1 to 256".to_owned()))
        );
        assert_eq!(engine.set_option("ResignValue", Some("1")), None);
    }

    // 指示書: USI_Hashは探索木の節点数の上限に換算し、容量に比例する。
    #[test]
    fn hash_sets_the_tree_capacity() {
        let mut engine = engine();
        assert_eq!(engine.set_option("USI_Hash", Some("1")), Some(Ok(())));
        let small = engine.config.max_nodes.get();
        assert_eq!(small as usize, (1 << 20) / NODE_BYTES);
        assert_eq!(engine.set_option("USI_Hash", Some("2")), Some(Ok(())));
        assert_eq!(engine.config.max_nodes.get(), (2 << 20) / NODE_BYTES as u32);
        assert!(matches!(
            engine.set_option("USI_Hash", Some("0")),
            Some(Err(_))
        ));
        assert!(matches!(engine.set_option("USI_Hash", None), Some(Err(_))));
        assert_eq!(engine.config.max_nodes.get(), (2 << 20) / NODE_BYTES as u32);
    }

    // 指示書: 推論の設定を変えると推論のセッションを捨て、同じ値では捨てない。不正値は状態を変えない。
    #[test]
    fn network_settings_discard_the_session_only_when_changed() {
        let mut engine = engine();
        assert_eq!(
            engine.set_option("NnBackend", Some("tensorrt")),
            Some(Ok(()))
        );
        assert!(engine.evaluator.is_some());
        assert!(matches!(
            engine.set_option("NnBackend", Some("gpu")),
            Some(Err(_))
        ));
        assert!(engine.evaluator.is_some());
        assert_eq!(engine.set_option("NnBackend", Some("cpu")), Some(Ok(())));
        assert_eq!(engine.backend, NnBackend::Cpu);
        assert!(engine.evaluator.is_none());

        let mut engine = self::engine();
        assert!(matches!(
            engine.set_option("NnBatchSize", Some("1025")),
            Some(Err(_))
        ));
        assert!(engine.evaluator.is_some());
        assert_eq!(engine.set_option("NnBatchSize", Some("8")), Some(Ok(())));
        assert_eq!(engine.config.batch_size.get(), 8);
        assert!(engine.evaluator.is_none());

        let mut engine = self::engine();
        assert_eq!(
            engine.set_option("NnCacheDir", Some("/tmp/minase-trt")),
            Some(Ok(()))
        );
        assert_eq!(engine.cache_dir, PathBuf::from("/tmp/minase-trt"));
        assert!(engine.evaluator.is_none());
        assert_eq!(
            engine.set_option("NnCacheDir", Some("<empty>")),
            Some(Ok(()))
        );
        assert_eq!(engine.cache_dir, PathBuf::new());
    }
}
