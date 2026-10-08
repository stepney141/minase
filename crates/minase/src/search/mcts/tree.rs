//! 探索木の節点と辺、確定した勝敗の伝播、および木の再利用。
//!
//! 節点はアリーナ（`Vec`）に置き、根は常に番号0である。辺は親の節点が持ち、
//! 訪問回数N、価値の和W（親の手番側から見た勝率の和）、事前確率、および確定した勝敗を記録する。

use core::cmp::Ordering;

use crate::search::MAX_PLY;

/// アリーナ内の節点の番号。
pub(super) type NodeId = u32;

/// 根の節点の番号。
pub(super) const ROOT: NodeId = 0;

/// 確定した勝敗と、終局までのply数。
///
/// 節点に付くときは節点の手番側から、辺に付くときは親の節点の手番側から見た勝敗である。
/// 終局は、相手の最後の王駒を取る着手（RULES.md第21条第1項）または合法手のない局面
/// （第23条）である。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Proven {
    /// 勝ちが確定している。
    Win(u32),
    /// 負けが確定している。
    Loss(u32),
}

impl Proven {
    /// 確定値を勝率で返す。
    pub(super) const fn value(self) -> f64 {
        match self {
            Self::Win(_) => 1.0,
            Self::Loss(_) => 0.0,
        }
    }

    /// 子の節点の確定値を、親の手番側から見たその辺の確定値へ写す。手数は[`MAX_PLY`]で飽和させる。
    const fn through_edge(self) -> Self {
        match self {
            Self::Win(plies) => Self::Loss(next_ply(plies)),
            Self::Loss(plies) => Self::Win(next_ply(plies)),
        }
    }
}

/// 1手進めた手数を返す。詰みの帯を外れないよう[`MAX_PLY`]で止める。
const fn next_ply(plies: u32) -> u32 {
    if plies < MAX_PLY { plies + 1 } else { MAX_PLY }
}

/// 節点から子への辺。
#[derive(Clone, Debug)]
pub(super) struct Edge<A> {
    /// 着手。
    pub(super) action: A,
    /// 方策の出力位置。
    pub(super) label: usize,
    /// ネットが出力した方策logit。
    pub(super) logit: f32,
    /// 合法手のlogitだけをsoftmaxで正規化した事前確率。
    pub(super) prior: f32,
    /// 評価を終えた訪問の回数N。
    pub(super) visits: u32,
    /// 評価待ちの訪問の回数。仮想損失として負けに数える。
    pub(super) in_flight: u32,
    /// 親の手番側から見た勝率の和W。
    pub(super) value_sum: f64,
    /// 親の手番側から見た確定した勝敗。
    pub(super) proven: Option<Proven>,
    /// 子の節点。辺を初めて通るまでは`None`。
    pub(super) child: Option<NodeId>,
}

impl<A> Edge<A> {
    /// まだ評価されていない辺を作る。
    pub(super) const fn new(action: A, label: usize) -> Self {
        Self {
            action,
            label,
            logit: 0.0,
            prior: 0.0,
            visits: 0,
            in_flight: 0,
            value_sum: 0.0,
            proven: None,
            child: None,
        }
    }

    /// 訪問した辺の平均勝率Q＝W/Nを返す。
    pub(super) fn q(&self) -> Option<f64> {
        (self.visits > 0).then(|| self.value_sum / f64::from(self.visits))
    }
}

/// 節点の評価の状態。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum NodeState {
    /// 合法手をまだ列挙していない。展開前の根だけがこの状態をとる。
    Unexpanded,
    /// 合法手を列挙し、ネットの評価を待っている。
    Pending,
    /// 評価を終えたか、評価なしで勝敗が確定した。
    Ready,
}

/// 探索木の節点。
#[derive(Clone, Debug)]
pub(super) struct Node<A> {
    /// 探索局面キー（RULES.md第24条第1項）。木の再利用で局面を照合する。
    pub(super) key: u64,
    /// 評価の状態。
    pub(super) state: NodeState,
    /// 合法手ごとの辺。
    pub(super) edges: Vec<Edge<A>>,
    /// ネットが出力した手番側の勝率。
    pub(super) raw_value: f32,
    /// 手番側から見た確定した勝敗。
    pub(super) proven: Option<Proven>,
    /// PUCTのFPUに使う、選んだ子の事前確率の累計（dlshogiの`visited_nnrate`）。
    pub(super) visited_policy: f32,
}

impl<A> Node<A> {
    /// 合法手を列挙していない節点を作る。
    const fn new(key: u64) -> Self {
        Self {
            key,
            state: NodeState::Unexpanded,
            edges: Vec::new(),
            raw_value: 0.5,
            proven: None,
            visited_policy: 0.0,
        }
    }

    /// 辺の確定値から節点の確定値を求め直す。
    ///
    /// 勝ちの辺が1つでもあれば最短の勝ち、すべての辺が負けなら最長の負けとする。
    /// 辺を持たない節点（合法手のない局面）の確定値は変えない。
    pub(super) fn refresh_proven(&mut self) {
        if self.edges.is_empty() {
            return;
        }
        let shortest_win = self
            .edges
            .iter()
            .filter_map(|edge| match edge.proven {
                Some(Proven::Win(plies)) => Some(plies),
                _ => None,
            })
            .min();
        self.proven = match shortest_win {
            Some(plies) => Some(Proven::Win(plies)),
            None => self
                .edges
                .iter()
                .map(|edge| match edge.proven {
                    Some(Proven::Loss(plies)) => Some(plies),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
                .and_then(|plies| plies.into_iter().max())
                .map(Proven::Loss),
        };
    }

    /// 合法手のlogitをsoftmaxで正規化し、事前確率を設定する。
    pub(super) fn normalize_priors(&mut self) {
        let max = self
            .edges
            .iter()
            .map(|edge| edge.logit)
            .fold(f32::NEG_INFINITY, f32::max);
        let sum: f64 = self
            .edges
            .iter()
            .map(|edge| f64::from(edge.logit - max).exp())
            .sum();
        for edge in &mut self.edges {
            edge.prior = (f64::from(edge.logit - max).exp() / sum) as f32;
        }
    }

    /// 評価を終えた訪問の総数ΣNを返す。
    pub(super) fn total_visits(&self) -> u32 {
        self.edges.iter().map(|edge| edge.visits).sum()
    }

    /// 最善の辺を選ぶ。PUCTの着手と主変化に使う。
    ///
    /// 勝ちが確定していれば最短の勝ちの辺を、そうでなければ負けの確定していない辺のうち
    /// 訪問回数、Q、事前確率の順に大きい辺を選ぶ。全辺が負けなら最長の負けの辺を選ぶ。
    /// 同点は先の辺を採る。辺がなければ`None`を返す。
    pub(super) fn best_edge(&self) -> Option<usize> {
        if let Some(Proven::Win(plies)) = self.proven {
            return self
                .edges
                .iter()
                .position(|edge| edge.proven == Some(Proven::Win(plies)));
        }
        let open = (0..self.edges.len())
            .filter(|&index| !matches!(self.edges[index].proven, Some(Proven::Loss(_))))
            .reduce(|best, index| {
                if self.compare_edges(index, best) == Ordering::Greater {
                    index
                } else {
                    best
                }
            });
        open.or_else(|| {
            (0..self.edges.len()).reduce(|best, index| {
                if loss_plies(&self.edges[index]) > loss_plies(&self.edges[best]) {
                    index
                } else {
                    best
                }
            })
        })
    }

    /// 2つの辺を訪問回数、Q、事前確率の順に比べる。
    fn compare_edges(&self, left: usize, right: usize) -> Ordering {
        let (left, right) = (&self.edges[left], &self.edges[right]);
        left.visits
            .cmp(&right.visits)
            .then_with(|| {
                left.q()
                    .unwrap_or(f64::NEG_INFINITY)
                    .total_cmp(&right.q().unwrap_or(f64::NEG_INFINITY))
            })
            .then_with(|| left.prior.total_cmp(&right.prior))
    }
}

/// 負けが確定した辺の手数を返す。負けでなければ0とする。
fn loss_plies<A>(edge: &Edge<A>) -> u32 {
    match edge.proven {
        Some(Proven::Loss(plies)) => plies,
        _ => 0,
    }
}

/// 節点のアリーナ。番号0が根である。
#[derive(Clone, Debug)]
pub(super) struct Tree<A> {
    nodes: Vec<Node<A>>,
}

impl<A: Copy + PartialEq> Tree<A> {
    /// 合法手を列挙していない根だけの木を作る。
    pub(super) fn new(root_key: u64) -> Self {
        Self {
            nodes: vec![Node::new(root_key)],
        }
    }

    /// 節点を返す。
    pub(super) fn node(&self, id: NodeId) -> &Node<A> {
        &self.nodes[id as usize]
    }

    /// 節点を可変で返す。
    pub(super) fn node_mut(&mut self, id: NodeId) -> &mut Node<A> {
        &mut self.nodes[id as usize]
    }

    /// 根を返す。
    pub(super) fn root(&self) -> &Node<A> {
        self.node(ROOT)
    }

    /// 節点の数を返す。
    pub(super) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// 合法手を列挙していない節点を加え、その番号を返す。
    pub(super) fn push(&mut self, key: u64) -> NodeId {
        let id = NodeId::try_from(self.nodes.len()).expect("the search tree exceeds u32 nodes");
        self.nodes.push(Node::new(key));
        id
    }

    /// 経路の各辺の仮想損失を取り消す。
    pub(super) fn revert(&mut self, path: &[(NodeId, usize)]) {
        for &(node, edge) in path {
            self.node_mut(node).edges[edge].in_flight -= 1;
        }
    }

    /// 経路を葉から根へたどって評価を反映する。
    ///
    /// `leaf_value`は経路の最後の辺の先の局面の手番側から見た勝率である。各辺の仮想損失を
    /// 取り消し、訪問回数と親の手番側から見た勝率を加え、子の確定値を辺と節点へ伝える。
    pub(super) fn backup(&mut self, path: &[(NodeId, usize)], leaf_value: f64) {
        let mut value = leaf_value;
        for &(node, edge) in path.iter().rev() {
            let child_proven = self.node(node).edges[edge]
                .child
                .and_then(|child| self.node(child).proven);
            let parent = self.node_mut(node);
            let edge = &mut parent.edges[edge];
            edge.in_flight -= 1;
            edge.visits += 1;
            edge.value_sum += 1.0 - value;
            if let Some(proven) = child_proven {
                edge.proven = Some(proven.through_edge());
            }
            parent.refresh_proven();
            value = 1.0 - value;
        }
    }

    /// `first`の辺から始め、各節点の最善の辺をたどった主変化を返す。
    pub(super) fn principal_variation(&self, first: usize) -> Vec<A> {
        let mut pv = Vec::new();
        let mut node = self.root();
        let mut edge = first;
        loop {
            let chosen = &node.edges[edge];
            pv.push(chosen.action);
            let Some(child) = chosen.child.map(|child| self.node(child)) else {
                break;
            };
            let next = child
                .best_edge()
                .filter(|&next| child.proven.is_some() || child.edges[next].visits > 0);
            match next {
                Some(next) if pv.len() < MAX_PLY as usize => {
                    node = child;
                    edge = next;
                }
                _ => break,
            }
        }
        pv
    }

    /// 根から`keys`の局面を順にたどった節点を根とする部分木を、新しいアリーナへ写して返す。
    ///
    /// `keys`の各局面が直前の節点の子として展開済みでなければ`None`を返す。
    pub(super) fn subtree(&self, keys: &[u64]) -> Option<Self> {
        let mut node = ROOT;
        for &key in keys {
            node = self
                .node(node)
                .edges
                .iter()
                .filter_map(|edge| edge.child)
                .find(|&child| self.node(child).key == key)?;
        }
        if self.node(node).state != NodeState::Ready {
            return None;
        }
        let mut tree = Self {
            nodes: vec![self.node(node).clone()],
        };
        // 写した節点の子の番号を、新しいアリーナの番号へ置き換える。
        let mut next = 0;
        while next < tree.nodes.len() {
            for index in 0..tree.nodes[next].edges.len() {
                if let Some(child) = tree.nodes[next].edges[index].child {
                    let copied = tree.push(0);
                    tree.nodes[copied as usize] = self.node(child).clone();
                    tree.nodes[next].edges[index].child = Some(copied);
                }
            }
            next += 1;
        }
        Some(tree)
    }

    /// 根の辺を`actions`に含まれる着手だけに絞り、事前確率と確定値を求め直す。
    ///
    /// `actions`の着手がすべて根の辺にあれば真を返す。
    pub(super) fn retain_root_actions(&mut self, actions: &[A]) -> bool {
        let root = self.node_mut(ROOT);
        if !actions
            .iter()
            .all(|action| root.edges.iter().any(|edge| edge.action == *action))
        {
            return false;
        }
        if root.edges.len() != actions.len() {
            root.edges.retain(|edge| actions.contains(&edge.action));
            root.normalize_priors();
            root.refresh_proven();
        }
        true
    }
}
