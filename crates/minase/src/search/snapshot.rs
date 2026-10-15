//! 探索開始時の局面と履歴の写し。

use crate::search::error::SearchError;
use minase_core::game::{Game, GameStatus};
use minase_core::mv::Move;
use minase_core::position::Position;
use minase_core::rules::{MoveRules, RepetitionRule};

/// 探索へ渡す反復規則と、規則が要する根の時点の状態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchRepetition {
    /// R1（4回反復の裁定）。根の時点の対局者ごとの攻撃連続数を持つ。
    R1 {
        /// 根の時点の攻撃連続数。添字は[`minase_core::Color::index`]に従う。
        consecutive_attacking_moves: [u32; 2],
    },
    /// R2（既出局面の再現の禁止）。
    R2,
    /// R3（4回目の出現の禁止）。
    R3,
}

/// 探索スレッドへ渡す不変の入力。
#[derive(Clone)]
pub struct SearchSnapshot {
    /// 探索を開始する局面。
    pub(super) position: Position,
    /// 探索内で着手へ適用する規則。
    pub(super) rules: MoveRules,
    /// 採用中の反復規則と、規則が要する根の時点の状態。
    pub(super) repetition: SearchRepetition,
    /// 対局開始から現局面までの探索局面キー。
    pub(super) history_keys: Vec<u64>,
    /// 探索の根で調べる候補手。
    pub(super) root_moves: Vec<Move>,
}

impl SearchSnapshot {
    /// 継続中の対局から探索用の不変入力を構築する。
    ///
    /// RULES.md第31条R1では、反復勝ちの手があれば、その手だけを根の候補手にする。
    /// なければ、自分の着手または相手の直後の応手で反復負けになる手を除く。
    /// 全合法手が反復負けに直結する場合は、全手を残す。
    /// 候補手は[`Game::legal_moves`]の順序を保ち、R1以外の規則では選別しない。
    ///
    /// # Errors
    ///
    /// 対局が終局済みの場合は[`SearchError::FinishedGame`]を返す。
    /// 継続中でもルート合法手がない場合は[`SearchError::NoLegalMoves`]を返す。
    pub fn from_game(game: &Game) -> Result<Self, SearchError> {
        if matches!(game.status(), GameStatus::Finished(_)) {
            return Err(SearchError::FinishedGame);
        }
        let mut root_moves = game.legal_moves();
        let classified = game.repetition_root_moves();
        if !classified.winning().is_empty() {
            root_moves.retain(|mv| classified.winning().contains(mv));
        } else if classified.losing().len() < root_moves.len() {
            root_moves.retain(|mv| !classified.losing().contains(mv));
        }
        let repetition = match game.rules().repetition {
            RepetitionRule::R1 => SearchRepetition::R1 {
                consecutive_attacking_moves: game
                    .consecutive_attacking_moves()
                    .expect("an R1 game must track consecutive attacking moves"),
            },
            RepetitionRule::R2 => SearchRepetition::R2,
            RepetitionRule::R3 => SearchRepetition::R3,
        };
        Self::from_parts(
            game.position().clone(),
            game.rules().moves,
            repetition,
            game.search_key_history().to_vec(),
            root_moves,
        )
    }

    /// 検証済みの対局管理層が確定した各入力からスナップショットを構築する。
    pub(super) fn from_parts(
        position: Position,
        rules: MoveRules,
        repetition: SearchRepetition,
        history_keys: Vec<u64>,
        root_moves: Vec<Move>,
    ) -> Result<Self, SearchError> {
        if root_moves.is_empty() {
            return Err(SearchError::NoLegalMoves);
        }
        Ok(Self {
            position,
            rules,
            repetition,
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

    /// 採用中の反復規則と、規則が要する根の時点の状態を返す。
    pub const fn repetition(&self) -> SearchRepetition {
        self.repetition
    }

    /// 対局開始から現局面までの探索局面キーを返す。
    pub fn history_keys(&self) -> &[u64] {
        &self.history_keys
    }

    /// 探索の根で調べる候補手を返す。
    pub fn root_moves(&self) -> &[Move] {
        &self.root_moves
    }
}

/// 反復検出と置換表に使う探索局面キーを計算する。
///
/// 第24条第1項dが区別する成り権の保留状態は、P1、P2、P5では駒配置だけで決まらず、次の合法手を変える。
/// そのため駒配置と手番の[`Position::zobrist`]に[`Position::rights_zobrist`]を合成する。
/// 先獅子の状態は`zobrist()`側に含まれる。
pub(super) fn search_key(position: &Position) -> u64 {
    position.zobrist() ^ position.rights_zobrist()
}
