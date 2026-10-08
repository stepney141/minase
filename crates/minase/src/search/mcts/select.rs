//! 子の選択規則（PUCTとGumbel）と、Gumbel探索の方策の目標。
//!
//! PUCTはdlshogiの`UCTSearcher::SelectMaxUcbChild`（`usi/UctSearch.cpp`、
//! TadaoYamaoka/DeepLearningShogiのmaster 0cc1685）の式と対局用の係数に従う。
//! Gumbelは、DeepMindのmctx 0.0.71の`gumbel_muzero_policy`（`policies.py`）、
//! `qtransform_completed_by_mix_value`（`qtransforms.py`）、
//! `gumbel_muzero_root_action_selection`と`gumbel_muzero_interior_action_selection`
//! （`action_selection.py`）、および`seq_halving.py`の定義に合わせる
//! （docs/plans/alphazero.md「RLの探索はGumbel AlphaZeroにする」「対局ではPUCTを使う」）。

use super::tree::{Node, Proven};

/// 子の選択規則。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Selection {
    /// 対局で使うPUCT。仮想損失で複数の葉を集めて1回の推論にかける。
    Puct,
    /// 自己対局で使うGumbel AlphaZero。根ではSequential Halving、内部の節点では決定的な選択をとる。
    Gumbel,
}

/// PUCTの係数。
struct PuctParams {
    /// 探索係数の初期値c_init。
    c_init: f32,
    /// 探索係数の増加の尺度c_base。
    c_base: f32,
    /// 未訪問の子の価値を親の価値から下げる量（FPU reduction）。
    fpu_reduction: f32,
}

/// 内部の節点のPUCTの係数（dlshogiの`C_init`＝144、`C_base`＝28288、`C_fpu_reduction`＝27）。
const PUCT_INTERIOR: PuctParams = PuctParams {
    c_init: 1.44,
    c_base: 28_288.0,
    fpu_reduction: 0.27,
};

/// 根のPUCTの係数（dlshogiの`C_init_root`＝116、`C_base_root`＝25617、`C_fpu_reduction_root`＝0）。
const PUCT_ROOT: PuctParams = PuctParams {
    c_init: 1.16,
    c_base: 25_617.0,
    fpu_reduction: 0.0,
};

/// PUCTで子を選び、FPUの累計を更新する。
///
/// dlshogiの`SelectMaxUcbChild`と同じく、ΣN＝Σ(N＋評価待ち)、
/// c＝log((ΣN＋c_base＋1)/c_base)＋c_init、未訪問の子のQは
/// max(0, ΣW/ΣN − FPU減算×√(選んだ子の事前確率の累計))（ΣW＝0なら0）、
/// 未訪問の子のuは√ΣN（ΣN＝0なら1）、訪問した子はQ＝W/(N＋評価待ち)、u＝√ΣN/(1＋N＋評価待ち)とし、
/// Q＋c·u·事前確率が最大の子を選ぶ。負けが確定した子は選ばない。
/// 呼び出し側は、勝敗が確定した節点では選択を行わない。
pub(super) fn puct<A>(node: &mut Node<A>, is_root: bool) -> usize {
    let params = if is_root { &PUCT_ROOT } else { &PUCT_INTERIOR };
    let sum: u32 = node
        .edges
        .iter()
        .map(|edge| edge.visits + edge.in_flight)
        .sum();
    let sum_win: f64 = node.edges.iter().map(|edge| edge.value_sum).sum();
    let sqrt_sum = (sum as f32).sqrt();
    let c = ((sum as f32 + params.c_base + 1.0) / params.c_base).ln() + params.c_init;
    let fpu_reduction = params.fpu_reduction * node.visited_policy.sqrt();
    let parent_q = if sum_win > 0.0 {
        (sum_win as f32 / sum as f32 - fpu_reduction).max(0.0)
    } else {
        0.0
    };
    let init_u = if sum == 0 { 1.0 } else { sqrt_sum };

    let mut best = None;
    let mut best_value = f32::NEG_INFINITY;
    for (index, edge) in node.edges.iter().enumerate() {
        if matches!(edge.proven, Some(Proven::Loss(_))) {
            continue;
        }
        let count = edge.visits + edge.in_flight;
        let (q, u) = if count == 0 {
            (parent_q, init_u)
        } else {
            (
                (edge.value_sum / f64::from(count)) as f32,
                sqrt_sum / (1 + count) as f32,
            )
        };
        let value = q + c * u * edge.prior;
        if best.is_none() || value > best_value {
            best = Some(index);
            best_value = value;
        }
    }
    let best = best.expect("a node without a proven result has an edge that is not lost");
    node.visited_policy += node.edges[best].prior;
    best
}

/// Gumbel探索の係数。
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct GumbelParams {
    /// σの訪問回数の初期値c_visit（mctxの`maxvisit_init`）。
    pub(super) c_visit: f64,
    /// σの尺度c_scale（mctxの`value_scale`）。
    pub(super) c_scale: f64,
}

/// 設計書の係数（c_visit＝50、c_scale＝1.0）。
pub(super) const GUMBEL_PARAMS: GumbelParams = GumbelParams {
    c_visit: 50.0,
    c_scale: 1.0,
};

/// mctxの`score_considered`が下限に使う値。
const LOW_LOGIT: f64 = -1e9;

/// 節点の各辺の変換済みのcompleted Q、σ(completed Q)を返す。
///
/// mctxの`qtransform_completed_by_mix_value`に従い、訪問した辺はQ＝W/N、未訪問の辺は混合価値
/// (v＋ΣN·q̄)/(1＋ΣN)とする。vは節点のネットの勝率、q̄は訪問した辺のQを事前確率で重みづけした平均である。
/// 辺の最小値と最大値で0〜1へ写し（幅の下限1e-8）、(c_visit＋max N)·c_scaleを掛ける。
pub(super) fn transformed_completed_q<A>(node: &Node<A>, params: GumbelParams) -> Vec<f64> {
    let probs = softmax(node.edges.iter().map(|edge| f64::from(edge.logit)));
    let total: u32 = node.total_visits();
    let mut visited_prob = 0.0;
    for (edge, &prob) in node.edges.iter().zip(&probs) {
        if edge.visits > 0 {
            visited_prob += prob.max(f64::MIN_POSITIVE);
        }
    }
    let weighted_q: f64 = node
        .edges
        .iter()
        .zip(&probs)
        .filter_map(|(edge, &prob)| {
            edge.q()
                .map(|q| prob.max(f64::MIN_POSITIVE) * q / visited_prob)
        })
        .sum();
    let mixed =
        (f64::from(node.raw_value) + f64::from(total) * weighted_q) / (f64::from(total) + 1.0);
    let completed: Vec<f64> = node
        .edges
        .iter()
        .map(|edge| edge.q().unwrap_or(mixed))
        .collect();
    let min = completed.iter().copied().fold(f64::INFINITY, f64::min);
    let max = completed.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let width = (max - min).max(1e-8);
    let max_visits = node.edges.iter().map(|edge| edge.visits).max().unwrap_or(0);
    let scale = (params.c_visit + f64::from(max_visits)) * params.c_scale;
    completed
        .into_iter()
        .map(|value| scale * (value - min) / width)
        .collect()
}

/// 改善後の方策softmax(logit＋σ(completed Q))を返す。
///
/// 根では自己対局の方策の目標（mctxの`action_weights`）になる。
pub(super) fn improved_policy<A>(node: &Node<A>, params: GumbelParams) -> Vec<f64> {
    let sigma = transformed_completed_q(node, params);
    softmax(
        node.edges
            .iter()
            .zip(sigma)
            .map(|(edge, sigma)| f64::from(edge.logit) + sigma),
    )
}

/// 内部の節点でGumbelの決定的な選択を行う。
///
/// mctxの`gumbel_muzero_interior_action_selection`に従い、
/// π′(a)−N(a)/(1＋ΣN)が最大の辺（同点は先の辺）を選ぶ。
pub(super) fn gumbel_interior<A>(node: &Node<A>, params: GumbelParams) -> usize {
    let total = f64::from(node.total_visits());
    let policy = improved_policy(node, params);
    argmax(
        node.edges
            .iter()
            .zip(policy)
            .map(|(edge, prob)| prob - f64::from(edge.visits) / (1.0 + total)),
    )
}

/// 根のGumbel乱数とSequential Halvingの訪問回数の表。
#[derive(Clone, Debug)]
pub(super) struct GumbelRoot {
    /// 根の辺ごとのGumbel乱数。
    gumbel: Vec<f64>,
    /// シミュレーションの番号ごとに、選んでよい子の訪問回数。
    considered_visits: Vec<u32>,
    /// 係数。
    params: GumbelParams,
}

impl GumbelRoot {
    /// 根の辺ごとのGumbel乱数、探索回数n、および候補数の上限から作る。
    ///
    /// 候補数はmctxと同じく min(上限, 根の辺の数) とする。
    /// 訪問回数の表をn個で打ち切るので、実際に訪問する候補は min(n, 上限, 辺の数) 個以下になる。
    pub(super) fn new(
        gumbel: Vec<f64>,
        simulations: u32,
        max_considered: u32,
        params: GumbelParams,
    ) -> Self {
        let considered = (max_considered as usize).min(gumbel.len());
        Self {
            gumbel,
            considered_visits: considered_visits(considered, simulations as usize),
            params,
        }
    }

    /// 探索回数nを返す。
    pub(super) fn simulations(&self) -> u32 {
        self.considered_visits.len() as u32
    }

    /// 根でSequential Halvingに従って子を選ぶ。
    ///
    /// mctxの`gumbel_muzero_root_action_selection`に従い、ΣN番目の表の値と訪問回数が等しい子のうち、
    /// Gumbel乱数＋(logit − 最大logit)＋σ(completed Q)が最大の子を選ぶ。
    pub(super) fn select<A>(&self, root: &Node<A>) -> usize {
        let index = (root.total_visits() as usize).min(self.considered_visits.len() - 1);
        self.argmax_considered(root, self.considered_visits[index])
    }

    /// 探索後に指す手を選ぶ。
    ///
    /// mctxの`gumbel_muzero_policy`に従い、訪問回数が最大の子のうち、
    /// Gumbel乱数＋(logit − 最大logit)＋σ(completed Q)が最大の子を選ぶ。
    pub(super) fn final_action<A>(&self, root: &Node<A>) -> usize {
        let max_visits = root.edges.iter().map(|edge| edge.visits).max().unwrap_or(0);
        self.argmax_considered(root, max_visits)
    }

    /// mctxの`score_considered`の最大の辺を返す。
    fn argmax_considered<A>(&self, root: &Node<A>, considered_visit: u32) -> usize {
        let sigma = transformed_completed_q(root, self.params);
        let max_logit = root
            .edges
            .iter()
            .map(|edge| f64::from(edge.logit))
            .fold(f64::NEG_INFINITY, f64::max);
        argmax(
            root.edges
                .iter()
                .zip(&self.gumbel)
                .zip(sigma)
                .map(|((edge, &gumbel), sigma)| {
                    if edge.visits == considered_visit {
                        (gumbel + f64::from(edge.logit) - max_logit + sigma).max(LOW_LOGIT)
                    } else {
                        f64::NEG_INFINITY
                    }
                }),
        )
    }
}

/// Sequential Halvingで、シミュレーションの番号ごとに選んでよい子の訪問回数を返す。
///
/// mctxの`get_sequence_of_considered_visits`に従う。log₂mを切り上げた段数で予算を分け、
/// 各段で残った候補に同じ回数ずつ訪問を割り当て、段ごとに候補を半分（2未満にはしない）にする。
pub(super) fn considered_visits(max_considered: usize, simulations: usize) -> Vec<u32> {
    if max_considered <= 1 {
        return (0..simulations as u32).collect();
    }
    let log2max = (usize::BITS - (max_considered - 1).leading_zeros()) as usize;
    let mut sequence = Vec::with_capacity(simulations);
    let mut visits = vec![0; max_considered];
    let mut considered = max_considered;
    while sequence.len() < simulations {
        let extra = (simulations / (log2max * considered)).max(1);
        for _ in 0..extra {
            sequence.extend_from_slice(&visits[..considered]);
            for visit in &mut visits[..considered] {
                *visit += 1;
            }
        }
        considered = (considered / 2).max(2);
    }
    sequence.truncate(simulations);
    sequence
}

/// 値の列のsoftmaxを返す。
fn softmax(values: impl Iterator<Item = f64> + Clone) -> Vec<f64> {
    let max = values.clone().fold(f64::NEG_INFINITY, f64::max);
    let exps: Vec<f64> = values.map(|value| (value - max).exp()).collect();
    let sum: f64 = exps.iter().sum();
    exps.into_iter().map(|value| value / sum).collect()
}

/// 最大値の最初の位置を返す。空なら0を返す。
fn argmax(values: impl Iterator<Item = f64>) -> usize {
    let mut best = 0;
    let mut best_value = f64::NEG_INFINITY;
    for (index, value) in values.enumerate() {
        if value > best_value {
            best = index;
            best_value = value;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    // alphazero.md「RLの探索はGumbel AlphaZeroにする」: 表はmctx 0.0.71の
    // get_sequence_of_considered_visitsが同じ引数で返す値と一致する。
    #[test]
    fn considered_visits_match_mctx_examples() {
        assert_eq!(considered_visits(1, 6), [0, 1, 2, 3, 4, 5]);
        assert_eq!(considered_visits(2, 6), [0, 0, 1, 1, 2, 2]);
        assert_eq!(considered_visits(4, 8), [0, 0, 0, 0, 1, 1, 2, 2]);
        assert_eq!(
            considered_visits(4, 16),
            [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5]
        );
    }
}
