//! MCTSの入力と木の操作の失敗。

use std::fmt;

use minase_core::{GameError, GameResult, Move};

use super::tree::TreeError;

/// MCTSを開始または駆動できない理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// 審判状態のない、αβ探索の試験用入力が渡された。
    MissingGame,
    /// 選択済みの経路を審判層が受理しなかった。
    Replay {
        /// 根から葉までの着手列。
        path: Vec<Move>,
        /// 拒否された着手の位置。根から1始まりで数える。
        ply: usize,
        /// 審判層が返した元のエラー。
        source: GameError,
    },
    /// 経路の末尾より前に終局した。
    PrematureTerminal {
        /// 根から葉までの着手列。
        path: Vec<Move>,
        /// 終局した着手の位置。根から1始まりで数える。
        ply: usize,
        /// 経路の途中で確定した対局結果。
        result: GameResult,
    },
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
            Self::MissingGame => f.write_str("MCTS requires a snapshot from a game"),
            Self::Replay { path, ply, source } => {
                write!(
                    f,
                    "MCTS replay failed at ply {ply} on path {path:?}: {source}"
                )
            }
            Self::PrematureTerminal { path, ply, result } => {
                write!(
                    f,
                    "MCTS path {path:?} finished before its leaf at ply {ply}: {result:?}"
                )
            }
            Self::MissingEvaluator => f.write_str("MCTS requires isready before go"),
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
            Self::Replay { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<TreeError> for Error {
    fn from(error: TreeError) -> Self {
        Self::Tree(error)
    }
}
