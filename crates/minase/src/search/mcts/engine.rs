//! MCTSの探索の開始と停止、時間管理、木の再利用、および評価値の写像。

use core::fmt;
use core::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use minase_core::mv::Move;
use minase_core::rng::XorShift64;
use minase_core::rules::MoveRules;

use crate::eval::Pst;
use crate::nn::encoding::EncodingError;
use crate::nn::evaluator::{Evaluator, EvaluatorError};
use crate::nn::policy::PolicyLabelError;
use crate::search::alphabeta::time::time_budget;
use crate::search::events::{SearchEvent, SearchResult, StopReason};
use crate::search::limits::SearchLimits;
use crate::search::snapshot::{SearchSnapshot, search_key};
use crate::search::{MATE, MAX_PLY};

use super::select::{GUMBEL_PARAMS, GumbelRoot, Selection, improved_policy};
use super::simulate::{PositionDomain, Rule, Simulator};
use super::tree::{NodeState, Proven, Tree};

/// Gumbel探索の1手の探索回数nの初期値（docs/plans/alphazero.md「RLの探索はGumbel AlphaZeroにする」）。
pub const DEFAULT_GUMBEL_SIMULATIONS: NonZeroU32 = NonZeroU32::new(32).unwrap();

/// Gumbel探索の候補数の上限m（同節）。
pub const MAX_CONSIDERED_ACTIONS: NonZeroU32 = NonZeroU32::new(16).unwrap();

/// 根の勝率を評価値へ写す前に丸める範囲（docs/plans/alphazero.md「共存の構造」）。
const WIN_RATE_RANGE: (f64, f64) = (0.001, 0.999);

/// MCTSの探索を行えない理由。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MctsError {
    /// 深さの制限が指定された。MCTSには深さに対応する概念がない。
    DepthLimit,
    /// 同期探索に無期限探索が指定された。
    InfiniteSynchronousSearch,
    /// 局面を入力平面へ符号化できない。
    Encoding(EncodingError),
    /// 合法手が方策のラベルを持たない。
    Label(PolicyLabelError),
    /// 推論に失敗した。
    Evaluator(EvaluatorError),
}

impl fmt::Display for MctsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DepthLimit => formatter.write_str("MCTS does not support a depth limit"),
            Self::InfiniteSynchronousSearch => {
                formatter.write_str("synchronous search cannot use an infinite limit")
            }
            Self::Encoding(error) => write!(formatter, "MCTS cannot encode a position: {error}"),
            Self::Label(error) => write!(formatter, "MCTS cannot label a move: {error}"),
            Self::Evaluator(error) => write!(formatter, "MCTS evaluation failed: {error}"),
        }
    }
}

impl std::error::Error for MctsError {}

/// MCTSの探索の設定。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Config {
    /// 子の選択規則。
    pub selection: Selection,
    /// PUCTで1回の推論にまとめる葉の数の上限。Gumbelでは葉を1つずつ評価する。
    pub batch_size: NonZeroUsize,
    /// Gumbel探索の1手の探索回数n。根の展開は含めない。
    pub gumbel_simulations: NonZeroU32,
    /// Gumbel探索の候補数の上限m。
    pub max_considered: NonZeroU32,
    /// Gumbel乱数の生成器のシード。
    pub seed: NonZeroU64,
}

/// 次の探索で再利用する木と、その根を探索したときの対局の履歴と規則。
struct Previous {
    tree: Tree<Move>,
    history_keys: Vec<u64>,
    rules: MoveRules,
}

/// 停止要求と的中の時刻。
struct Control<'a> {
    /// 呼び出し側からの停止要求。
    stop: &'a AtomicBool,
    /// 的中までのナノ秒数。先読み中はu64::MAX。
    hit_ns: &'a AtomicU64,
    /// 探索の開始時刻。
    started: Instant,
}

/// 1回の探索の結果。
struct Outcome {
    best_move: Move,
    score: i32,
    depth: u32,
    nodes: u64,
    elapsed: Duration,
    pv: Vec<Move>,
    stop_reason: StopReason,
}

/// 推論器、再利用する探索木、および探索の設定を探索の間で持ち越すMCTS。
pub struct Engine {
    /// ネットの推論器。
    evaluator: Box<dyn Evaluator + Send>,
    /// 探索の設定。
    config: Config,
    /// 勝率を評価値へ写す尺度K（PSTの学習で推定した値）。
    win_rate_scale: f32,
    /// Gumbel乱数の生成器。
    rng: XorShift64,
    /// 直前の探索の木。
    previous: Option<Box<Previous>>,
}

impl Engine {
    /// 推論器、勝率の尺度Kを持つ学習PST、および設定から作る。
    pub fn new(evaluator: Box<dyn Evaluator + Send>, pst: &Pst, config: Config) -> Self {
        Self {
            evaluator,
            config,
            win_rate_scale: pst.k(),
            rng: XorShift64::new(config.seed),
            previous: None,
        }
    }

    /// 探索の設定を返す。
    pub fn config(&self) -> Config {
        self.config
    }

    /// 前の対局の探索木を捨てる。
    pub fn new_game(&mut self) {
        self.previous = None;
    }

    /// 呼び出しスレッドで探索する。
    ///
    /// # Errors
    ///
    /// 無期限探索または深さの制限が指定された場合、局面を符号化できない場合、
    /// および推論に失敗した場合はエラーを返す。
    pub fn search(
        &mut self,
        snapshot: &SearchSnapshot,
        limits: &SearchLimits,
    ) -> Result<SearchResult, MctsError> {
        if limits.is_infinite() {
            return Err(MctsError::InfiniteSynchronousSearch);
        }
        let stop = AtomicBool::new(false);
        let hit_ns = AtomicU64::new(0);
        let control = Control {
            stop: &stop,
            hit_ns: &hit_ns,
            started: Instant::now(),
        };
        let outcome = self.run(snapshot, limits, &control, None)?;
        Ok(SearchResult {
            #[cfg(feature = "search-stats")]
            stats: crate::search::SearchStats::default(),
            best_move: outcome.best_move,
            score: outcome.score,
            depth: outcome.depth,
            nodes: outcome.nodes,
        })
    }

    /// 状態を探索スレッドへ移して探索を開始する。`ponder`が真なら時間制限は的中の通知まで無効とする。
    ///
    /// 探索スレッドは進捗を[`SearchEvent::Progress`]で、終了を[`SearchEvent::Finished`]で送る。
    /// 探索中に推論などが失敗した場合は[`SearchEvent::Finished`]を送らず、
    /// [`Handle::join`]が理由を返す。
    ///
    /// # Errors
    ///
    /// 深さの制限が指定された場合は、探索を開始せずに状態とエラーを返す。
    pub fn start(
        mut self,
        snapshot: SearchSnapshot,
        limits: SearchLimits,
        search_id: u64,
        ponder: bool,
    ) -> Result<Handle, (Self, MctsError)> {
        if limits.depth().is_some() {
            return Err((self, MctsError::DepthLimit));
        }
        let started = Instant::now();
        let hit_ns = Arc::new(AtomicU64::new(if ponder { u64::MAX } else { 0 }));
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, events) = mpsc::channel();
        let thread_hit_ns = Arc::clone(&hit_ns);
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            let control = Control {
                stop: &thread_stop,
                hit_ns: &thread_hit_ns,
                started,
            };
            let result = self
                .run(&snapshot, &limits, &control, Some((&sender, search_id)))
                .map(|outcome| {
                    let _ = sender.send(SearchEvent::Finished {
                        partial_score: None,
                        search_id,
                        best_move: outcome.best_move,
                        score: outcome.score,
                        depth: outcome.depth,
                        nodes: outcome.nodes,
                        elapsed: outcome.elapsed,
                        pv: outcome.pv,
                        stop_reason: outcome.stop_reason,
                    });
                });
            (self, result)
        });
        Ok(Handle {
            events,
            started,
            hit_ns,
            stop,
            thread: Some(thread),
        })
    }

    /// 直前の探索の根の改善後の方策softmax(logit＋σ(completed Q))を、根の着手ごとに返す。
    ///
    /// Gumbel探索の自己対局で方策の目標に使う（docs/plans/alphazero.md「強化学習」）。
    /// 根を評価した探索がなければ`None`を返す。
    pub fn policy_target(&self) -> Option<Vec<(Move, f32)>> {
        let root = self.previous.as_ref()?.tree.root();
        (root.state == NodeState::Ready).then(|| {
            root.edges
                .iter()
                .zip(improved_policy(root, GUMBEL_PARAMS))
                .map(|(edge, prob)| (edge.action, prob as f32))
                .collect()
        })
    }

    /// 探索木を用意して探索し、成功した場合だけ木を次の探索のために残す。
    fn run(
        &mut self,
        snapshot: &SearchSnapshot,
        limits: &SearchLimits,
        control: &Control<'_>,
        events: Option<(&mpsc::Sender<SearchEvent>, u64)>,
    ) -> Result<Outcome, MctsError> {
        if limits.depth().is_some() {
            return Err(MctsError::DepthLimit);
        }
        let mut tree = self.take_tree(snapshot);
        let outcome = self.search_tree(&mut tree, snapshot, limits, control, events)?;
        self.previous = Some(Box::new(Previous {
            tree,
            history_keys: snapshot.history_keys().to_vec(),
            rules: snapshot.rules(),
        }));
        Ok(outcome)
    }

    /// 直前の探索の木から、新しい根の部分木を取り出す。再利用できなければ根だけの木を返す。
    ///
    /// 新しい対局の履歴が直前の探索の履歴を延長したもので、その間の局面が木の中で展開済みなら、
    /// 最後の局面の部分木を根とする。根の辺は対局管理層が確定した合法手（R2とR3の禁止を除いたもの）
    /// に絞る。Gumbel探索は根の訪問回数0から予算を配るので、木を再利用しない。
    fn take_tree(&mut self, snapshot: &SearchSnapshot) -> Tree<Move> {
        let previous = self.previous.take();
        let reused = previous
            .filter(|_| self.config.selection == Selection::Puct)
            .and_then(|previous| {
                let history = snapshot.history_keys();
                if previous.rules != snapshot.rules()
                    || previous.history_keys.is_empty()
                    || !history.starts_with(&previous.history_keys)
                {
                    return None;
                }
                let mut tree = previous
                    .tree
                    .subtree(&history[previous.history_keys.len()..])?;
                tree.retain_root_actions(snapshot.root_moves())
                    .then_some(tree)
            });
        reused.unwrap_or_else(|| Tree::new(search_key(snapshot.position())))
    }

    /// 根を展開し、停止条件まで繰り返しシミュレーションを行う。
    fn search_tree(
        &mut self,
        tree: &mut Tree<Move>,
        snapshot: &SearchSnapshot,
        limits: &SearchLimits,
        control: &Control<'_>,
        events: Option<(&mpsc::Sender<SearchEvent>, u64)>,
    ) -> Result<Outcome, MctsError> {
        let k = self.win_rate_scale;
        let config = self.config;
        let evaluator = self.evaluator.as_mut();
        let mut simulator = Simulator::new(PositionDomain::new(
            snapshot.position().clone(),
            snapshot.rules(),
        ));
        let mut nodes = u64::from(simulator.expand_root(tree, evaluator, snapshot.root_moves())?);

        // 探索回数の上限が1なら、方策logitが最大の合法手を返す（alphazero.md「教師あり学習」の第2の診断）。
        if limits.nodes() == Some(1) {
            let best = policy_best_edge(tree);
            return Ok(outcome(
                tree,
                best,
                k,
                nodes,
                control,
                StopReason::NodeLimit,
            ));
        }

        let gumbel =
            (config.selection == Selection::Gumbel && tree.root().proven.is_none()).then(|| {
                let noise = (0..tree.root().edges.len())
                    .map(|_| gumbel_noise(&mut self.rng))
                    .collect();
                GumbelRoot::new(
                    noise,
                    config.gumbel_simulations.get(),
                    config.max_considered.get(),
                    GUMBEL_PARAMS,
                )
            });
        let rule = match &gumbel {
            Some(root) => Rule::Gumbel(root, GUMBEL_PARAMS),
            None => Rule::Puct,
        };
        let batch = match gumbel {
            Some(_) => 1,
            None => config.batch_size.get(),
        };
        let budget = time_budget(limits);
        let mut next_report = 1;
        let stop_reason = loop {
            if nodes >= next_report {
                report(tree, gumbel.as_ref(), k, nodes, control, events);
                while next_report <= nodes {
                    next_report *= 2;
                }
            }
            if control.stop.load(Ordering::Relaxed) {
                break StopReason::ExternalStop;
            }
            if tree.root().proven.is_some() {
                break StopReason::DepthCompleted;
            }
            let mut remaining = limits
                .nodes()
                .map_or(u64::MAX, |limit| limit.saturating_sub(nodes));
            if let Some(root) = &gumbel {
                remaining = remaining.min(u64::from(
                    root.simulations()
                        .saturating_sub(tree.root().total_visits()),
                ));
            }
            if remaining == 0 {
                break StopReason::NodeLimit;
            }
            if let Some(budget) = budget {
                let hit_ns = control.hit_ns.load(Ordering::Relaxed);
                if hit_ns != u64::MAX
                    && control
                        .started
                        .elapsed()
                        .saturating_sub(Duration::from_nanos(hit_ns))
                        >= budget.soft
                {
                    break StopReason::SoftLimit;
                }
            }
            let max_leaves =
                usize::try_from(remaining).map_or(batch, |remaining| batch.min(remaining));
            nodes += u64::from(simulator.run_batch(
                tree,
                self.evaluator.as_mut(),
                &rule,
                snapshot.history_keys(),
                max_leaves,
            )?);
        };
        let best = best_edge(tree, gumbel.as_ref());
        Ok(outcome(tree, best, k, nodes, control, stop_reason))
    }

    /// 試験用に、次の探索で再利用する木を返す。
    #[cfg(test)]
    pub(super) fn previous_tree(&self) -> Option<&Tree<Move>> {
        self.previous.as_ref().map(|previous| &previous.tree)
    }
}

/// 現在の最善手と評価値を進捗として送る。
fn report(
    tree: &Tree<Move>,
    gumbel: Option<&GumbelRoot>,
    k: f32,
    nodes: u64,
    control: &Control<'_>,
    events: Option<(&mpsc::Sender<SearchEvent>, u64)>,
) {
    let Some((sender, search_id)) = events else {
        return;
    };
    let best = best_edge(tree, gumbel);
    let pv = tree.principal_variation(best);
    let _ = sender.send(SearchEvent::Progress {
        search_id,
        depth: pv.len() as u32,
        score: score(tree, best, k),
        nodes,
        elapsed: control.started.elapsed(),
        pv,
    });
}

/// 根の辺`best`を指す結果を作る。
fn outcome(
    tree: &Tree<Move>,
    best: usize,
    k: f32,
    nodes: u64,
    control: &Control<'_>,
    stop_reason: StopReason,
) -> Outcome {
    let pv = tree.principal_variation(best);
    Outcome {
        best_move: pv[0],
        score: score(tree, best, k),
        depth: pv.len() as u32,
        nodes,
        elapsed: control.started.elapsed(),
        pv,
        stop_reason,
    }
}

/// 根の評価値を返す（docs/plans/alphazero.md「共存の構造」）。
///
/// 勝敗が確定していれば`MATE`から終局までの手数を引いた詰みの帯の値を返す。
/// そうでなければ、辺`best`のQ（未訪問なら根のネットの勝率）pを[0.001, 0.999]に丸め、
/// 勝率の尺度Kを使ってK·log(p/(1−p))をセンチポーン相当の値とする。
fn score(tree: &Tree<Move>, best: usize, k: f32) -> i32 {
    let root = tree.root();
    match root.proven {
        Some(Proven::Win(plies)) => MATE - plies.min(MAX_PLY) as i32,
        Some(Proven::Loss(plies)) => -MATE + plies.min(MAX_PLY) as i32,
        None => {
            let (low, high) = WIN_RATE_RANGE;
            let p = root.edges[best]
                .q()
                .unwrap_or(f64::from(root.raw_value))
                .clamp(low, high);
            (f64::from(k) * (p / (1.0 - p)).ln()).round() as i32
        }
    }
}

/// 探索後に指す根の辺を選ぶ。勝敗が確定していない根のGumbel探索はGumbelの規則で、
/// それ以外は訪問回数の最も多い辺（[`super::tree::Node::best_edge`]）を選ぶ。
fn best_edge(tree: &Tree<Move>, gumbel: Option<&GumbelRoot>) -> usize {
    let root = tree.root();
    match gumbel {
        Some(gumbel) if root.proven.is_none() => gumbel.final_action(root),
        _ => root
            .best_edge()
            .expect("an expanded search root has at least one edge"),
    }
}

/// 方策logitが最大の根の辺を返す。根の勝敗が確定していればその辺を返す。
fn policy_best_edge(tree: &Tree<Move>) -> usize {
    let root = tree.root();
    if root.proven.is_some() {
        return best_edge(tree, None);
    }
    (0..root.edges.len())
        .reduce(|best, index| {
            if root.edges[index].logit > root.edges[best].logit {
                index
            } else {
                best
            }
        })
        .expect("an expanded search root has at least one edge")
}

/// 標準Gumbel分布に従う乱数 −log(−log u) を返す。uは(0, 1)の一様乱数。
fn gumbel_noise(rng: &mut XorShift64) -> f64 {
    let uniform = ((rng.next() >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
    -(-uniform.ln()).ln()
}

/// 実行中のMCTSの探索スレッドを操作するハンドル。
///
/// 値を破棄すると探索スレッドへ停止を要求し、終了するまで待つ。
pub struct Handle {
    /// 探索イベントの受信端。
    events: mpsc::Receiver<SearchEvent>,
    /// スレッド生成前に取得した起点。
    started: Instant,
    /// 的中までのナノ秒数。先読み中はu64::MAX。
    hit_ns: Arc<AtomicU64>,
    /// 探索スレッドと共有する停止要求。
    stop: Arc<AtomicBool>,
    /// 探索の終了後に状態と結果を返すスレッド。
    thread: Option<thread::JoinHandle<(Engine, Result<(), MctsError>)>>,
}

impl Handle {
    /// 探索イベントの受信端を返す。
    pub fn events(&self) -> &mpsc::Receiver<SearchEvent> {
        &self.events
    }

    /// 先読みを的中の時点から計時する。重複した通知は無視する。
    pub fn ponderhit(&self) {
        let elapsed = self
            .started
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX - 1)) as u64;
        let _ =
            self.hit_ns
                .compare_exchange(u64::MAX, elapsed, Ordering::Relaxed, Ordering::Relaxed);
    }

    /// 探索スレッドへ停止を要求する。
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// 停止を要求して探索スレッドの終了を待ち、状態と探索の成否を返す。
    ///
    /// 探索スレッドがパニックした場合は、そのペイロードを`Err`で返す。
    pub fn join(mut self) -> thread::Result<(Engine, Result<(), MctsError>)> {
        self.stop_and_join()
            .expect("a live Handle must own its search thread")
    }

    /// 停止要求と探索スレッドの回収を1回だけ行う。
    fn stop_and_join(&mut self) -> Option<thread::Result<(Engine, Result<(), MctsError>)>> {
        self.request_stop();
        self.thread.take().map(thread::JoinHandle::join)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}
