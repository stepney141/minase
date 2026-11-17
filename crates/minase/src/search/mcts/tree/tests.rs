//! 設計書の10条件と、選択・逆伝播・付け替えの契約を検証する。
use super::*;
use crate::search::mcts::test_support::legal_moves;
use minase_core::Position;

fn moves() -> Vec<Move> {
    legal_moves(&Position::initial())[..2].to_vec()
}

fn tree() -> Tree {
    let mut tree = Tree::new(&moves(), 1 << 20).unwrap();
    tree.evaluate(&[], &[0.0; 2], 0.5).unwrap();
    tree
}

fn select(tree: &mut Tree) -> Selection {
    tree.select_leaf(SelectionRule::Puct(&PuctConfig::default()))
        .unwrap()
}

fn proof(outcome: Outcome) -> Proof {
    Proof {
        outcome,
        distance: 0,
        repetition: false,
        approximate: false,
    }
}

fn expanded_child() -> (Tree, Vec<usize>) {
    let mut tree = tree();
    let path = select(&mut tree).path;
    tree.create_child(&path, Child::Pending(&moves())).unwrap();
    tree.evaluate(&path, &[0.0; 2], 0.25).unwrap();
    assert_eq!(tree.validate(), Ok(()));
    (tree, path)
}

fn draw_child() -> (Tree, Vec<usize>) {
    let mut tree = tree();
    let path = select(&mut tree).path;
    tree.create_child(
        &path,
        Child::Draw {
            value: 0.25,
            approximate: false,
        },
    )
    .unwrap();
    assert_eq!(tree.validate(), Ok(()));
    (tree, path)
}

#[test]
fn accepts_multiple_pending_collision_and_cancellation() {
    let mut tree = tree();
    let first = select(&mut tree);
    tree.create_child(&first.path, Child::Pending(&moves()))
        .unwrap();
    let second = select(&mut tree);
    assert_ne!(first.path, second.path);
    tree.create_child(&second.path, Child::Pending(&moves()))
        .unwrap();
    assert_eq!(tree.validate(), Ok(()));
    assert_eq!(select(&mut tree).kind, LeafKind::Collision);
    assert_eq!(tree.validate(), Ok(()));
    tree.cancel_leaf(&first.path).unwrap();
    assert_eq!(tree.validate(), Ok(()));
    tree.cancel_leaf(&second.path).unwrap();
    assert_eq!(tree.validate(), Ok(()));
}

#[test]
fn accepts_proven_draw_and_fixed_selection() {
    let mut tree = tree();
    let first = select(&mut tree);
    tree.create_child(&first.path, Child::Proven(proof(Outcome::Win)))
        .unwrap();
    assert_eq!(tree.validate(), Ok(()));
    let second = select(&mut tree);
    tree.create_child(
        &second.path,
        Child::Draw {
            value: 0.25,
            approximate: false,
        },
    )
    .unwrap();
    assert_eq!(tree.validate(), Ok(()));
    let fixed = select(&mut tree);
    assert_eq!(fixed.kind, LeafKind::Value(0.25));
    assert_eq!(tree.validate(), Ok(()));
    tree.backup_leaf(&fixed.path).unwrap();
    assert_eq!(tree.validate(), Ok(()));
}

#[test]
fn accepts_capacity_exhaustion_with_pending_selection() {
    let mut tree = Tree::new(&moves(), size_of::<Node>() + 2 * size_of::<Edge>()).unwrap();
    tree.evaluate(&[], &[0.0; 2], 0.5).unwrap();
    let selection = select(&mut tree);
    assert_eq!(
        tree.create_child(&selection.path, Child::Unexpanded),
        Err(TreeError::DepthLimit)
    );
    assert_eq!(
        tree.create_child(&selection.path, Child::Pending(&moves())),
        Err(TreeError::CapacityExceeded)
    );
    assert_eq!(tree.validate(), Ok(()));
}

#[test]
fn accepts_depth_leaf_reroot_and_empty_selection() {
    let mut tree = tree();
    tree.depth_limit = 1;
    let path = select(&mut tree).path;
    tree.create_child(&path, Child::Unexpanded).unwrap();
    tree.evaluate(&path, &[], 0.25).unwrap();
    assert_eq!(tree.validate(), Ok(()));
    let mv = tree.root.edges[path[0]].mv;
    tree.reroot(&[mv]).unwrap();
    assert_eq!(tree.validate(), Ok(()));
    let selection = select(&mut tree);
    assert!(selection.path.is_empty());
    assert_eq!(selection.kind, LeafKind::Value(0.25));
    assert_eq!(tree.validate(), Ok(()));
}

#[test]
fn accepts_expanded_reroot() {
    let (mut tree, path) = expanded_child();
    let mv = tree.root.edges[path[0]].mv;
    assert_eq!(tree.validate(), Ok(()));
    tree.reroot(&[mv]).unwrap();
    assert_eq!(tree.validate(), Ok(()));
}

fn proven_parent() -> Tree {
    let mut tree = tree();
    for approximate in [false, true] {
        let path = select(&mut tree).path;
        tree.create_child(
            &path,
            Child::Proven(Proof {
                approximate,
                ..proof(Outcome::Win)
            }),
        )
        .unwrap();
    }
    assert_eq!(
        tree.root.state,
        NodeState::Proven(Proof {
            outcome: Outcome::Loss,
            distance: 1,
            repetition: false,
            approximate: true
        })
    );
    assert_eq!(tree.validate(), Ok(()));
    tree
}

#[test]
fn accepts_proof_propagation_with_approximate_child() {
    proven_parent();
    let mut tree = tree();
    let path = select(&mut tree).path;
    tree.create_child(&path, Child::Proven(proof(Outcome::Loss)))
        .unwrap();
    assert_eq!(tree.validate(), Ok(()));
}

#[test]
fn accepts_gumbel_pending_selection() {
    let mut tree = tree();
    let rule = Gumbel::new(vec![0.0; 2], 4, 1.0).unwrap();
    let path = tree.select_leaf(SelectionRule::Gumbel(&rule)).unwrap().path;
    assert_eq!(tree.validate(), Ok(()));
    tree.create_child(&path, Child::Pending(&moves())).unwrap();
    assert_eq!(tree.validate(), Ok(()));
}

#[test]
fn accepts_rounding_after_one_thousand_fixed_leaf_visits() {
    let mut tree = Tree::new(&moves()[..1], 1 << 20).unwrap();
    tree.evaluate(&[], &[0.0], 0.5).unwrap();
    let path = select(&mut tree).path;
    tree.create_child(
        &path,
        Child::Draw {
            value: 1e-10,
            approximate: false,
        },
    )
    .unwrap();
    for _ in 1..1000 {
        let path = select(&mut tree).path;
        tree.backup_leaf(&path).unwrap();
    }
    assert_eq!(tree.root.edges[0].visits, 1000);
    assert_ne!(
        tree.root.edges[0].value_sum,
        1000.0 * (1.0 - f64::from(1e-10_f32))
    );
    assert_eq!(tree.validate(), Ok(()));
}

#[test]
fn rejects_condition1_edge_statistics() {
    let mut tree = tree();
    tree.root.edges[0].value_sum = f64::NAN;
    assert!(matches!(
        tree.validate(),
        Err(TreeViolation::EdgeStatistics { .. })
    ));
}

#[test]
fn rejects_condition2_conservation() {
    let (mut tree, path) = expanded_child();
    // 根の未訪問の兄弟でなく、評価済みの子を選ぶ方策を与える。
    let mut logits = [-100.0; 2];
    logits[path[0]] = 100.0;
    tree.evaluate(&[], &logits, 0.5).unwrap();
    let grandchild = select(&mut tree).path;
    assert_eq!(grandchild.len(), 2);
    tree.create_child(&grandchild, Child::Proven(proof(Outcome::Loss)))
        .unwrap();
    assert_eq!(tree.validate(), Ok(()));
    // 子の確定後は条件3と4の対象外。N=2、A=B=1なのでWは1以下でなければならない。
    // W=1.5は条件1の0からNには収まるが、条件2だけを破る。
    tree.root.edges[path[0]].value_sum = 1.5;
    assert!(matches!(
        tree.validate(),
        Err(TreeViolation::Conservation { .. })
    ));
}

#[test]
fn rejects_condition3_expanded_statistics() {
    let (mut tree, path) = expanded_child();
    tree.root.edges[path[0]].visits = 2;
    assert!(matches!(
        tree.validate(),
        Err(TreeViolation::ExpandedStatistics { .. })
    ));
}

#[test]
fn rejects_condition4_fixed_statistics() {
    let (mut tree, path) = draw_child();
    tree.root.edges[path[0]].value_sum = 0.5;
    assert!(matches!(
        tree.validate(),
        Err(TreeViolation::FixedStatistics { .. })
    ));
}

#[test]
fn rejects_condition5_virtual_loss() {
    let mut tree = tree();
    tree.root.edges[0].virtual_loss = 1;
    assert!(matches!(
        tree.validate(),
        Err(TreeViolation::VirtualLoss { .. })
    ));
}

#[test]
fn rejects_condition6_invalid_endpoint() {
    let mut tree = tree();
    let rule = Gumbel::new(vec![0.0; 2], 4, 1.0).unwrap();
    tree.select_leaf(SelectionRule::Gumbel(&rule)).unwrap();
    tree.pending[0].path = Vec::new();
    assert!(matches!(
        tree.validate(),
        Err(TreeViolation::PendingPath { .. })
    ));
}

#[test]
fn rejects_condition6_duplicate_paths() {
    let mut tree = tree();
    let path = select(&mut tree).path;
    tree.pending.push(PendingSimulation {
        path,
        virtual_loss: true,
    });
    assert!(matches!(
        tree.validate(),
        Err(TreeViolation::PendingPath { .. })
    ));
}

#[test]
fn rejects_condition6_mixed_selection_rules() {
    let mut tree = tree();
    select(&mut tree);
    select(&mut tree);
    tree.pending[0].virtual_loss = false;
    assert!(matches!(
        tree.validate(),
        Err(TreeViolation::PendingPath { .. })
    ));
}

#[test]
fn rejects_condition7_memory() {
    let mut tree = tree();
    tree.used_bytes += 1;
    assert!(matches!(tree.validate(), Err(TreeViolation::Memory { .. })));
}

#[test]
fn rejects_condition8_evaluation() {
    let mut tree = tree();
    tree.root.value = None;
    assert!(matches!(
        tree.validate(),
        Err(TreeViolation::Evaluation { .. })
    ));
}

#[test]
fn rejects_condition9_proof() {
    let mut tree = proven_parent();
    if let NodeState::Proven(ref mut proof) = tree.root.state {
        proof.approximate = false;
    }
    assert!(matches!(tree.validate(), Err(TreeViolation::Proof { .. })));
}

#[test]
fn rejects_condition10_depth() {
    let (mut tree, _) = expanded_child();
    tree.depth_limit = 0;
    assert!(matches!(tree.validate(), Err(TreeViolation::Depth { .. })));
}

#[test]
fn rejects_condition10_proof_distance() {
    let mut tree = proven_parent();
    tree.root.state = NodeState::Proven(Proof {
        distance: MAX_PLY + 1,
        ..proof(Outcome::Loss)
    });
    assert!(matches!(tree.validate(), Err(TreeViolation::Depth { .. })));
}

#[test]
fn value_sum_tolerance_accepts_inside_and_rejects_outside() {
    // 条件4のN=1。初回逆伝播は親側の1 - 1/4 = 3/4。
    let width = (1.0 + f64::from(MAX_PLY)) * 2.0_f64.powi(-52);
    for (factor, accepted) in [(0.5, true), (2.0, false)] {
        let (mut tree, path) = draw_child();
        tree.root.edges[path[0]].value_sum = 0.75 + factor * width;
        if accepted {
            assert_eq!(tree.validate(), Ok(()));
        } else {
            assert!(matches!(
                tree.validate(),
                Err(TreeViolation::FixedStatistics { .. })
            ));
        }
    }
}

#[test]
fn policy_sum_tolerance_accepts_inside_and_rejects_outside() {
    // 条件8のn=2。各方策は0から1を保ち、和だけを変える。
    let width = (2.0 + 2.0) * 2.0_f64.powi(-23);
    for (factor, accepted) in [(0.5, true), (2.0, false)] {
        let mut tree = tree();
        tree.root.edges[0].policy = (0.5 + factor * width) as f32;
        if accepted {
            assert_eq!(tree.validate(), Ok(()));
        } else {
            assert!(matches!(
                tree.validate(),
                Err(TreeViolation::Evaluation { .. })
            ));
        }
    }
}

#[test]
#[should_panic(expected = "virtual loss underflow")]
fn cancellation_checks_virtual_loss_before_subtraction() {
    let mut tree = tree();
    let path = select(&mut tree).path;
    tree.root.edges[path[0]].virtual_loss = 0;
    tree.cancel_leaf(&path).unwrap();
}

#[test]
#[should_panic(expected = "virtual loss underflow")]
fn backup_checks_virtual_loss_before_subtraction() {
    let mut tree = tree();
    let path = select(&mut tree).path;
    tree.root.edges[path[0]].virtual_loss = 0;
    tree.create_child(
        &path,
        Child::Draw {
            value: 0.5,
            approximate: false,
        },
    )
    .unwrap();
}

#[test]
#[should_panic(expected = "MCTS condition 1")]
fn reroot_checks_discarded_edges_before_detaching() {
    let (mut tree, path) = expanded_child();
    let mv = tree.root.edges[path[0]].mv;
    tree.root.edges[1 - path[0]].value_sum = f64::NAN;
    tree.reroot(&[mv]).unwrap();
}

impl Tree {
    pub(in crate::search::mcts) fn corrupt_unvisited_edge_for_test(&mut self) {
        // 根の再評価はlogitと方策だけを更新する。nodes 1ならこの統計は訪問されない。
        self.root.edges[0].value_sum = f64::NAN;
    }
}

mod multipv;
