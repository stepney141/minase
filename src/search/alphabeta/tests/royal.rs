//! 王駒への利きを検査する。

use super::*;

// 同「検証」。王将が安全でも太子が攻撃されていれば真になる。
#[test]
fn royal_under_attack_includes_crown_prince_alone() {
    for (last_rank, attacked) in [("11K", false), ("5+E5K", true)] {
        let position =
            crate::parse_sfen(&format!("k11/12/12/12/12/5r6/12/12/12/12/12/{last_rank} b"))
                .unwrap();
        assert_eq!(royal_under_attack(&position), attacked);
    }
}

// 同「検証」とRULES.md第8条。獅子は間の駒を跳び越えて距離2の王駒を取れる。
#[test]
fn royal_under_attack_includes_lion_jump_and_only_opponents() {
    for (lion, side, attacked) in [("n", "b", true), ("N", "b", false), ("n", "w", false)] {
        let position = crate::parse_sfen(&format!(
            "k11/12/12/12/12/12/12/12/12/9{lion}2/10P1/11K {side}"
        ))
        .unwrap();
        assert_eq!(royal_under_attack(&position), attacked);
    }
}
