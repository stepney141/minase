//! 対局の進行管理。着手の受理と検証、終局裁定の呼出し、対局結果の保持を担う。

use super::adjudication::{
    AdjudicationContext, AdjudicationState, adjudicate_after_move, promoted_waiting_square,
};
use super::repetition::{
    RepetitionHistory, repetition_is_forbidden, retain_repetition_allowed_moves,
};
use crate::core::movegen::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::Color;
use crate::core::position::Position;
use crate::core::rules::Rules;

use super::error::{GameError, IllegalMoveCause};
use super::result::{DrawReason, GameResult, GameStatus, WinReason};

/// 1対局の進行を管理する対局管理層。
///
/// 着手の合法性検査、反復履歴の更新、終局裁定の呼出しをまとめ、
/// 終局後の着手を拒否する。
#[derive(Clone)]
pub struct Game {
    /// 現在の局面。
    position: Position,
    /// 対局で採用している完全な規則集合。
    rules: Rules,
    /// 採用規則に基づく合法手生成器。
    generator: MoveGenerator,
    /// 反復履歴・駒枯れ猶予・手数の裁定用状態。
    adjudication: AdjudicationState,
    /// 対局開始から現在までの探索局面キー。
    search_key_history: Vec<u64>,
    /// 終局していれば対局結果。
    result: Option<GameResult>,
}

impl Game {
    /// 指定した規則で初期局面から対局を構築する。
    pub fn new(rules: Rules) -> Self {
        Self::from_position(rules, Position::initial())
    }

    /// エンジン既定規則で初期局面から対局を構築する。
    pub fn with_default_rules() -> Self {
        Self::new(Rules::ENGINE_DEFAULT)
    }

    /// 着手を検証して適用し、王駒捕獲、反復、駒枯れ、合法手なし、詰みの順で裁定する。
    pub fn play(&mut self, mv: Move) -> Result<GameStatus, GameError> {
        self.ensure_ongoing()?;

        let mover = self.position.side_to_move();
        let undo = self.position.try_make_move_with_undo(mv, &self.generator)?;
        if repetition_is_forbidden(self.adjudication.repetition(), &self.position) {
            self.position.unmake_move(undo);
            return Err(GameError::IllegalMove {
                mv,
                cause: IllegalMoveCause::Repetition,
            });
        }
        let promoted_waiting_piece = promoted_waiting_square(mv, &undo);
        let repetition_result =
            self.adjudication
                .record_move(&self.position, &self.generator, mover, mv, &undo);
        let adjudication = adjudicate_after_move(
            &mut self.position,
            AdjudicationContext::new(self.rules, &self.adjudication, &self.generator),
            mover,
            promoted_waiting_piece,
            repetition_result,
        );
        self.adjudication
            .set_piece_exhaustion_grace(adjudication.piece_exhaustion_grace());
        self.search_key_history
            .push(self.position.zobrist() ^ self.position.rights_zobrist());

        Ok(match adjudication.result() {
            Some(result) => self.finish(result),
            None => GameStatus::Ongoing,
        })
    }

    /// 指定した対局者の投了(第21条第6項)で対局を終了する。
    pub fn resign(&mut self, color: Color) -> Result<GameStatus, GameError> {
        self.ensure_ongoing()?;
        Ok(self.finish(GameResult::Win {
            winner: color.opposite(),
            reason: WinReason::Resignation,
        }))
    }

    /// 双方の合意(第21条第7項)による引き分けで対局を終了する。
    pub fn agree_draw(&mut self) -> Result<GameStatus, GameError> {
        self.ensure_ongoing()?;
        Ok(self.finish(GameResult::Draw {
            reason: DrawReason::Agreement,
        }))
    }

    /// 終局していれば対局結果を返す。
    #[inline]
    pub const fn result(&self) -> Option<GameResult> {
        self.result
    }

    /// 対局の進行状態を返す。
    #[inline]
    pub const fn status(&self) -> GameStatus {
        match self.result {
            Some(result) => GameStatus::Finished(result),
            None => GameStatus::Ongoing,
        }
    }

    /// 現在の局面を返す。
    #[inline]
    pub const fn position(&self) -> &Position {
        &self.position
    }

    /// 対局開始から現局面までの探索局面キーを返す。
    ///
    /// 探索局面キーは、盤面・手番・先獅子状態のzobristハッシュと
    /// P1成り権保留状態のzobristハッシュをXORで合成した値である
    /// (第24条第1項)。
    #[inline]
    pub fn search_key_history(&self) -> &[u64] {
        &self.search_key_history
    }

    /// 現在の対局状態で合法な着手をすべて返す。
    ///
    /// R2またはR3で禁止される反復着手は除外する(第31条)。終局後は空の
    /// ベクタを返す。
    pub fn legal_moves(&self) -> Vec<Move> {
        if self.result.is_some() {
            return Vec::new();
        }

        let mut moves = Vec::new();
        self.generator.generate_moves(&self.position, &mut moves);
        if matches!(
            self.adjudication.repetition(),
            RepetitionHistory::R2(_) | RepetitionHistory::R3(_)
        ) {
            let mut position = self.position.clone();
            retain_repetition_allowed_moves(
                &mut position,
                &self.generator,
                self.adjudication.repetition(),
                &mut moves,
            );
        }
        moves
    }

    /// 対局で採用している規則を返す。
    #[inline]
    pub const fn rules(&self) -> Rules {
        self.rules
    }

    /// 対局開始からの手数を返す。
    #[inline]
    pub const fn ply_count(&self) -> u32 {
        self.adjudication.ply()
    }

    /// [`parse_sfen`](crate::parse_sfen)などで得た任意局面から対局を構築する。
    ///
    /// 王駒の存在や駒種ごとの枚数上限など、規則上の局面合法性は検証しない。
    /// 渡された局面は反復履歴上の第1回として記録する。
    pub fn from_position(rules: Rules, position: Position) -> Self {
        let adjudication = AdjudicationState::new(rules.repetition, &position);
        let search_key = position.zobrist() ^ position.rights_zobrist();
        Self {
            position,
            rules,
            generator: MoveGenerator::new(rules.moves),
            adjudication,
            search_key_history: vec![search_key],
            result: None,
        }
    }

    /// 対局が継続中であることを検査する(第26条第12項)。
    fn ensure_ongoing(&self) -> Result<(), GameError> {
        if self.result.is_some() {
            Err(GameError::GameAlreadyOver)
        } else {
            Ok(())
        }
    }

    /// 対局結果を確定して終局状態を返す。
    fn finish(&mut self, result: GameResult) -> GameStatus {
        debug_assert!(self.result.is_none());
        self.result = Some(result);
        GameStatus::Finished(result)
    }
}
