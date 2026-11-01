//! 合法手の方策logitと手番側の勝率をまとめて求める評価器。

use super::NnError;
use minase_core::{Move, Position};

/// 1局面の評価の依頼。
pub struct Query<'a> {
    /// 評価する局面。
    pub position: &'a Position,
    /// 方策のlogitを求める着手。`position`の合法手であること。
    pub moves: &'a [Move],
}

/// 局面の束を評価し、指定した合法手の方策logitと手番側の勝率を返す。
///
/// 方策全体は35,856要素で、32ビット浮動小数点（FP32）なら1局面あたり約143 KBになる。
/// 全要素の複写と非有限値の検査を行うと、ホスト側の費用が推論に匹敵するため、合法手のlogitだけを返す。
/// ONNX Runtimeによる実装は、出力バッファから各依頼の`moves`のラベル位置だけを集め、
/// 非有限値の検査も集めたlogitと価値だけに行う。
pub trait Evaluator {
    /// `queries`の各局面を評価する。
    ///
    /// `logits`と`values`を空にしてから、依頼の順に、各依頼の`moves`と同じ順の
    /// 方策logitを`logits`へ連結し、手番側の勝率（0〜1）を`values`へ1つずつ積む。
    /// `queries`が空なら、両方のVecを空にして成功する。
    ///
    /// # Errors
    /// 入力の符号化またはネットの評価に失敗した場合は[`NnError`]を返す。
    fn evaluate(
        &mut self,
        queries: &[Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), NnError>;
}
