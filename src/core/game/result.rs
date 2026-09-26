//! 対局の結果と勝敗の成立理由。

use crate::core::piece::Color;

/// 勝利の成立理由。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WinReason {
    /// 相手の最後の王駒の捕獲(第21条第1項)。
    RoyalCapture,
    /// 反復裁定による勝利(第31条R1)。
    Repetition,
    /// 駒枯れによる勝利(第22条)。
    PieceExhaustion,
    /// 裸玉による勝利(第32条E3)。
    BareKing,
    /// 相手に合法手がないことによる勝利(第23条)。
    Stalemate,
    /// 相手の最後の王駒の詰み(第21条第2項)。
    Mate,
    /// 相手の投了(第21条第6項)。
    Resignation,
}

/// 引き分けの成立理由。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DrawReason {
    /// 反復裁定による引き分け(第31条R1)。
    Repetition,
    /// 駒枯れによる引き分け(第22条第8項)。
    PieceExhaustion,
    /// 裸玉による引き分け(第32条E3)。
    BareKing,
    /// 双方の合意(第21条第7項)。
    Agreement,
}

/// 終局した対局の結果。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GameResult {
    /// 一方の勝利。
    Win {
        /// 勝者。
        winner: Color,
        /// 勝利の成立理由。
        reason: WinReason,
    },
    /// 引き分け。
    Draw {
        /// 引き分けの成立理由。
        reason: DrawReason,
    },
}

/// 対局の進行状態。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GameStatus {
    /// 対局継続中。
    Ongoing,
    /// 終局済み。
    Finished(GameResult),
}
