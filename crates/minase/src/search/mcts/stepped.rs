//! 推論を呼び出し側に任せて1手ずつ進める、自己対局用のGumbel探索。
//!
//! 自己対局の生成器は数百局を同時に進め、各局の探索が評価を必要とした葉を複数局からまとめて
//! 1回の推論にかける（docs/plans/alphazero.md「強化学習」）。
//! [`SteppedGumbelSearch`]は1局の1手分の探索を、評価が要る葉の入力平面を返して止まる段と、
//! 推論の出力を受けて続ける段に分け、推論を探索の外へ出す。
//! 根の展開、Gumbel乱数、Sequential Halving、および指す手の選び方は[`super::Engine`]のGumbel探索と同じである。

use core::num::NonZeroU32;

use minase_core::mv::Move;
use minase_core::rng::XorShift64;

use crate::nn::POLICY_SIZE;
use crate::search::snapshot::{SearchSnapshot, search_key};

use super::engine::{
    MAX_CONSIDERED_ACTIONS, MctsError, best_edge, gumbel_noise, root_policy_target, root_win_rate,
};
use super::select::{GUMBEL_PARAMS, GumbelRoot};
use super::simulate::{PositionDomain, Rule, Simulator};
use super::tree::Tree;

/// 探索を1段進めた結果。
pub enum Step {
    /// 葉の評価が必要である。入力平面は[`SteppedGumbelSearch::pending_input`]が返し、
    /// 推論の出力は[`SteppedGumbelSearch::supply`]へ渡す。
    Evaluate,
    /// 探索を終えた。
    Finished(SearchedMove),
}

/// 探索を終えた根の結果。
#[derive(Clone, PartialEq, Debug)]
pub struct SearchedMove {
    /// 指す手。Sequential Halvingが最後に残した手（根の勝敗が確定していれば最善の確定手）。
    pub best_move: Move,
    /// 根の手番側の勝率。[`super::Engine`]が評価値の元にする値と同じである。
    pub root_value: f32,
    /// 方策の目標softmax(logit＋σ(completed Q))を、根の着手ごとに並べたもの。
    pub policy_target: Vec<(Move, f32)>,
}

/// 推論を呼び出し側に任せるGumbel探索1手分。
pub struct SteppedGumbelSearch {
    /// 探索木。Gumbel探索は木を再利用しないので、1手ごとに作る。
    tree: Tree<Move>,
    /// 局面と評価待ちの葉。
    simulator: Simulator<PositionDomain>,
    /// 対局開始から根までの探索局面キー。
    history_keys: Vec<u64>,
    /// 1手の探索回数n。
    simulations: NonZeroU32,
    /// 根のGumbel乱数とSequential Halvingの表。根を評価した後に作る。
    gumbel: Option<GumbelRoot>,
}

impl SteppedGumbelSearch {
    /// 局面の探索を始める。根の評価が必要なら、最初の[`SteppedGumbelSearch::advance`]がその入力を返す。
    ///
    /// # Errors
    ///
    /// 根の合法手が方策のラベルを持たない場合と、根を符号化できない場合はエラーを返す。
    pub fn new(snapshot: &SearchSnapshot, simulations: NonZeroU32) -> Result<Self, MctsError> {
        let mut tree = Tree::new(search_key(snapshot.position()));
        let mut simulator = Simulator::new(PositionDomain::new(
            snapshot.position().clone(),
            snapshot.rules(),
        ));
        simulator.begin_root(&mut tree, snapshot.root_moves())?;
        Ok(Self {
            tree,
            simulator,
            history_keys: snapshot.history_keys().to_vec(),
            simulations,
            gumbel: None,
        })
    }

    /// 次に評価が必要な葉を出すか、探索を終えるまで進める。
    ///
    /// 評価待ちの葉があれば、[`SteppedGumbelSearch::supply`]を呼ぶまで[`Step::Evaluate`]を返す。
    /// Gumbel乱数は根を評価した直後に`rng`から根の辺の数だけ引く。
    ///
    /// # Errors
    ///
    /// 葉の符号化または方策のラベルの取得に失敗した場合はエラーを返す。
    pub fn advance(&mut self, rng: &mut XorShift64) -> Result<Step, MctsError> {
        loop {
            if self.simulator.pending_count() > 0 {
                return Ok(Step::Evaluate);
            }
            let root = self.tree.root();
            if root.proven.is_none() && self.gumbel.is_none() {
                let noise = (0..root.edges.len()).map(|_| gumbel_noise(rng)).collect();
                self.gumbel = Some(GumbelRoot::new(
                    noise,
                    self.simulations.get(),
                    MAX_CONSIDERED_ACTIONS.get(),
                    GUMBEL_PARAMS,
                ));
            }
            let Some(gumbel) = self.gumbel.as_ref().filter(|gumbel| {
                root.proven.is_none() && root.total_visits() < gumbel.simulations()
            }) else {
                return Ok(Step::Finished(self.outcome()));
            };
            self.simulator.collect_leaves(
                &mut self.tree,
                &Rule::Gumbel(gumbel, GUMBEL_PARAMS),
                &self.history_keys,
                1,
            )?;
        }
    }

    /// 評価待ちの葉の入力平面（[`crate::nn::evaluator::INPUT_SIZE`]個）を返す。評価待ちがなければ空を返す。
    pub fn pending_input(&self) -> &[f32] {
        self.simulator.pending_inputs()
    }

    /// 評価待ちの葉の方策logit（[`POLICY_SIZE`]個）と手番側の勝率を受け、逆伝播する。
    ///
    /// # Errors
    ///
    /// 出力に非有限値があれば、どの葉にも反映せずにエラーを返す。
    ///
    /// # Panics
    ///
    /// 評価待ちの葉がない場合と、`policy`の長さが[`POLICY_SIZE`]でない場合にパニックする。
    pub fn supply(&mut self, policy: &[f32], value: f32) -> Result<(), MctsError> {
        assert_eq!(policy.len(), POLICY_SIZE);
        self.simulator
            .apply_evaluations(&mut self.tree, policy, &[value])
    }

    /// 探索を終えた根の結果を作る。
    fn outcome(&self) -> SearchedMove {
        let best = best_edge(&self.tree, self.gumbel.as_ref());
        let root = self.tree.root();
        SearchedMove {
            best_move: root.edges[best].action,
            root_value: root_win_rate(&self.tree, best) as f32,
            policy_target: root_policy_target(root),
        }
    }
}
