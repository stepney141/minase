//! MCTSの入力と木の操作の失敗。

use std::fmt;

use super::tree::TreeError;

/// MCTSを開始または駆動できない理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// ネットの推論器が設定されていない。
    MissingEvaluator,
    /// MCTSが受け付けない深さ制限が指定された。
    DepthLimitUnsupported,
    /// 入力平面で表せない成り権の保留を生じる規則である。
    PromotionDeferred,
    /// 勝率尺度が有限の正数ではない。
    InvalidScale,
    /// 木の操作が契約を満たさなかった。
    Tree(TreeError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingEvaluator => f.write_str("MCTS requires a neural evaluator"),
            Self::DepthLimitUnsupported => f.write_str("MCTS does not support a depth limit"),
            Self::PromotionDeferred => f.write_str("MCTS cannot encode rules P1, P2 or P5"),
            Self::InvalidScale => f.write_str("MCTS cp_per_logit must be finite and positive"),
            Self::Tree(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Tree(error) => Some(error),
            _ => None,
        }
    }
}

impl From<TreeError> for Error {
    fn from(error: TreeError) -> Self {
        Self::Tree(error)
    }
}
