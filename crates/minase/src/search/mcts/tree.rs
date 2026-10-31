//! 探索木の所有、葉の受け渡し、価値と勝敗の逆伝播。
//!
//! 価値は節点の手番側から見た勝率であり、辺の価値の和は親の手番側から見る。
//! 確定した勝敗の順序は[`crate::search::MATE`]と[`crate::search::REPETITION_WIN`]に従う。
//! 確定までの手数は節点から数えるので、木を付け替えても変化しない。

use std::cmp::Ordering;
use std::fmt;
use std::mem::size_of;

use minase_core::Move;

use super::gumbel::Gumbel;
use super::puct::{self, PuctConfig};
use super::softmax;
use crate::search::{MATE, MAX_PLY, REPETITION_WIN};

/// 節点の手番側から見た確定済みの勝敗。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// 手番側の勝ち。
    Win,
    /// 手番側の負け。
    Loss,
}

/// 勝敗の確定、その距離、評価値の帯、および反復の近似への依存。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Proof {
    /// この節点の手番側から見た勝敗。
    pub outcome: Outcome,
    /// この節点から終局までの手数。
    pub distance: u32,
    /// 反復の帯を使うなら真。正確なR1の裁定も含む。
    pub repetition: bool,
    /// 反復の近似による打ち切りに依存するなら真。
    pub approximate: bool,
}

impl Proof {
    /// この節点を根としたαβ探索と同じ帯の評価値を返す。
    pub fn score(self) -> i32 {
        let magnitude = if self.repetition {
            REPETITION_WIN
        } else {
            MATE
        } - self.distance as i32;
        match self.outcome {
            Outcome::Win => magnitude,
            Outcome::Loss => -magnitude,
        }
    }

    fn value(self) -> f32 {
        match self.outcome {
            Outcome::Win => 1.0,
            Outcome::Loss => 0.0,
        }
    }

    fn for_parent(self) -> Self {
        Self {
            outcome: match self.outcome {
                Outcome::Win => Outcome::Loss,
                Outcome::Loss => Outcome::Win,
            },
            distance: self.distance + 1,
            repetition: self.repetition,
            approximate: self.approximate,
        }
    }
}

/// 節点の評価と探索の状態。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NodeState {
    /// 合法手は受け取り済みで、ネットの評価を待つ。
    Pending {
        /// 評価後も子を展開しない、深さ上限の葉なら真。
        unexpanded: bool,
    },
    /// 評価済みで、辺の選択と子の作成ができる。
    Expanded,
    /// 評価済みで、訪れるたびにネットの価値を返す葉。
    Unexpanded,
    /// 手番側の勝敗が確定している。
    Proven(Proof),
    /// 手番側の勝率を固定した引き分け。
    Draw {
        /// この節点の手番側から見た引き分けの勝率。
        value: f32,
        /// 反復の近似による打ち切りなら真。審判層の裁定では偽。
        approximate: bool,
    },
}

/// 親から子への手と、親の手番側から見た統計。
#[derive(Debug)]
pub struct Edge {
    /// この辺を進む手。
    pub mv: Move,
    /// ネットの方策のlogit。評価待ちでは未設定。
    pub logit: f32,
    /// 節点の全logitのsoftmax。評価待ちでは未設定。
    pub policy: f32,
    /// 完了したシミュレーションの回数。
    pub visits: u32,
    /// 勝率0の仮想的な訪問の数。
    pub virtual_loss: u32,
    /// 親の手番側から見た勝率の和。仮想損失は加えない。
    pub value_sum: f64,
    /// 作成済みの子。局面の合流は行わない。
    pub child: Option<Box<Node>>,
}

impl Edge {
    /// 訪問済みなら平均勝率を返す。
    pub fn q(&self) -> Option<f32> {
        (self.visits > 0).then(|| self.value_sum as f32 / self.visits as f32)
    }

    fn parent_proof(&self) -> Option<Proof> {
        match self.child.as_ref()?.state {
            NodeState::Proven(proof) => Some(proof.for_parent()),
            _ => None,
        }
    }
}

/// 局面を持たず、評価と子への辺だけを持つ節点。
#[derive(Debug)]
pub struct Node {
    /// ネットによる手番側の勝率。ネット未評価の節点では`None`。
    pub value: Option<f32>,
    /// 評価待ち、展開済み、または終端の状態。
    pub state: NodeState,
    /// 駆動部から渡された順序の辺。
    pub edges: Box<[Edge]>,
    /// PUCTで辺を選ぶたびに足した方策の確率の和。
    pub visited_policy: f32,
}

impl Node {
    /// この節点または子孫に、反復の近似で打ち切った引き分けか勝敗の確定があるかを返す。
    /// 自己対局では、反復の近似値に基づく投了を避けるために使う。
    pub fn has_approximate_result(&self) -> bool {
        matches!(
            self.state,
            NodeState::Draw {
                approximate: true,
                ..
            }
        ) || matches!(self.state, NodeState::Proven(proof) if proof.approximate)
            || self.edges.iter().any(|edge| {
                edge.child
                    .as_deref()
                    .is_some_and(Node::has_approximate_result)
            })
    }

    fn new(moves: &[Move], state: NodeState) -> Self {
        Self {
            value: None,
            state,
            edges: moves
                .iter()
                .map(|&mv| Edge {
                    mv,
                    logit: f32::NAN,
                    policy: f32::NAN,
                    visits: 0,
                    virtual_loss: 0,
                    value_sum: 0.0,
                    child: None,
                })
                .collect(),
            visited_policy: 0.0,
        }
    }

    fn fixed_value(&self) -> Option<f32> {
        match self.state {
            NodeState::Proven(proof) => Some(proof.value()),
            NodeState::Draw { value, .. } => Some(value),
            NodeState::Unexpanded => self.value,
            _ => None,
        }
    }

    fn refresh_proof(&mut self) {
        if self.edges.is_empty() {
            return;
        }
        let mut best_win: Option<Proof> = None;
        let mut best_loss: Option<Proof> = None;
        let mut all_losses = true;
        let mut approximate_loss = false;
        for edge in &self.edges {
            match edge.parent_proof() {
                Some(proof) if proof.outcome == Outcome::Win => {
                    all_losses = false;
                    if best_win.is_none_or(|best| proof.score() > best.score()) {
                        best_win = Some(proof);
                    }
                }
                Some(proof) => {
                    approximate_loss |= proof.approximate;
                    if best_loss.is_none_or(|best| proof.score() > best.score()) {
                        best_loss = Some(proof);
                    }
                }
                None => all_losses = false,
            }
        }
        if let Some(proof) = best_win {
            self.state = NodeState::Proven(proof);
        } else if all_losses {
            let mut proof = best_loss.expect("すべての辺に負けの確定がある");
            proof.approximate |= approximate_loss;
            self.state = NodeState::Proven(proof);
        }
    }

    /// 確定した勝ち、未確定、確定した負けの順に最善の辺の添字を返す。
    ///
    /// 未確定同士は訪問回数、訪問済みならQ、未訪問ならlogitで比べる。
    /// それでも同じなら先の辺を採り、評価待ちまたは辺なしなら`None`を返す。
    pub fn best_edge(&self) -> Option<usize> {
        if matches!(self.state, NodeState::Pending { .. }) {
            return None;
        }
        let mut best = None;
        for (index, edge) in self.edges.iter().enumerate() {
            if best.is_none_or(|previous| compare_edges(edge, &self.edges[previous]).is_gt()) {
                best = Some(index);
            }
        }
        best
    }

    fn subtree_bytes(&self) -> usize {
        size_of::<Self>()
            + self.edges.len() * size_of::<Edge>()
            + self
                .edges
                .iter()
                .filter_map(|edge| edge.child.as_ref())
                .map(|child| child.subtree_bytes())
                .sum::<usize>()
    }
}

fn compare_edges(a: &Edge, b: &Edge) -> Ordering {
    let rank = |edge: &Edge| match edge.parent_proof() {
        Some(proof) if proof.outcome == Outcome::Win => 2,
        Some(_) => 0,
        None => 1,
    };
    rank(a)
        .cmp(&rank(b))
        .then_with(|| match (a.parent_proof(), b.parent_proof()) {
            (Some(a), Some(b)) => a.score().cmp(&b.score()),
            (None, None) => a.visits.cmp(&b.visits).then_with(|| {
                let (a, b) = if a.visits > 0 {
                    (a.q().unwrap(), b.q().unwrap())
                } else {
                    (a.logit, b.logit)
                };
                a.partial_cmp(&b).expect("評価済みの辺の値は有限")
            }),
            _ => Ordering::Equal,
        })
}

/// 1回の降下で使う選択規則。
#[derive(Clone, Copy, Debug)]
pub enum SelectionRule<'a> {
    /// 対局用。経路の各辺に仮想損失を加える。
    Puct(&'a PuctConfig),
    /// 自己対局用。未完了の選択は1つまでとし、仮想損失は使わない。
    Gumbel(&'a Gumbel),
}

/// 降下が止まった理由。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LeafKind {
    /// 末端の辺に子がなく、駆動部による子の作成を待つ。
    New,
    /// ネットを呼ばず逆伝播できる、葉の手番側の勝率。
    Value(f32),
    /// 評価待ちの節点、または作成待ちの同じ辺に衝突した。
    Collision,
}

/// 選択した葉への経路と、駆動部が次に行う処理。
#[derive(Debug, PartialEq)]
pub struct Selection {
    /// 根から順にたどる辺の添字の列。
    pub path: Vec<usize>,
    /// 子の作成、既知の値の逆伝播、または衝突。
    pub kind: LeafKind,
}

/// 駆動部が局面を調べた結果として作成する子の種類。
#[derive(Debug)]
pub enum Child<'a> {
    /// 合法手を持つ、評価待ちの節点。
    Pending(&'a [Move]),
    /// 深さ上限に達した、辺を持たない評価待ちの葉。
    Unexpanded,
    /// 勝敗の確定した節点。作成と同時に逆伝播する。
    Proven(Proof),
    /// 手番側の勝率を固定した引き分け。作成と同時に逆伝播する。
    Draw {
        /// この節点の手番側から見た引き分けの勝率。
        value: f32,
        /// 反復の近似による打ち切りなら真。審判層の裁定では偽。
        approximate: bool,
    },
}

/// 木の操作が契約を満たさない場合、または容量や予算が尽きた場合のエラー。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TreeError {
    /// 節点と辺の合計が容量を超える。
    CapacityExceeded,
    /// 展開する節点の合法手が空である。
    EmptyMoves,
    /// 経路に対応する節点または辺がない。
    InvalidPath,
    /// 未完了の選択に対応しない操作である。
    InvalidSelection,
    /// 指定した操作を受け付ける節点の状態ではない。
    InvalidState,
    /// ネットの評価がまだない。
    NotEvaluated,
    /// logitの数、有限性、または勝率の範囲が不正である。
    InvalidEvaluation,
    /// 根からの手数、または確定までの手数が上限を超える。
    DepthLimit,
    /// 未完了の選択があり、Gumbelの追加選択や木の付け替えができない。
    PendingSimulation,
    /// PUCTの係数が不正である。
    InvalidPuctConfig,
    /// Gumbel乱数の数、有限性、候補数、予算、Qの尺度、または乱数の倍率が不正である。
    InvalidGumbelConfig,
    /// Gumbelのシミュレーションの予算を使い切った。
    BudgetExhausted,
    /// 選択できる辺がない。
    NoSelectableEdge,
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::CapacityExceeded => "MCTS tree capacity exceeded",
            Self::EmptyMoves => "an expanded MCTS node must have moves",
            Self::InvalidPath => "invalid MCTS path",
            Self::InvalidSelection => "no pending MCTS selection for this path",
            Self::InvalidState => "invalid MCTS node state for this operation",
            Self::NotEvaluated => "MCTS node has not been evaluated",
            Self::InvalidEvaluation => "invalid MCTS logits or win probability",
            Self::DepthLimit => "MCTS depth or proof distance exceeds MAX_PLY",
            Self::PendingSimulation => "MCTS simulation is still pending",
            Self::InvalidPuctConfig => "invalid PUCT coefficients",
            Self::InvalidGumbelConfig => "invalid Gumbel configuration",
            Self::BudgetExhausted => "Gumbel simulation budget exhausted",
            Self::NoSelectableEdge => "no selectable MCTS edge",
        };
        f.write_str(message)
    }
}

impl std::error::Error for TreeError {}

#[derive(Debug)]
struct PendingSimulation {
    path: Vec<usize>,
    virtual_loss: bool,
}

/// 単一の根を持ち、外から葉の選択と評価を1歩ずつ進められる木。
///
/// 容量は全節点の`size_of::<Node>()`と全辺の`size_of::<Edge>()`の和で数える。
/// アロケータの管理領域、木自体、および未完了の選択の経路はこの容量に含めない。
#[derive(Debug)]
pub struct Tree {
    root: Box<Node>,
    capacity_bytes: usize,
    used_bytes: usize,
    pending: Vec<PendingSimulation>,
    #[cfg(test)]
    pub(super) max_ply: u32,
}

impl Tree {
    /// 根の辺だけを持つ、評価待ちの木を作る。
    ///
    /// # Errors
    /// 合法手が空、または根だけで容量を超える場合は拒否する。
    pub fn new(root_moves: &[Move], capacity_bytes: usize) -> Result<Self, TreeError> {
        if root_moves.is_empty() {
            return Err(TreeError::EmptyMoves);
        }
        let used_bytes = node_bytes(root_moves.len())?;
        if used_bytes > capacity_bytes {
            return Err(TreeError::CapacityExceeded);
        }
        Ok(Self {
            root: Box::new(Node::new(
                root_moves,
                NodeState::Pending { unexpanded: false },
            )),
            capacity_bytes,
            used_bytes,
            pending: Vec::new(),
            #[cfg(test)]
            max_ply: MAX_PLY,
        })
    }

    /// 現在の根を返す。
    pub fn root(&self) -> &Node {
        &self.root
    }

    pub(super) fn depth_limit(&self) -> u32 {
        #[cfg(test)]
        return self.max_ply;
        #[cfg(not(test))]
        MAX_PLY
    }

    /// 節点と辺が使用する容量をバイト単位で返す。
    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    /// 経路上の作成済みの節点を返す。
    ///
    /// # Errors
    /// 経路の辺または子がなければ拒否する。
    pub fn node(&self, path: &[usize]) -> Result<&Node, TreeError> {
        let mut node = self.root();
        for &index in path {
            node = node
                .edges
                .get(index)
                .and_then(|edge| edge.child.as_deref())
                .ok_or(TreeError::InvalidPath)?;
        }
        Ok(node)
    }

    /// 規則に従って1つの葉を選ぶ。衝突時は今回の仮想損失だけを戻す。
    ///
    /// 衝突以外の選択は、評価、既知の葉の逆伝播、確定・引き分けの子の作成、
    /// または取り消しで必ず完了させる。PUCTの複数の評価は任意の順序で投入できる。
    ///
    /// # Errors
    /// 設定が不正、予算切れ、または未完了の選択とGumbelを併用した場合は拒否する。
    pub fn select_leaf(&mut self, rule: SelectionRule<'_>) -> Result<Selection, TreeError> {
        let virtual_loss = matches!(rule, SelectionRule::Puct(_));
        if !self.pending.is_empty()
            && (!virtual_loss || self.pending.iter().any(|pending| !pending.virtual_loss))
        {
            return Err(TreeError::PendingSimulation);
        }
        if let SelectionRule::Puct(config) = rule {
            config.validate()?;
        }
        if matches!(self.root.state, NodeState::Pending { .. }) {
            return Ok(Selection {
                path: Vec::new(),
                kind: LeafKind::Collision,
            });
        }
        if let SelectionRule::Gumbel(config) = rule {
            config.validate_root(&self.root)?;
        }
        let mut path = Vec::new();
        let result = descend(&mut self.root, rule, &mut path);
        let collision = matches!(result, Ok(LeafKind::Collision))
            || self.pending.iter().any(|pending| pending.path == path);
        if result.is_err() || collision {
            if virtual_loss {
                remove_virtual_loss(&mut self.root, &path);
            }
            return result.map(|_| Selection {
                path,
                kind: LeafKind::Collision,
            });
        }
        let kind = result?;
        self.pending.push(PendingSimulation {
            path: path.clone(),
            virtual_loss,
        });
        Ok(Selection { path, kind })
    }

    /// 選択した未作成の辺へ子を作る。確定と引き分けは直ちに逆伝播する。
    ///
    /// # Errors
    /// 未選択・作成済みの辺、空の合法手、不正な値、深さ上限違反、容量不足は拒否する。
    /// エラー時は木と選択を保持するので、再試行しない場合は`cancel_leaf`を呼ぶ。
    pub fn create_child(&mut self, path: &[usize], child: Child<'_>) -> Result<(), TreeError> {
        self.pending_index(path)?;
        let (&index, parent_path) = path.split_last().ok_or(TreeError::InvalidPath)?;
        let edge = self
            .node(parent_path)?
            .edges
            .get(index)
            .ok_or(TreeError::InvalidPath)?;
        if edge.child.is_some() {
            return Err(TreeError::InvalidState);
        }
        if path.len() > self.depth_limit() as usize {
            return Err(TreeError::DepthLimit);
        }
        let (moves, state): (&[Move], _) = match child {
            Child::Pending(moves) => {
                if moves.is_empty() {
                    return Err(TreeError::EmptyMoves);
                }
                if path.len() == self.depth_limit() as usize {
                    return Err(TreeError::DepthLimit);
                }
                (moves, NodeState::Pending { unexpanded: false })
            }
            Child::Unexpanded => {
                if path.len() != self.depth_limit() as usize {
                    return Err(TreeError::DepthLimit);
                }
                (&[], NodeState::Pending { unexpanded: true })
            }
            Child::Proven(proof) => {
                if proof.distance > MAX_PLY - path.len() as u32 {
                    return Err(TreeError::DepthLimit);
                }
                (&[], NodeState::Proven(proof))
            }
            Child::Draw { value, approximate } => {
                validate_value(value)?;
                (&[], NodeState::Draw { value, approximate })
            }
        };
        let bytes = node_bytes(moves.len())?;
        if bytes > self.capacity_bytes - self.used_bytes {
            return Err(TreeError::CapacityExceeded);
        }
        let child = Box::new(Node::new(moves, state));
        let fixed_value = child.fixed_value();
        node_mut(&mut self.root, parent_path)?.edges[index].child = Some(child);
        self.used_bytes += bytes;
        if let Some(value) = fixed_value {
            self.finish(path, value)?;
        }
        Ok(())
    }

    /// 評価待ちの節点へlogitと勝率を投入し、経路に沿って逆伝播する。
    ///
    /// 根の評価には空の経路を使い、辺への訪問は数えない。
    /// 再利用した展開済みの根も、統計を保ったまま再評価できる。
    /// 展開しない葉には空のlogit列を渡す。
    ///
    /// # Errors
    /// 評価待ちでない節点、不正な経路、logitの数や有限性、勝率の範囲を拒否する。
    /// エラー時は評価待ちと仮想損失を保ち、修正した評価の再投入または取り消しを待つ。
    pub fn evaluate(
        &mut self,
        path: &[usize],
        logits: &[f32],
        value: f32,
    ) -> Result<(), TreeError> {
        if !path.is_empty() {
            self.pending_index(path)?;
        }
        let node = self.node(path)?;
        let unexpanded = match node.state {
            NodeState::Pending { unexpanded } => unexpanded,
            NodeState::Expanded if path.is_empty() && self.pending.is_empty() => false,
            _ => return Err(TreeError::InvalidState),
        };
        validate_value(value)?;
        if logits.len() != node.edges.len() || logits.iter().any(|x| !x.is_finite()) {
            return Err(TreeError::InvalidEvaluation);
        }
        let probabilities = softmax(logits);
        let node = node_mut(&mut self.root, path)?;
        for ((edge, &logit), policy) in node.edges.iter_mut().zip(logits).zip(probabilities) {
            edge.logit = logit;
            edge.policy = policy;
        }
        node.value = Some(value);
        node.state = if unexpanded {
            NodeState::Unexpanded
        } else {
            NodeState::Expanded
        };
        if !path.is_empty() {
            self.finish(path, value)?;
        }
        Ok(())
    }

    /// 選択済みの確定、引き分け、または展開しない葉の値を逆伝播する。
    ///
    /// # Errors
    /// 未選択の経路、または既知の値を返す葉でない場合は拒否する。
    pub fn backup_leaf(&mut self, path: &[usize]) -> Result<(), TreeError> {
        let value = self
            .node(path)?
            .fixed_value()
            .ok_or(TreeError::InvalidState)?;
        self.finish(path, value)
    }

    /// 選択を取り消し、仮想損失と、作成済みなら評価待ちの子を除く。
    ///
    /// `visited_policy`は選択した回数の累積なので戻さない。
    ///
    /// # Errors
    /// 未完了の選択に対応しない経路は拒否する。
    pub fn cancel_leaf(&mut self, path: &[usize]) -> Result<(), TreeError> {
        let index = self.pending_index(path)?;
        let pending = self.pending.swap_remove(index);
        if pending.virtual_loss {
            remove_virtual_loss(&mut self.root, path);
        }
        if let Some((&edge_index, parent_path)) = path.split_last() {
            let edge = &mut node_mut(&mut self.root, parent_path)?.edges[edge_index];
            if edge
                .child
                .as_ref()
                .is_some_and(|child| matches!(child.state, NodeState::Pending { .. }))
            {
                self.used_bytes -= edge.child.take().unwrap().subtree_bytes();
            }
        }
        Ok(())
    }

    /// 根の最善手を返す。評価待ちまたは辺のない根では`None`。
    pub fn best_move(&self) -> Option<Move> {
        Some(self.root.edges[self.root.best_edge()?].mv)
    }

    /// 最善手の規則で主変化を返す。根では未訪問でも評価済みなら1手を含める。
    pub fn principal_variation(&self) -> Vec<Move> {
        let mut pv = Vec::new();
        let mut node = self.root();
        while let Some(index) = node.best_edge() {
            let edge = &node.edges[index];
            if !pv.is_empty() && edge.visits == 0 {
                break;
            }
            pv.push(edge.mv);
            match edge.child.as_deref() {
                Some(child)
                    if matches!(child.state, NodeState::Expanded | NodeState::Proven(_)) =>
                {
                    node = child;
                }
                _ => break,
            }
        }
        pv
    }

    /// 手の列をたどった部分木を新しい根にし、残りを捨てて使用容量を数え直す。
    ///
    /// 展開済み、または評価済みの展開しない葉だけを新しい根にできる。
    /// 後者は付け替え後も子を作らない。空の手順は現在の根を維持する。
    ///
    /// # Errors
    /// 未完了の選択、たどれない手順、または再利用できない状態なら、元の木を保って拒否する。
    pub fn reroot(&mut self, moves: &[Move]) -> Result<(), TreeError> {
        if !self.pending.is_empty() {
            return Err(TreeError::PendingSimulation);
        }
        let mut path = Vec::with_capacity(moves.len());
        let mut node = self.root();
        for mv in moves {
            let index = node
                .edges
                .iter()
                .position(|edge| edge.mv == *mv)
                .ok_or(TreeError::InvalidPath)?;
            node = node.edges[index]
                .child
                .as_deref()
                .ok_or(TreeError::InvalidPath)?;
            path.push(index);
        }
        if !matches!(node.state, NodeState::Expanded | NodeState::Unexpanded) {
            return Err(TreeError::InvalidState);
        }
        let bytes = node.subtree_bytes();
        if let Some((&index, parent_path)) = path.split_last() {
            let child = node_mut(&mut self.root, parent_path)?.edges[index]
                .child
                .take()
                .unwrap();
            self.root = child;
        }
        self.used_bytes = bytes;
        Ok(())
    }

    fn pending_index(&self, path: &[usize]) -> Result<usize, TreeError> {
        self.pending
            .iter()
            .position(|pending| pending.path == path)
            .ok_or(TreeError::InvalidSelection)
    }

    fn finish(&mut self, path: &[usize], value: f32) -> Result<(), TreeError> {
        let index = self.pending_index(path)?;
        let pending = self.pending.swap_remove(index);
        backup(&mut self.root, path, f64::from(value), pending.virtual_loss);
        Ok(())
    }
}

fn node_bytes(num_edges: usize) -> Result<usize, TreeError> {
    num_edges
        .checked_mul(size_of::<Edge>())
        .and_then(|bytes| bytes.checked_add(size_of::<Node>()))
        .ok_or(TreeError::CapacityExceeded)
}

fn validate_value(value: f32) -> Result<(), TreeError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(TreeError::InvalidEvaluation);
    }
    Ok(())
}

fn node_mut<'a>(root: &'a mut Node, path: &[usize]) -> Result<&'a mut Node, TreeError> {
    let mut node = root;
    for &index in path {
        node = node
            .edges
            .get_mut(index)
            .and_then(|edge| edge.child.as_deref_mut())
            .ok_or(TreeError::InvalidPath)?;
    }
    Ok(node)
}

fn descend(
    node: &mut Node,
    rule: SelectionRule<'_>,
    path: &mut Vec<usize>,
) -> Result<LeafKind, TreeError> {
    if let Some(value) = node.fixed_value() {
        return Ok(LeafKind::Value(value));
    }
    if matches!(node.state, NodeState::Pending { .. }) {
        return Ok(LeafKind::Collision);
    }
    let index = match rule {
        SelectionRule::Puct(config) => puct::select_child(node, path.is_empty(), config)?,
        SelectionRule::Gumbel(config) if path.is_empty() => config.select_root(node)?,
        SelectionRule::Gumbel(config) => config.select_interior(node)?,
    };
    let edge = &mut node.edges[index];
    if matches!(rule, SelectionRule::Puct(_)) {
        edge.virtual_loss += 1;
    }
    path.push(index);
    match edge.child.as_deref_mut() {
        Some(child) => descend(child, rule, path),
        None => Ok(LeafKind::New),
    }
}

fn remove_virtual_loss(node: &mut Node, path: &[usize]) {
    if let Some((&index, rest)) = path.split_first() {
        let edge = &mut node.edges[index];
        edge.virtual_loss -= 1;
        if !rest.is_empty() {
            remove_virtual_loss(edge.child.as_mut().expect("選択済みの経路"), rest);
        }
    }
}

fn backup(node: &mut Node, path: &[usize], value: f64, virtual_loss: bool) -> f64 {
    let Some((&index, rest)) = path.split_first() else {
        return value;
    };
    let edge = &mut node.edges[index];
    let value = 1.0
        - backup(
            edge.child.as_mut().expect("作成済みの経路"),
            rest,
            value,
            virtual_loss,
        );
    edge.visits += 1;
    edge.value_sum += value;
    if virtual_loss {
        edge.virtual_loss -= 1;
    }
    node.refresh_proof();
    value
}
