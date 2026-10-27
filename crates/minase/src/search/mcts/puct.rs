//! 対局用のPUCT（方策を探索項に使う上側信頼限界）の選択。
//!
//! 式と既定値はDeepLearningShogiのコミット79df19eの
//! [`SelectMaxUcbChild`](https://github.com/TadaoYamaoka/DeepLearningShogi/blob/79df19e/usi/UctSearch.cpp#L1638-L1705)
//! に従う。仮想損失は訪問回数だけに加え、選ぶたびに方策の確率を累積する。

use super::tree::{Node, NodeState, Outcome, TreeError};

/// 根と内部の節点で使うPUCTの係数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PuctConfig {
    /// 内部の探索項の定数。
    pub c_init: f32,
    /// 内部の探索項の対数の尺度。
    pub c_base: f32,
    /// 内部の未訪問の辺の価値から引く係数。
    pub c_fpu: f32,
    /// 根の探索項の定数。
    pub c_init_root: f32,
    /// 根の探索項の対数の尺度。
    pub c_base_root: f32,
    /// 根の未訪問の辺の価値から引く係数。
    pub c_fpu_root: f32,
}

impl Default for PuctConfig {
    fn default() -> Self {
        Self {
            c_init: 1.44,
            c_base: 28288.0,
            c_fpu: 0.27,
            c_init_root: 1.16,
            c_base_root: 25617.0,
            c_fpu_root: 0.0,
        }
    }
}

impl PuctConfig {
    pub(super) fn validate(&self) -> Result<(), TreeError> {
        if [self.c_init, self.c_init_root, self.c_fpu, self.c_fpu_root]
            .iter()
            .any(|x| !x.is_finite() || *x < 0.0)
            || [self.c_base, self.c_base_root]
                .iter()
                .any(|x| !x.is_finite() || *x <= 0.0)
        {
            return Err(TreeError::InvalidPuctConfig);
        }
        Ok(())
    }
}

pub(super) fn select_child(
    node: &mut Node,
    is_root: bool,
    config: &PuctConfig,
) -> Result<usize, TreeError> {
    let (base, init, fpu_factor) = if is_root {
        (config.c_base_root, config.c_init_root, config.c_fpu_root)
    } else {
        (config.c_base, config.c_init, config.c_fpu)
    };
    let sum = node
        .edges
        .iter()
        .map(|edge| u64::from(edge.visits) + u64::from(edge.virtual_loss))
        .sum::<u64>() as f32;
    let sum_win = node.edges.iter().map(|edge| edge.value_sum).sum::<f64>() as f32;
    let c = ((sum + base + 1.0) / base).ln() + init;
    let fpu = fpu_factor * node.visited_policy.sqrt();
    let parent_q = if sum_win > 0.0 {
        (sum_win / sum - fpu).max(0.0)
    } else {
        0.0
    };
    let sqrt_sum = sum.sqrt();
    let init_u = if sum == 0.0 { 1.0 } else { sqrt_sum };
    let mut best = None;
    let mut best_score = f32::NEG_INFINITY;
    for (index, edge) in node.edges.iter().enumerate() {
        if let Some(child) = &edge.child
            && let NodeState::Proven(proof) = child.state
        {
            match proof.outcome {
                Outcome::Win => continue,
                Outcome::Loss => {
                    best = Some(index);
                    break;
                }
            }
        }
        let n = (u64::from(edge.visits) + u64::from(edge.virtual_loss)) as f32;
        let (q, u) = if n == 0.0 {
            (parent_q, init_u)
        } else {
            (edge.value_sum as f32 / n, sqrt_sum / (1.0 + n))
        };
        let score = q + c * u * edge.policy;
        if score > best_score {
            best = Some(index);
            best_score = score;
        }
    }
    let index = best.ok_or(TreeError::NoSelectableEdge)?;
    node.visited_policy += node.edges[index].policy;
    Ok(index)
}
