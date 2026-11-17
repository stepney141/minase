//! 固定15局面をハッシュの評価器でPV-MCTS探索し、根の全統計を出力する。
//!
//! `--search mcts`で起動したときの`bench`は、次の引数を受け付ける。
//!
//! ```text
//! bench [nodes <N>] [batch <N>]
//! ```
//!
//! `nodes`は1局面のシミュレーションの上限回数で、省略時は800、範囲は1〜4,294,967,295である。
//! `batch`は1回の推論に送る葉の数の上限で、省略時は256、範囲は1〜1,024である。
//! `depth`、`threads`、および`repetitions`は、`crates/minase/src/protocol/usi/diagnostics.rs`で`<語> requires --search alphabeta`として拒否する。
//! `--features tuning`のビルドでは、引数を受け付けて実行した時点から`Tune_`で始まるオプションの変更を拒否する。
//!
//! 局面集はαβ探索の`bench`と同じ15局面を[`super::bench::BENCH_POSITIONS`]から共有する。
//! 規則は日本中将棋連盟の規則に基づく`JCSA`に固定し、各局面で新しい探索木を作る。
//! 木の容量256 MiBと`batch`の既定値256は、それぞれ`USI_Hash`と`NnBatchSize`の既定値と同じである。
//! 引き分けには、評価値の既定値0に当たる勝率0.5を返す[`DrawValue::SelfPlay`]を使う。
//! PUCTの係数と方策の温度は`SearchConfig::default()`から取り、セッションの`PolicyTemperature`は使わない。
//! 対局と同じ[`crate::search::mcts::run`]に、時間の制限なしでシミュレーションの上限回数だけを与え、評価器には[`HashEvaluator`]を使う。
//! セッションの局面、設定、推論器、および探索木には依存せず、規則の設定も`isready`によるネットの準備も要らない。
//!
//! 出力は探索条件の行に始まり、各局面の結果と根の統計が続き、全局面の集計で終わる。
//! 各行の書式は次のとおりで、`position`から`movestats move`までを局面ごとに出力する。
//!
//! ```text
//! bench mcts nodes <N> batch <N> capacity-bytes <容量> policy-temperature <温度> c-init <値> c-base <値> c-fpu <値> c-init-root <値> c-base-root <値> c-fpu-root <値>
//! position <局面名> best <着手> stop <nodes|depth> pv <着手の列>
//! movestats root ...
//! movestats move ...
//! summary simulations <合計> elapsed-ms <合計> sps <毎秒のシミュレーション数>
//! ```
//!
//! `movestats root`は根の要約を、`movestats move`は全候補手の統計を1手1行で示し、各欄の定義は[`crate::search::mcts::move_stats::lines`]に従う。
//! USIの層が各行に`info string `を付け、成功時だけ末尾に`info string end`を加える。
//! 停止理由は、上限回数に達したときと木の容量が尽きたときは`nodes`、根の勝敗が確定して上限より前に止まったときは`depth`となる。
//! 勝率から評価値への換算に使う勝率尺度はセッションの設定に依存するため、評価値をセンチポーン（cp）では出さず、根の表の`q`に勝率を示す。
//!
//! 比較する前に、両方の実行が終了コード0で終わり、`info string error:`の行がなく、15局面の`position`の行と`info string end`があることを確かめる。
//! 同じ機械で同じ設定でビルドした2つのコミットの出力が、`elapsed-ms`と`sps`の値を除いて全行一致すれば、探索の結果は変わっていないと判定する。
//! softmax、対数、および平方根の浮動小数点の計算は機械や数学ライブラリによって変わり得るため、機械をまたいだ一致は求めない。
//!
//! 設計判断の理由は`docs/adr/run-bench-with-the-launched-search-and-detect-pv-mcts-regressions-with-a-hash-evaluator.md`にある。

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
