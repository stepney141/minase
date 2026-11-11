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
/// 方策全体のラベルは35,856個である。ONNX Runtimeによる実装は、メタデータ`policy_labels`の
/// 表に従って詰めた16ビット浮動小数点（FP16）の出力を受け取り、各依頼の`moves`のラベルから
/// 詰めた後の添字を逆引きして、必要なlogitだけをf32へ変換して集める。
/// 非有限値の検査も、変換後のlogitとf32の価値だけに行う。
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
