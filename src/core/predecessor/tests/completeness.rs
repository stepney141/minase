//! 設計書「完全性の根拠」第2層の全順方向辺を検査する。

use super::*;
use crate::test_util::{bench_positions, sampled_random_positions};
use crate::{RuleCode::*, Rules};

fn all_forward_edges(rules: MoveRules) {
    let forward = MoveGenerator::new(rules);
    let mut edges = 0;
    let mut staged = 0;
    // 標準規則の全15bench局面は30秒を超えたため、指示書の縮小規定に従い
    // 規則共通のbenchコーパスを先頭3局面に限る。固定シード32局面はすべて使う。
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

#[test]
#[ignore = "標準規則の全合法辺を検査する(約60秒、直前局面の集合が大きい)"]
fn predecessor_completeness_standard() {
    // 設計書「完全性の根拠」第2層: 標準規則の全合法手と返却集合の健全性。
    all_forward_edges(MoveRules::standard());
}

#[test]
#[ignore = "ローカルルールの全合法辺を検査する"]
fn predecessor_completeness_l1_l2_l3() {
    // 設計書「完全性の根拠」「ローカルルール」: L1・L2・L3の組合せ。
    all_forward_edges(Rules::from_codes(&[L1, L2, L3, P0, R1, E0]).unwrap().moves);
}

#[test]
#[ignore = "ローカルルールの全合法辺を検査する"]
fn predecessor_completeness_l4_p1() {
    // 設計書「完全性の根拠」「ローカルルール」: L4とP1の組合せ。
    all_forward_edges(Rules::from_codes(&[L0, L4, P1, R1, E0]).unwrap().moves);
}

#[test]
#[ignore = "ローカルルールの全合法辺を検査する"]
fn predecessor_completeness_p2_p5_p6() {
    // 設計書「完全性の根拠」「ローカルルール」: P2・P5・P6の組合せ。
    all_forward_edges(Rules::from_codes(&[L0, P2, P5, P6, R1, E0]).unwrap().moves);
}
