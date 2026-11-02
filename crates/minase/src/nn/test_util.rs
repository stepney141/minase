//! 学習済みのネットや推論ランタイムを使わない試験用の評価器。

use super::{Evaluator, NnError, Query};

/// 各着手のlogitを0.0、価値を0.5にする試験用の評価器。
///
/// GPUのない環境でモンテカルロ木探索（MCTS）の単体試験に使い、
/// lc0のrandom/mockバックエンドと同じ役割を持つ。
#[derive(Default)]
pub struct UniformEvaluator;

impl Evaluator for UniformEvaluator {
    fn evaluate(
        &mut self,
        queries: &[Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        logits.clear();
        values.clear();
        for query in queries {
            logits.resize(logits.len() + query.moves.len(), 0.0);
            values.push(0.5);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase_core::{MoveGenerator, Position, Rules};

    // [`UniformEvaluator`]の契約: 依頼の着手数に応じた0.0と各局面の0.5で出力を上書きする。
    #[test]
    fn uniform_evaluation_returns_legal_move_logits_and_clears_empty_batches() {
        let position = Position::initial();
        let mut moves = Vec::new();
        MoveGenerator::new(Rules::ENGINE_DEFAULT.moves).generate_moves(&position, &mut moves);
        let queries = [
            Query {
                position: &position,
                moves: &moves[..3],
            },
            Query {
                position: &position,
                moves: &[],
            },
            Query {
                position: &position,
                moves: &moves[..5],
            },
        ];
        let mut evaluator = UniformEvaluator;
        let mut logits = vec![7.0; 20];
        let mut values = vec![9.0; 10];
        evaluator
            .evaluate(&queries, &mut logits, &mut values)
            .unwrap();
        assert_eq!(logits, vec![0.0; 8]);
        assert_eq!(values, vec![0.5; 3]);
        evaluator.evaluate(&[], &mut logits, &mut values).unwrap();
        assert!(logits.is_empty());
        assert!(values.is_empty());
    }
}
