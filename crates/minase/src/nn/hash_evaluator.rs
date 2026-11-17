//! 固定局面の探索を再現するための、状態を持たないハッシュの評価器。

use minase_core::Position;
use minase_core::notation::{sfen, usi};

use super::{Evaluator, NnError, Query};

/// 拡張SFENの手数を除く4欄を鍵とし、64ビットFNV-1aの上位24ビットを
/// 2²⁴で割った値を勝率にする。鍵とUSIの着手を空白でつないだ文字列には
/// 同じ計算を適用し、その3倍をlogitにする。
///
/// バッチの大きさ、依頼や着手の順序、過去の呼び出しに依存しない。
/// 出力を空にしてから依頼順に積み、合法手が空の依頼にも価値を1つ返す。
/// 通常のビルドでPV-MCTSの`bench`だけが使い、対局では選べない。
pub(crate) struct HashEvaluator;

impl Evaluator for HashEvaluator {
    fn evaluate(
        &mut self,
        queries: &[Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        logits.clear();
        values.clear();
        for query in queries {
            let key = position_key(query.position);
            values.push(unit_hash(&key));
            for &mv in query.moves {
                logits.push(
                    3.0_f32
                        * unit_hash(&format!(
                            "{key} {}",
                            usi::text_generated(query.position, mv)
                        )),
                );
            }
        }
        Ok(())
    }
}

fn position_key(position: &Position) -> String {
    let setup = sfen::SetupPosition::new(position.clone(), position.lion_capture_square(), 1)
        .expect("search positions must satisfy the extended SFEN setup contract");
    sfen::to_extended_sfen(&setup)
        .split_whitespace()
        .enumerate()
        .filter_map(|(index, field)| (index != 3).then_some(field))
        .collect::<Vec<_>>()
        .join(" ")
}

fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

fn unit_hash(text: &str) -> f32 {
    (fnv1a(text) >> 40) as f32 / 16_777_216.0
}

#[cfg(test)]
mod tests;
