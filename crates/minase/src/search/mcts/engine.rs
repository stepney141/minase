//! 対局用PUCTのバッチ評価、時間管理、および探索木の持ち越し。

use std::ffi::OsStr;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use super::Error;
use super::driver::{DrawValue, Driver, EvaluationRequest, Step, validate_rules, validate_scale};
use super::puct::PuctConfig;
use super::tree::{NodeState, SelectionRule};
use crate::nn::{Evaluator, ExecutionProvider, Network, NnError, OnnxEvaluator};
use crate::search::budget::{TimeBudget, time_budget};
use crate::search::clock::{MonotonicClock, SearchClock};
use crate::search::{
    PrepareError, SearchEvent, SearchHandle, SearchJoinError, SearchLimits, SearchOption,
    SearchOptionKind, SearchOptionValue, SearchSnapshot, Searcher, SetOptionError, StartError,
    StartFailure, StopReason,
};

/// 対局用MCTSの設定。勝率尺度は探索の境界から値で受け取る。
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// 節点と辺に使える容量。単位はバイト。
    pub capacity_bytes: usize,
    /// 1回の推論に送る葉の数の上限。
    pub batch_size: NonZeroUsize,
    /// PUCTの係数。
    pub puct: PuctConfig,
    /// 1ロジットあたりのcp。
    pub cp_per_logit: f32,
}

impl Config {
    /// 容量256 MiB、バッチ256、および既定のPUCT係数を使う。
    pub fn new(cp_per_logit: f32) -> Self {
        Self {
            capacity_bytes: 256 * 1024 * 1024,
            batch_size: NonZeroUsize::new(256).unwrap(),
            puct: PuctConfig::default(),
            cp_per_logit,
        }
    }
}

/// 対局内で持ち越す推論器、探索木、および設定。
pub struct Engine {
    network: Network,
    backend: String,
    cache_dir: PathBuf,
    default_cache_dir: PathBuf,
    evaluator: Option<Box<dyn Evaluator + Send>>,
    driver: Option<Box<Driver>>,
    config: Config,
    #[cfg(test)]
    reused: bool,
}

impl Engine {
    /// ネットと設定を保持し、推論器の作成は`prepare`まで遅延する。
    /// TensorRTを既定とし、キャッシュ先は空でない`XDG_CACHE_HOME`の下に置く。
    pub fn new(network: Network, config: Config) -> Self {
        let cache_dir = default_cache_dir(std::env::var_os("XDG_CACHE_HOME").as_deref());
        Self {
            network,
            backend: "tensorrt".to_owned(),
            default_cache_dir: cache_dir.clone(),
            cache_dir,
            evaluator: None,
            driver: None,
            config,
            #[cfg(test)]
            reused: false,
        }
    }

    /// ONNX Runtimeを使わない試験でだけ推論器を注入する。
    #[cfg(test)]
    pub(crate) fn with_evaluator(
        evaluator: Option<Box<dyn Evaluator + Send>>,
        config: Config,
    ) -> Self {
        let mut engine = Self::new(Network::from_bytes(Vec::new()), config);
        engine.evaluator = evaluator;
        engine
    }

    pub(in crate::search) fn options(&self) -> Vec<SearchOption> {
        vec![
            SearchOption {
                name: "USI_Hash",
                kind: SearchOptionKind::Spin {
                    default: 256,
                    min: 1,
                    max: 65_536,
                },
            },
            SearchOption {
                name: "Threads",
                kind: SearchOptionKind::Spin {
                    default: 1,
                    min: 1,
                    max: 1,
                },
            },
            SearchOption {
                name: "NnBackend",
                kind: SearchOptionKind::Combo {
                    default: "tensorrt",
                    choices: &["tensorrt", "cuda", "cpu"],
                },
            },
            SearchOption {
                name: "NnCacheDir",
                kind: SearchOptionKind::String {
                    default: self.default_cache_dir.to_string_lossy().into_owned(),
                },
            },
            SearchOption {
                name: "NnBatchSize",
                kind: SearchOptionKind::Spin {
                    default: 256,
                    min: 1,
                    max: 1024,
                },
            },
        ]
    }

    pub(in crate::search) fn set_option(
        &mut self,
        name: &str,
        value: SearchOptionValue,
    ) -> Result<(), SetOptionError> {
        let option = self
            .options()
            .into_iter()
            .find(|option| option.name == name)
            .ok_or_else(|| SetOptionError::UnknownOption {
                name: name.to_owned(),
            })?;
        option.validate(&value)?;
        match (option.name, value) {
            ("USI_Hash", SearchOptionValue::Spin(value)) => {
                let capacity_bytes =
                    value
                        .checked_mul(1024 * 1024)
                        .ok_or(SetOptionError::OutOfRange {
                            name: option.name,
                            value,
                            min: 1,
                            max: 65_536,
                        })?;
                if self.config.capacity_bytes != capacity_bytes {
                    self.clear();
                    self.config.capacity_bytes = capacity_bytes;
                }
            }
            ("Threads", SearchOptionValue::Spin(_)) => {}
            ("NnBackend", SearchOptionValue::Combo(value)) => {
                let backend = value.to_ascii_lowercase();
                if self.backend != backend {
                    self.backend = backend;
                    self.evaluator = None;
                }
            }
            ("NnCacheDir", SearchOptionValue::String(value)) => {
                let cache_dir = PathBuf::from(value);
                if self.cache_dir != cache_dir {
                    self.cache_dir = cache_dir;
                    self.evaluator = None;
                }
            }
            ("NnBatchSize", SearchOptionValue::Spin(value)) => {
                let batch_size = NonZeroUsize::new(value).expect("validated non-zero batch size");
                if self.config.batch_size != batch_size {
                    self.config.batch_size = batch_size;
                    self.evaluator = None;
                }
            }
            _ => unreachable!("all validated MCTS options are handled"),
        }
        Ok(())
    }

    fn execution_provider(&self) -> Result<ExecutionProvider, PrepareError> {
        match self.backend.as_str() {
            "tensorrt" if self.cache_dir.as_os_str().is_empty() => {
                Err(PrepareError::MissingCacheDir)
            }
            "tensorrt" => Ok(ExecutionProvider::TensorRt {
                cache_root: self.cache_dir.clone(),
            }),
            "cuda" => Ok(ExecutionProvider::Cuda),
            "cpu" => Ok(ExecutionProvider::Cpu),
            _ => unreachable!("the backend was validated when set"),
        }
    }

    pub(in crate::search) fn prepare(&mut self) -> Result<(), PrepareError> {
        if self.evaluator.is_none() {
            let provider = self.execution_provider()?;
            let evaluator = OnnxEvaluator::new(&self.network, &provider, self.config.batch_size)
                .map_err(PrepareError::Network)?;
            self.evaluator = Some(Box::new(evaluator));
        }
        Ok(())
    }

    pub(in crate::search) fn clear(&mut self) {
        self.driver = None;
    }

    #[allow(clippy::result_large_err)]
    pub(in crate::search) fn start(
        self,
        snapshot: SearchSnapshot,
        limits: SearchLimits,
        search_id: u64,
        ponder: bool,
    ) -> Result<SearchHandle, StartFailure> {
        self.start_with_clock(
            snapshot,
            limits,
            search_id,
            ponder,
            Arc::new(MonotonicClock::new()),
        )
    }

    #[allow(clippy::result_large_err)]
    fn start_with_clock(
        mut self,
        snapshot: SearchSnapshot,
        limits: SearchLimits,
        search_id: u64,
        ponder: bool,
        clock: Arc<dyn SearchClock>,
    ) -> Result<SearchHandle, StartFailure> {
        if let Err(error) = self.prepare_driver(snapshot, limits) {
            return Err(StartFailure {
                searcher: Searcher::Mcts(self),
                error: StartError::Mcts(error),
            });
        }
        Ok(SearchHandle::spawn(
            clock,
            ponder,
            move |clock, hit_ns, stop, sender| {
                let driver = self
                    .driver
                    .as_mut()
                    .expect("the driver was prepared before spawning");
                let evaluator = self
                    .evaluator
                    .as_mut()
                    .expect("the evaluator was checked before spawning");
                let stop_reason = run(
                    driver,
                    evaluator.as_mut(),
                    self.config,
                    limits,
                    clock.as_ref(),
                    &hit_ns,
                    &stop,
                )?;
                let pv = driver.tree().principal_variation();
                let score = driver
                    .score(self.config.cp_per_logit)
                    .map_err(SearchJoinError::Mcts)?;
                let best_move = driver
                    .tree()
                    .best_move()
                    .expect("an evaluated root has candidates");
                let _ = sender.send(SearchEvent::Finished {
                    partial_score: None,
                    search_id,
                    best_move,
                    score,
                    depth: pv.len() as u32,
                    nodes: driver.simulations(),
                    elapsed: clock.elapsed(),
                    pv,
                    stop_reason,
                });
                Ok(Searcher::Mcts(self))
            },
        ))
    }

    fn prepare_driver(
        &mut self,
        snapshot: SearchSnapshot,
        limits: SearchLimits,
    ) -> Result<(), Error> {
        if self.evaluator.is_none() {
            return Err(Error::MissingEvaluator);
        }
        if limits.depth().is_some() {
            return Err(Error::DepthLimitUnsupported);
        }
        validate_rules(&snapshot)?;
        validate_scale(self.config.cp_per_logit)?;
        self.config.puct.validate()?;
        let draw = DrawValue::Game {
            cp_per_logit: self.config.cp_per_logit,
        };
        // nodes 1は過去の訪問回数によらず、今回のネットの最大logitの手を返す。
        let reused = limits.nodes() != Some(1)
            && match &mut self.driver {
                Some(driver) => driver.reuse(&snapshot, draw)?,
                None => false,
            };
        if !reused {
            self.driver = Some(Box::new(Driver::new(
                snapshot,
                self.config.capacity_bytes,
                draw,
            )?));
        }
        #[cfg(test)]
        {
            self.reused = reused;
        }
        Ok(())
    }
}

fn default_cache_dir(xdg_cache_home: Option<&OsStr>) -> PathBuf {
    match xdg_cache_home {
        Some(path) if !path.is_empty() => PathBuf::from(path).join("minase/tensorrt"),
        _ => PathBuf::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    driver: &mut Driver,
    evaluator: &mut dyn Evaluator,
    config: Config,
    limits: SearchLimits,
    clock: &dyn SearchClock,
    hit_ns: &AtomicU64,
    stop: &AtomicBool,
) -> Result<StopReason, SearchJoinError> {
    let budget = time_budget(&limits);
    let rule = SelectionRule::Puct(&config.puct);
    let mut logits = Vec::new();
    let mut values = Vec::new();
    let started = clock.elapsed();
    let Step::Evaluate(root) = driver.select_leaf(rule).map_err(SearchJoinError::Mcts)? else {
        return Err(SearchJoinError::Mcts(
            super::tree::TreeError::InvalidState.into(),
        ));
    };
    evaluate_batch(driver, evaluator, &[root], &mut logits, &mut values)?;
    let mut batch_time = clock.elapsed().saturating_sub(started);
    let mut capacity_exhausted = false;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(StopReason::ExternalStop);
        }
        if !limits.is_infinite()
            && (capacity_exhausted
                || limits
                    .nodes()
                    .is_some_and(|limit| driver.simulations() >= limit))
        {
            return Ok(StopReason::NodeLimit);
        }
        if matches!(driver.tree().root().state, NodeState::Proven(_)) {
            return Ok(StopReason::DepthCompleted);
        }
        if let Some(reason) = time_stop(budget, batch_time, clock, hit_ns) {
            return Ok(reason);
        }
        if capacity_exhausted {
            // 無期限探索では容量不足を停止理由にせず、外部停止を待つ。
            std::thread::park_timeout(Duration::from_millis(1));
            continue;
        }
        let started = clock.elapsed();
        let mut requests = Vec::new();
        for _ in 0..config.batch_size.get() {
            if stop.load(Ordering::Relaxed)
                || limits
                    .nodes()
                    .is_some_and(|limit| driver.simulations() + requests.len() as u64 >= limit)
                || matches!(driver.tree().root().state, NodeState::Proven(_))
            {
                break;
            }
            match driver.select_leaf(rule).map_err(SearchJoinError::Mcts)? {
                Step::Evaluate(request) => requests.push(request),
                Step::BackedUp => {}
                Step::Collision => break,
                Step::Capacity => {
                    capacity_exhausted = true;
                    break;
                }
            }
        }
        if !requests.is_empty() {
            evaluate_batch(driver, evaluator, &requests, &mut logits, &mut values)?;
        }
        batch_time = clock.elapsed().saturating_sub(started);
    }
}

fn time_stop(
    budget: Option<TimeBudget>,
    batch_time: Duration,
    clock: &dyn SearchClock,
    hit_ns: &AtomicU64,
) -> Option<StopReason> {
    let budget = budget?;
    let hit = hit_ns.load(Ordering::Relaxed);
    if hit == u64::MAX {
        return None;
    }
    let elapsed = clock.elapsed().saturating_sub(Duration::from_nanos(hit));
    if elapsed >= budget.soft {
        Some(StopReason::SoftLimit)
    } else if elapsed.saturating_add(batch_time) >= budget.hard {
        Some(StopReason::HardLimit)
    } else {
        None
    }
}

fn evaluate_batch(
    driver: &mut Driver,
    evaluator: &mut dyn Evaluator,
    requests: &[EvaluationRequest],
    logits: &mut Vec<f32>,
    values: &mut Vec<f32>,
) -> Result<(), SearchJoinError> {
    let queries: Vec<_> = requests.iter().map(EvaluationRequest::query).collect();
    evaluator
        .evaluate(&queries, logits, values)
        .map_err(SearchJoinError::Inference)?;
    let total_logits = queries.iter().map(|query| query.moves.len()).sum::<usize>();
    for (name, expected, actual) in [
        ("values", requests.len(), values.len()),
        ("logits", total_logits, logits.len()),
    ] {
        if actual != expected {
            return Err(SearchJoinError::Inference(NnError::OutputShape {
                name,
                expected: [expected, 1],
                actual: vec![actual as i64, 1],
            }));
        }
    }
    let mut offset = 0;
    for (request, &value) in requests.iter().zip(values.iter()) {
        let end = offset + request.query().moves.len();
        driver
            .evaluate(request, &logits[offset..end], value)
            .map_err(SearchJoinError::Mcts)?;
        offset = end;
    }
    Ok(())
}

#[cfg(test)]
mod options_tests;
#[cfg(test)]
mod tests;
