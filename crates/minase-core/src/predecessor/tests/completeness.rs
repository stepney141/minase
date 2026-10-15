//! コーパスの全順方向辺について元の局面が逆生成に含まれることと、返却集合全体の健全性を検査する。

use super::*;
use crate::test_util::{bench_positions, sampled_random_positions};
use crate::{RuleCode::*, Rules};

fn all_forward_edges(rules: MoveRules) {
    let forward = MoveGenerator::new(rules);
    let mut edges = 0;
    let mut staged = 0;
    // 初期局面、benchの先頭3局面、および固定シードの32局面をコーパスとする。
    for (index, predecessor) in std::iter::once(Position::initial())
        .chain(bench_positions().into_iter().take(3))
        .chain(sampled_random_positions(rules))
        .enumerate()
    {
        let mut moves = Vec::new();
        forward.generate_moves(&predecessor, &mut moves);
        for mv in moves {
            let mut target = predecessor.clone();
            target.try_make_move(mv, &forward).unwrap();
            let result = checked(rules, &target);
            assert!(result.contains(&predecessor), "corpus {index}, move {mv:?}");
            edges += 1;
            staged += usize::from(mv.mid.is_some());
        }
    }
    assert!(edges > 0 && staged > 0);
    println!("predecessor_completeness {rules:?}: edges={edges}, staged={staged}");
}

/// 標準規則で、コーパスの全順方向辺の逆包含と返却集合の健全性を検査する。
#[test]
#[ignore = "標準規則の全合法辺を検査する(約60秒、直前局面の集合が大きい)"]
fn predecessor_completeness_standard() {
    all_forward_edges(MoveRules::standard());
}

/// L1、L2、L3の組合せで、全順方向辺の逆包含と返却集合の健全性を検査する。
#[test]
#[ignore = "ローカルルールの全合法辺を検査する(標準規則版と同程度以上)"]
fn predecessor_completeness_l1_l2_l3() {
    all_forward_edges(Rules::from_codes(&[L1, L2, L3, P0, R1, E0]).unwrap().moves);
}

/// L4とP1の組合せで、全順方向辺の逆包含と返却集合の健全性を検査する。
#[test]
#[ignore = "ローカルルールの全合法辺を検査する(標準規則版と同程度以上)"]
fn predecessor_completeness_l4_p1() {
    all_forward_edges(Rules::from_codes(&[L0, L4, P1, R1, E0]).unwrap().moves);
}

/// P2、P5、P6の組合せで、全順方向辺の逆包含と返却集合の健全性を検査する。
#[test]
#[ignore = "ローカルルールの全合法辺を検査する(標準規則版と同程度以上)"]
fn predecessor_completeness_p2_p5_p6() {
    all_forward_edges(Rules::from_codes(&[L0, P2, P5, P6, R1, E0]).unwrap().moves);
}
