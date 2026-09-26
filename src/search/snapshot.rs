//! 探索開始時の局面と履歴の写し。

use crate::core::game::{Game, GameStatus};
use crate::core::mv::Move;
use crate::core::position::Position;
use crate::core::rules::MoveRules;
use crate::search::error::SearchError;

/// 探索スレッドへ渡す不変の入力。
#[derive(Clone)]
pub struct SearchSnapshot {
    /// 探索を開始する局面。
    pub(super) position: Position,
    /// 探索内で着手へ適用する規則。
    pub(super) rules: MoveRules,
    /// 対局開始から現局面までの探索局面キー。
    pub(super) history_keys: Vec<u64>,
    /// 対局管理層が確定したルート合法手。
    pub(super) root_moves: Vec<Move>,
}

impl SearchSnapshot {
    /// 継続中の対局から探索用の不変入力を構築する。
    ///
    /// # Errors
    ///
    /// 対局が終局済みの場合は[`SearchError::FinishedGame`]を返す。
    /// 継続中でもルート合法手がない場合は[`SearchError::NoLegalMoves`]を返す。
    pub fn from_game(game: &Game) -> Result<Self, SearchError> {
        if matches!(game.status(), GameStatus::Finished(_)) {
            return Err(SearchError::FinishedGame);
        }
        Self::from_parts(
            game.position().clone(),
            game.rules().moves,
            game.search_key_history().to_vec(),
            game.legal_moves(),
        )
    }

    /// 検証済みの対局管理層が確定した各入力からスナップショットを構築する。
    pub(super) fn from_parts(
        position: Position,
        rules: MoveRules,
        history_keys: Vec<u64>,
        root_moves: Vec<Move>,
    ) -> Result<Self, SearchError> {
        if root_moves.is_empty() {
            return Err(SearchError::NoLegalMoves);
        }
        Ok(Self {
            position,
            rules,
            history_keys,
            root_moves,
        })
    }

    /// 探索を開始する局面を返す。
    pub const fn position(&self) -> &Position {
        &self.position
    }

    /// 探索内で着手へ適用する規則を返す。
    pub const fn rules(&self) -> MoveRules {
        self.rules
    }

    /// 対局開始から現局面までの探索局面キーを返す。
    pub fn history_keys(&self) -> &[u64] {
        &self.history_keys
    }

    /// 対局管理層が確定したルート合法手を返す。
    pub fn root_moves(&self) -> &[Move] {
        &self.root_moves
    }
}

/// 反復検出に使う探索局面キー(第24条第1項)を計算する。
pub(super) fn search_key(position: &Position) -> u64 {
    position.zobrist() ^ position.rights_zobrist()
}
