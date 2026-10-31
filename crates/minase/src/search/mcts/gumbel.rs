//! Gumbel AlphaZeroの逐次選択と方策の目標。
//!
//! mctxのコミット428bfb7の[`policies.py`](https://github.com/google-deepmind/mctx/blob/428bfb7/mctx/_src/policies.py)、
//! [`qtransforms.py`](https://github.com/google-deepmind/mctx/blob/428bfb7/mctx/_src/qtransforms.py)、
//! [`seq_halving.py`](https://github.com/google-deepmind/mctx/blob/428bfb7/mctx/_src/seq_halving.py)、
//! [`action_selection.py`](https://github.com/google-deepmind/mctx/blob/428bfb7/mctx/_src/action_selection.py)
//! の式に従い、Qの尺度と最後に指す手のGumbel乱数の倍率を引数で指定する。
//! 価値は勝率へ写して保持し、節点内の再尺度化も単精度で行う。

use super::softmax;
use super::tree::{Edge, Node, NodeState, Outcome, TreeError};

/// 根のGumbel乱数、Sequential Halvingの予算、およびQの尺度。
///
/// 根の評価後のシミュレーションだけを数える。選択番号は根の辺の訪問回数の和なので、
/// 指定した回数を終えた木への追加の選択はエラーになる。
#[derive(Debug)]
pub struct Gumbel {
    root_gumbel: Vec<f32>,
    considered_visits: Vec<u32>,
    num_considered: usize,
    value_scale: f32,
}

impl Gumbel {
    /// 全辺のGumbel乱数、正のシミュレーション回数、およびQの尺度から選択規則を作る。
    /// 尺度はmctxの`qtransform_completed_by_mix_value`の`value_scale`に当たる。
    ///
    /// # Errors
    /// 辺がない場合、回数が0の場合、および乱数が有限でない場合は拒否する。
    /// 尺度も有限かつ正でなければ拒否する。
    pub fn new(
        root_gumbel: Vec<f32>,
        num_simulations: u32,
        value_scale: f32,
    ) -> Result<Self, TreeError> {
        if root_gumbel.is_empty()
            || num_simulations == 0
            || root_gumbel.iter().any(|x| !x.is_finite())
            || !value_scale.is_finite()
            || value_scale <= 0.0
        {
            return Err(TreeError::InvalidGumbelConfig);
        }
        let num_considered = 16.min(num_simulations as usize).min(root_gumbel.len());
        Ok(Self {
            root_gumbel,
            considered_visits: considered_visits(num_considered, num_simulations)?,
            num_considered,
            value_scale,
        })
    }

    /// 合法手数と予算で制限した根の候補数を返す。
    pub fn num_considered(&self) -> usize {
        self.num_considered
    }

    /// 確定した勝ちを優先し、確定した負けを除いた最大訪問回数の辺からスコアで選ぶ。
    ///
    /// 根が勝ちに確定した場合と全辺が負けの場合は、[`Node::best_edge`]と同じ順序を使う。
    /// 得点は`gumbel_scale × g + (logit − max_logit) + σ(q̂)`とし、
    /// 倍率はこの選択だけに使い、根の逐次半減や方策の目標には使わない。
    ///
    /// # Errors
    /// 根が未評価、辺がない、または乱数の数と辺の数が異なる場合は拒否する。
    /// 倍率も有限かつ0以上でなければ拒否する。
    pub fn action(&self, root: &Node, gumbel_scale: f32) -> Result<usize, TreeError> {
        if !gumbel_scale.is_finite() || gumbel_scale < 0.0 {
            return Err(TreeError::InvalidGumbelConfig);
        }
        self.validate_root(root)?;
        if matches!(root.state, NodeState::Proven(proof) if proof.outcome == Outcome::Win) {
            return root.best_edge().ok_or(TreeError::NoSelectableEdge);
        }
        let eligible = |edge: &Edge| {
            !matches!(edge.child.as_deref().map(|child| child.state),
                Some(NodeState::Proven(proof)) if proof.outcome == Outcome::Win)
        };
        let considered = root
            .edges
            .iter()
            .filter(|edge| eligible(edge))
            .map(|edge| edge.visits)
            .max();
        match considered {
            Some(visits) => self.score_considered(root, visits, gumbel_scale, eligible),
            None => root.best_edge().ok_or(TreeError::NoSelectableEdge),
        }
    }

    /// 混合価値で未訪問の辺を補い、構築時に指定した尺度の変換σを施したQを返す。
    ///
    /// # Errors
    /// ネットの評価または辺を持たない節点は拒否する。
    pub fn completed_qvalues(&self, node: &Node) -> Result<Vec<f32>, TreeError> {
        let mixed = mixed_value(node)?;
        let max_visits = node
            .edges
            .iter()
            .map(|edge| edge.visits)
            .max()
            .ok_or(TreeError::NoSelectableEdge)?;
        let mut completed: Vec<_> = node
            .edges
            .iter()
            .map(|edge| {
                if edge.visits > 0 {
                    edge.value_sum as f32 / edge.visits as f32
                } else {
                    mixed
                }
            })
            .collect();
        let min = completed.iter().copied().fold(f32::INFINITY, f32::min);
        let max = completed.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let scale = (50.0 + max_visits as f32) * self.value_scale;
        for q in &mut completed {
            *q = ((*q - min) / (max - min).max(1e-8)) * scale;
        }
        Ok(completed)
    }

    /// 節点の全辺に対する方策の目標`softmax(logit + σq)`を返す。
    ///
    /// # Errors
    /// ネットの評価または辺を持たない節点は拒否する。
    pub fn policy_target(&self, node: &Node) -> Result<Vec<f32>, TreeError> {
        let completed = self.completed_qvalues(node)?;
        let logits: Vec<_> = node
            .edges
            .iter()
            .zip(completed)
            .map(|(edge, q)| edge.logit + q)
            .collect();
        Ok(softmax(&logits))
    }

    pub(super) fn select_interior(&self, node: &Node) -> Result<usize, TreeError> {
        let policy = self.policy_target(node)?;
        let sum = node
            .edges
            .iter()
            .map(|edge| edge.visits as f32)
            .sum::<f32>();
        argmax(
            node.edges
                .iter()
                .zip(policy)
                .map(|(edge, p)| p - edge.visits as f32 / (1.0 + sum)),
        )
    }

    pub(super) fn validate_root(&self, root: &Node) -> Result<(), TreeError> {
        if self.root_gumbel.len() != root.edges.len() {
            return Err(TreeError::InvalidGumbelConfig);
        }
        check_evaluated(root)
    }

    pub(super) fn select_root(&self, root: &Node) -> Result<usize, TreeError> {
        let visits = root
            .edges
            .iter()
            .map(|edge| u64::from(edge.visits))
            .sum::<u64>();
        let considered = *self
            .considered_visits
            .get(visits as usize)
            .ok_or(TreeError::BudgetExhausted)?;
        self.score_considered(root, considered, 1.0, |_| true)
    }

    fn score_considered(
        &self,
        node: &Node,
        considered: u32,
        gumbel_scale: f32,
        eligible: impl Fn(&Edge) -> bool,
    ) -> Result<usize, TreeError> {
        let completed = self.completed_qvalues(node)?;
        let max_logit = node
            .edges
            .iter()
            .map(|edge| edge.logit)
            .fold(f32::NEG_INFINITY, f32::max);
        argmax(node.edges.iter().enumerate().map(|(index, edge)| {
            if edge.visits == considered && eligible(edge) {
                (gumbel_scale * self.root_gumbel[index]
                    + (edge.logit - max_logit)
                    + completed[index])
                    .max(-1e9)
            } else {
                f32::NEG_INFINITY
            }
        }))
    }
}

/// Sequential Halvingで各シミュレーションが選択対象とする訪問回数の列。
///
/// # Errors
/// 候補数が0の場合は拒否する。予算0の列は空である。
pub fn considered_visits(
    num_considered: usize,
    num_simulations: u32,
) -> Result<Vec<u32>, TreeError> {
    if num_considered == 0 {
        return Err(TreeError::InvalidGumbelConfig);
    }
    if num_considered == 1 {
        return Ok((0..num_simulations).collect());
    }
    let log2max = (usize::BITS - (num_considered - 1).leading_zeros()) as usize;
    let budget = num_simulations as usize;
    let mut sequence = Vec::with_capacity(budget);
    let mut visits = vec![0; num_considered];
    let mut remaining = num_considered;
    while sequence.len() < budget {
        let extra = (budget / (log2max * remaining)).max(1);
        for _ in 0..extra {
            for visit in &mut visits[..remaining] {
                if sequence.len() == budget {
                    return Ok(sequence);
                }
                sequence.push(*visit);
                *visit += 1;
            }
        }
        remaining = (remaining / 2).max(2);
    }
    Ok(sequence)
}

/// mctxの`qtransform_completed_by_mix_value`が未訪問の辺に与える混合価値を返す。
/// 節点の手番側から見たネットの価値と、訪問済みの辺の方策で重み付けしたQを混合する。
///
/// # Errors
/// ネットの評価または辺を持たない節点は拒否する。
pub fn mixed_value(node: &Node) -> Result<f32, TreeError> {
    check_evaluated(node)?;
    let value = node.value.ok_or(TreeError::NotEvaluated)?;
    let mut total_visits = 0.0_f32;
    let mut sum_policy = 0.0_f32;
    let mut weighted_q = 0.0_f32;
    for edge in &node.edges {
        total_visits += edge.visits as f32;
        if edge.visits > 0 {
            let p = edge.policy.max(f32::MIN_POSITIVE);
            sum_policy += p;
            weighted_q += p * (edge.value_sum as f32 / edge.visits as f32);
        }
    }
    Ok(if total_visits > 0.0 {
        (value + total_visits * (weighted_q / sum_policy)) / (total_visits + 1.0)
    } else {
        value
    })
}

fn check_evaluated(node: &Node) -> Result<(), TreeError> {
    if matches!(node.state, NodeState::Pending { .. }) || node.value.is_none() {
        return Err(TreeError::NotEvaluated);
    }
    if node.edges.is_empty() {
        return Err(TreeError::NoSelectableEdge);
    }
    Ok(())
}

fn argmax(scores: impl Iterator<Item = f32>) -> Result<usize, TreeError> {
    let mut best = None;
    let mut maximum = f32::NEG_INFINITY;
    for (index, score) in scores.enumerate() {
        if score > maximum {
            maximum = score;
            best = Some(index);
        }
    }
    best.ok_or(TreeError::NoSelectableEdge)
}
