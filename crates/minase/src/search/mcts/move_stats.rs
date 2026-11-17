//! 根の探索統計を、対局と固定局面の診断で共有する書式へ変換する。

use std::fmt::Display;
use std::time::Duration;

use minase_core::Position;
use minase_core::notation::usi;

use super::tree::{NodeState, Outcome, Tree};

/// 根の要約と、最善手の比較規則で並べた全候補手を返す。
///
/// 要約のsimulationsは今回の評価・逆伝播の回数S、visitsは根の辺の訪問回数の和R、
/// reusedは前回から持ち越した訪問数`R - (S - 1)`である。根の評価はSにだけ1回含む。
/// 要約のvalueは根のネットの勝率、tree-bytesとcapacity-bytesは木の使用量と容量である。
/// elapsed-msは完了イベントの経過時間で、出力までの待ち時間を含めない。
/// `budget`は目安時間softと上限時間hardの組で、soft-msとhard-msにミリ秒で出す。
/// 予算がなければ両方とも`-`とする。先読みでは予算をponderhitから測るので、
/// 探索開始から測るelapsed-msとは起点が異なる。
///
/// 各move行のvisitsは辺の訪問回数、value-sumは根の手番側の勝率の和、
/// qは辺の平均勝率で未訪問なら`-`、priorは温度適用後の方策の確率である。
/// valueは子のネットの勝率vを反転した`1 - v`で、ネットの評価がなければ`-`とする。
/// resultは未確定ならnone、引き分けならdraw、確定した勝敗ならwinまたはlossと
/// distanceの手数を出す。move行では子の勝敗を反転して手数を1増やす。
/// 反復による勝敗にはrepetition、反復の近似に依存する結果にはapproximateを付ける。
/// 同順位の候補は元の順を保つため、先頭のmove行は探索が選んだ最善手である。
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
