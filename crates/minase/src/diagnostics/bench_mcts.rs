//! 固定15局面をハッシュの評価器でPV-MCTS探索し、根の全統計を出力する。
//!
//! `nodes`は1〜4,294,967,295で既定800、`batch`は1〜1,024で既定256。
//! 規則はJCSA、探索設定は既定値とし、各局面に新しい木を作る。
//! セッションの局面、設定、推論器、および探索木には依存しない。
//! 出力は探索条件、局面ごとの最善手・停止理由・主変化・根の統計、全局面の集計の順。
//! 時間の欄を除く出力を、同じ機械とビルド設定での回帰検査に使う。

use std::fmt;
use std::io;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::time::{Duration, Instant};

use minase_core::notation::usi;
use minase_core::{Game, Rules, parse_sfen};

use super::bench::BENCH_POSITIONS;
use crate::nn::hash_evaluator::HashEvaluator;
use crate::search::mcts::{self, DrawValue, Driver, SearchConfig, move_stats};
use crate::search::{MonotonicClock, SearchJoinError, SearchLimits, SearchSnapshot};

/// 省略時のシミュレーション数。
pub(crate) const DEFAULT_NODES: NonZeroU32 = NonZeroU32::new(800).unwrap();
/// 省略時の推論バッチ上限。
pub(crate) const DEFAULT_BATCH: NonZeroUsize = NonZeroUsize::new(256).unwrap();

/// PV-MCTSのbenchの検査済み引数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Arguments {
    nodes: NonZeroU32,
    batch: NonZeroUsize,
}

/// バッチ上限が許容範囲を超えた。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ArgumentError;

impl fmt::Display for ArgumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("batch must be from 1 to 1024")
    }
}

impl std::error::Error for ArgumentError {}

impl Arguments {
    /// 正のシミュレーション数と、1〜1,024のバッチ上限を受け付ける。
    pub(crate) fn new(nodes: NonZeroU32, batch: NonZeroUsize) -> Result<Self, ArgumentError> {
        if batch.get() > 1024 {
            return Err(ArgumentError);
        }
        Ok(Self { nodes, batch })
    }
}

/// 固定局面の準備、探索、または出力に失敗した原因。
#[derive(Debug)]
pub(crate) enum Error {
    Position(minase_core::notation::sfen::SfenError),
    Snapshot(crate::search::SearchError),
    Driver(mcts::Error),
    Search(mcts::Error),
    Inference(crate::nn::NnError),
    Output(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Position(error) => write!(f, "bench mcts position is invalid: {error}"),
            Self::Snapshot(error) => write!(f, "bench mcts search input is invalid: {error}"),
            Self::Driver(error) => write!(f, "bench mcts driver failed: {error}"),
            Self::Search(error) => write!(f, "bench mcts search failed: {error}"),
            Self::Inference(error) => write!(f, "bench mcts inference failed: {error}"),
            Self::Output(error) => write!(f, "bench mcts output failed: {error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Position(error) => Some(error),
            Self::Snapshot(error) => Some(error),
            Self::Driver(error) => Some(error),
            Self::Search(error) => Some(error),
            Self::Inference(error) => Some(error),
            Self::Output(error) => Some(error),
        }
    }
}

impl From<SearchJoinError> for Error {
    fn from(error: SearchJoinError) -> Self {
        match error {
            SearchJoinError::Mcts(error) => Self::Search(error),
            SearchJoinError::Inference(error) => Self::Inference(error),
            // 同期のrunではPanicを作らないため、届いた場合は元のパニックとして再開する。
            SearchJoinError::Panic(payload) => std::panic::resume_unwind(payload),
        }
    }
}

/// 対局と同じ探索ループを呼び、接頭辞なしの診断行を出力先へ渡す。
pub(crate) fn run(
    arguments: Arguments,
    output: &mut dyn FnMut(&str) -> io::Result<()>,
) -> Result<(), Error> {
    let config = SearchConfig {
        batch_size: arguments.batch,
        ..SearchConfig::default()
    };
    let puct = config.puct;
    output(&format!(
        "bench mcts nodes {} batch {} capacity-bytes {} policy-temperature {} c-init {} c-base {} c-fpu {} c-init-root {} c-base-root {} c-fpu-root {}",
        arguments.nodes, config.batch_size, config.capacity_bytes, config.policy_temperature,
        puct.c_init, puct.c_base, puct.c_fpu, puct.c_init_root, puct.c_base_root, puct.c_fpu_root,
    )).map_err(Error::Output)?;
    let limits = SearchLimits::new(None, Some(u64::from(arguments.nodes.get())), None, None)
        .map_err(Error::Snapshot)?;
    let mut total_simulations = 0_u64;
    let mut total_elapsed = Duration::ZERO;
    for entry in BENCH_POSITIONS {
        let position = parse_sfen(entry.sfen).map_err(Error::Position)?;
        let game = Game::from_position(Rules::JCSA, position);
        let snapshot = SearchSnapshot::from_game(&game).map_err(Error::Snapshot)?;
        let mut driver = Driver::new(snapshot, config.capacity_bytes, DrawValue::SelfPlay)
            .map_err(Error::Driver)?;
        let clock = MonotonicClock::new();
        let start = Instant::now();
        let stop = mcts::run(
            &mut driver,
            &mut HashEvaluator,
            &config,
            limits,
            &clock,
            &AtomicU64::new(0),
            &AtomicBool::new(false),
        )
        .map_err(Error::from)?;
        let elapsed = start.elapsed();
        total_elapsed += elapsed;
        total_simulations += driver.simulations();
        let tree = driver.tree();
        let best = tree
            .best_move()
            .expect("a completed search has an evaluated root with moves");
        let mut position = game.position().clone();
        let mut pv = Vec::new();
        for mv in tree.principal_variation() {
            pv.push(usi::text_generated(&position, mv));
            position.make_move_unchecked(mv, Rules::JCSA.moves);
        }
        let mut line = format!(
            "position {} best {} stop {} pv",
            entry.name,
            usi::text_generated(game.position(), best),
            stop.as_str()
        );
        if !pv.is_empty() {
            line.push(' ');
            line.push_str(&pv.join(" "));
        }
        output(&line).map_err(Error::Output)?;
        for line in move_stats::lines(game.position(), tree, driver.simulations(), elapsed, None) {
            output(&line).map_err(Error::Output)?;
        }
    }
    let sps = if total_elapsed.is_zero() {
        0.0
    } else {
        total_simulations as f64 / total_elapsed.as_secs_f64()
    };
    output(&format!(
        "summary simulations {total_simulations} elapsed-ms {} sps {sps:.0}",
        total_elapsed.as_millis()
    ))
    .map_err(Error::Output)
}

#[cfg(test)]
mod tests;
