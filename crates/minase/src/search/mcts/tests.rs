//! フェーズ2aの契約を、mctxの固定出力と合成木の手計算で検証する。
//!
//! 照合する量は経路、訪問回数、最終手、変換済みQ、および方策の目標である。
//! PUCTは未訪問・訪問済み・仮想損失・根の係数を分けて検査し、木の操作は
//! 勝率の視点、確定の順序、衝突、取り消し、容量上限、再利用、深さ上限を検査する。
//! ネットの推論や学習済みの重みは使わない。Moveは合成環境の行動の識別子である。

use std::mem::size_of;

use minase_core::{Move, MoveGenerator, Position, Rules};
use serde::Deserialize;

use super::gumbel::{Gumbel, completed_qvalues, considered_visits, policy_target};
use super::puct::{PuctConfig, select_child};
use super::tree::{
    Child, Edge, LeafKind, Node, NodeState, Outcome, Proof, SelectionRule, Tree, TreeError,
};
use crate::search::MAX_PLY;

fn moves(count: usize) -> Vec<Move> {
    let mut moves = Vec::new();
    MoveGenerator::new(Rules::ENGINE_DEFAULT.moves)
        .generate_moves(&Position::initial(), &mut moves);
    assert!(moves.len() >= count);
    moves.truncate(count);
    moves
}

fn tree(logits: &[f32], value: f32) -> Tree {
    let mut tree = Tree::new(&moves(logits.len()), 1 << 20).unwrap();
    tree.evaluate(&[], logits, value).unwrap();
    tree
}

fn proof(outcome: Outcome, distance: u32, repetition: bool) -> Proof {
    Proof {
        outcome,
        distance,
        repetition,
    }
}

fn terminal(proof: Proof) -> Box<Node> {
    Box::new(Node {
        value: None,
        state: NodeState::Proven(proof),
        edges: Box::new([]),
        visited_policy: 0.0,
    })
}

#[test]
fn gumbel_action_keeps_a_proven_win_despite_a_losing_edges_higher_logit() {
    let mut root = statistics_node(&[(0.5, 1, 0.0), (0.5, 1, 1.0)], 0.5, 0.0);
    root.edges[0].logit = 10.0;
    root.edges[1].logit = 0.0;
    root.edges[0].child = Some(terminal(proof(Outcome::Win, 0, false)));
    root.edges[1].child = Some(terminal(proof(Outcome::Loss, 0, false)));
    root.state = NodeState::Proven(proof(Outcome::Win, 1, false));
    let gumbel = Gumbel::new(vec![0.0; 2], 2).unwrap();
    assert_eq!(gumbel.action(&root).unwrap(), 1);
}

#[test]
fn gumbel_action_compares_visits_and_scores_after_excluding_proven_losses() {
    for losing_visits in [1, 4] {
        let mut root = statistics_node(
            &[(0.5, losing_visits, 0.0), (0.25, 1, 0.5), (0.25, 1, 0.5)],
            0.5,
            0.0,
        );
        root.edges[0].logit = 10.0;
        root.edges[1].logit = 0.0;
        root.edges[2].logit = 1.0;
        root.edges[0].child = Some(terminal(proof(Outcome::Win, 0, false)));
        let gumbel = Gumbel::new(vec![0.0; 3], losing_visits + 2).unwrap();
        assert_eq!(gumbel.action(&root).unwrap(), 2);
    }
}

#[test]
fn gumbel_action_uses_proof_order_when_every_edge_is_a_loss() {
    for repetition in [false, true] {
        let mut root = statistics_node(&[(0.5, 4, 0.0), (0.5, 1, 0.0)], 0.5, 0.0);
        root.edges[0].logit = 10.0;
        root.edges[1].logit = 0.0;
        root.edges[0].child = Some(terminal(proof(Outcome::Win, 0, false)));
        root.edges[1].child = Some(terminal(proof(Outcome::Win, 20, repetition)));
        root.state = NodeState::Proven(proof(Outcome::Loss, 21, repetition));
        let gumbel = Gumbel::new(vec![0.0; 2], 5).unwrap();
        // 同じ負けなら遅い詰みを選び、反復負けは詰み負けより優先する。
        assert_eq!(gumbel.action(&root).unwrap(), 1);
    }
}

/// 純粋な選択式の入力となる、手計算で指定した統計。
fn statistics_node(statistics: &[(f32, u32, f64)], value: f32, visited_policy: f32) -> Node {
    Node {
        value: Some(value),
        state: NodeState::Expanded,
        edges: moves(statistics.len())
            .into_iter()
            .zip(statistics)
            .map(|(mv, &(policy, visits, value_sum))| Edge {
                mv,
                logit: policy.ln(),
                policy,
                visits,
                value_sum,
                virtual_loss: 0,
                child: None,
            })
            .collect(),
        visited_policy,
    }
}

fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 1e-5,
        "actual={actual}, expected={expected}"
    );
}

#[derive(Deserialize)]
struct Reference {
    considered_visits: Vec<SequenceCase>,
    cases: Vec<SearchCase>,
}

#[derive(Deserialize)]
struct SequenceCase {
    max_num_considered_actions: usize,
    num_simulations: u32,
    sequence: Vec<u32>,
}

#[derive(Deserialize)]
struct SearchCase {
    name: String,
    num_actions: usize,
    num_simulations: u32,
    root_gumbel: Vec<f32>,
    nodes: Vec<ReferenceNode>,
    expected: Expected,
}

#[derive(Deserialize)]
struct ReferenceNode {
    path: Vec<usize>,
    prior_logits: Vec<f32>,
    value: f32,
}

#[derive(Deserialize)]
struct Expected {
    action: usize,
    action_weights: Vec<f32>,
    root_children_visits: Vec<u32>,
    root_completed_qvalues: Vec<f32>,
}

fn reference() -> Reference {
    serde_json::from_str(include_str!("testdata/gumbel-mctx.json")).unwrap()
}

// mctx 428bfb7の固定出力をそのまま読み、端数と候補1の列も完全一致で照合する。
#[test]
fn sequential_halving_matches_all_mctx_sequences() {
    for case in reference().considered_visits {
        assert_eq!(
            considered_visits(case.max_num_considered_actions, case.num_simulations).unwrap(),
            case.sequence,
            "m={}, n={}",
            case.max_num_considered_actions,
            case.num_simulations,
        );
    }
}

// 期待値は tools/train/zero/gumbel_reference.py がmctxから採取したものである。
// 最終結果だけでなく各シミュレーションの新規葉も照合し、途中の選択の誤りを検出する。
#[test]
fn gumbel_matches_mctx_paths_visits_action_qvalues_and_policy() {
    for case in reference().cases {
        let actions = moves(case.num_actions);
        let mut tree = Tree::new(&actions, 1 << 20).unwrap();
        let root = &case.nodes[0];
        assert!(root.path.is_empty());
        tree.evaluate(&[], &root.prior_logits, (root.value + 1.0) / 2.0)
            .unwrap();
        let gumbel = Gumbel::new(case.root_gumbel, case.num_simulations).unwrap();
        assert_eq!(
            gumbel.num_considered(),
            16.min(case.num_actions).min(case.num_simulations as usize)
        );
        assert_eq!(case.nodes.len(), case.num_simulations as usize + 1);
        for (simulation, expected) in case.nodes.iter().enumerate().skip(1) {
            let leaf = tree.select_leaf(SelectionRule::Gumbel(&gumbel)).unwrap();
            assert_eq!(
                leaf.kind,
                LeafKind::New,
                "{} simulation {simulation}",
                case.name
            );
            assert_eq!(
                leaf.path, expected.path,
                "{} simulation {simulation}",
                case.name
            );
            tree.create_child(&leaf.path, Child::Pending(&actions))
                .unwrap();
            tree.evaluate(
                &leaf.path,
                &expected.prior_logits,
                (expected.value + 1.0) / 2.0,
            )
            .unwrap();
        }
        assert_eq!(
            gumbel.action(tree.root()).unwrap(),
            case.expected.action,
            "{}",
            case.name
        );
        let visits: Vec<_> = tree.root().edges.iter().map(|edge| edge.visits).collect();
        assert_eq!(visits, case.expected.root_children_visits, "{}", case.name);
        assert_eq!(visits.iter().sum::<u32>(), case.num_simulations);
        let policy = policy_target(tree.root()).unwrap();
        assert_eq!(policy.len(), case.expected.action_weights.len());
        for (actual, expected) in policy.into_iter().zip(case.expected.action_weights) {
            close(actual, expected);
        }
        let qvalues = completed_qvalues(tree.root()).unwrap();
        assert_eq!(qvalues.len(), case.expected.root_completed_qvalues.len());
        for (actual, expected) in qvalues
            .into_iter()
            .zip(case.expected.root_completed_qvalues)
        {
            close(actual, expected);
        }
        assert_eq!(
            tree.select_leaf(SelectionRule::Gumbel(&gumbel)),
            Err(TreeError::BudgetExhausted)
        );
    }
}

// Q=(3/4,1/4)、訪問済みの方策=(1/2,1/4)なので重みつき平均は7/12。
// v=1/2、総訪問3から混合価値は9/16。尺度5.2でσQ=(5.2,0,3.25)となる。
#[test]
fn completed_q_uses_policy_weighted_mix_and_single_precision_scale() {
    let node = statistics_node(&[(0.5, 2, 1.5), (0.25, 1, 0.25), (0.25, 0, 0.0)], 0.5, 0.0);
    for (actual, expected) in completed_qvalues(&node)
        .unwrap()
        .into_iter()
        .zip([5.2, 0.0, 3.25])
    {
        close(actual, expected);
    }
    // 未訪問だけなら全Qがvになり、σは0で、方策は元のsoftmaxに戻る。
    let node = statistics_node(&[(0.5, 0, 0.0), (0.25, 0, 0.0), (0.25, 0, 0.0)], 0.125, 0.0);
    assert_eq!(completed_qvalues(&node).unwrap(), vec![0.0; 3]);
    for (actual, expected) in policy_target(&node)
        .unwrap()
        .into_iter()
        .zip([0.5, 0.25, 0.25])
    {
        close(actual, expected);
    }
}

// 方策が単精度で0へ丸められた訪問済みの辺にも最小正規化数を使う。
#[test]
fn completed_q_clamps_underflowed_prior_before_mixing() {
    let mut node = statistics_node(&[(0.5, 1, 0.25), (0.5, 0, 0.0)], 0.75, 0.0);
    node.edges[0].logit = -1000.0;
    node.edges[0].policy = 0.0;
    node.edges[1].logit = 0.0;
    node.edges[1].policy = 1.0;
    // 混合価値=(3/4+1/4)/2=1/2なので、変換後は(0,5.1)。
    let values = completed_qvalues(&node).unwrap();
    close(values[0], 0.0);
    close(values[1], 5.1);
}

#[test]
fn puct_defaults_match_dlshogi() {
    assert_eq!(
        PuctConfig::default(),
        PuctConfig {
            c_init: 1.44,
            c_base: 28288.0,
            c_fpu: 0.27,
            c_init_root: 1.16,
            c_base_root: 25617.0,
            c_fpu_root: 0.0,
        }
    );
}

// sum=4, sum_win=3.2, c=ln(6), sqrt(sum)=2。
// 根のスコアは(0.8+0.3ln6, 0.8+0.5ln6)=(1.33753,1.69588)。
// 内部のFPU=0.5では第2辺だけ0.5下がり、(1.33753,1.19588)となる。
#[test]
fn puct_fpu_uses_parent_statistics_and_root_fpu_coefficient() {
    let config = PuctConfig {
        c_init: 0.0,
        c_base: 1.0,
        c_fpu: 0.5,
        c_init_root: 0.0,
        c_base_root: 1.0,
        c_fpu_root: 0.0,
    };
    let mut root = statistics_node(&[(0.75, 4, 3.2), (0.25, 0, 0.0)], 0.1, 1.0);
    let mut interior = statistics_node(&[(0.75, 4, 3.2), (0.25, 0, 0.0)], 0.1, 1.0);
    assert_eq!(select_child(&mut root, true, &config).unwrap(), 1);
    assert_eq!(select_child(&mut interior, false, &config).unwrap(), 0);
    close(root.visited_policy, 1.25);
    close(interior.visited_policy, 1.75);
}

// FPU=0.5の同じ統計で、第2辺が勝つ条件はc>2.5。
// 根はln(6)<2.5、内部は2+ln(6)>2.5となる。
#[test]
fn puct_uses_root_init_and_base() {
    let mut config = PuctConfig {
        c_init: 2.0,
        c_base: 1.0,
        c_fpu: 0.5,
        c_init_root: 0.0,
        c_base_root: 1.0,
        c_fpu_root: 0.5,
    };
    let stats = [(0.75, 4, 3.2), (0.25, 0, 0.0)];
    assert_eq!(
        select_child(&mut statistics_node(&stats, 0.1, 1.0), true, &config).unwrap(),
        0
    );
    assert_eq!(
        select_child(&mut statistics_node(&stats, 0.1, 1.0), false, &config).unwrap(),
        1
    );
    // c_initを同じ2としても、根のbase=100ならc=2+ln(1.05)<2.5。
    config.c_init_root = 2.0;
    config.c_base_root = 100.0;
    assert_eq!(
        select_child(&mut statistics_node(&stats, 0.1, 1.0), true, &config).unwrap(),
        0
    );
}

// sum_win=0なら未訪問Qは0であり、節点のネット価値には置き換えない。
// sum=4でスコアは(0.36ln6,0.2ln6)。ネット価値1を使うと第2辺が勝ってしまう。
#[test]
fn puct_zero_sum_win_does_not_use_network_value() {
    let config = PuctConfig {
        c_init: 0.0,
        c_base: 1.0,
        c_fpu: 0.0,
        c_init_root: 0.0,
        c_base_root: 1.0,
        c_fpu_root: 0.0,
    };
    let mut node = statistics_node(&[(0.9, 4, 0.0), (0.1, 0, 0.0)], 1.0, 0.0);
    assert_eq!(select_child(&mut node, true, &config).unwrap(), 0);
}

// visited_policy=1/4ならFPU=0.9×1/2=0.45。
// 内部のスコアは(1.33753,1.24588)となり、平方根を省くと第2辺を誤って選ぶ。
#[test]
fn puct_fpu_takes_square_root_and_clamps_negative_parent_q() {
    let mut config = PuctConfig {
        c_init: 0.0,
        c_base: 1.0,
        c_fpu: 0.9,
        c_init_root: 0.0,
        c_base_root: 1.0,
        c_fpu_root: 0.0,
    };
    let mut node = statistics_node(&[(0.75, 4, 3.2), (0.25, 0, 0.0)], 0.1, 0.25);
    assert_eq!(select_child(&mut node, false, &config).unwrap(), 0);
    // FPU=3では未訪問Qを0へ丸める。スコアは(0.8+0.16ln6,1.2ln6)。
    // 丸めなければ第2辺に-2.2が加わり、第1辺を誤って選ぶ。
    config.c_fpu = 3.0;
    let mut node = statistics_node(&[(0.4, 4, 3.2), (0.6, 0, 0.0)], 0.1, 1.0);
    assert_eq!(select_child(&mut node, false, &config).unwrap(), 1);
}

// 訪問がないときのu=1は、最大方策の手を選べるだけの正の探索項を与える。
#[test]
fn puct_first_visit_uses_policy_and_repeated_selections_add_policy_again() {
    let mut tree = tree(&[0.0, 3.0_f32.ln()], 0.5);
    let config = PuctConfig::default();
    for count in 1..=3 {
        let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
        assert_eq!(leaf.path, [1]);
        close(tree.root().visited_policy, 0.75 * count as f32);
        tree.cancel_leaf(&leaf.path).unwrap();
    }
}

#[test]
fn virtual_loss_spreads_pending_leaves_and_policy_accumulates_on_collision() {
    let config = PuctConfig::default();
    let mut tree = tree(&[0.0, 0.0], 0.5);
    let first = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    assert_eq!(first.path, [0]);
    tree.create_child(&first.path, Child::Pending(&moves(1)))
        .unwrap();
    let second = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    assert_eq!(second.path, [1]);
    tree.create_child(&second.path, Child::Pending(&moves(1)))
        .unwrap();
    let collision = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    assert_eq!(collision.path, [0]);
    assert_eq!(collision.kind, LeafKind::Collision);
    close(tree.root().visited_policy, 1.5);
    for edge in &tree.root().edges {
        assert_eq!(
            (edge.visits, edge.virtual_loss, edge.value_sum),
            (0, 1, 0.0)
        );
    }
    // バッチの評価結果は選択とは逆の順序でも投入できる。
    tree.evaluate(&second.path, &[0.0], 0.75).unwrap();
    tree.evaluate(&first.path, &[0.0], 0.25).unwrap();
    for edge in &tree.root().edges {
        assert_eq!((edge.visits, edge.virtual_loss), (1, 0));
    }
    assert_eq!(tree.root().edges[0].value_sum, 0.75);
    assert_eq!(tree.root().edges[1].value_sum, 0.25);
}

#[test]
fn collision_and_cancellation_restore_virtual_loss_along_the_entire_path() {
    let config = PuctConfig::default();
    let mut tree = tree(&[0.0], 0.5);
    let first = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    tree.create_child(&first.path, Child::Pending(&moves(1)))
        .unwrap();
    tree.evaluate(&first.path, &[0.0], 0.25).unwrap();
    let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    assert_eq!(leaf.path, [0, 0]);
    let before = tree.used_bytes();
    tree.create_child(&leaf.path, Child::Pending(&moves(1)))
        .unwrap();
    let collision = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    assert_eq!(collision.kind, LeafKind::Collision);
    assert_eq!(collision.path, leaf.path);
    assert_eq!(tree.root().edges[0].virtual_loss, 1);
    assert_eq!(tree.node(&[0]).unwrap().edges[0].virtual_loss, 1);
    tree.cancel_leaf(&leaf.path).unwrap();
    assert_eq!(tree.root().edges[0].virtual_loss, 0);
    assert_eq!(tree.node(&[0]).unwrap().edges[0].virtual_loss, 0);
    assert!(tree.node(&[0]).unwrap().edges[0].child.is_none());
    assert_eq!(tree.used_bytes(), before);
    assert_eq!(tree.root().edges[0].value_sum, 0.75);
    assert_eq!(tree.root().edges[0].visits, 1);
    assert_eq!(tree.root().visited_policy, 3.0);
}

#[test]
fn selecting_an_uncreated_edge_twice_is_also_a_collision() {
    let mut tree = tree(&[0.0], 0.5);
    let config = PuctConfig::default();
    let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    assert_eq!(
        tree.select_leaf(SelectionRule::Puct(&config)).unwrap().kind,
        LeafKind::Collision
    );
    assert_eq!(tree.root().edges[0].virtual_loss, 1);
    tree.cancel_leaf(&leaf.path).unwrap();
    assert_eq!(tree.root().edges[0].virtual_loss, 0);
}

// 深さ1,2,3の値を1/4,7/8,3/4とすると、根の増分は3/4,7/8,1/4。
#[test]
fn backup_alternates_win_probability_and_never_counts_root_evaluation() {
    let mut tree = tree(&[0.0], 0.125);
    assert_eq!(
        (tree.root().edges[0].visits, tree.root().edges[0].value_sum),
        (0, 0.0)
    );
    let config = PuctConfig::default();
    for value in [0.25, 0.875, 0.75] {
        let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
        tree.create_child(&leaf.path, Child::Pending(&moves(1)))
            .unwrap();
        tree.evaluate(&leaf.path, &[0.0], value).unwrap();
    }
    assert_eq!(
        (tree.root().edges[0].visits, tree.root().edges[0].value_sum),
        (3, 1.875)
    );
    assert_eq!(
        (
            tree.node(&[0]).unwrap().edges[0].visits,
            tree.node(&[0]).unwrap().edges[0].value_sum
        ),
        (2, 0.875)
    );
    assert_eq!(
        (
            tree.node(&[0, 0]).unwrap().edges[0].visits,
            tree.node(&[0, 0]).unwrap().edges[0].value_sum
        ),
        (1, 0.25)
    );
    for path in [vec![], vec![0], vec![0, 0]] {
        assert_eq!(tree.node(&path).unwrap().edges[0].virtual_loss, 0);
    }
}

#[test]
fn all_children_winning_proves_loss_but_a_missing_child_prevents_it() {
    let mut tree = tree(&[0.0, 0.0], 0.5);
    let config = PuctConfig::default();
    let first = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    tree.create_child(&first.path, Child::Proven(proof(Outcome::Win, 3, false)))
        .unwrap();
    assert_eq!(tree.root().state, NodeState::Expanded);
    let second = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    assert_eq!(second.path, [1]);
    tree.create_child(&second.path, Child::Proven(proof(Outcome::Win, 5, true)))
        .unwrap();
    assert_eq!(
        tree.root().state,
        NodeState::Proven(proof(Outcome::Loss, 6, true))
    );
    assert_eq!(tree.best_move(), Some(moves(2)[1]));
    assert_eq!(tree.principal_variation(), vec![moves(2)[1]]);
    assert_eq!(tree.root().edges[0].virtual_loss, 0);
    assert_eq!(tree.root().edges[1].virtual_loss, 0);
}

// 同じバッチで先に反復勝ちが判明しても、後に判明した詰み勝ちが優先される。
#[test]
fn losing_child_proves_win_and_later_mate_outranks_repetition() {
    let mut tree = tree(&[0.0, 0.0], 0.5);
    let config = PuctConfig::default();
    let first = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    let second = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    tree.create_child(&first.path, Child::Proven(proof(Outcome::Loss, 0, true)))
        .unwrap();
    assert_eq!(
        tree.root().state,
        NodeState::Proven(proof(Outcome::Win, 1, true))
    );
    tree.create_child(
        &second.path,
        Child::Proven(proof(Outcome::Loss, 200, false)),
    )
    .unwrap();
    assert_eq!(
        tree.root().state,
        NodeState::Proven(proof(Outcome::Win, 201, false))
    );
    assert_eq!(tree.best_move(), Some(moves(2)[1]));
    assert_eq!(tree.root().edges[0].value_sum, 1.0);
    assert_eq!(tree.root().edges[1].value_sum, 1.0);
    let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    assert!(leaf.path.is_empty());
    assert_eq!(leaf.kind, LeafKind::Value(1.0));
    tree.backup_leaf(&leaf.path).unwrap();
}

#[test]
fn proof_propagation_flips_outcome_and_adds_distance_at_every_level() {
    let mut tree = tree(&[0.0], 0.5);
    let config = PuctConfig::default();
    let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    tree.create_child(&leaf.path, Child::Pending(&moves(1)))
        .unwrap();
    tree.evaluate(&leaf.path, &[0.0], 0.5).unwrap();
    let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    tree.create_child(&leaf.path, Child::Proven(proof(Outcome::Loss, 4, true)))
        .unwrap();
    assert_eq!(
        tree.node(&[0]).unwrap().state,
        NodeState::Proven(proof(Outcome::Win, 5, true))
    );
    assert_eq!(
        tree.root().state,
        NodeState::Proven(proof(Outcome::Loss, 6, true))
    );
}

// PUCT自体の契約を検証する。子の視点の勝ちは除外し、子の視点の負けは即座に選ぶ。
#[test]
fn puct_skips_parent_losses_and_immediately_selects_parent_wins() {
    let config = PuctConfig::default();
    let mut node = statistics_node(&[(0.8, 4, 4.0), (0.19, 0, 0.0), (0.01, 0, 0.0)], 0.5, 0.0);
    node.edges[0].child = Some(terminal(proof(Outcome::Win, 0, false)));
    node.edges[2].child = Some(terminal(proof(Outcome::Loss, 0, true)));
    assert_eq!(select_child(&mut node, true, &config).unwrap(), 2);
    close(node.visited_policy, 0.01);
    node.edges[2].child = None;
    assert_eq!(select_child(&mut node, true, &config).unwrap(), 1);
    close(node.visited_policy, 0.20);
}

#[test]
fn best_move_uses_logit_without_visits_then_visits_q_and_stable_order() {
    let mut tree = Tree::new(&moves(3), 4096).unwrap();
    assert_eq!(tree.best_move(), None);
    assert!(tree.principal_variation().is_empty());
    tree.evaluate(&[], &[0.0, 2.0, 2.0], 0.5).unwrap();
    assert_eq!(tree.best_move(), Some(moves(3)[1]));
    assert_eq!(tree.principal_variation(), vec![moves(3)[1]]);
    let mut node = statistics_node(&[(0.6, 2, 0.5), (0.3, 2, 1.0), (0.1, 1, 1.0)], 0.5, 0.0);
    assert_eq!(node.best_edge(), Some(1));
    node.edges[0].value_sum = 1.0;
    assert_eq!(node.best_edge(), Some(0));
}

#[test]
fn best_move_orders_proven_results_and_treats_draw_as_unproven() {
    let mut node = statistics_node(
        &[
            (0.25, 1, 0.0),
            (0.25, 9, 9.0),
            (0.25, 2, 1.0),
            (0.25, 0, 0.0),
        ],
        0.5,
        0.0,
    );
    node.edges[0].child = Some(terminal(proof(Outcome::Loss, 0, true)));
    node.edges[1].child = Some(terminal(proof(Outcome::Win, 0, true)));
    node.edges[3].child = Some(terminal(proof(Outcome::Loss, 250, false)));
    assert_eq!(node.best_edge(), Some(3));
    node.edges[3].child = Some(terminal(proof(Outcome::Win, 250, false)));
    assert_eq!(node.best_edge(), Some(0));
    node.edges[0].child = Some(terminal(proof(Outcome::Win, 0, false)));
    assert_eq!(node.best_edge(), Some(2));
    node.edges[2].child = Some(Box::new(Node {
        value: None,
        state: NodeState::Draw(0.1),
        edges: Box::new([]),
        visited_policy: 0.0,
    }));
    assert_eq!(node.best_edge(), Some(2));
    node.edges[2].child = Some(terminal(proof(Outcome::Win, 0, false)));
    assert_eq!(node.best_edge(), Some(1));
    // 同じ詰み負けの帯では、終局を遅らせる手を選ぶ。
    node.edges[1].child = Some(terminal(proof(Outcome::Win, 3, false)));
    assert_eq!(node.best_edge(), Some(3));
}

#[test]
fn principal_variation_requires_interior_visits_and_includes_the_draw_move() {
    let mut tree = tree(&[0.0], 0.5);
    let config = PuctConfig::default();
    let action = moves(1)[0];
    let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    tree.create_child(&leaf.path, Child::Pending(&[action]))
        .unwrap();
    tree.evaluate(&leaf.path, &[0.0], 0.5).unwrap();
    assert_eq!(tree.principal_variation(), [action]);
    let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    tree.create_child(&leaf.path, Child::Draw(0.75)).unwrap();
    assert_eq!(tree.principal_variation(), [action, action]);
    assert_eq!(tree.node(&[0]).unwrap().edges[0].value_sum, 0.25);
    let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    assert_eq!(leaf.kind, LeafKind::Value(0.75));
    tree.backup_leaf(&leaf.path).unwrap();
    assert_eq!(tree.node(&[0]).unwrap().edges[0].value_sum, 0.5);
    assert_eq!(tree.root().state, NodeState::Expanded);
}

#[test]
fn capacity_errors_preserve_the_tree_and_selection_can_be_cancelled() {
    let actions = moves(2);
    let bytes = size_of::<Node>() + 2 * size_of::<Edge>();
    assert!(matches!(
        Tree::new(&actions, bytes - 1),
        Err(TreeError::CapacityExceeded)
    ));
    let mut tree = Tree::new(&actions, bytes).unwrap();
    tree.evaluate(&[], &[0.0, 0.0], 0.5).unwrap();
    let config = PuctConfig::default();
    let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    assert_eq!(
        tree.create_child(&leaf.path, Child::Pending(&actions)),
        Err(TreeError::CapacityExceeded)
    );
    assert_eq!(tree.used_bytes(), bytes);
    assert_eq!(tree.root().edges[0].visits, 0);
    assert!(tree.root().edges[0].child.is_none());
    tree.cancel_leaf(&leaf.path).unwrap();
    assert_eq!(tree.root().edges[0].virtual_loss, 0);
    assert_eq!(tree.best_move(), Some(actions[0]));
}

#[test]
fn reroot_discards_siblings_counts_exact_bytes_and_preserves_local_proof_distance() {
    let actions = moves(2);
    let mut tree = tree(&[0.0, 0.0], 0.5);
    let config = PuctConfig::default();
    let first = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    let second = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    tree.create_child(&first.path, Child::Pending(&actions))
        .unwrap();
    tree.create_child(&second.path, Child::Pending(&actions[..1]))
        .unwrap();
    assert_eq!(
        tree.reroot(&actions[..1]),
        Err(TreeError::PendingSimulation)
    );
    tree.evaluate(&first.path, &[0.0, 0.0], 0.0).unwrap();
    tree.evaluate(&second.path, &[0.0], 1.0).unwrap();
    // 根から見たQの高い第1辺の下へ進み、子から見た負けの辺だけを確定させる。
    let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    assert_eq!(leaf.path, [0, 0]);
    tree.create_child(&leaf.path, Child::Proven(proof(Outcome::Win, 7, true)))
        .unwrap();
    let node_bytes = size_of::<Node>();
    let edge_bytes = size_of::<Edge>();
    assert_eq!(tree.used_bytes(), 4 * node_bytes + 5 * edge_bytes);
    let before = tree.used_bytes();
    assert_eq!(
        tree.reroot(&[actions[1], actions[0]]),
        Err(TreeError::InvalidPath)
    );
    assert_eq!(tree.used_bytes(), before);
    tree.reroot(&actions[..1]).unwrap();
    assert_eq!(tree.used_bytes(), 2 * node_bytes + 2 * edge_bytes);
    assert_eq!(
        tree.node(&[0]).unwrap().state,
        NodeState::Proven(proof(Outcome::Win, 7, true))
    );
    assert_eq!(tree.root().value, Some(0.0));
    assert_eq!(tree.reroot(&actions[..1]), Err(TreeError::InvalidState));
    assert_eq!(tree.used_bytes(), 2 * node_bytes + 2 * edge_bytes);
    // 確定した負けの辺は付け替え後も除外される。
    let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    assert_eq!(leaf.path, [1]);
    tree.cancel_leaf(&leaf.path).unwrap();
}

#[test]
fn depth_limit_leaf_is_evaluated_once_and_remains_unexpanded_after_reroot() {
    let action = moves(1)[0];
    let mut tree = tree(&[0.0], 0.5);
    let config = PuctConfig::default();
    for depth in 1..=MAX_PLY {
        let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
        assert_eq!(leaf.path.len(), depth as usize);
        if depth == MAX_PLY {
            assert_eq!(
                tree.create_child(&leaf.path, Child::Pending(&[action])),
                Err(TreeError::DepthLimit)
            );
            assert_eq!(
                tree.create_child(&leaf.path, Child::Proven(proof(Outcome::Win, 1, false))),
                Err(TreeError::DepthLimit)
            );
            tree.create_child(&leaf.path, Child::Unexpanded).unwrap();
            tree.evaluate(&leaf.path, &[], 0.75).unwrap();
        } else {
            tree.create_child(&leaf.path, Child::Pending(&[action]))
                .unwrap();
            tree.evaluate(&leaf.path, &[0.0], 0.5).unwrap();
        }
    }
    let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    assert_eq!(leaf.path.len(), MAX_PLY as usize);
    assert_eq!(leaf.kind, LeafKind::Value(0.75));
    tree.backup_leaf(&leaf.path).unwrap();
    assert_eq!(tree.root().edges[0].visits, MAX_PLY + 1);
    tree.reroot(&vec![action; MAX_PLY as usize]).unwrap();
    assert_eq!(tree.used_bytes(), size_of::<Node>());
    assert_eq!(tree.root().state, NodeState::Unexpanded);
    let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    assert!(leaf.path.is_empty());
    assert_eq!(leaf.kind, LeafKind::Value(0.75));
    tree.backup_leaf(&leaf.path).unwrap();
}

#[test]
fn gumbel_enforces_one_pending_leaf_and_uses_no_virtual_loss() {
    let mut tree = tree(&[0.0, 0.0], 0.5);
    let config = Gumbel::new(vec![0.0, 1.0], 1).unwrap();
    assert_eq!(config.num_considered(), 1);
    let leaf = tree.select_leaf(SelectionRule::Gumbel(&config)).unwrap();
    assert_eq!(leaf.path, [1]);
    assert_eq!(tree.root().edges[1].virtual_loss, 0);
    assert_eq!(
        tree.select_leaf(SelectionRule::Gumbel(&config)),
        Err(TreeError::PendingSimulation)
    );
    assert_eq!(
        tree.select_leaf(SelectionRule::Puct(&PuctConfig::default())),
        Err(TreeError::PendingSimulation)
    );
    tree.create_child(&leaf.path, Child::Pending(&moves(1)))
        .unwrap();
    assert_eq!(
        tree.select_leaf(SelectionRule::Gumbel(&config)),
        Err(TreeError::PendingSimulation)
    );
    tree.evaluate(&leaf.path, &[0.0], 0.25).unwrap();
    assert_eq!(tree.root().edges[1].value_sum, 0.75);
    assert_eq!(tree.root().edges[1].virtual_loss, 0);
    assert_eq!(tree.root().visited_policy, 0.0);
    assert_eq!(config.action(tree.root()).unwrap(), 1);
}

#[test]
fn gumbel_cancellation_releases_the_pending_slot_and_budget() {
    let mut tree = tree(&[0.0], 0.5);
    let config = Gumbel::new(vec![0.0], 2).unwrap();
    let before = tree.used_bytes();
    let leaf = tree.select_leaf(SelectionRule::Gumbel(&config)).unwrap();
    tree.create_child(&leaf.path, Child::Pending(&moves(1)))
        .unwrap();
    tree.cancel_leaf(&leaf.path).unwrap();
    assert_eq!(tree.used_bytes(), before);
    let leaf = tree.select_leaf(SelectionRule::Gumbel(&config)).unwrap();
    tree.create_child(&leaf.path, Child::Draw(0.5)).unwrap();
    let leaf = tree.select_leaf(SelectionRule::Gumbel(&config)).unwrap();
    assert_eq!(leaf.kind, LeafKind::Value(0.5));
    tree.backup_leaf(&leaf.path).unwrap();
    assert_eq!(tree.root().edges[0].visits, 2);
    assert_eq!(policy_target(tree.root()).unwrap(), vec![1.0]);
    assert_eq!(
        tree.select_leaf(SelectionRule::Gumbel(&config)),
        Err(TreeError::BudgetExhausted)
    );
}

#[test]
fn invalid_evaluations_preserve_pending_state_and_can_be_retried() {
    let actions = moves(2);
    let mut tree = Tree::new(&actions, 4096).unwrap();
    let config = PuctConfig::default();
    assert_eq!(
        tree.select_leaf(SelectionRule::Puct(&config)).unwrap().kind,
        LeafKind::Collision
    );
    for (logits, value) in [
        (vec![0.0], 0.5),
        (vec![0.0, f32::NAN], 0.5),
        (vec![f32::INFINITY, 0.0], 0.5),
        (vec![0.0, 0.0], f32::NAN),
        (vec![0.0, 0.0], -0.1),
        (vec![0.0, 0.0], 1.1),
    ] {
        assert_eq!(
            tree.evaluate(&[], &logits, value),
            Err(TreeError::InvalidEvaluation)
        );
        assert!(matches!(tree.root().state, NodeState::Pending { .. }));
        assert_eq!(tree.root().value, None);
    }
    tree.evaluate(&[], &[1000.0, 1000.0], 0.5).unwrap();
    assert_eq!(tree.root().edges[0].policy, 0.5);
    let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    tree.create_child(&leaf.path, Child::Pending(&actions))
        .unwrap();
    assert_eq!(
        tree.evaluate(&leaf.path, &[0.0], 0.5),
        Err(TreeError::InvalidEvaluation)
    );
    assert_eq!(tree.root().edges[0].virtual_loss, 1);
    tree.evaluate(&leaf.path, &[0.0, 0.0], 0.0).unwrap();
    assert_eq!(tree.root().edges[0].virtual_loss, 0);
    assert_eq!(
        tree.evaluate(&leaf.path, &[0.0, 0.0], 0.0),
        Err(TreeError::InvalidSelection)
    );
    assert_eq!(tree.root().edges[0].visits, 1);
}

#[test]
fn invalid_configuration_and_child_requests_do_not_mutate_the_tree() {
    assert!(matches!(Tree::new(&[], 4096), Err(TreeError::EmptyMoves)));
    assert!(Gumbel::new(vec![], 1).is_err());
    assert!(Gumbel::new(vec![0.0], 0).is_err());
    assert!(Gumbel::new(vec![f32::INFINITY], 1).is_err());
    assert!(considered_visits(0, 1).is_err());
    assert!(considered_visits(2, 0).unwrap().is_empty());
    let mut tree = tree(&[0.0], 0.5);
    let config = PuctConfig {
        c_base_root: 0.0,
        ..PuctConfig::default()
    };
    assert_eq!(
        tree.select_leaf(SelectionRule::Puct(&config)),
        Err(TreeError::InvalidPuctConfig)
    );
    let gumbel = Gumbel::new(vec![0.0, 0.0], 1).unwrap();
    assert_eq!(
        tree.select_leaf(SelectionRule::Gumbel(&gumbel)),
        Err(TreeError::InvalidGumbelConfig)
    );
    let leaf = tree
        .select_leaf(SelectionRule::Puct(&PuctConfig::default()))
        .unwrap();
    let bytes = tree.used_bytes();
    assert_eq!(
        tree.create_child(&leaf.path, Child::Pending(&[])),
        Err(TreeError::EmptyMoves)
    );
    assert_eq!(
        tree.create_child(&leaf.path, Child::Draw(f32::NAN)),
        Err(TreeError::InvalidEvaluation)
    );
    assert_eq!(
        tree.create_child(&leaf.path, Child::Unexpanded),
        Err(TreeError::DepthLimit)
    );
    assert_eq!(
        tree.create_child(&[7], Child::Draw(0.5)),
        Err(TreeError::InvalidSelection)
    );
    assert_eq!(tree.used_bytes(), bytes);
    tree.cancel_leaf(&leaf.path).unwrap();
}

// 確定した勝敗も、根から終端までの選択済みの手順を主変化として返す。
#[test]
fn principal_variation_follows_proven_interior_nodes_to_the_terminal() {
    let actions = moves(1);
    let mut tree = tree(&[0.0], 0.5);
    let config = PuctConfig::default();
    for _ in 0..2 {
        let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
        tree.create_child(&leaf.path, Child::Pending(&actions))
            .unwrap();
        tree.evaluate(&leaf.path, &[0.0], 0.5).unwrap();
    }
    let leaf = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    tree.create_child(&leaf.path, Child::Proven(proof(Outcome::Loss, 0, false)))
        .unwrap();
    assert_eq!(
        tree.root().state,
        NodeState::Proven(proof(Outcome::Win, 3, false))
    );
    assert_eq!(tree.principal_variation(), vec![actions[0]; 3]);
}
