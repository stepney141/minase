//! 根の探索統計を、対局と固定局面の診断で共有する書式へ変換する。

use std::fmt::Display;
use std::time::Duration;

use minase_core::Position;
use minase_core::notation::usi;

use super::tree::{NodeState, Outcome, Tree};

/// 根の要約と、最善手の比較規則で並べた全候補手を返す。
///
/// `budget`は目安時間softと上限時間hardの組である。
/// `info string `は呼び出し側が付ける。浮動小数点は丸めず`Display`で出す。
/// 子のネットの価値と確定した勝敗は根の手番側へ反転し、確定から価値を補わない。
/// 根の評価を1回含む正常終了の探索だけを受け付け、`S = 0`または
/// 持ち越し訪問数`R - (S - 1)`が負なら内部の契約違反としてパニックする。
pub(crate) fn lines(
    root_position: &Position,
    tree: &Tree,
    simulations: u64,
    elapsed: Duration,
    budget: Option<(Duration, Duration)>,
) -> Vec<String> {
    let root = tree.root();
    let visits: u64 = root.edges.iter().map(|edge| u64::from(edge.visits)).sum();
    let reused = simulations
        .checked_sub(1)
        .and_then(|new| visits.checked_sub(new))
        .unwrap_or_else(|| panic!("invalid root statistics: S={simulations}, R={visits}"));
    let mut lines = vec![format!(
        "movestats root simulations {simulations} reused {reused} visits {visits} value {} tree-bytes {} capacity-bytes {} elapsed-ms {} soft-ms {} hard-ms {} result {}",
        optional(root.value),
        tree.used_bytes(),
        tree.capacity_bytes(),
        elapsed.as_millis(),
        optional(budget.map(|b| b.0.as_millis())),
        optional(budget.map(|b| b.1.as_millis())),
        result(root.state),
    )];
    for index in root.ranked_edges() {
        let edge = &root.edges[index];
        let value = edge
            .child
            .as_ref()
            .and_then(|child| child.value)
            .map(|value| 1.0 - f64::from(value));
        let state = if let Some(proof) = edge.parent_proof() {
            NodeState::Proven(proof)
        } else {
            match edge.child.as_ref() {
                Some(child) => child.state,
                None => NodeState::Expanded,
            }
        };
        lines.push(format!(
            "movestats move {} visits {} value-sum {} q {} prior {} value {} result {}",
            usi::text_generated(root_position, edge.mv),
            edge.visits,
            edge.value_sum,
            optional(edge.q()),
            edge.policy,
            optional(value),
            result(state),
        ));
    }
    lines
}

fn optional(value: Option<impl Display>) -> String {
    match value {
        Some(value) => value.to_string(),
        None => "-".to_owned(),
    }
}

fn result(state: NodeState) -> String {
    match state {
        NodeState::Proven(proof) => {
            let outcome = match proof.outcome {
                Outcome::Win => "win",
                Outcome::Loss => "loss",
            };
            format!(
                "{outcome} distance {}{}{}",
                proof.distance,
                if proof.repetition { " repetition" } else { "" },
                if proof.approximate {
                    " approximate"
                } else {
                    ""
                }
            )
        }
        NodeState::Draw { approximate, .. } => if approximate {
            "draw approximate"
        } else {
            "draw"
        }
        .to_owned(),
        _ => "none".to_owned(),
    }
}

#[cfg(test)]
mod tests;
