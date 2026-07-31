use core::fmt;
use std::collections::HashMap;

use crate::core::movegen::{IllegalMove, MoveGenerator};
use crate::core::mv::Move;
use crate::core::piece::Color;
use crate::core::position::Position;
use crate::core::rules::Rules;
use crate::mate::{has_no_legal_move, is_mate};

const LION_TRIGGER_PROJECTION: u64 = 0x6e71_4f59_2d83_c0a5;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WinReason {
    RoyalCapture,
    Repetition,
    Stalemate,
    Mate,
    Resignation,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DrawReason {
    Repetition,
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
    UnsupportedPieceExhaustion,
    GameAlreadyOver,
    IllegalMove(IllegalMove),
}

impl fmt::Display for GameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedStandardRepetition => formatter
                .write_str("standard repetition adjudication is unsupported; R1 is required"),
            Self::UnsupportedPieceExhaustion => {
                formatter.write_str("piece-exhaustion adjudication is unsupported; E2 is required")
            }
            Self::GameAlreadyOver => formatter.write_str("the game is already over"),
            Self::IllegalMove(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for GameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IllegalMove(error) => Some(error),
            Self::UnsupportedStandardRepetition
            | Self::UnsupportedPieceExhaustion
            | Self::GameAlreadyOver => None,
        }
    }
}

impl From<IllegalMove> for GameError {
    fn from(error: IllegalMove) -> Self {
        Self::IllegalMove(error)
    }
}

#[derive(Clone, Copy)]
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
    mate_adjudication_enabled: bool,
}

impl Game {
    pub fn new(rules: Rules) -> Result<Self, GameError> {
        if !rules.repetition_is_r1() {
            return Err(GameError::UnsupportedStandardRepetition);
        }
        if !rules.piece_exhaustion_disabled() {
            return Err(GameError::UnsupportedPieceExhaustion);
        }

        Ok(Self::from_position(rules, Position::initial()))
    }

    pub fn with_default_rules() -> Self {
        Self::new(Rules::engine_default()).expect("engine-default rules must be supported by Game")
    }

    /// Validates and applies one move, then adjudicates royal capture,
    /// repetition, stalemate, and mate in that order.
    pub fn play(&mut self, mv: Move) -> Result<GameStatus, GameError> {
        self.ensure_ongoing()?;

        let mover = self.position.side_to_move();
        self.position.try_make_move(mv, &self.generator)?;
        self.ply = self
            .ply
            .checked_add(1)
            .expect("a game cannot exceed u32::MAX plies");

        // Adjudication order is fixed as royal capture, repetition,
        // stalemate, then mate.
        if self.position.royal_pieces(mover.opposite()).is_empty() {
            return Ok(self.finish(GameResult::Win {
                winner: mover,
                reason: WinReason::RoyalCapture,
            }));
        }

        self.update_attacking_counter(mover, mv);
        if let Some(result) = self.repetition_result() {
            return Ok(self.finish(result));
        }

        let side_to_move = self.position.side_to_move();
        if has_no_legal_move(&self.position, &self.generator) {
            return Ok(self.finish(GameResult::Win {
                winner: side_to_move.opposite(),
                reason: WinReason::Stalemate,
            }));
        }

        if self.mate_adjudication_enabled && is_mate(&mut self.position, &self.generator) {
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
        let key = position_key(&position);
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
            mate_adjudication_enabled: rules.mate_adjudication_enabled(),
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
        let key = position_key(&self.position);
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
}

fn position_key(position: &Position) -> u64 {
    position.zobrist()
        ^ if position.lion_taken_by_non_lion().is_some() {
            LION_TRIGGER_PROJECTION
        } else {
            0
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
    use crate::core::rules::RuleCode;
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
    fn plan_game_3_game_new_requires_r1_and_e2_and_accepts_engine_default() {
        assert!(matches!(
            Game::new(Rules::standard()),
            Err(GameError::UnsupportedStandardRepetition)
        ));
        assert!(matches!(
            Game::new(Rules::from_codes(&[RuleCode::E2]).unwrap()),
            Err(GameError::UnsupportedStandardRepetition)
        ));
        assert!(matches!(
            Game::new(Rules::from_codes(&[RuleCode::R1]).unwrap()),
            Err(GameError::UnsupportedPieceExhaustion)
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
    fn article_24_1_c_position_key_includes_one_bit_senjishi_projection() {
        let captured_lion = sq(1, 1);
        let mut with_trigger = position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::Bishop)),
                (captured_lion, piece(Color::White, PieceKind::Lion)),
                (sq(10, 10), piece(Color::White, PieceKind::Pawn)),
            ],
        );
        with_trigger.make_move_unchecked(step(sq(0, 0), captured_lion));
        let without_trigger = position(
            Color::White,
            &[
                (captured_lion, piece(Color::Black, PieceKind::Bishop)),
                (sq(10, 10), piece(Color::White, PieceKind::Pawn)),
            ],
        );

        assert_eq!(with_trigger.zobrist(), without_trigger.zobrist());
        assert_ne!(position_key(&with_trigger), position_key(&without_trigger));

        let mut counts = HashMap::new();
        *counts.entry(position_key(&with_trigger)).or_insert(0) += 1;
        *counts.entry(position_key(&without_trigger)).or_insert(0) += 1;
        assert_eq!(counts.len(), 2);
        assert!(counts.values().all(|&count| count == 1));
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
             mate={mate}, repetition_draw={repetition_draw}"
        );
    }
}
