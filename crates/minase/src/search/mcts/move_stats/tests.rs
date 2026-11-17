use super::*;
use crate::search::mcts::puct::PuctConfig;
use crate::search::mcts::tree::{Child, Edge, Node, Proof, SelectionRule};
use minase_core::{Game, Move, Rules};

fn moves() -> Vec<Move> {
    Game::from_position(Rules::JCSA, Position::initial()).legal_moves()
}

fn node(state: NodeState, edges: Vec<Edge>) -> Node {
    Node {
        value: None,
        state,
        edges: edges.into_boxed_slice(),
        visited_policy: 0.0,
    }
}

fn proof(outcome: Outcome, distance: u32, repetition: bool, approximate: bool) -> Proof {
    Proof {
        outcome,
        distance,
        repetition,
        approximate,
    }
}

#[test]
fn ranked_edges_follow_proofs_visits_q_logits_and_stable_ties() {
    let mv = moves()[0];
    let mut edges: Vec<_> = (0..11)
        .map(|_| Edge {
            mv,
            logit: 0.0,
            policy: 0.0,
            visits: 0,
            virtual_loss: 0,
            value_sum: 0.0,
            child: None,
        })
        .collect();
    for (index, outcome, distance) in [
        (0, Outcome::Win, 1),
        (3, Outcome::Loss, 4),
        (5, Outcome::Win, 6),
        (7, Outcome::Loss, 1),
    ] {
        edges[index].child = Some(Box::new(node(
            NodeState::Proven(proof(outcome, distance, false, false)),
            vec![],
        )));
    }
    for (index, visits, value_sum) in [(1, 2, 0.5), (4, 2, 1.5), (6, 3, 0.0), (8, 2, 1.5)] {
        edges[index].visits = visits;
        edges[index].value_sum = value_sum;
    }
    edges[2].logit = 1.0;
    edges[9].logit = 2.0;
    edges[10].logit = 2.0;
    let mut root = node(NodeState::Expanded, edges);
    // 親の勝ち7,3は短い順。未確定6,4,8,1は訪問数とQの順。
    // 未訪問9,10,2はlogit順、同値は元の順。親の負け5,0は長い順。
    let expected = [7, 3, 6, 4, 8, 1, 9, 10, 2, 5, 0];
    assert_eq!(root.ranked_edges(), expected);
    assert_eq!(root.best_edge(), Some(expected[0]));
    assert_eq!(root.edges[2].logit, 1.0); // 辺の配列は変更しない。
    root.state = NodeState::Pending { unexpanded: false };
    assert!(root.ranked_edges().is_empty());
    assert_eq!(root.best_edge(), None);
}

fn evaluated_tree() -> Tree {
    let mut tree = Tree::new(&moves()[..3], 1_000_000).unwrap();
    tree.evaluate(&[], &[0.0; 3], 0.5).unwrap();
    tree
}

#[test]
fn statistics_follow_backup_and_reused_visit_identity() {
    let mut tree = evaluated_tree();
    let config = PuctConfig::default();
    let selection = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    let index = selection.path[0];
    tree.create_child(&selection.path, Child::Pending(&moves()[..1]))
        .unwrap();
    tree.evaluate(&selection.path, &[0.0], 0.25).unwrap();
    // 子の初回評価0.25を親では0.75として1回逆伝播する。
    let output = lines(
        &Position::initial(),
        &tree,
        2,
        Duration::from_micros(12345),
        None,
    );
    assert_eq!(
        output[0],
        format!(
            "movestats root simulations 2 reused 0 visits 1 value 0.5 tree-bytes {} capacity-bytes 1000000 elapsed-ms 12 soft-ms - hard-ms - result none",
            tree.used_bytes()
        )
    );
    assert_eq!(
        output[1],
        format!(
            "movestats move {} visits 1 value-sum 0.75 q 0.75 prior {} value 0.75 result none",
            usi::text_generated(&Position::initial(), tree.root().edges[index].mv),
            tree.root().edges[index].policy
        )
    );
    for line in &output[2..] {
        assert!(line.contains("visits 0 value-sum 0 q -"));
        assert!(line.ends_with("value - result none"));
    }
    assert_eq!(output.len(), tree.root().edges.len() + 1);
    for (line, index) in output[1..].iter().zip(tree.root().ranked_edges()) {
        assert_eq!(
            line.split_whitespace().nth(2).unwrap(),
            usi::text_generated(&Position::initial(), tree.root().edges[index].mv)
        );
    }
    let budget = (Duration::from_micros(23567), Duration::from_micros(45678));
    // 根を再評価しただけなら今回S=1、以前の辺の訪問1回を持ち越す。
    let reused = lines(&Position::initial(), &tree, 1, Duration::ZERO, Some(budget));
    assert!(reused[0].contains("simulations 1 reused 1 visits 1"));
    assert!(reused[0].contains("soft-ms 23 hard-ms 45"));
}

#[test]
fn results_distinguish_proofs_draws_and_approximation_without_inventing_values() {
    for (child, expected, root_result) in [
        (
            Child::Proven(proof(Outcome::Loss, 2, false, false)),
            "win distance 3",
            "win distance 3",
        ),
        (
            Child::Proven(proof(Outcome::Loss, 2, true, false)),
            "win distance 3 repetition",
            "win distance 3 repetition",
        ),
        (
            Child::Proven(proof(Outcome::Loss, 2, false, true)),
            "win distance 3 approximate",
            "win distance 3 approximate",
        ),
        (
            Child::Proven(proof(Outcome::Win, 2, true, true)),
            "loss distance 3 repetition approximate",
            "none",
        ),
        (
            Child::Draw {
                value: 0.5,
                approximate: false,
            },
            "draw",
            "none",
        ),
        (
            Child::Draw {
                value: 0.5,
                approximate: true,
            },
            "draw approximate",
            "none",
        ),
    ] {
        let mut tree = evaluated_tree();
        let selection = tree
            .select_leaf(SelectionRule::Puct(&PuctConfig::default()))
            .unwrap();
        let index = selection.path[0];
        tree.create_child(&selection.path, child).unwrap();
        let output = lines(&Position::initial(), &tree, 2, Duration::ZERO, None);
        let prefix = format!(
            "movestats move {} ",
            usi::text_generated(&Position::initial(), tree.root().edges[index].mv)
        );
        let row = output
            .iter()
            .find(|line| line.starts_with(&prefix))
            .unwrap();
        assert!(
            row.ends_with(&format!("value - result {expected}")),
            "{row}"
        );
        assert!(
            output[0].ends_with(&format!("result {root_result}")),
            "{}",
            output[0]
        );
    }
    // 根の引き分けも、子と同じ書式を使う。
    assert_eq!(
        result(NodeState::Draw {
            value: 0.5,
            approximate: false
        }),
        "draw"
    );
    assert_eq!(
        result(NodeState::Draw {
            value: 0.5,
            approximate: true
        }),
        "draw approximate"
    );
}

#[test]
#[should_panic(expected = "S=0, R=0")]
fn zero_simulations_violate_root_accounting() {
    lines(
        &Position::initial(),
        &evaluated_tree(),
        0,
        Duration::ZERO,
        None,
    );
}

#[test]
#[should_panic(expected = "S=2, R=0")]
fn negative_reused_visits_violate_root_accounting() {
    lines(
        &Position::initial(),
        &evaluated_tree(),
        2,
        Duration::ZERO,
        None,
    );
}

#[test]
fn repeated_backups_sum_parent_values_and_keep_the_child_network_value() {
    let choices = moves();
    let mut tree = Tree::new(&choices[..1], 1_000_000).unwrap();
    tree.evaluate(&[], &[0.0], 0.5).unwrap();
    let config = PuctConfig::default();
    let first = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    tree.create_child(&first.path, Child::Pending(&choices[..1]))
        .unwrap();
    tree.evaluate(&first.path, &[0.0], 0.25).unwrap();
    let second = tree.select_leaf(SelectionRule::Puct(&config)).unwrap();
    tree.create_child(&second.path, Child::Pending(&choices[..1]))
        .unwrap();
    tree.evaluate(&second.path, &[0.0], 0.125).unwrap();
    // 深さ1の0.25は親で0.75、深さ2の0.125は2回反転して0.125となる。
    // 和は0.875、2回の平均は0.4375。子自身のネットの価値は再評価しない。
    let output = lines(&Position::initial(), &tree, 3, Duration::ZERO, None);
    assert!(output[1].contains("visits 2 value-sum 0.875 q 0.4375"));
    assert!(output[1].ends_with("value 0.75 result none"));
}

#[test]
fn statistics_preserve_f64_complements_without_decimal_rounding() {
    let mut tree = evaluated_tree();
    let selection = tree
        .select_leaf(SelectionRule::Puct(&PuctConfig::default()))
        .unwrap();
    tree.create_child(&selection.path, Child::Pending(&moves()[..1]))
        .unwrap();
    let child_value = 1.0e-10_f32;
    tree.evaluate(&selection.path, &[0.0], child_value).unwrap();
    let output = lines(&Position::initial(), &tree, 2, Duration::ZERO, None);
    let words: Vec<_> = output[1].split_whitespace().collect();
    let expected = 1.0 - f64::from(child_value);
    assert_ne!(expected, f64::from(1.0_f32 - child_value));
    for key in ["value-sum", "value"] {
        let index = words.iter().position(|word| *word == key).unwrap();
        assert_eq!(words[index + 1].parse::<f64>().unwrap(), expected);
    }
}
