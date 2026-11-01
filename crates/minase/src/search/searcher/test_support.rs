//! 対局内の履歴表の保持を検査するための参照。

use super::Searcher;
use crate::search::HistoryTables;

impl Searcher {
    /// 対局内の状態保持を検査するため、履歴表を参照する。
    pub(crate) fn histories_for_test(&self) -> &HistoryTables {
        match self {
            Self::AlphaBeta(engine) => &engine.histories,
            Self::Mcts(_) => panic!("alpha-beta histories requested by a test of MCTS"),
        }
    }

    /// 対局内の状態保持を検査するため、履歴表へ値を埋める。
    pub(crate) fn histories_mut_for_test(&mut self) -> &mut HistoryTables {
        match self {
            Self::AlphaBeta(engine) => &mut engine.histories,
            Self::Mcts(_) => panic!("alpha-beta histories requested by a test of MCTS"),
        }
    }
}
