//! 探索途中にも適用できる、木の10条件の検査。

use std::collections::{BTreeMap, BTreeSet};

use super::*;

/// 探索木の違反。経路は根からの辺の添字で、空なら根を指す。
/// 各列挙子は条件番号に対応し、診断に必要な実測値と期待値を保持する。
#[derive(Debug, PartialEq)]
pub enum TreeViolation {
    /// 条件1。辺の勝率の和または未訪問の統計が不正である。
    EdgeStatistics {
        /// 根からの辺の添字。
        path: Vec<usize>,
        /// 入る辺の訪問回数。
        visits: u32,
        /// 入る辺の勝率の和。
        value_sum: f64,
        /// 浮動小数点数の許容幅。
        tolerance: f64,
        /// 子の状態。未作成ならNone。
        child_state: Option<NodeState>,
    },
    /// 条件2。親から入る統計と子へ出る統計が両立しない。
    Conservation {
        /// 根からの辺の添字。
        path: Vec<usize>,
        /// 入る辺の訪問回数。
        visits: u32,
        /// 入る辺の勝率の和。
        value_sum: f64,
        /// 出る辺の訪問回数の和。
        child_visits: u64,
        /// 出る辺の勝率の和。
        child_value_sum: f64,
        /// 浮動小数点数の許容幅。
        tolerance: f64,
    },
    /// 条件3。展開済みの節点の初回評価と訪問の統計が一致しない。
    ExpandedStatistics {
        /// 根からの辺の添字。
        path: Vec<usize>,
        /// 入る辺の訪問回数。
        visits: u32,
        /// 入る辺の勝率の和。
        value_sum: f64,
        /// 出る辺の訪問回数の和。
        child_visits: u64,
        /// 出る辺の勝率の和。
        child_value_sum: f64,
        /// 節点の手番側の勝率。
        value: f32,
        /// 浮動小数点数の許容幅。
        tolerance: f64,
    },
    /// 条件4。固定値の葉の勝率の和が一致しない。
    FixedStatistics {
        /// 根からの辺の添字。
        path: Vec<usize>,
        /// 入る辺の訪問回数。
        visits: u32,
        /// 入る辺の勝率の和。
        value_sum: f64,
        /// 節点の手番側の勝率。
        value: f64,
        /// 浮動小数点数の許容幅。
        tolerance: f64,
    },
    /// 条件5。仮想損失と未完了の選択の数が一致しない。
    VirtualLoss {
        /// 根からの辺の添字。
        path: Vec<usize>,
        /// 実際の仮想損失の数。
        actual: u32,
        /// 子または未完了の選択から求めた期待値。
        expected: usize,
    },
    /// 条件6。未完了の選択の経路または選択規則の組み合わせが不正である。
    PendingPath {
        /// 根からの辺の添字。
        path: Vec<usize>,
        /// 経路が不正である理由。
        reason: &'static str,
        /// 未完了の選択の総数。
        selections: usize,
        /// 仮想損失を使う選択なら真。
        virtual_loss: bool,
    },
    /// 条件7。到達可能な節点と辺の大きさが使用量または容量と一致しない。
    Memory {
        /// 根からの辺の添字。
        path: Vec<usize>,
        /// 記録された使用量。単位はバイト。
        used_bytes: usize,
        /// 到達可能な節点と辺の大きさ。単位はバイト。
        expected_bytes: usize,
        /// 容量の上限。単位はバイト。
        capacity_bytes: usize,
    },
    /// 条件8。評価値、方策、logit、または選択済み方策の累積値が不正である。
    Evaluation {
        /// 根からの辺の添字。
        path: Vec<usize>,
        /// 節点に記録された状態。
        state: NodeState,
        /// 節点の手番側の勝率。
        value: Option<f32>,
        /// 選択済みの方策の累積値。
        visited_policy: f32,
        /// 違反した辺の事前確率。
        policy: Option<f32>,
        /// 違反した辺のlogit。
        logit: Option<f32>,
        /// 節点の事前確率の和。
        policy_sum: f64,
        /// 浮動小数点数の許容幅。
        tolerance: f64,
    },
    /// 条件9。子から求めた勝敗と節点の状態が一致しない。
    Proof {
        /// 根からの辺の添字。
        path: Vec<usize>,
        /// 節点に記録された状態。
        state: NodeState,
        /// 子または未完了の選択から求めた期待値。
        expected: Option<Proof>,
    },
    /// 条件10。根からの手数または確定までの手数が上限を超える。
    Depth {
        /// 根からの辺の添字。
        path: Vec<usize>,
        /// 根からの手数。
        depth: usize,
        /// 根からの手数の上限。
        depth_limit: u32,
        /// 節点から勝敗の確定までの手数。
        proof_distance: Option<u32>,
        /// 探索全体の手数の上限。
        max_ply: u32,
    },
}

impl fmt::Display for TreeViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (condition, description) = match self {
            Self::EdgeStatistics { .. } => (1, "edge statistics"),
            Self::Conservation { .. } => (2, "visit/value conservation"),
            Self::ExpandedStatistics { .. } => (3, "expanded node statistics"),
            Self::FixedStatistics { .. } => (4, "fixed leaf statistics"),
            Self::VirtualLoss { .. } => (5, "virtual loss count"),
            Self::PendingPath { .. } => (6, "pending selection path"),
            Self::Memory { .. } => (7, "tree memory accounting"),
            Self::Evaluation { .. } => (8, "evaluation and policy"),
            Self::Proof { .. } => (9, "proof from children"),
            Self::Depth { .. } => (10, "depth and proof distance"),
        };
        write!(f, "MCTS condition {condition} ({description}): {self:?}")
    }
}

impl std::error::Error for TreeViolation {}

impl Tree {
    /// 根から全節点をたどり、次の10条件の最初の違反を返す。
    ///
    /// 1. 辺の勝率の和は有限で0から訪問回数の間にあり、未訪問なら0である。
    ///    子が未作成または評価待ちなら、訪問回数と勝率の和は0である。
    /// 2. 根以外では、出る訪問数Aは入る訪問数N以下であり、入る勝率の和Wと
    ///    出る勝率の和Bについて、0 ≤ W + B − A ≤ N − Aが成り立つ。
    /// 3. 展開済みの根以外では、ネットの勝率VについてN = 1 + A、W + B = A + 1 − Vである。
    /// 4. 辺のない固定値vの葉では、W = N(1 − v)である。
    /// 5. 仮想損失は、その辺を通る仮想損失付きの未完了の選択数に等しい。
    /// 6. 未完了の経路は評価待ち、未作成の子、固定値の節点で終わり、重複しない。
    ///    Gumbelの未完了の選択があれば、それ以外の未完了の選択はない。
    /// 7. 到達可能な節点と辺の大きさの合計は使用量に等しく、容量以下である。
    /// 8. 評価済みの辺の方策とlogitは有限、方策は0から1で和が1に近い。
    ///    展開済みと非展開の節点はネットの値を持ち、値は0から1にある。
    ///    選択済み方策の累積値は有限で非負であり、上限は設けない。
    /// 9. 辺を持つ評価済みの節点の勝敗は、子から再計算した勝敗と一致する。
    /// 10. 根からの手数は深さ上限以下で、確定までの手数を足してもMAX_PLY以下である。
    ///
    /// 勝率の式はf64で計算し、許容幅はk(k + MAX_PLY)·2⁻⁵²とする。
    /// 条件1と4のkはmax(1, N)、条件2と3ではmax(1, N + A)である。
    /// 訪問ごとの加算と経路上の1 − xの計算による丸め誤差を許すためである。
    /// 方策はf32で正規化されるので、n本の辺の和の許容幅は(n + 2)·2⁻²³とする。
    /// 整数の関係には幅を設けず、確定した節点の平均勝率を0や1へ制限しない。
    ///
    /// 評価待ちや逆伝播待ちが残る途中の状態も受け入れる。
    /// 探索終了時は`Driver::assert_finished`が未完了の選択と根の評価待ちも検査する。
    ///
    /// # Errors
    /// 条件に違反した場合、経路と関係する値を含む`TreeViolation`を返す。
    pub fn validate(&self) -> Result<(), TreeViolation> {
        let losses = self.validate_pending()?;
        let bytes = validate_node(&self.root, &mut Vec::new(), None, self.depth_limit, &losses)?;
        if bytes != self.used_bytes || self.used_bytes > self.capacity_bytes {
            return Err(TreeViolation::Memory {
                path: Vec::new(),
                used_bytes: self.used_bytes,
                expected_bytes: bytes,
                capacity_bytes: self.capacity_bytes,
            });
        }
        Ok(())
    }

    pub(in crate::search::mcts) fn pending_count(&self) -> usize {
        self.pending.len()
    }

    fn validate_pending(&self) -> Result<BTreeMap<Vec<usize>, usize>, TreeViolation> {
        let mut paths = BTreeSet::new();
        let mut losses = BTreeMap::new();
        for pending in &self.pending {
            let violation = |reason| TreeViolation::PendingPath {
                path: pending.path.clone(),
                reason,
                selections: self.pending.len(),
                virtual_loss: pending.virtual_loss,
            };
            if !pending.virtual_loss && self.pending.len() != 1 {
                return Err(violation("Gumbel selection must be exclusive"));
            }
            if !paths.insert(&pending.path) {
                return Err(violation("duplicate path"));
            }
            let mut node = self.root();
            let mut endpoint = pending.path.is_empty() && node.fixed_value().is_some();
            for (depth, &index) in pending.path.iter().enumerate() {
                let edge = node
                    .edges
                    .get(index)
                    .ok_or_else(|| violation("missing edge"))?;
                if pending.virtual_loss {
                    *losses.entry(pending.path[..=depth].to_vec()).or_insert(0) += 1;
                }
                let last = depth + 1 == pending.path.len();
                match edge.child.as_deref() {
                    Some(child) => {
                        node = child;
                        endpoint = last
                            && (matches!(node.state, NodeState::Pending { .. })
                                || node.fixed_value().is_some());
                    }
                    None if last => endpoint = true,
                    None => return Err(violation("missing intermediate child")),
                }
            }
            if !endpoint {
                return Err(violation("endpoint is not pending, uncreated, or fixed"));
            }
        }
        Ok(losses)
    }
}

fn tolerance(visits: f64) -> f64 {
    let k = visits.max(1.0);
    k * (k + f64::from(MAX_PLY)) * f64::EPSILON
}

fn validate_node(
    node: &Node,
    path: &mut Vec<usize>,
    incoming: Option<&Edge>,
    depth_limit: u32,
    losses: &BTreeMap<Vec<usize>, usize>,
) -> Result<usize, TreeViolation> {
    let proof_distance = match node.state {
        NodeState::Proven(proof) => Some(proof.distance),
        _ => None,
    };
    if path.len() > depth_limit as usize
        || proof_distance
            .is_some_and(|distance| path.len() as u64 + u64::from(distance) > u64::from(MAX_PLY))
    {
        return Err(TreeViolation::Depth {
            path: path.clone(),
            depth: path.len(),
            depth_limit,
            proof_distance,
            max_ply: MAX_PLY,
        });
    }
    let pending = matches!(node.state, NodeState::Pending { .. });
    let policy_sum = node
        .edges
        .iter()
        .map(|edge| f64::from(edge.policy))
        .sum::<f64>();
    let policy_tolerance = (node.edges.len() as f64 + 2.0) * f64::from(f32::EPSILON);
    let evaluation_error = |path, edge: Option<&Edge>| TreeViolation::Evaluation {
        path,
        state: node.state,
        value: node.value,
        visited_policy: node.visited_policy,
        policy: edge.map(|edge| edge.policy),
        logit: edge.map(|edge| edge.logit),
        policy_sum,
        tolerance: policy_tolerance,
    };
    if !node.visited_policy.is_finite()
        || node.visited_policy < 0.0
        || (matches!(node.state, NodeState::Expanded | NodeState::Unexpanded)
            && node.value.is_none())
        || node
            .value
            .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return Err(evaluation_error(path.clone(), None));
    }
    if !pending && !node.edges.is_empty() {
        for (index, edge) in node.edges.iter().enumerate() {
            if !edge.policy.is_finite()
                || !(0.0..=1.0).contains(&edge.policy)
                || !edge.logit.is_finite()
            {
                let mut edge_path = path.clone();
                edge_path.push(index);
                return Err(evaluation_error(edge_path, Some(edge)));
            }
        }
        if (policy_sum - 1.0).abs() > policy_tolerance {
            return Err(evaluation_error(path.clone(), None));
        }
    }
    if let Some(edge) = incoming {
        let a = node
            .edges
            .iter()
            .map(|edge| u64::from(edge.visits))
            .sum::<u64>();
        let b = node.edges.iter().map(|edge| edge.value_sum).sum::<f64>();
        let n = f64::from(edge.visits);
        let width = tolerance(n + a as f64);
        let residual = edge.value_sum + b - a as f64;
        if a > u64::from(edge.visits)
            || !residual.is_finite()
            || residual < -width
            || residual > n - a as f64 + width
        {
            return Err(TreeViolation::Conservation {
                path: path.clone(),
                visits: edge.visits,
                value_sum: edge.value_sum,
                child_visits: a,
                child_value_sum: b,
                tolerance: width,
            });
        }
        if node.state == NodeState::Expanded {
            let value = node.value.expect("condition 8 checked the value");
            if u64::from(edge.visits) != 1 + a
                || (edge.value_sum + b - (a as f64 + 1.0 - f64::from(value))).abs() > width
            {
                return Err(TreeViolation::ExpandedStatistics {
                    path: path.clone(),
                    visits: edge.visits,
                    value_sum: edge.value_sum,
                    child_visits: a,
                    child_value_sum: b,
                    value,
                    tolerance: width,
                });
            }
        }
        if node.edges.is_empty()
            && let Some(value) = node.fixed_value()
        {
            let value = f64::from(value);
            let width = tolerance(n);
            if !value.is_finite() || (edge.value_sum - n * (1.0 - value)).abs() > width {
                return Err(TreeViolation::FixedStatistics {
                    path: path.clone(),
                    visits: edge.visits,
                    value_sum: edge.value_sum,
                    value,
                    tolerance: width,
                });
            }
        }
    }
    let mut bytes = size_of::<Node>() + node.edges.len() * size_of::<Edge>();
    for (index, edge) in node.edges.iter().enumerate() {
        path.push(index);
        let width = tolerance(f64::from(edge.visits));
        let child_state = edge.child.as_deref().map(|child| child.state);
        let must_be_unvisited =
            child_state.is_none_or(|state| matches!(state, NodeState::Pending { .. }));
        if !edge.value_sum.is_finite()
            || edge.value_sum < -width
            || edge.value_sum > f64::from(edge.visits) + width
            || (edge.visits == 0 && edge.value_sum.abs() > width)
            || (must_be_unvisited && (edge.visits != 0 || edge.value_sum.abs() > width))
        {
            return Err(TreeViolation::EdgeStatistics {
                path: path.clone(),
                visits: edge.visits,
                value_sum: edge.value_sum,
                tolerance: width,
                child_state,
            });
        }
        // 経路が表にない辺を通る未完了の選択は0個である。
        let expected = match losses.get(path) {
            Some(&count) => count,
            None => 0,
        };
        if u64::from(edge.virtual_loss) != expected as u64 {
            return Err(TreeViolation::VirtualLoss {
                path: path.clone(),
                actual: edge.virtual_loss,
                expected,
            });
        }
        if let Some(child) = edge.child.as_deref() {
            bytes += validate_node(child, path, Some(edge), depth_limit, losses)?;
        }
        path.pop();
    }
    // 子の距離を先に検査し、壊れた距離の加算で検査自体が桁あふれしないようにする。
    if !pending && !node.edges.is_empty() {
        let expected = node.proof_from_children();
        let valid = match node.state {
            NodeState::Proven(proof) => expected == Some(proof),
            NodeState::Expanded => expected.is_none(),
            _ => true,
        };
        if !valid {
            return Err(TreeViolation::Proof {
                path: path.clone(),
                state: node.state,
                expected,
            });
        }
    }
    Ok(bytes)
}
