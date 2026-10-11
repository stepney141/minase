//! MCTSが宣言するUSIオプションと、推論のセッションの作成。
//!
//! オプションの集合は docs/plans/alphazero.md「共存の構造」と「推論層」に従う。`Threads`は
//! ハーネスが既定値を照会するのでαβ探索と同じ名前で宣言する。置換表を持たないので`USI_Hash`は
//! 宣言せず、探索木の大きさは、dlshogiに倣い、節点数の上限`UCT_NodeLimit`で制御する。
//! 推論に使うネットは既定で埋め込みのネットとし、`NnModelPath`で別のONNXファイルを与えられる。

use core::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::env;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::nn::ort_evaluator::{NETWORK, NnBackend, OrtEvaluator, SessionConfig, read_model};
use crate::search::searcher::{OptionKind, OptionSpec};

use super::engine::{Config, DEFAULT_GUMBEL_SIMULATIONS, Engine, MAX_CONSIDERED_ACTIONS};
use super::select::Selection;

/// オプション`Threads`の上限。初版のMCTSは1スレッドで探索するので、宣言の範囲のうち1だけを受理する。
const MAX_THREADS: usize = 256;

/// オプション`NnBatchSize`の既定値。
const DEFAULT_BATCH_SIZE: NonZeroUsize = NonZeroUsize::new(64).unwrap();

/// オプション`NnBatchSize`の上限。入力と出力のバッチの領域は1局面あたり約200 KBなので、
/// 上限でも約200 MBに収まる。
const MAX_BATCH_SIZE: usize = 1024;

/// オプション`UCT_NodeLimit`の既定値。1節点は辺を含めて約4.2 KBなので、約4.2 GBに当たる
/// （docs/plans/alphazero.md「共存の構造」）。
const DEFAULT_NODE_LIMIT: NonZeroU32 = NonZeroU32::new(1_000_000).unwrap();

/// オプション`UCT_NodeLimit`の下限。dlshogiと同じ値とする。
const MIN_NODE_LIMIT: usize = 100_000;

/// オプション`UCT_NodeLimit`の上限。dlshogiと同じ値とする。
const MAX_NODE_LIMIT: usize = 1_000_000_000;

/// 対局で使う探索の乱数のシード。PUCTは乱数を使わないので値は探索に影響しない。
const PLAY_SEED: NonZeroU64 = NonZeroU64::new(1).unwrap();

/// `NnCacheDir`の既定値を返す。`$XDG_CACHE_HOME/minase/tensorrt`、未設定なら
/// `$HOME/.cache/minase/tensorrt`とし、どちらもなければ空とする。
pub fn default_cache_dir() -> PathBuf {
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
    /// 選択規則はPUCT（docs/plans/alphazero.md「対局ではPUCTを使う」）、探索木の節点数の上限は
    /// `UCT_NodeLimit`の既定値とする。推論のセッションは[`Engine::prepare`]が作る。
    pub fn for_play(win_rate_scale: f32) -> Self {
        Self::without_evaluator(Config {
            selection: Selection::Puct,
            batch_size: DEFAULT_BATCH_SIZE,
            gumbel_simulations: DEFAULT_GUMBEL_SIMULATIONS,
            max_considered: MAX_CONSIDERED_ACTIONS,
            seed: PLAY_SEED,
            win_rate_scale,
            max_nodes: DEFAULT_NODE_LIMIT,
        })
    }

    /// 宣言するオプションを宣言の順に返す。
    pub fn options(&self) -> &'static [OptionSpec] {
        static OPTIONS: OnceLock<[OptionSpec; 6]> = OnceLock::new();
        OPTIONS.get_or_init(|| {
            [
                OptionSpec {
                    name: "Threads",
                    kind: OptionKind::Spin {
                        default: 1,
                        min: 1,
                        max: MAX_THREADS as i64,
                    },
                },
                OptionSpec {
                    name: "UCT_NodeLimit",
                    kind: OptionKind::Spin {
                        default: i64::from(DEFAULT_NODE_LIMIT.get()),
                        min: MIN_NODE_LIMIT as i64,
                        max: MAX_NODE_LIMIT as i64,
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
                    name: "NnModelPath",
                    kind: OptionKind::String { default: "" },
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
    /// `NnBackend`、`NnModelPath`、`NnCacheDir`、または`NnBatchSize`を変えると推論のセッションを捨て、
    /// 次の[`Engine::prepare`]で作り直す。
    pub fn set_option(&mut self, name: &str, value: Option<&str>) -> Option<Result<(), String>> {
        let result = if name.eq_ignore_ascii_case("Threads") {
            parse_range("Threads", value, 1, MAX_THREADS)
                .and_then(|threads| self.set_threads(threads))
        } else if name.eq_ignore_ascii_case("UCT_NodeLimit") {
            parse_range("UCT_NodeLimit", value, MIN_NODE_LIMIT, MAX_NODE_LIMIT).map(|limit| {
                self.config.max_nodes =
                    NonZeroU32::try_from(limit).expect("MAX_NODE_LIMIT fits in u32");
                self.new_game();
            })
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
        } else if name.eq_ignore_ascii_case("NnModelPath") {
            let model_path = path_value(value);
            if self.model_path != model_path {
                self.model_path = model_path;
                self.evaluator = None;
            }
            Ok(())
        } else if name.eq_ignore_ascii_case("NnCacheDir") {
            let cache_dir = path_value(value);
            if self.cache_dir != cache_dir {
                self.cache_dir = cache_dir;
                self.evaluator = None;
            }
            Ok(())
        } else if name.eq_ignore_ascii_case("NnBatchSize") {
            parse_range("NnBatchSize", value, 1, MAX_BATCH_SIZE).map(|batch_size| {
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
    /// ネットは、`NnModelPath`が空なら埋め込みのネット、空でなければそのファイルとする。
    /// TensorRTではエンジンの作成に数分かかることがあるので、USIでは`isready`の応答の前に呼ぶ
    /// （docs/plans/alphazero.md「推論層」）。
    ///
    /// # Errors
    ///
    /// ネットのファイルを読めない場合、ファイルが空の場合、およびセッションを作れない場合は理由を返し、
    /// 状態を変えない。
    pub fn prepare(&mut self) -> Result<(), String> {
        if self.evaluator.is_none() {
            let config = SessionConfig {
                backend: self.backend,
                cache_dir: self.cache_dir.clone(),
                max_batch: self.config.batch_size,
            };
            let file;
            let model = if self.model_path.as_os_str().is_empty() {
                NETWORK
            } else {
                file = read_model(&self.model_path).map_err(|error| error.to_string())?;
                &file
            };
            let evaluator = OrtEvaluator::new(model, &config).map_err(|error| error.to_string())?;
            self.evaluator = Some(Box::new(evaluator));
        }
        Ok(())
    }
}

/// 文字列のオプションの値を経路として読む。値がない場合と、USIが空の文字列を表す`<empty>`は空とする。
fn path_value(value: Option<&str>) -> PathBuf {
    PathBuf::from(value.filter(|&value| value != "<empty>").unwrap_or(""))
}

/// `min..=max`の整数の値を解析する。`min`は1以上とする。
fn parse_range(
    name: &str,
    value: Option<&str>,
    min: usize,
    max: usize,
) -> Result<NonZeroUsize, String> {
    let value = value.ok_or_else(|| format!("{name} requires a value"))?;
    value
        .parse::<usize>()
        .ok()
        .and_then(NonZeroUsize::new)
        .filter(|parsed| (min..=max).contains(&parsed.get()))
        .ok_or_else(|| format!("{name} must be an integer from {min} to {max}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nn::evaluator::UniformEvaluator;

    fn engine() -> Engine {
        Engine::new(Box::new(UniformEvaluator), Engine::for_play(1.0).config)
    }

    // alphazero.md「共存の構造」: MCTSはUSI_Hashを宣言せず、Threadsに続けて節点数の上限
    // UCT_NodeLimitと推論の設定を宣言する。
    #[test]
    fn options_declare_threads_node_limit_and_network_settings() {
        let names: Vec<_> = engine()
            .options()
            .iter()
            .map(|option| option.name)
            .collect();
        assert_eq!(
            names,
            [
                "Threads",
                "UCT_NodeLimit",
                "NnBackend",
                "NnModelPath",
                "NnCacheDir",
                "NnBatchSize"
            ]
        );
        assert_eq!(
            engine().options()[3].kind,
            OptionKind::String { default: "" }
        );
        assert_eq!(
            engine().options()[1].kind,
            OptionKind::Spin {
                default: 1_000_000,
                min: 100_000,
                max: 1_000_000_000,
            }
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

    // alphazero.md「共存の構造」: UCT_NodeLimitの値が探索木の節点数の上限になり、範囲外の値は
    // 状態を変えずに理由を返す。
    #[test]
    fn node_limit_sets_the_tree_capacity() {
        let mut engine = engine();
        assert_eq!(engine.config.max_nodes.get(), 1_000_000);
        assert_eq!(
            engine.set_option("uct_nodelimit", Some("100000")),
            Some(Ok(()))
        );
        assert_eq!(engine.config.max_nodes.get(), 100_000);
        assert_eq!(
            engine.set_option("UCT_NodeLimit", Some("1000000000")),
            Some(Ok(()))
        );
        assert_eq!(engine.config.max_nodes.get(), 1_000_000_000);
        for value in [Some("99999"), Some("1000000001"), Some("0"), Some("x")] {
            assert_eq!(
                engine.set_option("UCT_NodeLimit", value),
                Some(Err(
                    "UCT_NodeLimit must be an integer from 100000 to 1000000000".to_owned()
                ))
            );
        }
        assert!(matches!(
            engine.set_option("UCT_NodeLimit", None),
            Some(Err(_))
        ));
        assert_eq!(engine.config.max_nodes.get(), 1_000_000_000);
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

        let mut engine = self::engine();
        assert_eq!(
            engine.set_option("NnModelPath", Some("<empty>")),
            Some(Ok(()))
        );
        assert!(engine.evaluator.is_some());
        assert_eq!(
            engine.set_option("nnmodelpath", Some("/tmp/net.onnx")),
            Some(Ok(()))
        );
        assert_eq!(engine.model_path, PathBuf::from("/tmp/net.onnx"));
        assert!(engine.evaluator.is_none());
    }

    /// `NnModelPath`を設定した推論器のない状態を作る。
    fn engine_with_model(path: &std::path::Path) -> Engine {
        let mut engine = engine();
        assert_eq!(
            engine.set_option("NnModelPath", path.to_str()),
            Some(Ok(()))
        );
        assert!(engine.evaluator.is_none());
        engine
    }

    // alphazero.md「推論層」: NnModelPathが空でなければ、prepareは埋め込みのネットではなくそのファイルを
    // 読む。存在しないファイルと空のファイルは、ONNX Runtimeを読み込む前にエラーとし、状態を変えない。
    #[test]
    fn prepare_reads_the_model_path_before_loading_onnx_runtime() {
        let missing = env::temp_dir().join(format!("minase-missing-{}.onnx", std::process::id()));
        let mut engine = engine_with_model(&missing);
        let error = engine.prepare().unwrap_err();
        assert!(
            error.starts_with(&format!("cannot read the network: {}: ", missing.display())),
            "{error}"
        );
        assert!(engine.evaluator.is_none());

        let empty = env::temp_dir().join(format!("minase-empty-{}.onnx", std::process::id()));
        std::fs::write(&empty, b"").unwrap();
        let result = engine_with_model(&empty).prepare();
        std::fs::remove_file(&empty).unwrap();
        assert_eq!(
            result,
            Err(format!(
                "cannot read the network: {} is empty",
                empty.display()
            ))
        );
    }

    // 同節: 読めるファイルを指せば、セッションの作成へ進む。ORT_DYLIB_PATHのない環境では、
    // 埋め込みのネットと同じくそのエラーになる。環境変数は他の試験と共有するので、設定されている環境では
    // この試験を省く。
    #[test]
    fn prepare_with_a_readable_model_path_reaches_onnx_runtime() {
        if env::var_os("ORT_DYLIB_PATH").is_some() {
            return;
        }
        let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/nets/zero.onnx"));
        assert_eq!(
            engine_with_model(path).prepare(),
            Err(crate::nn::ort_evaluator::OrtError::MissingDylibPath.to_string())
        );
    }
}
