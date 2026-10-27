//! 局面と対局履歴を扱い、木の選択をネットへの依頼へ写す駆動部。

use minase_core::{
    Color, Move, MoveGenerator, Position, PromotionRule, move_was_attacking, repetition_key,
};

use super::Error;
use super::tree::{Child, LeafKind, NodeState, Outcome, Proof, SelectionRule, Tree, TreeError};
use crate::nn::Query;
use crate::search::{SearchRepetition, SearchSnapshot};

/// R1の引き分けを勝率へ写す方法。
#[derive(Clone, Copy, Debug)]
pub enum DrawValue {
    /// 根の手番側から見たcpを、指定された勝率尺度で換算する。
    Game {
        /// 呼び出し側の評価関数が持つ1ロジットあたりのcp。
        cp_per_logit: f32,
    },
    /// 自己対局では引き分けの設定によらず0.5を使う。
    SelfPlay,
}

impl DrawValue {
    fn probability(self, snapshot: &SearchSnapshot) -> Result<f32, Error> {
        match self {
            Self::SelfPlay => Ok(0.5),
            Self::Game { cp_per_logit } => {
                validate_scale(cp_per_logit)?;
                Ok((1.0
                    / (1.0 + (-f64::from(snapshot.draw_value()) / f64::from(cp_per_logit)).exp()))
                    as f32)
            }
        }
    }
}

pub(super) fn validate_scale(scale: f32) -> Result<(), Error> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(Error::InvalidScale);
    }
    Ok(())
}

pub(super) fn validate_rules(snapshot: &SearchSnapshot) -> Result<(), Error> {
    if snapshot.rules().promotion != PromotionRule::P0 || snapshot.rules().p5 {
        return Err(Error::PromotionDeferred);
    }
    Ok(())
}

/// 評価待ちの局面、合法手、および元の駆動部へ結果を返すための経路。
///
/// 複数局から集めた依頼を[`Self::query`]で借用し、1回の推論へまとめられる。
/// 依頼は選択した駆動部へ返し、評価または取り消しを1回だけ行う。
pub struct EvaluationRequest {
    path: Vec<usize>,
    position: Box<Position>,
    moves: Vec<Move>,
}

impl EvaluationRequest {
    /// 推論器へ渡す局面と合法手を借用する。
    pub fn query(&self) -> Query<'_> {
        Query {
            position: &self.position,
            moves: &self.moves,
        }
    }
}

/// 1回の降下で駆動部が行った処理。
pub enum Step {
    /// 葉の評価を依頼する。根の初回評価も含む。
    Evaluate(EvaluationRequest),
    /// 既知の葉を逆伝播し、シミュレーションを1回数えた。
    BackedUp,
    /// 評価待ちの葉に衝突した。現在の収集を終えて評価結果を返す。
    Collision,
    /// 木の容量が尽きた。現在の収集を終えて探索を停止する。
    Capacity,
}

/// 1局の局面、履歴、および探索木を持つ、推論器に依存しないMCTSの駆動部。
pub struct Driver {
    snapshot: SearchSnapshot,
    tree: Tree,
    draw_probability: f32,
    simulations: u64,
    root_evaluated: bool,
    root_pending: bool,
}

impl Driver {
    /// 根の候補手から新しい探索を作る。
    ///
    /// # Errors
    /// 成り規則、勝率尺度、または木の容量が不正なら拒否する。
    pub fn new(
        snapshot: SearchSnapshot,
        capacity_bytes: usize,
        draw: DrawValue,
    ) -> Result<Self, Error> {
        validate_rules(&snapshot)?;
        let draw_probability = draw.probability(&snapshot)?;
        let tree = Tree::new(snapshot.root_moves(), capacity_bytes)?;
        Ok(Self {
            snapshot,
            tree,
            draw_probability,
            simulations: 0,
            root_evaluated: false,
            root_pending: false,
        })
    }

    /// 葉を1つ選ぶ。評価が要らない葉はその場で逆伝播する。
    ///
    /// # Errors
    /// 選択規則または木の操作が不正なら、元のエラーを保持して返す。
    pub fn select_leaf(&mut self, rule: SelectionRule<'_>) -> Result<Step, Error> {
        if !self.root_evaluated {
            if self.root_pending {
                return Ok(Step::Collision);
            }
            self.root_pending = true;
            return Ok(Step::Evaluate(EvaluationRequest {
                path: Vec::new(),
                position: Box::new(self.snapshot.position().clone()),
                moves: self.tree.root().edges.iter().map(|edge| edge.mv).collect(),
            }));
        }
        let selection = self.tree.select_leaf(rule)?;
        match selection.kind {
            LeafKind::Collision => Ok(Step::Collision),
            LeafKind::Value(_) => {
                self.tree.backup_leaf(&selection.path)?;
                self.simulations += 1;
                Ok(Step::BackedUp)
            }
            LeafKind::New => {
                let (position, moves, state) = self.classify_child(&selection.path);
                let child = match state {
                    NodeState::Pending { unexpanded: false } => Child::Pending(&moves),
                    NodeState::Pending { unexpanded: true } => Child::Unexpanded,
                    NodeState::Proven(proof) => Child::Proven(proof),
                    NodeState::Draw(value) => Child::Draw(value),
                    _ => unreachable!("a new child is pending or terminal"),
                };
                if let Err(error) = self.tree.create_child(&selection.path, child) {
                    self.tree.cancel_leaf(&selection.path)?;
                    return if error == TreeError::CapacityExceeded {
                        Ok(Step::Capacity)
                    } else {
                        Err(error.into())
                    };
                }
                if matches!(state, NodeState::Pending { .. }) {
                    Ok(Step::Evaluate(EvaluationRequest {
                        path: selection.path,
                        position: Box::new(position),
                        moves,
                    }))
                } else {
                    self.simulations += 1;
                    Ok(Step::BackedUp)
                }
            }
        }
    }

    /// 選んだ葉へネットの結果を入れる。成功したときだけ1回を数える。
    ///
    /// # Errors
    /// 依頼と異なる数のlogit、不正な勝率、または処理済みの依頼を拒否する。
    /// 失敗時は依頼を保持し、再投入または[`Self::cancel`]ができる。
    pub fn evaluate(
        &mut self,
        request: &EvaluationRequest,
        logits: &[f32],
        value: f32,
    ) -> Result<(), Error> {
        if request.path.is_empty() && !self.root_pending {
            return Err(TreeError::InvalidSelection.into());
        }
        self.tree.evaluate(&request.path, logits, value)?;
        if request.path.is_empty() {
            self.root_pending = false;
            self.root_evaluated = true;
        }
        self.simulations += 1;
        Ok(())
    }

    /// 評価を中止し、葉の仮想損失を戻す。
    ///
    /// # Errors
    /// 未完了の依頼でなければ拒否する。
    pub fn cancel(&mut self, request: &EvaluationRequest) -> Result<(), Error> {
        if request.path.is_empty() {
            if !self.root_pending {
                return Err(TreeError::InvalidSelection.into());
            }
            self.root_pending = false;
        } else {
            self.tree.cancel_leaf(&request.path)?;
        }
        Ok(())
    }

    /// 探索木を参照する。Gumbelの最終手と方策目標もこの根から求める。
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// この探索で完了したシミュレーション数。根の評価を1回目とする。
    pub fn simulations(&self) -> u64 {
        self.simulations
    }

    /// 根の確定値、または最善手の勝率をcpへ写す。
    ///
    /// # Errors
    /// 根が未評価、または勝率尺度が不正なら拒否する。
    pub fn score(&self, cp_per_logit: f32) -> Result<i32, Error> {
        validate_scale(cp_per_logit)?;
        let root = self.tree.root();
        if let NodeState::Proven(proof) = root.state {
            return Ok(proof.score());
        }
        let edge = root.best_edge().ok_or(TreeError::NotEvaluated)?;
        let value = root.edges[edge]
            .q()
            .or(root.value)
            .ok_or(TreeError::NotEvaluated)?;
        let p = f64::from(value).clamp(0.001, 0.999);
        Ok((f64::from(cp_per_logit) * (p / (1.0 - p)).ln()).round() as i32)
    }

    /// 履歴の差分をたどれ、根の規則と候補手が一致するときだけ木を付け替える。
    pub(super) fn reuse(
        &mut self,
        snapshot: &SearchSnapshot,
        draw: DrawValue,
    ) -> Result<bool, Error> {
        validate_rules(snapshot)?;
        let probability = draw.probability(snapshot)?;
        if self.root_pending
            || snapshot.rules() != self.snapshot.rules()
            || snapshot.repetition().rule() != self.snapshot.repetition().rule()
            || probability.to_bits() != self.draw_probability.to_bits()
            || !snapshot
                .history_keys()
                .starts_with(self.snapshot.history_keys())
        {
            return Ok(false);
        }
        // 根の手番が変わると非中立の引き分けの価値も反転する。逆伝播済みのQは使えない。
        if probability != 0.5
            && snapshot.position().side_to_move() != self.snapshot.position().side_to_move()
        {
            return Ok(false);
        }
        let mut position = self.snapshot.position().clone();
        let mut node = self.tree.root();
        let mut moves = Vec::new();
        for &key in &snapshot.history_keys()[self.snapshot.history_keys().len()..] {
            let found = node.edges.iter().find_map(|edge| {
                let child = edge.child.as_deref()?;
                let mut next = position.clone();
                next.make_move_unchecked(edge.mv, snapshot.rules());
                (repetition_key(snapshot.repetition().rule(), &next) == key)
                    .then_some((edge.mv, child, next))
            });
            let Some((mv, child, next)) = found else {
                return Ok(false);
            };
            moves.push(mv);
            node = child;
            position = next;
        }
        if position != *snapshot.position()
            || node.state != NodeState::Expanded
            || node.edges.len() != snapshot.root_moves().len()
            || node
                .edges
                .iter()
                .any(|edge| !snapshot.root_moves().contains(&edge.mv))
        {
            return Ok(false);
        }
        if self.tree.reroot(&moves).is_err() {
            return Ok(false);
        }
        self.snapshot = snapshot.clone();
        self.simulations = 0;
        self.root_evaluated = false;
        Ok(true)
    }

    fn classify_child(&self, path: &[usize]) -> (Position, Vec<Move>, NodeState) {
        let mut position = self.snapshot.position().clone();
        let mut keys = vec![repetition_key(self.snapshot.repetition().rule(), &position)];
        let mut played = Vec::with_capacity(path.len());
        let mut node = self.tree.root();
        let mut royal_capture = false;
        for (depth, &index) in path.iter().enumerate() {
            let edge = &node.edges[index];
            if depth + 1 == path.len() {
                royal_capture = captures_last_royal(&position, edge.mv);
            }
            position.make_move_unchecked(edge.mv, self.snapshot.rules());
            played.push(edge.mv);
            if depth + 1 < path.len() {
                keys.push(repetition_key(self.snapshot.repetition().rule(), &position));
                node = edge
                    .child
                    .as_deref()
                    .expect("selection follows existing children");
            }
        }
        let state = if royal_capture {
            proven(Outcome::Loss, false)
        } else if let Some(state) = self.repetition(&position, &keys, &played) {
            state
        } else if path.len() == self.tree.depth_limit() as usize {
            NodeState::Pending { unexpanded: true }
        } else {
            let generator = MoveGenerator::new(self.snapshot.rules());
            let mut moves = Vec::new();
            generator.generate_moves(&position, &mut moves);
            if moves.is_empty() {
                proven(Outcome::Loss, false)
            } else {
                keys.push(repetition_key(self.snapshot.repetition().rule(), &position));
                moves.retain(|&mv| !self.move_is_forbidden(&mut position, mv, &keys));
                if moves.is_empty() {
                    proven(Outcome::Loss, true)
                } else {
                    return (position, moves, NodeState::Pending { unexpanded: false });
                }
            }
        };
        (position, Vec::new(), state)
    }

    fn history_before_root(&self) -> &[u64] {
        let history = self.snapshot.history_keys();
        &history[..history.len().saturating_sub(1)]
    }

    fn move_is_forbidden(&self, position: &mut Position, mv: Move, keys: &[u64]) -> bool {
        let limit = match self.snapshot.repetition() {
            SearchRepetition::R1 { .. } => return false,
            SearchRepetition::R2 => 1,
            SearchRepetition::R3 => 3,
        };
        let undo = position.make_move_unchecked(mv, self.snapshot.rules());
        let key = repetition_key(self.snapshot.repetition().rule(), position);
        position.unmake_move(undo);
        self.history_before_root()
            .iter()
            .chain(keys)
            .filter(|&&past| past == key)
            .take(limit)
            .count()
            == limit
    }

    fn repetition(&self, position: &Position, keys: &[u64], moves: &[Move]) -> Option<NodeState> {
        let key = repetition_key(self.snapshot.repetition().rule(), position);
        let path_match = keys.iter().rposition(|&past| past == key);
        let history_match = if path_match.is_none() {
            self.history_before_root()
                .iter()
                .rposition(|&past| past == key)
        } else {
            None
        };
        if path_match.is_none() && history_match.is_none() {
            return None;
        }
        let SearchRepetition::R1 {
            consecutive_attacking_moves,
        } = self.snapshot.repetition()
        else {
            return Some(NodeState::Draw(0.5));
        };
        let mut attackers = [true; 2];
        let start = if let Some(index) = path_match {
            index
        } else {
            let plies = self.snapshot.history_keys().len() - 1 - history_match.unwrap();
            let last_mover = self.snapshot.position().side_to_move().opposite();
            for color in Color::ALL {
                let count = plies / 2 + usize::from(plies % 2 == 1 && color == last_mover);
                attackers[color.index()] =
                    count <= consecutive_attacking_moves[color.index()] as usize;
            }
            0
        };
        let generator = MoveGenerator::new(self.snapshot.rules());
        let mut replay = self.snapshot.position().clone();
        for (index, &mv) in moves.iter().enumerate() {
            let mover = replay.side_to_move();
            replay.make_move_unchecked(mv, self.snapshot.rules());
            if index >= start {
                attackers[mover.index()] &= move_was_attacking(&replay, &generator, mover, mv);
            }
            if attackers == [false; 2] {
                break;
            }
        }
        let side = position.side_to_move();
        Some(
            match (attackers[side.index()], attackers[side.opposite().index()]) {
                (true, false) => proven(Outcome::Loss, true),
                (false, true) => proven(Outcome::Win, true),
                _ => NodeState::Draw(if side == self.snapshot.position().side_to_move() {
                    self.draw_probability
                } else {
                    1.0 - self.draw_probability
                }),
            },
        )
    }
}

fn proven(outcome: Outcome, repetition: bool) -> NodeState {
    NodeState::Proven(Proof {
        outcome,
        distance: 0,
        repetition,
    })
}

fn captures_last_royal(position: &Position, mv: Move) -> bool {
    let royals = position.royal_pieces(position.side_to_move().opposite());
    !royals.is_empty()
        && position
            .captured_squares(mv)
            .into_iter()
            .flatten()
            .filter(|&square| royals.contains(square))
            .count()
            == royals.popcount() as usize
}

#[cfg(test)]
mod tests;
