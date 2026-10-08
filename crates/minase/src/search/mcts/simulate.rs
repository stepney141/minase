//! シミュレーション（根から選択で降り、葉を評価して逆伝播する）とバッチ評価。
//!
//! 探索木は局面を直接扱わず、[`Domain`]を通じて合法手の列挙、着手の適用、終局の判定、
//! 方策ラベル、および入力平面の符号化を問い合わせる。対局では[`PositionDomain`]を使い、
//! 試験では表で与えた木を同じ抽象の上で探索する。

use minase_core::MoveGenerator;
use minase_core::mv::Move;
use minase_core::position::{Position, Undo};
use minase_core::rules::MoveRules;

use crate::nn::encoding::encode_into;
use crate::nn::evaluator::{Evaluator, EvaluatorError, INPUT_SIZE};
use crate::nn::{POLICY_SIZE, policy};
use crate::search::snapshot::search_key;

use super::engine::MctsError;
use super::select::{GumbelParams, GumbelRoot, gumbel_interior, puct};
use super::tree::{Edge, NodeId, NodeState, Proven, ROOT, Tree};

/// 探索木が局面について問い合わせる操作。
pub(super) trait Domain {
    /// 着手。
    type Action: Copy + PartialEq;

    /// 現局面の探索局面キー（RULES.md第24条第1項）を返す。反復の検出と木の再利用に使う。
    fn key(&self) -> u64;

    /// 現局面の手番側の合法手を`out`へ書く。
    fn legal_actions(&mut self, out: &mut Vec<Self::Action>);

    /// 着手が相手の最後の王駒を取るか（RULES.md第21条第1項）を返す。
    fn wins_at_once(&self, action: Self::Action) -> bool;

    /// 現局面での着手の方策の出力位置を返す。
    fn label(&self, action: Self::Action) -> Result<usize, MctsError>;

    /// 現局面の入力平面を`out`（[`INPUT_SIZE`]個）へ書く。
    fn encode(&self, out: &mut [f32]) -> Result<(), MctsError>;

    /// 着手を適用する。
    fn play(&mut self, action: Self::Action);

    /// 直前に適用した着手を取り消す。
    fn undo(&mut self);
}

/// 中将棋の局面を探索する[`Domain`]。
pub(super) struct PositionDomain {
    position: Position,
    rules: MoveRules,
    generator: MoveGenerator,
    undos: Vec<Undo>,
}

impl PositionDomain {
    /// 探索の根の局面と規則から作る。
    pub(super) fn new(position: Position, rules: MoveRules) -> Self {
        Self {
            position,
            rules,
            generator: MoveGenerator::new(rules),
            undos: Vec::new(),
        }
    }
}

impl Domain for PositionDomain {
    type Action = Move;

    fn key(&self) -> u64 {
        search_key(&self.position)
    }

    fn legal_actions(&mut self, out: &mut Vec<Move>) {
        out.clear();
        self.generator.generate_moves(&self.position, out);
    }

    fn wins_at_once(&self, mv: Move) -> bool {
        let royals = self
            .position
            .royal_pieces(self.position.side_to_move().opposite());
        let royal_count = royals.popcount() as usize;
        royal_count > 0
            && self
                .position
                .captured_squares(mv)
                .into_iter()
                .flatten()
                .filter(|&square| royals.contains(square))
                .count()
                == royal_count
    }

    fn label(&self, mv: Move) -> Result<usize, MctsError> {
        policy::label(mv, self.position.side_to_move())
            .map(usize::from)
            .map_err(MctsError::Label)
    }

    fn encode(&self, out: &mut [f32]) -> Result<(), MctsError> {
        encode_into(&self.position, out).map_err(MctsError::Encoding)
    }

    fn play(&mut self, mv: Move) {
        self.undos
            .push(self.position.make_move_unchecked(mv, self.rules));
    }

    fn undo(&mut self) {
        let undo = self
            .undos
            .pop()
            .expect("an undo requires a move played in the search");
        self.position.unmake_move(undo);
    }
}

/// シミュレーションで子を選ぶ規則と、その状態。
pub(super) enum Rule<'a> {
    /// すべての節点でPUCTをとる。
    Puct,
    /// 根でSequential Halving、内部の節点で決定的な選択をとる。
    Gumbel(&'a GumbelRoot, GumbelParams),
}

/// 根から1回降りた結果。
enum Descent {
    /// 葉の勝敗または反復が評価なしで決まった。値は葉の手番側から見た勝率。
    Resolved(f64),
    /// 葉を展開し、入力平面を書いて評価を待っている。
    Pending(NodeId),
    /// 評価待ちの節点へ再び到達した。
    Collision,
}

/// 節点を展開した結果。
enum Expansion {
    /// 評価なしで勝敗が決まった。値は節点の手番側から見た勝率。
    Resolved(f64),
    /// 評価を待っている。
    Pending,
}

/// 評価待ちの葉と、根からの経路。
struct PendingLeaf {
    path: Vec<(NodeId, usize)>,
    node: NodeId,
}

/// シミュレーションの実行に使う局面とバッチの領域。
pub(super) struct Simulator<D: Domain> {
    /// 探索の根の局面から着手を適用・取消しする局面。
    pub(super) domain: D,
    /// バッチの入力平面。
    inputs: Vec<f32>,
    /// バッチの方策logit。
    policy: Vec<f32>,
    /// バッチの勝率。
    values: Vec<f32>,
    /// 合法手の列挙に再利用する領域。
    actions: Vec<D::Action>,
    /// 根から現在の節点までの経路の局面キー（根を含む）。
    path_keys: Vec<u64>,
    /// 評価待ちの葉。
    pending: Vec<PendingLeaf>,
}

impl<D: Domain> Simulator<D> {
    /// 根の局面にある`domain`から作る。
    pub(super) fn new(domain: D) -> Self {
        Self {
            domain,
            inputs: Vec::new(),
            policy: Vec::new(),
            values: Vec::new(),
            actions: Vec::new(),
            path_keys: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// 根が展開されていなければ、`root_actions`を合法手として展開し、評価する。
    ///
    /// 展開した場合は1回のシミュレーションとして真を返す。
    ///
    /// # Errors
    ///
    /// 符号化または推論に失敗した場合はエラーを返す。
    pub(super) fn expand_root(
        &mut self,
        tree: &mut Tree<D::Action>,
        evaluator: &mut dyn Evaluator,
        root_actions: &[D::Action],
    ) -> Result<bool, MctsError> {
        if tree.root().state != NodeState::Unexpanded {
            return Ok(false);
        }
        self.pending.clear();
        if let Expansion::Pending = self.expand(tree, ROOT, root_actions)? {
            self.pending.push(PendingLeaf {
                path: Vec::new(),
                node: ROOT,
            });
            self.evaluate_pending(tree, evaluator)?;
        }
        Ok(true)
    }

    /// 最大`max_leaves`回のシミュレーションを行い、評価が必要な葉を1回の推論にまとめる。
    ///
    /// 評価待ちの節点へ再び到達した場合と、根の勝敗が確定した場合は、そこで葉の収集をやめる。
    /// 行ったシミュレーションの回数を返す。
    ///
    /// # Errors
    ///
    /// 符号化または推論に失敗した場合はエラーを返す。このとき木には仮想損失と評価待ちの
    /// 節点が残るので、呼び出し側は木を捨てる。
    pub(super) fn run_batch(
        &mut self,
        tree: &mut Tree<D::Action>,
        evaluator: &mut dyn Evaluator,
        rule: &Rule<'_>,
        history_keys: &[u64],
        max_leaves: usize,
    ) -> Result<u32, MctsError> {
        self.pending.clear();
        let mut resolved = 0;
        while (resolved + self.pending.len()) < max_leaves && tree.root().proven.is_none() {
            let mut path = Vec::new();
            let descent = self.descend(tree, rule, history_keys, &mut path);
            for _ in 0..path.len() {
                self.domain.undo();
            }
            match descent? {
                Descent::Resolved(value) => {
                    tree.backup(&path, value);
                    resolved += 1;
                }
                Descent::Pending(node) => self.pending.push(PendingLeaf { path, node }),
                Descent::Collision => {
                    tree.revert(&path);
                    break;
                }
            }
        }
        let evaluated = self.pending.len();
        self.evaluate_pending(tree, evaluator)?;
        Ok((resolved + evaluated) as u32)
    }

    /// 根から選択で降り、葉を展開する。通った辺を`path`へ積み、局面は葉のまま返す。
    fn descend(
        &mut self,
        tree: &mut Tree<D::Action>,
        rule: &Rule<'_>,
        history_keys: &[u64],
        path: &mut Vec<(NodeId, usize)>,
    ) -> Result<Descent, MctsError> {
        let mut node = ROOT;
        self.path_keys.clear();
        self.path_keys.push(tree.root().key);
        loop {
            if let Some(proven) = tree.node(node).proven {
                return Ok(Descent::Resolved(proven.value()));
            }
            let edge = match rule {
                Rule::Puct => puct(tree.node_mut(node), node == ROOT),
                Rule::Gumbel(root, _) if node == ROOT => root.select(tree.node(node)),
                Rule::Gumbel(_, params) => gumbel_interior(tree.node(node), *params),
            };
            let chosen = &mut tree.node_mut(node).edges[edge];
            chosen.in_flight += 1;
            let (action, child) = (chosen.action, chosen.child);
            path.push((node, edge));
            self.domain.play(action);
            let key = self.domain.key();
            // 木の内側の反復は、経路の長さによらず毎回求め直して引き分けとする。
            if history_keys.contains(&key) || self.path_keys.contains(&key) {
                return Ok(Descent::Resolved(0.5));
            }
            self.path_keys.push(key);
            match child {
                Some(child) if tree.node(child).state == NodeState::Pending => {
                    return Ok(Descent::Collision);
                }
                Some(child) => node = child,
                None => {
                    let child = tree.push(key);
                    tree.node_mut(node).edges[edge].child = Some(child);
                    let mut actions = core::mem::take(&mut self.actions);
                    self.domain.legal_actions(&mut actions);
                    let expansion = self.expand(tree, child, &actions);
                    self.actions = actions;
                    return Ok(match expansion? {
                        Expansion::Resolved(value) => Descent::Resolved(value),
                        Expansion::Pending => Descent::Pending(child),
                    });
                }
            }
        }
    }

    /// 現局面の節点を合法手`actions`で展開する。
    ///
    /// 合法手がなければ負け（RULES.md第23条）、相手の最後の王駒を取る手があればその手だけを
    /// 辺とする勝ち（第21条第1項）として確定する。どちらでもなければ入力平面をバッチの次の行へ書く。
    fn expand(
        &mut self,
        tree: &mut Tree<D::Action>,
        id: NodeId,
        actions: &[D::Action],
    ) -> Result<Expansion, MctsError> {
        if actions.is_empty() {
            let node = tree.node_mut(id);
            node.proven = Some(Proven::Loss(0));
            node.state = NodeState::Ready;
            return Ok(Expansion::Resolved(0.0));
        }
        if let Some(&winning) = actions
            .iter()
            .find(|&&action| self.domain.wins_at_once(action))
        {
            let mut edge = Edge::new(winning, self.domain.label(winning)?);
            edge.prior = 1.0;
            edge.proven = Some(Proven::Win(1));
            let node = tree.node_mut(id);
            node.edges = vec![edge];
            node.proven = Some(Proven::Win(1));
            node.state = NodeState::Ready;
            return Ok(Expansion::Resolved(1.0));
        }
        let edges = actions
            .iter()
            .map(|&action| Ok(Edge::new(action, self.domain.label(action)?)))
            .collect::<Result<Vec<_>, MctsError>>()?;
        let row = self.pending.len();
        if self.inputs.len() < (row + 1) * INPUT_SIZE {
            self.inputs.resize((row + 1) * INPUT_SIZE, 0.0);
        }
        self.domain
            .encode(&mut self.inputs[row * INPUT_SIZE..(row + 1) * INPUT_SIZE])?;
        let node = tree.node_mut(id);
        node.edges = edges;
        node.state = NodeState::Pending;
        Ok(Expansion::Pending)
    }

    /// 評価待ちの葉を1回の推論にかけ、方策と勝率を設定して逆伝播する。
    ///
    /// 推論の出力に非有限値があれば、どの葉にも反映せずに
    /// [`EvaluatorError::NonFinite`]を返す（docs/plans/alphazero.md「推論層」）。
    fn evaluate_pending(
        &mut self,
        tree: &mut Tree<D::Action>,
        evaluator: &mut dyn Evaluator,
    ) -> Result<(), MctsError> {
        let batch = self.pending.len();
        if batch == 0 {
            return Ok(());
        }
        self.policy.resize(batch * POLICY_SIZE, 0.0);
        self.values.resize(batch, 0.0);
        let (policy, values) = (
            &mut self.policy[..batch * POLICY_SIZE],
            &mut self.values[..batch],
        );
        evaluator
            .evaluate(&self.inputs[..batch * INPUT_SIZE], batch, policy, values)
            .map_err(MctsError::Evaluator)?;
        if !policy
            .iter()
            .chain(values.iter())
            .all(|value| value.is_finite())
        {
            return Err(MctsError::Evaluator(EvaluatorError::NonFinite));
        }
        for (row, leaf) in self.pending.iter().enumerate() {
            let logits = &policy[row * POLICY_SIZE..(row + 1) * POLICY_SIZE];
            let node = tree.node_mut(leaf.node);
            for edge in &mut node.edges {
                edge.logit = logits[edge.label];
            }
            node.normalize_priors();
            node.raw_value = values[row];
            node.state = NodeState::Ready;
            tree.backup(&leaf.path, f64::from(values[row]));
        }
        self.pending.clear();
        Ok(())
    }
}
