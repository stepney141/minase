//! 診断用の推論の失敗と、探索と共有する出力の検査。

use crate::nn::{NnError, Query};
use std::fmt;

/// 準備済みの探索器による推論を実行できない理由。
#[derive(Debug)]
pub enum EvaluationError {
    /// αβ探索には方策と勝率を返す推論器がない。
    UnsupportedSearch,
    /// `prepare`が成功した後に推論器が破棄されたか、まだ準備していない。
    NotPrepared,
    /// 入力の局面をネットへ符号化できない。
    Input(NnError),
    /// 推論器の実行または出力の検査に失敗した。
    Inference(NnError),
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSearch => f.write_str("network evaluation requires --search mcts"),
            Self::NotPrepared => f.write_str("eval requires isready"),
            Self::Input(error) | Self::Inference(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for EvaluationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input(error) | Self::Inference(error) => Some(error),
            Self::UnsupportedSearch | Self::NotPrepared => None,
        }
    }
}

pub(super) fn validate_output_shape(
    queries: &[Query<'_>],
    logits: &[f32],
    values: &[f32],
) -> Result<(), NnError> {
    let total_logits = queries.iter().map(|query| query.moves.len()).sum::<usize>();
    for (name, expected, actual) in [
        ("values", queries.len(), values.len()),
        ("logits", total_logits, logits.len()),
    ] {
        if actual != expected {
            return Err(NnError::OutputShape {
                name,
                expected: [expected, 1],
                actual: vec![actual as i64, 1],
            });
        }
    }
    Ok(())
}
