//! multipv.mdの検証11〜12。順位と値は仕様から固定する。
use super::*;
use crate::search::mcts::engine::pv_lines;

fn fixture(count: usize) -> Tree {
    let moves = legal_moves(&Position::initial());
    let mut tree = Tree::new(&moves[..count], 1 << 20).unwrap();
    tree.evaluate(&[], &vec![0.0; count], 0.75).unwrap();
    tree
}

fn proven_child(outcome: Outcome, distance: u32, repetition: bool) -> Box<Node> {
    Box::new(Node::new(
        &[],
        NodeState::Proven(Proof {
            outcome,
            distance,
            repetition,
            approximate: false,
        }),
    ))
}

#[test]
fn multipv_ranks_proofs_visits_q_logits_and_stable_ties_without_mutating_tree() {
    let mut tree = fixture(12);
    let moves: Vec<_> = tree.root.edges.iter().map(|edge| edge.mv).collect();
    for (index, outcome, distance) in [
        (0, Outcome::Win, 1),
        (3, Outcome::Loss, 4),
        (5, Outcome::Win, 6),
        (7, Outcome::Loss, 1),
    ] {
        tree.root.edges[index].child = Some(proven_child(outcome, distance, false));
    }
    for (index, visits, value_sum) in [(1, 1, 1.0), (4, 2, 1.5), (6, 3, 0.0), (8, 2, 0.5)] {
        let edge = &mut tree.root.edges[index];
        edge.visits = visits;
        edge.value_sum = value_sum;
    }
    for (index, logit) in [(2, 1.0), (9, 2.0), (10, 2.0), (11, -1.0)] {
        tree.root.edges[index].logit = logit;
    }
    tree.root.state = NodeState::Proven(Proof {
        outcome: Outcome::Win,
        distance: 2,
        repetition: false,
        approximate: false,
    });
    let before = format!("{tree:?}");
    let lines = pv_lines(&tree, 12, MATE - 2, 100.0);
    // 親の勝ち7,3は短い順、未確定6,4,8,1は訪問数とQの順。
    // 未訪問9,10,2,11はlogit順と元の順、親の負け5,0は長い順。
    let expected = [7, 3, 6, 4, 8, 1, 9, 10, 2, 11, 5, 0];
    let scores = [
        Some(MATE - 2),
        Some(MATE - 5),
        Some(-691),
        Some(110),
        Some(-110),
        Some(691),
        None,
        None,
        None,
        None,
        Some(-MATE + 7),
        Some(-MATE + 2),
    ];
    // Q=0,1は0.001,0.999に丸めて±round(100 ln 999)=±691。
    // Q=0.75,0.25は±round(100 ln 3)=±110。
    assert_eq!(lines.len(), expected.len());
    for ((line, index), score) in lines.iter().zip(expected).zip(scores) {
        assert_eq!(line.moves, [moves[index]]);
        assert_eq!(line.depth, 1);
        assert_eq!(line.score, score);
    }
    assert_eq!(lines[0].moves[0], tree.best_move().unwrap());
    assert_eq!(lines[0].moves, tree.principal_variation());
    assert_eq!(format!("{tree:?}"), before);
    assert_eq!(pv_lines(&tree, 3, MATE - 2, 100.0), lines[..3]);
}

#[test]
fn multipv_proven_edges_invert_outcome_and_add_one_in_each_score_band() {
    let mut tree = fixture(5);
    // 先頭は別に確保し、4種類とも辺からの値の変換を検査する。
    tree.root.edges[0].child = Some(proven_child(Outcome::Loss, 0, false));
    for (index, outcome, repetition) in [
        (1, Outcome::Loss, false),
        (2, Outcome::Loss, true),
        (3, Outcome::Win, true),
        (4, Outcome::Win, false),
    ] {
        let edge = &mut tree.root.edges[index];
        edge.child = Some(proven_child(outcome, 4, repetition));
        edge.visits = 1;
        edge.value_sum = 0.25; // 確定値は、このQに優先する。
    }
    let lines = pv_lines(&tree, 5, MATE - 1, 100.0);
    assert_eq!(
        lines.iter().map(|line| line.score).collect::<Vec<_>>(),
        [
            Some(MATE - 1),
            Some(MATE - 5),
            Some(REPETITION_WIN - 5),
            Some(-REPETITION_WIN + 5),
            Some(-MATE + 5)
        ]
    );
}

#[test]
fn multipv_follows_visited_best_edges_only_in_expanded_or_proven_children() {
    let mut tree = fixture(2);
    let moves = legal_moves(&Position::initial());
    for state in [
        NodeState::Expanded,
        NodeState::Proven(proof(Outcome::Loss)),
        NodeState::Pending { unexpanded: false },
        NodeState::Unexpanded,
        NodeState::Draw {
            value: 0.5,
            approximate: false,
        },
    ] {
        let mut child = Node::new(&moves[2..4], state);
        child.edges[0].logit = 0.0;
        child.edges[0].visits = 1;
        child.edges[1].logit = 10.0;
        let mut grandchild = Node::new(&moves[4..5], NodeState::Expanded);
        grandchild.edges[0].logit = 0.0;
        child.edges[0].child = Some(Box::new(grandchild));
        tree.root.edges[0].child = Some(Box::new(child));
        let expected = if matches!(state, NodeState::Expanded | NodeState::Proven(_)) {
            vec![moves[0], moves[2]]
        } else {
            vec![moves[0]]
        };
        assert_eq!(tree.variation_from_edge(0), expected);
        assert_eq!(tree.principal_variation(), expected);
        let lines = pv_lines(&tree, 2, 110, 100.0);
        assert_eq!(lines[0].moves, expected);
        assert_eq!(lines[0].depth, expected.len() as u32);
        assert_eq!(lines[1].moves, [moves[1]]);
        assert_eq!(lines[1].depth, 1);
        assert_eq!(lines[1].score, None);
    }
}
