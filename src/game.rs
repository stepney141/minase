use core::fmt;
use std::collections::HashMap;

use crate::core::movegen::{IllegalMove, MoveGenerator};
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceKind};
use crate::core::position::Position;
use crate::core::rules::{RepetitionRule, Rules};
use crate::core::square::{BOARD_RANKS, Square};
use crate::mate::{has_no_legal_move, is_mate};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WinReason {
    RoyalCapture,
    Repetition,
    PieceExhaustion,
    Stalemate,
    Mate,
    Resignation,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DrawReason {
    Repetition,
    PieceExhaustion,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameError {
    UnsupportedStandardRepetition,
    GameAlreadyOver,
    IllegalMove(IllegalMove),
}

impl fmt::Display for GameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedStandardRepetition => formatter
                .write_str("R0 repetition adjudication is unsupported; R1 or R2 is required"),
            Self::GameAlreadyOver => formatter.write_str("the game is already over"),
            Self::IllegalMove(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for GameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IllegalMove(error) => Some(error),
            Self::UnsupportedStandardRepetition | Self::GameAlreadyOver => None,
        }
    }
}

impl From<IllegalMove> for GameError {
    fn from(error: IllegalMove) -> Self {
        Self::IllegalMove(error)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct RepetitionState {
    occurrences: u8,
    first_ply: u32,
}

pub struct Game {
    position: Position,
    generator: MoveGenerator,
    result: Option<GameResult>,
    repetitions: HashMap<u64, RepetitionState>,
    consecutive_attacking_moves: [u32; 2],
    ply: u32,
    repetition_rule: RepetitionRule,
    mate_adjudication_enabled: bool,
    piece_exhaustion_disabled: bool,
    piece_exhaustion_grace: bool,
}

impl Game {
    pub fn new(rules: Rules) -> Result<Self, GameError> {
        if rules.repetition_rule() == RepetitionRule::R0 {
            return Err(GameError::UnsupportedStandardRepetition);
        }

        Ok(Self::from_position(rules, Position::initial()))
    }

    pub fn with_default_rules() -> Self {
        Self::new(Rules::engine_default()).expect("engine-default rules must be supported by Game")
    }

    /// Validates and applies one move, then adjudicates royal capture,
    /// repetition, piece exhaustion, stalemate, and mate in that order.
    pub fn play(&mut self, mv: Move) -> Result<GameStatus, GameError> {
        self.ensure_ongoing()?;

        let mover = self.position.side_to_move();
        let undo = self.position.try_make_move(mv, &self.generator)?;
        let position_key = self.position.zobrist();
        if self.repetition_rule == RepetitionRule::R2
            && self.repetitions.contains_key(&position_key)
        {
            self.position.unmake_move(undo);
            return Err(GameError::IllegalMove(IllegalMove(mv)));
        }
        // Only a non-capturing promotion qualifies for the immediate Article
        // 22-2/22-3 win: with a capture, the exhaustion condition did not hold
        // before the move, so the fresh condition takes the Article 22-5 grace
        // path instead.
        let promoted_waiting_piece = (mv.is_promoting()
            && undo.captured.iter().all(Option::is_none)
            && matches!(
                undo.moved_piece_before.kind(),
                Some(PieceKind::Pawn | PieceKind::GoBetween)
            ))
        .then_some(mv.destination());
        self.ply = self
            .ply
            .checked_add(1)
            .expect("a game cannot exceed u32::MAX plies");

        // Adjudication order is fixed as royal capture, repetition,
        // piece exhaustion, stalemate, then mate.
        if self.position.royal_pieces(mover.opposite()).is_empty() {
            return Ok(self.finish(GameResult::Win {
                winner: mover,
                reason: WinReason::RoyalCapture,
            }));
        }

        match self.repetition_rule {
            RepetitionRule::R1 => {
                self.update_attacking_counter(mover, mv);
                if let Some(result) = self.repetition_result() {
                    return Ok(self.finish(result));
                }
            }
            RepetitionRule::R2 => self.record_position(position_key),
            RepetitionRule::R0 => unreachable!("Game rejects unsupported R0 repetition"),
        }

        if let Some(result) = self.piece_exhaustion_result(promoted_waiting_piece) {
            return Ok(self.finish(result));
        }

        let side_to_move = self.position.side_to_move();
        let repetitions = &self.repetitions;
        let forbidden_position = |key| repetitions.contains_key(&key);
        let history_filter = (self.repetition_rule == RepetitionRule::R2)
            .then_some(&forbidden_position as &dyn Fn(u64) -> bool);
        if has_no_legal_move(&mut self.position, &self.generator, history_filter) {
            return Ok(self.finish(GameResult::Win {
                winner: side_to_move.opposite(),
                reason: WinReason::Stalemate,
            }));
        }

        if self.mate_adjudication_enabled
            && is_mate(&mut self.position, &self.generator, history_filter)
        {
            return Ok(self.finish(GameResult::Win {
                winner: side_to_move.opposite(),
                reason: WinReason::Mate,
            }));
        }

        Ok(GameStatus::Ongoing)
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

    #[inline]
    pub const fn ply_count(&self) -> u32 {
        self.ply
    }

    fn from_position(rules: Rules, position: Position) -> Self {
        let key = position.zobrist();
        Self {
            position,
            generator: MoveGenerator::new(rules),
            result: None,
            repetitions: HashMap::from([(
                key,
                RepetitionState {
                    occurrences: 1,
                    first_ply: 0,
                },
            )]),
            consecutive_attacking_moves: [0; 2],
            ply: 0,
            repetition_rule: rules.repetition_rule(),
            mate_adjudication_enabled: rules.mate_adjudication_enabled(),
            piece_exhaustion_disabled: rules.piece_exhaustion_disabled(),
            piece_exhaustion_grace: false,
        }
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

    fn update_attacking_counter(&mut self, mover: Color, mv: Move) {
        let counter = &mut self.consecutive_attacking_moves[mover.index()];
        if move_was_attacking(&self.position, &self.generator, mover, mv) {
            *counter = counter
                .checked_add(1)
                .expect("an attack sequence cannot exceed u32::MAX plies");
        } else {
            *counter = 0;
        }
    }

    fn repetition_result(&mut self) -> Option<GameResult> {
        let key = self.position.zobrist();
        let first_ply = {
            let state = self.repetitions.entry(key).or_insert(RepetitionState {
                occurrences: 0,
                first_ply: self.ply,
            });
            state.occurrences += 1;
            if state.occurrences < 4 {
                return None;
            }
            state.first_ply
        };

        let attackers = Color::ALL.map(|color| {
            let distance =
                moves_by_color_through(self.ply, color) - moves_by_color_through(first_ply, color);
            self.consecutive_attacking_moves[color.index()] >= distance
        });

        match attackers {
            [true, false] => Some(GameResult::Win {
                winner: Color::White,
                reason: WinReason::Repetition,
            }),
            [false, true] => Some(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::Repetition,
            }),
            [false, false] | [true, true] => Some(GameResult::Draw {
                reason: DrawReason::Repetition,
            }),
        }
    }

    fn record_position(&mut self, key: u64) {
        let previous = self.repetitions.insert(
            key,
            RepetitionState {
                occurrences: 1,
                first_ply: self.ply,
            },
        );
        debug_assert!(previous.is_none(), "R2 must reject repeated positions");
    }

    fn piece_exhaustion_result(
        &mut self,
        promoted_waiting_piece: Option<Square>,
    ) -> Option<GameResult> {
        if self.piece_exhaustion_disabled {
            return None;
        }
        let grace_pending = core::mem::take(&mut self.piece_exhaustion_grace);

        let royals = Color::ALL.map(|color| self.position.royal_pieces(color));
        let non_royals = self.position.occupied() & !(royals[0] | royals[1]);
        if non_royals.is_empty() {
            return Some(GameResult::Draw {
                reason: DrawReason::PieceExhaustion,
            });
        }
        if royals.into_iter().any(|royal| royal.popcount() != 1) || non_royals.popcount() != 1 {
            return None;
        }

        let extra_square = non_royals
            .lsb()
            .expect("one non-royal piece must have a square");
        let extra_piece = self
            .position
            .piece_at(extra_square)
            .expect("the non-royal square must contain a piece");
        let extra_color = extra_piece
            .color()
            .expect("the extra piece must have an owner");
        let extra_kind = extra_piece
            .kind()
            .expect("the extra piece must have a kind");

        if !extra_piece.is_promoted()
            && matches!(extra_kind, PieceKind::Pawn | PieceKind::Lance)
            && is_last_rank(extra_color, extra_square)
        {
            return None;
        }
        if !extra_piece.is_promoted()
            && matches!(extra_kind, PieceKind::Pawn | PieceKind::GoBetween)
        {
            return None;
        }

        let win = GameResult::Win {
            winner: extra_color,
            reason: WinReason::PieceExhaustion,
        };
        if promoted_waiting_piece == Some(extra_square) {
            return Some(win);
        }
        if grace_pending {
            return Some(win);
        }
        if self.position.side_to_move() == extra_color {
            return Some(win);
        }

        let mut moves = Vec::new();
        self.generator.generate_moves(&self.position, &mut moves);
        let extra_piece_can_be_captured = moves.into_iter().any(|candidate| {
            self.position
                .captured_squares(candidate)
                .into_iter()
                .flatten()
                .any(|capture| capture == extra_square)
        });
        if extra_piece_can_be_captured {
            self.piece_exhaustion_grace = true;
            None
        } else {
            Some(win)
        }
    }
}

fn is_last_rank(color: Color, square: Square) -> bool {
    match color {
        Color::Black => square.rank() == BOARD_RANKS - 1,
        Color::White => square.rank() == 0,
    }
}

fn moves_by_color_through(ply: u32, color: Color) -> u32 {
    match color {
        Color::Black => ply / 2 + ply % 2,
        Color::White => ply / 2,
    }
}

fn move_was_attacking(
    position: &Position,
    generator: &MoveGenerator,
    mover: Color,
    played: Move,
) -> bool {
    let probe = position.clone_with_side_to_move(mover);
    let opponent_royals = probe.royal_pieces(mover.opposite());
    let destination = played.destination();
    let mut moves = Vec::new();
    generator.generate_moves(&probe, &mut moves);

    moves.into_iter().any(|candidate| {
        let captures = probe.captured_squares(candidate);
        captures
            .into_iter()
            .flatten()
            .any(|square| opponent_royals.contains(square))
            || (candidate.origin() == destination
                && captures.into_iter().any(|capture| capture.is_some()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::piece::{PieceCode, PieceKind};
    use crate::core::rules::{RuleCode, RulesError};
    use crate::core::square::Square;
    use crate::test_util::{position_from_codes as position, sq};

    fn piece(color: Color, kind: PieceKind) -> PieceCode {
        PieceCode::new(color, kind)
    }

    fn prince(color: Color) -> PieceCode {
        PieceCode::new_promoted(color, PieceKind::CrownPrince).unwrap()
    }

    fn game(position: Position) -> Game {
        Game::from_position(Rules::engine_default(), position)
    }

    fn game_with_codes(position: Position, codes: &[RuleCode]) -> Game {
        Game::from_position(Rules::from_codes(codes).unwrap(), position)
    }

    fn piece_exhaustion_game(position: Position) -> Game {
        game_with_codes(position, &[RuleCode::R1])
    }

    fn r2_game(position: Position) -> Game {
        game_with_codes(position, &[RuleCode::R2, RuleCode::E2])
    }

    fn step(from: Square, to: Square) -> Move {
        Move {
            from,
            mid: None,
            to,
            promote: false,
        }
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
    fn article_25_3_explicit_r0_normalizes_to_standard() {
        assert_eq!(
            Rules::from_codes(&[RuleCode::R0]).unwrap(),
            Rules::standard()
        );
    }

    #[test]
    fn article_31_r0_and_r1_are_conflicting() {
        assert_eq!(
            Rules::from_codes(&[RuleCode::R0, RuleCode::R1]),
            Err(RulesError::Conflicting {
                first: RuleCode::R0,
                second: RuleCode::R1,
            })
        );
    }

    #[test]
    fn plan_game_3_explicit_r0_game_is_unsupported() {
        assert!(matches!(
            Game::new(Rules::from_codes(&[RuleCode::R0]).unwrap()),
            Err(GameError::UnsupportedStandardRepetition)
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

        assert_eq!(rules.repetition_rule(), RepetitionRule::R2);
        assert_eq!(game.status(), GameStatus::Ongoing);
        assert_eq!(game.position(), &Position::initial());
        assert_eq!(game.ply_count(), 0);
    }

    #[test]
    fn plan_game_4_engine_default_is_standard_minus_r0_plus_r1() {
        assert!(matches!(
            Game::new(Rules::from_codes(&[RuleCode::E2]).unwrap()),
            Err(GameError::UnsupportedStandardRepetition)
        ));

        let game = Game::new(Rules::engine_default()).unwrap();
        assert_eq!(game.status(), GameStatus::Ongoing);
        assert_eq!(game.position(), &Position::initial());
        assert_eq!(game.ply_count(), 0);
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
            assert!(game.piece_exhaustion_grace);
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
        assert!(!game.piece_exhaustion_grace);
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

        let mut prince_and_kings = piece_exhaustion_game(position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(2, 0), prince(Color::Black)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(prince_and_kings.play(step(sq(0, 0), sq(0, 1))), expected);
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
        assert!(!condition_c.piece_exhaustion_grace);

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
        assert_eq!(game.consecutive_attacking_moves, [0, 0]);
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
        assert_eq!(game.consecutive_attacking_moves, [6, 0]);
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
        let repetitions_before = game.repetitions.clone();
        let counters_before = game.consecutive_attacking_moves;
        let exhaustion_grace_before = game.piece_exhaustion_grace;
        assert_eq!(
            game.play(repeated),
            Err(GameError::IllegalMove(IllegalMove(repeated)))
        );
        assert_eq!(game.position(), &position_before);
        assert_eq!(game.ply_count(), 3);
        assert_eq!(game.status(), GameStatus::Ongoing);
        assert_eq!(game.repetitions, repetitions_before);
        assert_eq!(game.consecutive_attacking_moves, counters_before);
        assert_eq!(game.piece_exhaustion_grace, exhaustion_grace_before);

        assert_eq!(game.play(step(sq(8, 7), sq(9, 7))), Ok(GameStatus::Ongoing));
        assert_eq!(game.ply_count(), 4);
        assert_eq!(game.consecutive_attacking_moves, [0, 0]);
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
            Err(GameError::IllegalMove(IllegalMove(forbidden_noncapture)))
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
            Err(GameError::IllegalMove(IllegalMove(forbidden_escape)))
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

    struct XorShift64 {
        state: u64,
    }

    impl XorShift64 {
        fn new(seed: u64) -> Self {
            assert_ne!(seed, 0);
            Self { state: seed }
        }

        fn next(&mut self) -> u64 {
            let mut value = self.state;
            value ^= value << 13;
            value ^= value >> 7;
            value ^= value << 17;
            self.state = value;
            value
        }
    }

    fn play_random_r2_move(game: &mut Game, rng: &mut XorShift64) -> GameStatus {
        let mut moves = Vec::new();
        game.generator.generate_moves(game.position(), &mut moves);
        assert!(
            !moves.is_empty(),
            "an ongoing game must have a generated move"
        );

        let start = (rng.next() as usize) % moves.len();
        for offset in 0..moves.len() {
            let selected = moves[(start + offset) % moves.len()];
            match game.play(selected) {
                Ok(status) => return status,
                Err(GameError::IllegalMove(IllegalMove(rejected))) => {
                    assert_eq!(rejected, selected);
                }
                Err(error) => panic!("unexpected self-play error: {error}"),
            }
        }

        panic!("Article 23 must finish a game before every generated move is R2-forbidden");
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
                    if let GameStatus::Finished(result) = play_random_r2_move(&mut game, &mut rng) {
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
                assert_eq!(game.consecutive_attacking_moves, [0, 0]);
                assert!(
                    game.repetitions
                        .values()
                        .all(|state| state.occurrences == 1)
                );
            }
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
