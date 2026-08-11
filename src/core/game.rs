use core::fmt;

use crate::core::adjudication::{
    AdjudicationContext, AdjudicationState, adjudicate_after_move, promoted_waiting_square,
};
use crate::core::movegen::{IllegalMove, MoveGenerator};
use crate::core::mv::Move;
use crate::core::piece::Color;
use crate::core::position::Position;
use crate::core::repetition::{
    RepetitionHistory, repetition_is_forbidden, retain_repetition_allowed_moves,
};
use crate::core::rules::Rules;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WinReason {
    RoyalCapture,
    Repetition,
    PieceExhaustion,
    /// 裸玉による勝利(第32条E3)。
    BareKing,
    Stalemate,
    Mate,
    Resignation,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DrawReason {
    Repetition,
    PieceExhaustion,
    /// 裸玉による引き分け(第32条E3)。
    BareKing,
    Agreement,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GameResult {
    Win { winner: Color, reason: WinReason },
    Draw { reason: DrawReason },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GameStatus {
    Ongoing,
    Finished(GameResult),
}

/// 対局管理層が着手を拒否した原因。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IllegalMoveCause {
    /// 駒の動きまたは獅子の捕獲制限に反する着手。
    Movement,
    /// R2またはR3が禁止する反復着手(第31条)。
    Repetition,
}

impl fmt::Display for IllegalMoveCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Movement => formatter.write_str("illegal movement"),
            Self::Repetition => formatter.write_str("forbidden repetition"),
        }
    }
}

impl std::error::Error for IllegalMoveCause {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameError {
    GameAlreadyOver,
    IllegalMove { mv: Move, cause: IllegalMoveCause },
}

impl fmt::Display for GameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GameAlreadyOver => formatter.write_str("the game is already over"),
            Self::IllegalMove {
                mv,
                cause: IllegalMoveCause::Movement,
            } => IllegalMove(*mv).fmt(formatter),
            Self::IllegalMove {
                mv,
                cause: IllegalMoveCause::Repetition,
            } => write!(formatter, "the move is forbidden by repetition: {mv:?}"),
        }
    }
}

impl std::error::Error for GameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IllegalMove { cause, .. } => Some(cause),
            Self::GameAlreadyOver => None,
        }
    }
}

impl From<IllegalMove> for GameError {
    fn from(IllegalMove(mv): IllegalMove) -> Self {
        Self::IllegalMove {
            mv,
            cause: IllegalMoveCause::Movement,
        }
    }
}

/// 対局の構築エラー。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GameBuildError {
    /// 実行可能な反復規則が指定されていない。
    MissingRepetitionRule,
}

impl fmt::Display for GameBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRepetitionRule => formatter.write_str("missing repetition rule"),
        }
    }
}

impl std::error::Error for GameBuildError {}

#[derive(Clone)]
pub struct Game {
    position: Position,
    generator: MoveGenerator,
    adjudication: AdjudicationState,
    /// 対局開始から現在までの探索局面キー。
    search_key_history: Vec<u64>,
    result: Option<GameResult>,
}

impl Game {
    /// 指定した規則で初期局面から対局を構築する。
    ///
    /// # エラー
    ///
    /// R1、R2またはR3が指定されていなければ
    /// [`GameBuildError::MissingRepetitionRule`]を返す。
    pub fn new(rules: Rules) -> Result<Self, GameBuildError> {
        Self::from_position(rules, Position::initial())
    }

    /// エンジン既定規則で初期局面から対局を構築する。
    pub fn with_default_rules() -> Self {
        Self::new(Rules::engine_default())
            .expect("engine default rules must contain a repetition rule")
    }

    /// 着手を検証して適用し、王駒捕獲、反復、駒枯れ、合法手なし、詰みの順で裁定する。
    pub fn play(&mut self, mv: Move) -> Result<GameStatus, GameError> {
        self.ensure_ongoing()?;

        let mover = self.position.side_to_move();
        let undo = self.position.try_make_move(mv, &self.generator)?;
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
                .record_move(&self.position, &self.generator, mover, mv);
        let adjudication = adjudicate_after_move(
            &mut self.position,
            AdjudicationContext::new(&self.adjudication, &self.generator),
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

    pub fn resign(&mut self, color: Color) -> Result<GameStatus, GameError> {
        self.ensure_ongoing()?;
        Ok(self.finish(GameResult::Win {
            winner: color.opposite(),
            reason: WinReason::Resignation,
        }))
    }

    pub fn agree_draw(&mut self) -> Result<GameStatus, GameError> {
        self.ensure_ongoing()?;
        Ok(self.finish(GameResult::Draw {
            reason: DrawReason::Agreement,
        }))
    }

    #[inline]
    pub const fn result(&self) -> Option<GameResult> {
        self.result
    }

    #[inline]
    pub const fn status(&self) -> GameStatus {
        match self.result {
            Some(result) => GameStatus::Finished(result),
            None => GameStatus::Ongoing,
        }
    }

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
    pub fn rules(&self) -> Rules {
        self.generator.rules()
    }

    #[inline]
    pub const fn ply_count(&self) -> u32 {
        self.adjudication.ply()
    }

    /// [`parse_sfen`](crate::parse_sfen)などで得た任意局面から対局を構築する。
    ///
    /// 王駒の存在や駒種ごとの枚数上限など、規則上の局面合法性は検証しない。
    /// 渡された局面は反復履歴上の第1回として記録する。
    ///
    /// # エラー
    ///
    /// R1、R2またはR3が指定されていなければ
    /// [`GameBuildError::MissingRepetitionRule`]を返す。
    pub fn from_position(rules: Rules, position: Position) -> Result<Self, GameBuildError> {
        let repetition_rule = rules
            .repetition_rule()
            .ok_or(GameBuildError::MissingRepetitionRule)?;
        let adjudication = AdjudicationState::new(repetition_rule, &position);
        let search_key = position.zobrist() ^ position.rights_zobrist();
        Ok(Self {
            position,
            generator: MoveGenerator::new(rules),
            adjudication,
            search_key_history: vec![search_key],
            result: None,
        })
    }

    fn ensure_ongoing(&self) -> Result<(), GameError> {
        if self.result.is_some() {
            Err(GameError::GameAlreadyOver)
        } else {
            Ok(())
        }
    }

    fn finish(&mut self, result: GameResult) -> GameStatus {
        debug_assert!(self.result.is_none());
        self.result = Some(result);
        GameStatus::Finished(result)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::core::adjudication::move_was_attacking;
    use crate::core::piece::{PieceCode, PieceKind};
    use crate::core::position::PositionBuilder;
    use crate::core::repetition::{R1History, R2History, R3History};
    use crate::core::rules::{RepetitionRule, RuleCode};
    use crate::core::square::Square;
    use crate::parse_sfen;
    use crate::rng::XorShift64;
    use crate::test_util::{position_from_codes as position, sq};

    fn piece(color: Color, kind: PieceKind) -> PieceCode {
        PieceCode::new(color, kind)
    }

    fn prince(color: Color) -> PieceCode {
        PieceCode::new_promoted(color, PieceKind::CrownPrince).unwrap()
    }

    fn game(position: Position) -> Game {
        Game::from_position(Rules::engine_default(), position).unwrap()
    }

    fn game_with_codes(position: Position, codes: &[RuleCode]) -> Game {
        Game::from_position(Rules::from_codes(codes).unwrap(), position).unwrap()
    }

    fn piece_exhaustion_game(position: Position) -> Game {
        game_with_codes(position, &[RuleCode::R1])
    }

    fn bare_king_game(position: Position) -> Game {
        game_with_codes(position, &[RuleCode::R1, RuleCode::E1, RuleCode::E3])
    }

    fn r2_game(position: Position) -> Game {
        game_with_codes(position, &[RuleCode::R2, RuleCode::E2])
    }

    fn r3_game(position: Position) -> Game {
        game_with_codes(position, &[RuleCode::R3, RuleCode::E2])
    }

    fn r1_history(game: &Game) -> &R1History {
        let RepetitionHistory::R1(history) = game.adjudication.repetition() else {
            panic!("game must use R1 history");
        };
        history
    }

    fn r1_history_mut(game: &mut Game) -> &mut R1History {
        let RepetitionHistory::R1(history) = game.adjudication.repetition_mut() else {
            panic!("game must use R1 history");
        };
        history
    }

    fn r2_history(game: &Game) -> &R2History {
        let RepetitionHistory::R2(history) = game.adjudication.repetition() else {
            panic!("game must use R2 history");
        };
        history
    }

    fn r3_history(game: &Game) -> &R3History {
        let RepetitionHistory::R3(history) = game.adjudication.repetition() else {
            panic!("game must use R3 history");
        };
        history
    }

    fn step(from: Square, to: Square) -> Move {
        Move {
            from,
            mid: None,
            to,
            promote: false,
        }
    }

    #[test]
    fn illegal_move_error_classifies_movement_and_preserves_cause_chain() {
        let illegal = step(sq(5, 5), sq(5, 6));
        let mut game = Game::with_default_rules();
        let error = game.play(illegal).unwrap_err();

        assert_eq!(
            error,
            GameError::IllegalMove {
                mv: illegal,
                cause: IllegalMoveCause::Movement,
            }
        );
        assert_eq!(error.to_string(), IllegalMove(illegal).to_string());
        assert_eq!(
            std::error::Error::source(&error).map(ToString::to_string),
            Some(IllegalMoveCause::Movement.to_string())
        );

        let repetition = GameError::IllegalMove {
            mv: illegal,
            cause: IllegalMoveCause::Repetition,
        };
        assert!(repetition.to_string().contains("repetition"));
        assert_eq!(
            std::error::Error::source(&repetition).map(ToString::to_string),
            Some(IllegalMoveCause::Repetition.to_string())
        );
    }

    fn mate_predecessor() -> (Position, Move) {
        let mv = step(sq(10, 8), sq(10, 9));
        (
            position(
                Color::White,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(0, 11), piece(Color::White, PieceKind::Rook)),
                    (sq(11, 0), piece(Color::White, PieceKind::Rook)),
                    (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
                    (sq(10, 8), piece(Color::White, PieceKind::King)),
                ],
            ),
            mv,
        )
    }

    #[test]
    fn game_construction_requires_a_repetition_rule() {
        assert!(matches!(
            Game::new(Rules::standard()),
            Err(GameBuildError::MissingRepetitionRule)
        ));
        assert!(matches!(
            Game::from_position(Rules::standard(), Position::initial()),
            Err(GameBuildError::MissingRepetitionRule)
        ));
        assert!(matches!(
            Game::new(Rules::from_codes(&[RuleCode::E2]).unwrap()),
            Err(GameBuildError::MissingRepetitionRule)
        ));
    }

    #[test]
    fn articles_22_and_32_r1_without_e2_allows_game_construction() {
        let game = Game::new(Rules::from_codes(&[RuleCode::R1]).unwrap()).unwrap();

        assert_eq!(game.status(), GameStatus::Ongoing);
        assert_eq!(game.position(), &Position::initial());
        assert_eq!(game.ply_count(), 0);
    }

    #[test]
    fn articles_25_and_31_r2_allows_game_construction() {
        let rules = Rules::from_codes(&[RuleCode::R2]).unwrap();
        let game = Game::new(rules).unwrap();

        assert_eq!(rules.repetition_rule(), Some(RepetitionRule::R2));
        assert_eq!(game.status(), GameStatus::Ongoing);
        assert_eq!(game.position(), &Position::initial());
        assert_eq!(game.ply_count(), 0);
    }

    #[test]
    fn articles_24_1_d_and_31_r2_r3_choose_the_correct_repetition_key() {
        let deferred = sq(4, 9);
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(
                deferred,
                PieceCode::new(Color::Black, PieceKind::SilverGeneral),
            )
            .unwrap();
        builder.mark_promotion_deferred(deferred).unwrap();
        let position = builder.finish().unwrap();

        let r1 = Game::from_position(
            Rules::from_codes(&[RuleCode::P1, RuleCode::R1, RuleCode::E2]).unwrap(),
            position.clone(),
        )
        .unwrap();
        let r2 = Game::from_position(
            Rules::from_codes(&[RuleCode::P1, RuleCode::R2, RuleCode::E2]).unwrap(),
            position.clone(),
        )
        .unwrap();
        let r3 = Game::from_position(
            Rules::from_codes(&[RuleCode::P1, RuleCode::R3, RuleCode::E2]).unwrap(),
            position.clone(),
        )
        .unwrap();

        assert_eq!(r1_history(&r1).occurrences(&position), Some(1));
        assert!(r2_history(&r2).contains(&position));
        assert_eq!(r3_history(&r3).occurrences(&position), Some(1));

        let search_key = position.zobrist() ^ position.rights_zobrist();
        assert_ne!(position.rights_zobrist(), 0);
        assert_eq!(r1.search_key_history(), &[search_key]);
        assert_eq!(r2.search_key_history(), &[search_key]);
        assert_eq!(r3.search_key_history(), &[search_key]);
    }

    #[test]
    fn engine_default_rules_allow_game_construction() {
        let game = Game::new(Rules::engine_default()).unwrap();
        assert!(matches!(
            game.adjudication.repetition(),
            RepetitionHistory::R1(_)
        ));
        assert_eq!(game.status(), GameStatus::Ongoing);
        assert_eq!(game.position(), &Position::initial());
        assert_eq!(game.ply_count(), 0);
    }

    #[test]
    fn search_key_history_contains_the_start_and_each_accepted_move() {
        let mut game = Game::with_default_rules();
        let initial_key = game.position().zobrist() ^ game.position().rights_zobrist();
        assert_eq!(game.search_key_history(), &[initial_key]);

        let illegal = step(sq(5, 5), sq(5, 6));
        assert!(game.play(illegal).is_err());
        assert_eq!(game.search_key_history(), &[initial_key]);

        let mv = game.legal_moves()[0];
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        let current_key = game.position().zobrist() ^ game.position().rights_zobrist();
        assert_eq!(game.search_key_history(), &[initial_key, current_key]);
    }

    #[test]
    fn cloned_games_can_be_played_independently() {
        let mut original = Game::with_default_rules();
        let moves = original.legal_moves();
        let original_move = moves[0];
        let clone_move = moves[1];
        let initial = original.position().clone();
        let mut cloned = original.clone();

        assert_eq!(cloned.play(clone_move), Ok(GameStatus::Ongoing));
        assert_eq!(original.position(), &initial);

        let cloned_position = cloned.position().clone();
        assert_eq!(original.play(original_move), Ok(GameStatus::Ongoing));
        assert_eq!(cloned.position(), &cloned_position);
        assert_ne!(original.position(), cloned.position());
    }

    #[test]
    fn articles_26_11_and_26_12_legal_moves_match_generator_and_stop_after_game_end() {
        let mut game = Game::new(Rules::engine_default()).unwrap();
        let mut generated = Vec::new();
        game.generator
            .generate_moves(game.position(), &mut generated);

        assert_eq!(
            game.legal_moves().into_iter().collect::<HashSet<_>>(),
            generated.into_iter().collect::<HashSet<_>>()
        );

        assert!(matches!(
            game.agree_draw(),
            Ok(GameStatus::Finished(GameResult::Draw {
                reason: DrawReason::Agreement,
            }))
        ));
        assert!(game.legal_moves().is_empty());
    }

    #[test]
    fn from_position_accepts_parsed_sfen_and_rules_returns_adopted_rules() {
        let position = parse_sfen("12/12/12/8k3/12/12/12/12/3K8/12/12/12 b").unwrap();
        let rules = Rules::from_codes(&[RuleCode::R1, RuleCode::E2]).unwrap();
        let mut game = Game::from_position(rules, position).unwrap();

        assert_eq!(game.rules(), rules);
        assert_eq!(game.play(step(sq(3, 3), sq(3, 4))), Ok(GameStatus::Ongoing));
    }

    #[test]
    fn article_32_e1_disables_mate_adjudication() {
        let (position, mv) = mate_predecessor();
        let mut game = game_with_codes(position, &[RuleCode::R1, RuleCode::E1, RuleCode::E2]);

        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        assert_eq!(game.result(), None);
        assert_eq!(game.ply_count(), 1);
    }

    #[test]
    fn article_21_1_capturing_the_last_royal_wins() {
        let mut game = game(position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(5, 5), piece(Color::Black, PieceKind::Rook)),
                (sq(5, 8), piece(Color::White, PieceKind::King)),
            ],
        ));

        let result = GameResult::Win {
            winner: Color::Black,
            reason: WinReason::RoyalCapture,
        };
        assert_eq!(
            game.play(step(sq(5, 5), sq(5, 8))),
            Ok(GameStatus::Finished(result))
        );
        assert_eq!(game.result(), Some(result));
    }

    #[test]
    fn article_21_5_lion_double_capture_of_both_royals_wins() {
        let mut game = game(position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(5, 5), piece(Color::Black, PieceKind::Lion)),
                (sq(5, 6), piece(Color::White, PieceKind::King)),
                (sq(6, 6), prince(Color::White)),
            ],
        ));
        let double_capture = Move {
            from: sq(5, 5),
            mid: Some(sq(5, 6)),
            to: sq(6, 6),
            promote: false,
        };

        assert_eq!(
            game.play(double_capture),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::RoyalCapture,
            }))
        );
        assert!(game.position().royal_pieces(Color::White).is_empty());
    }

    #[test]
    fn article_23_side_with_no_legal_move_loses_through_play() {
        let mut game = game(position(
            Color::White,
            &[
                (sq(0, 11), piece(Color::Black, PieceKind::King)),
                (sq(1, 11), piece(Color::Black, PieceKind::Pawn)),
                (sq(0, 10), piece(Color::Black, PieceKind::Lance)),
                (sq(1, 10), piece(Color::Black, PieceKind::Pawn)),
                (sq(10, 0), piece(Color::White, PieceKind::King)),
            ],
        ));

        assert_eq!(
            game.play(step(sq(10, 0), sq(11, 0))),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::White,
                reason: WinReason::Stalemate,
            }))
        );
    }

    #[test]
    fn articles_21_2_and_21_3_mate_is_adjudicated_through_play() {
        let (position, mv) = mate_predecessor();
        let mut game = game(position);

        assert_eq!(
            game.play(mv),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::White,
                reason: WinReason::Mate,
            }))
        );
    }

    #[test]
    fn shared_adjudication_matches_game_play_for_the_same_position_and_history() {
        let (position, mv) = mate_predecessor();
        let rules = Rules::from_codes(&[RuleCode::R1, RuleCode::E2]).unwrap();
        let mut shared = Game::from_position(rules, position.clone()).unwrap();
        let mut through_play = Game::from_position(rules, position).unwrap();

        let mover = shared.position.side_to_move();
        let undo = shared
            .position
            .try_make_move(mv, &shared.generator)
            .unwrap();
        assert!(!repetition_is_forbidden(
            shared.adjudication.repetition(),
            &shared.position
        ));
        let promoted_waiting_piece = promoted_waiting_square(mv, &undo);
        let repetition_result =
            shared
                .adjudication
                .record_move(&shared.position, &shared.generator, mover, mv);
        let outcome = adjudicate_after_move(
            &mut shared.position,
            AdjudicationContext::new(&shared.adjudication, &shared.generator),
            mover,
            promoted_waiting_piece,
            repetition_result,
        );
        shared
            .adjudication
            .set_piece_exhaustion_grace(outcome.piece_exhaustion_grace());

        let status = through_play.play(mv).unwrap();
        assert_eq!(status, GameStatus::Finished(outcome.result().unwrap()));
        assert_eq!(shared.position, through_play.position);
        assert_eq!(shared.adjudication, through_play.adjudication);
    }

    #[test]
    fn all_repetition_and_exception_combinations_preserve_adjudication_results() {
        for repetition in [RuleCode::R1, RuleCode::R2, RuleCode::R3] {
            for e1 in [false, true] {
                for e2 in [false, true] {
                    let mut codes = vec![repetition];
                    if e1 {
                        codes.push(RuleCode::E1);
                    }
                    if e2 {
                        codes.push(RuleCode::E2);
                    }

                    let mut capture = game_with_codes(
                        position(
                            Color::Black,
                            &[
                                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                                (sq(5, 5), piece(Color::Black, PieceKind::Rook)),
                                (sq(5, 8), piece(Color::White, PieceKind::King)),
                            ],
                        ),
                        &codes,
                    );
                    assert_eq!(
                        capture.play(step(sq(5, 5), sq(5, 8))),
                        Ok(GameStatus::Finished(GameResult::Win {
                            winner: Color::Black,
                            reason: WinReason::RoyalCapture,
                        })),
                        "codes={codes:?}"
                    );

                    let (mate_position, mate_move) = mate_predecessor();
                    let mut mate = game_with_codes(mate_position, &codes);
                    let expected_mate = if e1 {
                        GameStatus::Ongoing
                    } else {
                        GameStatus::Finished(GameResult::Win {
                            winner: Color::White,
                            reason: WinReason::Mate,
                        })
                    };
                    assert_eq!(mate.play(mate_move), Ok(expected_mate), "codes={codes:?}");

                    let mut exhaustion = game_with_codes(
                        position(
                            Color::Black,
                            &[
                                (sq(4, 4), piece(Color::Black, PieceKind::King)),
                                (sq(4, 5), piece(Color::White, PieceKind::Pawn)),
                                (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
                                (sq(11, 11), piece(Color::White, PieceKind::King)),
                            ],
                        ),
                        &codes,
                    );
                    let expected_exhaustion = if e2 {
                        GameStatus::Ongoing
                    } else {
                        GameStatus::Finished(GameResult::Win {
                            winner: Color::White,
                            reason: WinReason::PieceExhaustion,
                        })
                    };
                    assert_eq!(
                        exhaustion.play(step(sq(4, 4), sq(4, 5))),
                        Ok(expected_exhaustion),
                        "codes={codes:?}"
                    );

                    let mut repeated = game_with_codes(
                        position(
                            Color::Black,
                            &[
                                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                                (sq(3, 3), piece(Color::Black, PieceKind::GoldGeneral)),
                                (sq(11, 11), piece(Color::White, PieceKind::King)),
                                (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
                            ],
                        ),
                        &codes,
                    );
                    let cycle = [
                        step(sq(3, 3), sq(3, 4)),
                        step(sq(8, 8), sq(8, 7)),
                        step(sq(3, 4), sq(3, 3)),
                        step(sq(8, 7), sq(8, 8)),
                    ];
                    let accepted_plies = match repetition {
                        RuleCode::R1 | RuleCode::R3 => 11,
                        RuleCode::R2 => 3,
                        _ => unreachable!(),
                    };
                    for ply in 0..accepted_plies {
                        assert_eq!(
                            repeated.play(cycle[ply % cycle.len()]),
                            Ok(GameStatus::Ongoing),
                            "codes={codes:?}, ply={} ",
                            ply + 1
                        );
                    }
                    let terminal_move = cycle[accepted_plies % cycle.len()];
                    let expected_repetition = match repetition {
                        RuleCode::R1 => Ok(GameStatus::Finished(GameResult::Draw {
                            reason: DrawReason::Repetition,
                        })),
                        RuleCode::R2 | RuleCode::R3 => Err(GameError::IllegalMove {
                            mv: terminal_move,
                            cause: IllegalMoveCause::Repetition,
                        }),
                        _ => unreachable!(),
                    };
                    assert_eq!(
                        repeated.play(terminal_move),
                        expected_repetition,
                        "codes={codes:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn article_21_3_c_piece_exhaustion_win_prevents_mate() {
        let mut game = game_with_codes(
            position(
                Color::White,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(1, 0), piece(Color::Black, PieceKind::Lance)),
                    (sq(0, 2), piece(Color::White, PieceKind::King)),
                    (sq(5, 1), piece(Color::White, PieceKind::Rook)),
                ],
            ),
            &[RuleCode::R1],
        );

        assert_eq!(game.play(step(sq(5, 1), sq(0, 1))), Ok(GameStatus::Ongoing));
        assert!(!game.adjudication.piece_exhaustion_grace());
        assert_eq!(
            game.play(step(sq(0, 0), sq(0, 1))),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::PieceExhaustion,
            }))
        );
    }

    #[test]
    fn article_21_3_c_repetition_win_prevents_mate() {
        let (position, mv) = mate_predecessor();
        let reply = step(sq(0, 0), sq(0, 1));
        let rules = Rules::from_codes(&[RuleCode::R1, RuleCode::E2]).unwrap();
        let mut repeated_position = position.clone();
        repeated_position.make_move_unchecked(mv, rules);
        repeated_position.make_move_unchecked(reply, rules);
        let mut game = Game::from_position(rules, position).unwrap();

        // This synthetic history makes Black's reply create the fourth occurrence.
        r1_history_mut(&mut game).set_state(&repeated_position, 3, 0);
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        assert_eq!(r1_history(&game).consecutive_attacking_moves(), [0, 1]);
        assert_eq!(r1_history(&game).occurrences(&repeated_position), Some(3));
        assert_eq!(
            game.play(reply),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::Repetition,
            }))
        );
        assert_eq!(r1_history(&game).occurrences(&repeated_position), Some(4));
    }

    #[test]
    fn article_21_3_c_non_attacking_repetition_reply_resets_counter_and_prevents_mate() {
        let (position, mv) = mate_predecessor();
        let reply = step(sq(0, 0), sq(0, 1));
        let rules = Rules::from_codes(&[RuleCode::R1, RuleCode::E2]).unwrap();
        let mut repeated_position = position.clone();
        repeated_position.make_move_unchecked(mv, rules);
        repeated_position.make_move_unchecked(reply, rules);
        let mut game = Game::from_position(rules, position).unwrap();

        assert!(!move_was_attacking(
            &repeated_position,
            &game.generator,
            Color::Black,
            reply
        ));
        r1_history_mut(&mut game).set_state(&repeated_position, 3, 0);
        r1_history_mut(&mut game).set_attacking_counters([1, 0]);

        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        assert_eq!(r1_history(&game).consecutive_attacking_moves(), [1, 1]);
        assert_eq!(r1_history(&game).occurrences(&repeated_position), Some(3));
        assert_eq!(
            game.play(reply),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::Repetition,
            }))
        );
        assert_eq!(r1_history(&game).consecutive_attacking_moves(), [0, 1]);
        assert_eq!(r1_history(&game).occurrences(&repeated_position), Some(4));
    }

    #[test]
    fn article_22_1_extra_piece_side_to_move_wins_immediately() {
        let mut game = piece_exhaustion_game(position(
            Color::Black,
            &[
                (sq(4, 4), piece(Color::Black, PieceKind::King)),
                (sq(4, 5), piece(Color::White, PieceKind::Pawn)),
                (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));

        assert_eq!(
            game.play(step(sq(4, 4), sq(4, 5))),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::White,
                reason: WinReason::PieceExhaustion,
            }))
        );
    }

    #[test]
    fn article_22_1_royal_only_side_without_capture_loses_immediately() {
        let mut game = piece_exhaustion_game(position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(4, 4), piece(Color::Black, PieceKind::GoldGeneral)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));

        assert_eq!(
            game.play(step(sq(4, 4), sq(4, 5))),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::PieceExhaustion,
            }))
        );
    }

    #[test]
    fn article_22_2_pawn_waits_and_promotion_wins_without_grace() {
        let mut game = piece_exhaustion_game(position(
            Color::White,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(4, 10), piece(Color::Black, PieceKind::Pawn)),
                (sq(3, 9), piece(Color::White, PieceKind::King)),
            ],
        ));

        assert_eq!(
            game.play(step(sq(3, 9), sq(3, 10))),
            Ok(GameStatus::Ongoing)
        );
        assert_eq!(
            game.play(Move {
                from: sq(4, 10),
                mid: None,
                to: sq(4, 11),
                promote: true,
            }),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::PieceExhaustion,
            }))
        );

        let mut responses = Vec::new();
        game.generator
            .generate_moves(game.position(), &mut responses);
        assert!(responses.contains(&step(sq(3, 10), sq(4, 11))));
    }

    #[test]
    fn article_22_3_go_between_waits_and_promotion_wins_without_grace() {
        let mut game = piece_exhaustion_game(position(
            Color::White,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(4, 7), piece(Color::Black, PieceKind::GoBetween)),
                (sq(3, 7), piece(Color::White, PieceKind::King)),
            ],
        ));

        assert_eq!(game.play(step(sq(3, 7), sq(3, 8))), Ok(GameStatus::Ongoing));
        assert_eq!(
            game.play(Move {
                from: sq(4, 7),
                mid: None,
                to: sq(4, 8),
                promote: true,
            }),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::PieceExhaustion,
            }))
        );

        let mut responses = Vec::new();
        game.generator
            .generate_moves(game.position(), &mut responses);
        assert!(responses.contains(&step(sq(3, 8), sq(4, 8))));
    }

    #[test]
    fn articles_22_1_and_22_5_promotion_with_capture_takes_the_grace_path() {
        let capturing_promotion = Move {
            from: sq(5, 9),
            mid: None,
            to: sq(5, 10),
            promote: true,
        };
        let condition_arises = |first_reply: Move| {
            let mut game = piece_exhaustion_game(position(
                Color::Black,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(5, 9), piece(Color::Black, PieceKind::Pawn)),
                    (sq(5, 10), piece(Color::White, PieceKind::GoldGeneral)),
                    (sq(4, 11), piece(Color::White, PieceKind::King)),
                ],
            ));
            assert_eq!(game.play(capturing_promotion), Ok(GameStatus::Ongoing));
            assert!(game.adjudication.piece_exhaustion_grace());
            game.play(first_reply)
        };

        assert_eq!(
            condition_arises(step(sq(4, 11), sq(5, 10))),
            Ok(GameStatus::Finished(GameResult::Draw {
                reason: DrawReason::PieceExhaustion,
            }))
        );
        assert_eq!(
            condition_arises(step(sq(4, 11), sq(3, 11))),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::PieceExhaustion,
            }))
        );
    }

    #[test]
    fn article_22_4_immobile_pawn_and_lance_do_not_trigger_adjudication() {
        for kind in [PieceKind::Pawn, PieceKind::Lance] {
            let mut game = piece_exhaustion_game(position(
                Color::White,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(4, 11), piece(Color::Black, kind)),
                    (sq(11, 11), piece(Color::White, PieceKind::King)),
                ],
            ));

            assert_eq!(
                game.play(step(sq(11, 11), sq(10, 11))),
                Ok(GameStatus::Ongoing),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn articles_22_3_22_4_and_30_p3_p4_keep_piece_exhaustion_conditions() {
        let mut p3_game = game_with_codes(
            position(
                Color::White,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(4, 11), piece(Color::Black, PieceKind::Lance)),
                    (sq(11, 11), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::P3, RuleCode::R1],
        );
        // 第30条P3は成る機会だけを追加する。最奥段で不成を選んだ香車は、
        // P3採用後も第22条4項の移動不能な余分の駒であり、勝利を成立させない。
        assert_eq!(
            p3_game.play(step(sq(11, 11), sq(10, 11))),
            Ok(GameStatus::Ongoing)
        );

        let mut p4_game = game_with_codes(
            position(
                Color::White,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(4, 10), piece(Color::Black, PieceKind::GoBetween)),
                    (sq(3, 10), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::P4, RuleCode::R1],
        );
        assert_eq!(
            p4_game.play(step(sq(3, 10), sq(3, 11))),
            Ok(GameStatus::Ongoing)
        );
        assert_eq!(
            p4_game.play(Move {
                from: sq(4, 10),
                mid: None,
                to: sq(4, 11),
                promote: true,
            }),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::PieceExhaustion,
            }))
        );
        // 第30条P4は仲人が醉象へ成る経路を増やすが、勝利条件自体は第22条3項のまま。
        // 成った時点で成立するため、白王が次手で(4,11)を取れる配置でも猶予を与えない。
    }

    fn article_22_grace_predecessor() -> (Position, Move) {
        (
            position(
                Color::Black,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(5, 5), piece(Color::Black, PieceKind::GoldGeneral)),
                    (sq(4, 7), piece(Color::White, PieceKind::King)),
                    (sq(5, 6), piece(Color::White, PieceKind::Pawn)),
                ],
            ),
            step(sq(5, 5), sq(5, 6)),
        )
    }

    #[test]
    fn articles_22_5_22_6_and_22_8_grace_capture_draws() {
        let (position, establishes_condition) = article_22_grace_predecessor();
        let mut game = piece_exhaustion_game(position);

        assert_eq!(game.play(establishes_condition), Ok(GameStatus::Ongoing));
        assert_eq!(
            game.play(step(sq(4, 7), sq(5, 6))),
            Ok(GameStatus::Finished(GameResult::Draw {
                reason: DrawReason::PieceExhaustion,
            }))
        );
        assert!(!game.adjudication.piece_exhaustion_grace());
    }

    #[test]
    fn articles_22_5_and_22_7_declining_grace_loses() {
        let (position, establishes_condition) = article_22_grace_predecessor();
        let mut game = piece_exhaustion_game(position);

        assert_eq!(game.play(establishes_condition), Ok(GameStatus::Ongoing));
        assert_eq!(
            game.play(step(sq(4, 7), sq(3, 7))),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::PieceExhaustion,
            }))
        );
    }

    #[test]
    fn article_22_8_only_royals_is_a_draw() {
        let expected = Ok(GameStatus::Finished(GameResult::Draw {
            reason: DrawReason::PieceExhaustion,
        }));
        let mut kings = piece_exhaustion_game(position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(kings.play(step(sq(0, 0), sq(0, 1))), expected);
    }

    #[test]
    fn article_22_8_asymmetric_royals_continue_the_game() {
        let mut prince_and_kings = piece_exhaustion_game(position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(2, 0), prince(Color::Black)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(
            prince_and_kings.play(step(sq(0, 0), sq(0, 1))),
            Ok(GameStatus::Ongoing)
        );
    }

    #[test]
    fn article_32_e2_disables_piece_exhaustion_adjudication() {
        let e2_game = |position| game_with_codes(position, &[RuleCode::R1, RuleCode::E2]);

        let mut only_royals = e2_game(position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(
            only_royals.play(step(sq(0, 0), sq(0, 1))),
            Ok(GameStatus::Ongoing)
        );

        let (grace_position, establishes_condition) = article_22_grace_predecessor();
        let mut condition_c = e2_game(grace_position);
        assert_eq!(
            condition_c.play(establishes_condition),
            Ok(GameStatus::Ongoing)
        );
        assert!(!condition_c.adjudication.piece_exhaustion_grace());

        let mut immediate_win = e2_game(position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(4, 4), piece(Color::Black, PieceKind::GoldGeneral)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(
            immediate_win.play(step(sq(4, 4), sq(4, 5))),
            Ok(GameStatus::Ongoing)
        );
    }

    #[test]
    fn article_32_e3_bare_king_loses_when_all_conditions_hold() {
        let mut game = bare_king_game(position(
            Color::Black,
            &[
                (sq(5, 4), piece(Color::Black, PieceKind::King)),
                (sq(9, 9), piece(Color::White, PieceKind::Rook)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));

        assert_eq!(
            game.play(step(sq(5, 4), sq(5, 5))),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::White,
                reason: WinReason::BareKing,
            }))
        );
    }

    #[test]
    fn article_32_e3_king_and_pawn_are_not_enough_effective_material() {
        let mut game = bare_king_game(position(
            Color::Black,
            &[
                (sq(5, 4), piece(Color::Black, PieceKind::King)),
                (sq(8, 8), piece(Color::White, PieceKind::Pawn)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));

        assert_eq!(game.play(step(sq(5, 4), sq(5, 5))), Ok(GameStatus::Ongoing));
    }

    #[test]
    fn article_32_e3_bare_king_check_defers_the_loss() {
        let mut game = bare_king_game(position(
            Color::Black,
            &[
                (sq(5, 4), piece(Color::Black, PieceKind::King)),
                (sq(0, 10), piece(Color::White, PieceKind::Rook)),
                (sq(6, 6), piece(Color::White, PieceKind::King)),
                (sq(10, 0), piece(Color::White, PieceKind::Rook)),
            ],
        ));

        assert_eq!(game.play(step(sq(5, 4), sq(5, 5))), Ok(GameStatus::Ongoing));
    }

    #[test]
    fn article_32_e3_adjacency_defers_two_effective_pieces_but_not_three() {
        let pieces = [
            (sq(5, 4), piece(Color::Black, PieceKind::King)),
            (sq(6, 5), piece(Color::White, PieceKind::GoldGeneral)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ];
        let mut two_effective_pieces = bare_king_game(position(Color::Black, &pieces));
        assert_eq!(
            two_effective_pieces.play(step(sq(5, 4), sq(5, 5))),
            Ok(GameStatus::Ongoing)
        );

        let mut pieces = pieces.to_vec();
        pieces.push((sq(9, 0), piece(Color::White, PieceKind::Bishop)));
        let mut three_effective_pieces = bare_king_game(position(Color::Black, &pieces));
        assert_eq!(
            three_effective_pieces.play(step(sq(5, 4), sq(5, 5))),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::White,
                reason: WinReason::BareKing,
            }))
        );
    }

    #[test]
    fn article_32_e3_two_unchecked_bare_kings_draw() {
        let mut game = bare_king_game(position(
            Color::Black,
            &[
                (sq(5, 4), piece(Color::Black, PieceKind::King)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));

        assert_eq!(
            game.play(step(sq(5, 4), sq(5, 5))),
            Ok(GameStatus::Finished(GameResult::Draw {
                reason: DrawReason::BareKing,
            }))
        );
    }

    #[test]
    fn article_32_e3_checked_bare_kings_continue() {
        let mut game = bare_king_game(position(
            Color::Black,
            &[
                (sq(5, 4), piece(Color::Black, PieceKind::King)),
                (sq(6, 6), piece(Color::White, PieceKind::King)),
            ],
        ));

        assert_eq!(game.play(step(sq(5, 4), sq(5, 5))), Ok(GameStatus::Ongoing));
    }

    #[test]
    fn article_32_e3_dead_pawns_and_lances_are_excluded_from_counts() {
        let mut bare_side_dead_pieces = bare_king_game(position(
            Color::Black,
            &[
                (sq(5, 4), piece(Color::Black, PieceKind::King)),
                (sq(3, 11), piece(Color::Black, PieceKind::Pawn)),
                (sq(4, 11), piece(Color::Black, PieceKind::Lance)),
                (sq(9, 9), piece(Color::White, PieceKind::Rook)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(
            bare_side_dead_pieces.play(step(sq(5, 4), sq(5, 5))),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::White,
                reason: WinReason::BareKing,
            }))
        );

        let mut opponent_dead_pieces = bare_king_game(position(
            Color::Black,
            &[
                (sq(5, 4), piece(Color::Black, PieceKind::King)),
                (sq(3, 0), piece(Color::White, PieceKind::Pawn)),
                (sq(4, 0), piece(Color::White, PieceKind::Lance)),
                (sq(6, 5), piece(Color::White, PieceKind::Rook)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(
            opponent_dead_pieces.play(step(sq(5, 4), sq(5, 5))),
            Ok(GameStatus::Ongoing)
        );

        let mut both_sides_dead_pieces = bare_king_game(position(
            Color::Black,
            &[
                (sq(5, 4), piece(Color::Black, PieceKind::King)),
                (sq(3, 11), piece(Color::Black, PieceKind::Pawn)),
                (sq(4, 11), piece(Color::Black, PieceKind::Lance)),
                (sq(3, 0), piece(Color::White, PieceKind::Pawn)),
                (sq(4, 0), piece(Color::White, PieceKind::Lance)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(
            both_sides_dead_pieces.play(step(sq(5, 4), sq(5, 5))),
            Ok(GameStatus::Finished(GameResult::Draw {
                reason: DrawReason::BareKing,
            }))
        );
    }

    #[test]
    fn article_31_r1_fourth_repetition_is_a_draw_at_ply_12() {
        let mut game = game(position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(3, 3), piece(Color::Black, PieceKind::GoldGeneral)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
                (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
            ],
        ));
        let cycle = [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
            step(sq(8, 7), sq(8, 8)),
        ];

        for ply in 1..=12 {
            let status = game.play(cycle[(ply - 1) % cycle.len()]).unwrap();
            if ply < 12 {
                assert_eq!(status, GameStatus::Ongoing, "ended at ply {ply}");
            } else {
                assert_eq!(
                    status,
                    GameStatus::Finished(GameResult::Draw {
                        reason: DrawReason::Repetition,
                    })
                );
            }
        }
        assert_eq!(game.ply_count(), 12);
        assert_eq!(r1_history(&game).consecutive_attacking_moves(), [0, 0]);
    }

    #[test]
    fn article_31_r1_continuous_attacker_loses_on_fourth_repetition() {
        let mut game = game(position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(3, 3), piece(Color::Black, PieceKind::FreeKing)),
                (sq(3, 10), piece(Color::White, PieceKind::King)),
                (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
            ],
        ));
        let cycle = [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
            step(sq(8, 7), sq(8, 8)),
        ];

        for ply in 1..=12 {
            let status = game.play(cycle[(ply - 1) % cycle.len()]).unwrap();
            if ply < 12 {
                assert_eq!(status, GameStatus::Ongoing, "ended at ply {ply}");
            } else {
                assert_eq!(
                    status,
                    GameStatus::Finished(GameResult::Win {
                        winner: Color::White,
                        reason: WinReason::Repetition,
                    })
                );
            }
        }
        assert_eq!(r1_history(&game).consecutive_attacking_moves(), [6, 0]);
    }

    #[test]
    fn article_26_11_r2_rejects_a_repeated_position_without_changing_game_state() {
        let mut game = r2_game(position(
            Color::Black,
            &[
                (sq(3, 3), piece(Color::Black, PieceKind::King)),
                (sq(8, 8), piece(Color::White, PieceKind::King)),
            ],
        ));
        let first_three = [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
        ];
        for mv in first_three {
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        }

        let repeated = step(sq(8, 7), sq(8, 8));
        let position_before = game.position().clone();
        let adjudication_before = game.adjudication.clone();
        assert_eq!(
            game.play(repeated),
            Err(GameError::IllegalMove {
                mv: repeated,
                cause: IllegalMoveCause::Repetition,
            })
        );
        assert_eq!(game.position(), &position_before);
        assert_eq!(game.ply_count(), 3);
        assert_eq!(game.status(), GameStatus::Ongoing);
        assert_eq!(game.adjudication, adjudication_before);

        assert_eq!(game.play(step(sq(8, 7), sq(9, 7))), Ok(GameStatus::Ongoing));
        assert_eq!(game.ply_count(), 4);
        assert!(matches!(
            game.adjudication.repetition(),
            RepetitionHistory::R2(_)
        ));
    }

    #[test]
    fn article_31_r3_allows_second_and_third_occurrences_but_rejects_fourth_without_changing_state()
    {
        let initial = position(
            Color::Black,
            &[
                (sq(3, 3), piece(Color::Black, PieceKind::King)),
                (sq(8, 8), piece(Color::White, PieceKind::King)),
            ],
        );
        let mut game = r3_game(initial.clone());
        let cycle = [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
            step(sq(8, 7), sq(8, 8)),
        ];

        for ply in 1..=11 {
            assert_eq!(
                game.play(cycle[(ply - 1) % cycle.len()]),
                Ok(GameStatus::Ongoing),
                "ended at ply {ply}"
            );
            if ply == 4 || ply == 8 {
                assert_eq!(game.position(), &initial);
                assert_eq!(
                    r3_history(&game).occurrences(&initial),
                    Some((ply / 4 + 1) as u8)
                );
            }
        }

        let fourth_occurrence = cycle[3];
        let mut generated = Vec::new();
        game.generator
            .generate_moves(game.position(), &mut generated);
        assert!(generated.contains(&fourth_occurrence));
        assert!(!game.legal_moves().contains(&fourth_occurrence));

        game.adjudication.set_piece_exhaustion_grace(true);
        let position_before = game.position().clone();
        let adjudication_before = game.adjudication.clone();
        let ply_before = game.ply_count();
        assert_eq!(
            game.play(fourth_occurrence),
            Err(GameError::IllegalMove {
                mv: fourth_occurrence,
                cause: IllegalMoveCause::Repetition,
            })
        );
        assert_eq!(game.position(), &position_before);
        assert_eq!(game.adjudication, adjudication_before);
        assert_eq!(game.ply_count(), ply_before);
        assert_eq!(game.status(), GameStatus::Ongoing);
        assert_eq!(r3_history(&game).occurrences(&initial), Some(3));
    }

    #[test]
    fn shared_r2_r3_filter_matches_game_legal_moves() {
        let initial = position(
            Color::Black,
            &[
                (sq(3, 3), piece(Color::Black, PieceKind::King)),
                (sq(8, 8), piece(Color::White, PieceKind::King)),
            ],
        );
        let cycle = [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
            step(sq(8, 7), sq(8, 8)),
        ];

        for (rule, plies) in [(RuleCode::R2, 3), (RuleCode::R3, 11)] {
            let mut game = game_with_codes(initial.clone(), &[rule, RuleCode::E2]);
            for ply in 0..plies {
                assert_eq!(game.play(cycle[ply % cycle.len()]), Ok(GameStatus::Ongoing));
            }

            let mut filtered = Vec::new();
            game.generator
                .generate_moves(game.position(), &mut filtered);
            assert!(filtered.contains(&cycle[3]));
            let mut probe = game.position().clone();
            retain_repetition_allowed_moves(
                &mut probe,
                &game.generator,
                game.adjudication.repetition(),
                &mut filtered,
            );

            assert_eq!(&probe, game.position());
            assert!(!filtered.contains(&cycle[3]));
            assert_eq!(
                filtered.into_iter().collect::<HashSet<_>>(),
                game.legal_moves().into_iter().collect::<HashSet<_>>(),
                "rule={rule:?}"
            );
        }
    }

    #[test]
    fn articles_26_11_and_27_4_legal_moves_exclude_r2_repeated_positions() {
        let mut game = r2_game(position(
            Color::Black,
            &[
                (sq(3, 3), piece(Color::Black, PieceKind::King)),
                (sq(8, 8), piece(Color::White, PieceKind::King)),
            ],
        ));
        for mv in [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
        ] {
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        }

        let repeated = step(sq(8, 7), sq(8, 8));
        let accepted = step(sq(8, 7), sq(9, 7));
        let legal = game.legal_moves();
        let mut generated = Vec::new();
        game.generator
            .generate_moves(game.position(), &mut generated);

        assert!(!legal.contains(&repeated));
        assert!(generated.contains(&repeated));
        assert!(legal.contains(&accepted));
        assert_eq!(game.play(accepted), Ok(GameStatus::Ongoing));
    }

    #[test]
    fn article_23_r2_history_filter_adjudicates_stalemate() {
        let pieces = r2_stalemate_pieces();
        for codes in [
            &[RuleCode::R2, RuleCode::E2][..],
            &[RuleCode::R2, RuleCode::E1, RuleCode::E2][..],
        ] {
            let mut game = game_with_codes(position(Color::White, &pieces), codes);
            assert_eq!(game.play(step(sq(5, 5), sq(5, 4))), Ok(GameStatus::Ongoing));
            assert_eq!(game.play(step(sq(0, 0), sq(0, 1))), Ok(GameStatus::Ongoing));
            assert_eq!(
                game.play(step(sq(5, 4), sq(5, 5))),
                Ok(GameStatus::Finished(GameResult::Win {
                    winner: Color::White,
                    reason: WinReason::Stalemate,
                })),
                "codes={codes:?}"
            );
        }
    }

    #[test]
    fn article_21_r2_forbidden_only_noncapture_escape_establishes_mate() {
        let pieces = [
            (sq(0, 1), piece(Color::Black, PieceKind::King)),
            (sq(11, 0), piece(Color::White, PieceKind::Rook)),
            (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
            (sq(10, 9), piece(Color::White, PieceKind::King)),
        ];
        let sequence = [
            step(sq(10, 9), sq(10, 8)),
            step(sq(0, 1), sq(0, 0)),
            step(sq(10, 8), sq(10, 9)),
        ];

        let mut r1 = game_with_codes(
            position(Color::White, &pieces),
            &[RuleCode::R1, RuleCode::E2],
        );
        for mv in sequence {
            assert_eq!(r1.play(mv), Ok(GameStatus::Ongoing));
        }

        let mut r2 = r2_game(position(Color::White, &pieces));
        assert_eq!(r2.play(sequence[0]), Ok(GameStatus::Ongoing));
        assert_eq!(r2.play(sequence[1]), Ok(GameStatus::Ongoing));
        assert_eq!(
            r2.play(sequence[2]),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::White,
                reason: WinReason::Mate,
            }))
        );
    }

    #[test]
    fn plan_game_6_capture_escape_is_never_filtered_by_r2_history() {
        let mut game = r2_game(position(
            Color::White,
            &[
                (sq(0, 1), piece(Color::Black, PieceKind::King)),
                (sq(1, 0), piece(Color::White, PieceKind::King)),
                (sq(10, 9), piece(Color::White, PieceKind::GoBetween)),
            ],
        ));
        for mv in [
            step(sq(10, 9), sq(10, 8)),
            step(sq(0, 1), sq(0, 0)),
            step(sq(10, 8), sq(10, 9)),
        ] {
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        }

        let forbidden_noncapture = step(sq(0, 0), sq(0, 1));
        assert_eq!(
            game.play(forbidden_noncapture),
            Err(GameError::IllegalMove {
                mv: forbidden_noncapture,
                cause: IllegalMoveCause::Repetition,
            })
        );
        assert_eq!(
            game.play(step(sq(0, 0), sq(1, 0))),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::RoyalCapture,
            }))
        );
    }

    #[test]
    fn article_32_e1_disables_mate_but_not_the_r2_move_filter() {
        let mut game = game_with_codes(
            position(
                Color::White,
                &[
                    (sq(0, 1), piece(Color::Black, PieceKind::King)),
                    (sq(11, 0), piece(Color::White, PieceKind::Rook)),
                    (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
                    (sq(10, 9), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R2, RuleCode::E1, RuleCode::E2],
        );
        for mv in [
            step(sq(10, 9), sq(10, 8)),
            step(sq(0, 1), sq(0, 0)),
            step(sq(10, 8), sq(10, 9)),
        ] {
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        }

        let forbidden_escape = step(sq(0, 0), sq(0, 1));
        assert_eq!(
            game.play(forbidden_escape),
            Err(GameError::IllegalMove {
                mv: forbidden_escape,
                cause: IllegalMoveCause::Repetition,
            })
        );
        assert_eq!(game.status(), GameStatus::Ongoing);
        assert_eq!(game.ply_count(), 3);
    }

    fn r2_stalemate_pieces() -> Vec<(Square, PieceCode)> {
        let mut pieces = vec![
            (sq(0, 0), piece(Color::Black, PieceKind::GoBetween)),
            (sq(11, 11), piece(Color::Black, PieceKind::King)),
            (sq(10, 10), piece(Color::Black, PieceKind::Pawn)),
            (sq(10, 11), piece(Color::Black, PieceKind::Pawn)),
            (sq(11, 10), piece(Color::Black, PieceKind::Pawn)),
            (sq(5, 5), piece(Color::White, PieceKind::King)),
        ];
        pieces.extend((2..=11).map(|rank| (sq(0, rank), piece(Color::Black, PieceKind::Pawn))));
        pieces
    }

    #[test]
    fn articles_21_6_and_21_7_resignation_and_draw_agreement_end_games() {
        let mut resigned = Game::with_default_rules();
        assert_eq!(
            resigned.resign(Color::Black),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::White,
                reason: WinReason::Resignation,
            }))
        );

        let mut drawn = Game::with_default_rules();
        assert_eq!(
            drawn.agree_draw(),
            Ok(GameStatus::Finished(GameResult::Draw {
                reason: DrawReason::Agreement,
            }))
        );
        assert_eq!(resigned.position(), &Position::initial());
        assert_eq!(drawn.position(), &Position::initial());
        assert_eq!(resigned.ply_count(), 0);
        assert_eq!(drawn.ply_count(), 0);
    }

    #[test]
    fn article_26_12_play_rejects_moves_after_the_game_is_over() {
        let mut game = Game::with_default_rules();
        let mut moves = Vec::new();
        game.generator.generate_moves(game.position(), &mut moves);
        let legal = moves[0];
        game.agree_draw().unwrap();

        assert_eq!(game.play(legal), Err(GameError::GameAlreadyOver));
        assert_eq!(game.ply_count(), 0);
        assert_eq!(game.position(), &Position::initial());
    }

    fn assert_random_game_position_invariants(game: &Game, rule_set_name: &str) {
        assert_eq!(
            game.position().validate(),
            Ok(()),
            "rule_set={rule_set_name}, ply={}",
            game.ply_count()
        );
        assert_eq!(
            game.position().zobrist(),
            game.position().recompute_zobrist(),
            "rule_set={rule_set_name}, ply={}",
            game.ply_count()
        );
        assert_eq!(
            game.position().rights_zobrist(),
            game.position().recompute_rights_zobrist(),
            "rule_set={rule_set_name}, ply={}",
            game.ply_count()
        );
    }

    fn play_random_move(game: &mut Game, rng: &mut XorShift64) -> GameStatus {
        let mut moves = Vec::new();
        game.generator.generate_moves(game.position(), &mut moves);
        assert!(
            !moves.is_empty(),
            "an ongoing game must have a generated move"
        );
        assert_eq!(
            moves.iter().copied().collect::<HashSet<_>>().len(),
            moves.len(),
            "an ongoing game must not have duplicate generated moves"
        );

        let start = (rng.next() as usize) % moves.len();
        for offset in 0..moves.len() {
            let selected = moves[(start + offset) % moves.len()];
            match game.play(selected) {
                Ok(status) => return status,
                Err(GameError::IllegalMove {
                    mv: rejected,
                    cause: IllegalMoveCause::Repetition,
                }) => {
                    assert_eq!(rejected, selected);
                }
                Err(error) => panic!("unexpected self-play error: {error}"),
            }
        }

        panic!("Article 23 must finish a game before every generated move is repetition-forbidden");
    }

    #[test]
    fn deterministic_random_r2_self_play_reaches_terminal_result_without_repetition() {
        const GAMES_PER_RULE_SET: usize = 3;
        const PLY_CAP: u32 = 1_500;
        const SEED: u64 = 0x5232_5f53_4f41_4b21;

        let mut rng = XorShift64::new(SEED);
        for codes in [
            &[RuleCode::R2][..],
            &[RuleCode::R2, RuleCode::E1, RuleCode::E2][..],
        ] {
            for game_index in 0..GAMES_PER_RULE_SET {
                let mut game = Game::new(Rules::from_codes(codes).unwrap()).unwrap();
                let mut terminated = false;

                for _ in 0..PLY_CAP {
                    if let GameStatus::Finished(result) = play_random_move(&mut game, &mut rng) {
                        assert!(!matches!(
                            result,
                            GameResult::Win {
                                reason: WinReason::Repetition,
                                ..
                            } | GameResult::Draw {
                                reason: DrawReason::Repetition,
                            }
                        ));
                        terminated = true;
                        break;
                    }
                }

                assert!(
                    terminated,
                    "R2 random game {game_index} with {codes:?} exceeded the {PLY_CAP}-ply cap"
                );
                assert!(matches!(
                    game.adjudication.repetition(),
                    RepetitionHistory::R2(_)
                ));
            }
        }
    }

    #[test]
    fn representative_rule_sets_random_self_play_returns_no_unexpected_errors() {
        const PLY_CAP: u32 = 1_500;
        const RULE_SETS: [(&str, &[RuleCode], u64); 7] = [
            (
                "L1+L2+P3+R1+E1",
                &[
                    RuleCode::L1,
                    RuleCode::L2,
                    RuleCode::P3,
                    RuleCode::R1,
                    RuleCode::E1,
                ],
                0x5255_4c45_4741_4d01,
            ),
            (
                "L3+P4+R1",
                &[RuleCode::L3, RuleCode::P4, RuleCode::R1],
                0x5255_4c45_4741_4d02,
            ),
            (
                "P1+R1",
                &[RuleCode::P1, RuleCode::R1],
                0x5255_4c45_4741_4d03,
            ),
            (
                "P2+R1",
                &[RuleCode::P2, RuleCode::R1],
                0x5255_4c45_4741_4d04,
            ),
            (
                "L1+L2+L3+P1+P3+P4+R2+E1+E2",
                &[
                    RuleCode::L1,
                    RuleCode::L2,
                    RuleCode::L3,
                    RuleCode::P1,
                    RuleCode::P3,
                    RuleCode::P4,
                    RuleCode::R2,
                    RuleCode::E1,
                    RuleCode::E2,
                ],
                0x5255_4c45_4741_4d05,
            ),
            (
                "L1+L2+L3+P1+P3+P4+R3+E1+E2",
                &[
                    RuleCode::L1,
                    RuleCode::L2,
                    RuleCode::L3,
                    RuleCode::P1,
                    RuleCode::P3,
                    RuleCode::P4,
                    RuleCode::R3,
                    RuleCode::E1,
                    RuleCode::E2,
                ],
                0x5255_4c45_4741_4d06,
            ),
            (
                "L1+L2+P3+R1+E1+E3",
                &[
                    RuleCode::L1,
                    RuleCode::L2,
                    RuleCode::P3,
                    RuleCode::R1,
                    RuleCode::E1,
                    RuleCode::E3,
                ],
                0x5255_4c45_4741_4d07,
            ),
        ];

        for (rule_set_name, codes, seed) in RULE_SETS {
            let mut rng = XorShift64::new(seed);
            let mut game = Game::new(Rules::from_codes(codes).unwrap()).unwrap();

            for _ in 0..PLY_CAP {
                assert_random_game_position_invariants(&game, rule_set_name);
                if let GameStatus::Finished(_) = play_random_move(&mut game, &mut rng) {
                    break;
                }
            }

            assert_random_game_position_invariants(&game, rule_set_name);
            assert!(
                matches!(game.status(), GameStatus::Finished(_)) || game.ply_count() == PLY_CAP,
                "random game ended before reaching a terminal result or the ply cap: \
                 rule_set={rule_set_name}, ply={}",
                game.ply_count()
            );
        }
    }

    #[test]
    fn deterministic_random_self_play_reaches_a_terminal_result() {
        const GAME_COUNT: usize = 4;
        const PLY_CAP: u32 = 1_500;
        const SEED: u64 = 0x4741_4d45_5f53_4f41;

        let mut rng = XorShift64::new(SEED);
        let mut royal_capture = 0;
        let mut repetition_loss = 0;
        let mut stalemate = 0;
        let mut mate = 0;
        let mut repetition_draw = 0;
        let mut exhaustion_win = 0;
        let mut exhaustion_draw = 0;

        for game_index in 0..GAME_COUNT {
            let mut game = Game::with_default_rules();
            let mut terminated = false;

            for _ in 0..PLY_CAP {
                let mut moves = Vec::new();
                game.generator.generate_moves(game.position(), &mut moves);
                assert!(
                    !moves.is_empty(),
                    "ongoing game {game_index} has no legal move"
                );
                let selected = moves[(rng.next() as usize) % moves.len()];
                if let GameStatus::Finished(result) = game.play(selected).unwrap() {
                    match result {
                        GameResult::Win {
                            reason: WinReason::RoyalCapture,
                            ..
                        } => royal_capture += 1,
                        GameResult::Win {
                            reason: WinReason::Repetition,
                            ..
                        } => repetition_loss += 1,
                        GameResult::Win {
                            reason: WinReason::Stalemate,
                            ..
                        } => stalemate += 1,
                        GameResult::Win {
                            reason: WinReason::Mate,
                            ..
                        } => mate += 1,
                        GameResult::Draw {
                            reason: DrawReason::Repetition,
                        } => repetition_draw += 1,
                        GameResult::Win {
                            reason: WinReason::PieceExhaustion,
                            ..
                        } => exhaustion_win += 1,
                        GameResult::Draw {
                            reason: DrawReason::PieceExhaustion,
                        } => exhaustion_draw += 1,
                        GameResult::Win {
                            reason: WinReason::BareKing,
                            ..
                        }
                        | GameResult::Draw {
                            reason: DrawReason::BareKing,
                        } => panic!("default rules cannot produce an E3 bare-king result"),
                        GameResult::Win {
                            reason: WinReason::Resignation,
                            ..
                        }
                        | GameResult::Draw {
                            reason: DrawReason::Agreement,
                        } => panic!("self-play cannot produce an external-input result"),
                    }
                    terminated = true;
                    break;
                }
            }

            assert!(
                terminated,
                "random game {game_index} exceeded the {PLY_CAP}-ply cap"
            );
        }

        println!(
            "termination distribution: royal_capture={royal_capture}, \
             repetition_loss={repetition_loss}, stalemate={stalemate}, \
             mate={mate}, repetition_draw={repetition_draw}, \
             exhaustion_win={exhaustion_win}, exhaustion_draw={exhaustion_draw}"
        );
    }
}
