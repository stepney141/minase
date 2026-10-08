//! ネットの推論の抽象と、試験用の一様評価器。
//!
//! 探索は局面の束を[`encoding::encode_into`](super::encoding::encode_into)で入力平面へ符号化し、
//! [`Evaluator::evaluate`]に渡して各局面の方策logitと手番側の勝率を受け取る
//! （docs/plans/alphazero.md「推論層」）。
//! 出力に非有限値がないことの検査は、評価器の実装ではなく探索の側で1か所にまとめて行う。

use core::fmt;

use minase_core::BOARD_SQUARE_COUNT;

use super::{INPUT_PLANES, POLICY_SIZE};

/// 1局面の入力平面の要素数（95面×144升）。
pub const INPUT_SIZE: usize = INPUT_PLANES * BOARD_SQUARE_COUNT;

/// 推論の失敗。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EvaluatorError {
    /// 方策logitまたは勝率に非有限値（NaNまたは無限大）がある。
    NonFinite,
    /// 推論の実行系が失敗した。理由を持つ。
    Inference(String),
}

impl fmt::Display for EvaluatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite => formatter.write_str("network output contains a non-finite value"),
            Self::Inference(reason) => write!(formatter, "inference failed: {reason}"),
        }
    }
}

impl std::error::Error for EvaluatorError {}

/// 局面の束を評価して方策logitと勝率を返す推論器。
pub trait Evaluator {
    /// `batch`局面の入力平面を受け、各局面の方策logitと手番側の勝率を書く。
    ///
    /// `inputs`は局面ごとに[`INPUT_SIZE`]個の面優先の値を並べた`batch * INPUT_SIZE`個の配列、
    /// `out_policy`は局面ごとに[`POLICY_SIZE`]個のlogitを書く`batch * POLICY_SIZE`個の配列、
    /// `out_value`は局面ごとに勝率を1つ書く`batch`個の配列である。
    ///
    /// # Errors
    ///
    /// 推論に失敗した場合はエラーを返す。
    fn evaluate(
        &mut self,
        inputs: &[f32],
        batch: usize,
        out_policy: &mut [f32],
        out_value: &mut [f32],
    ) -> Result<(), EvaluatorError>;
}

/// すべての方策logitを0、勝率を0.5とする評価器。
///
/// 合法手のlogitだけをsoftmaxで正規化すると、すべての合法手に等しい確率を与える。
/// GPUのない環境でMCTSの単体試験に使う（docs/plans/alphazero.md「推論層」）。
#[derive(Clone, Copy, Default, Debug)]
pub struct UniformEvaluator;

impl Evaluator for UniformEvaluator {
    /// # Panics
    ///
    /// 配列の長さが`batch`と合わない場合にパニックする。
    fn evaluate(
        &mut self,
        inputs: &[f32],
        batch: usize,
        out_policy: &mut [f32],
        out_value: &mut [f32],
    ) -> Result<(), EvaluatorError> {
        assert_eq!(inputs.len(), batch * INPUT_SIZE);
        assert_eq!(out_policy.len(), batch * POLICY_SIZE);
        assert_eq!(out_value.len(), batch);
        out_policy.fill(0.0);
        out_value.fill(0.5);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// alphazero.md「推論層」: 一様評価器はすべてのlogitを0、勝率を0.5とする。
    #[test]
    fn uniform_evaluator_returns_flat_policy_and_even_value() {
        let batch = 3;
        let inputs = vec![0.0; batch * INPUT_SIZE];
        let mut policy = vec![f32::NAN; batch * POLICY_SIZE];
        let mut value = vec![f32::NAN; batch];
        UniformEvaluator
            .evaluate(&inputs, batch, &mut policy, &mut value)
            .unwrap();
        assert!(policy.iter().all(|&logit| logit == 0.0));
        assert!(value.iter().all(|&value| value == 0.5));
    }
}
