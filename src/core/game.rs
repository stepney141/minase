//! 対局の進行管理。着手の受理と検証、終局裁定の呼出し、対局結果の保持を担う。

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

/// 着手または対局操作が受理されなかった原因。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameError {
    /// 対局が既に終了している(第26条第12項)。
    GameAlreadyOver,
    /// 不合法な着手(第26条)。
    IllegalMove {
        /// 拒否された着手。
        mv: Move,
        /// 拒否の原因。
        cause: IllegalMoveCause,
    },
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
                .record_move(&self.position, &self.generator, mover, mv);
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::num::NonZeroU64;

    use super::*;
    use crate::core::piece::{PieceCode, PieceKind};
    use crate::core::rules::{ExhaustionRule, RepetitionRule, RuleCode, RuleGroup, RulesError};
    use crate::core::square::Square;
    use crate::parse_sfen;
    use crate::rng::XorShift64;
    use crate::test_util::{position_from_codes as position, sq};

    fn piece(color: Color, kind: PieceKind) -> PieceCode {
        PieceCode::new(color, kind).expect("fixture uses an unpromoted-capable kind")
    }

    fn prince(color: Color) -> PieceCode {
        PieceCode::new_promoted(color, PieceKind::CrownPrince).unwrap()
    }

    fn game(position: Position) -> Game {
        Game::from_position(Rules::ENGINE_DEFAULT, position)
    }

    fn game_with_codes(position: Position, codes: &[RuleCode]) -> Game {
        let lion = if codes.contains(&RuleCode::L1) {
            RuleCode::L1
        } else {
            RuleCode::L0
        };
        let promotion = if codes.contains(&RuleCode::P1) {
            RuleCode::P1
        } else if codes.contains(&RuleCode::P2) {
            RuleCode::P2
        } else {
            RuleCode::P0
        };
        let repetition = [RuleCode::R1, RuleCode::R2, RuleCode::R3]
            .into_iter()
            .find(|code| codes.contains(code))
            .unwrap_or(RuleCode::R1);
        let exhaustion = if codes.contains(&RuleCode::E3) {
            RuleCode::E3
        } else if codes.contains(&RuleCode::E2) {
            RuleCode::E2
        } else {
            RuleCode::E0
        };
        let mut complete = vec![lion];
        complete.extend(codes.iter().copied().filter(|code| {
            !matches!(
                code,
                RuleCode::L0
                    | RuleCode::L1
                    | RuleCode::P0
                    | RuleCode::P1
                    | RuleCode::P2
                    | RuleCode::R1
                    | RuleCode::R2
                    | RuleCode::R3
                    | RuleCode::E0
                    | RuleCode::E2
                    | RuleCode::E3
            )
        }));
        complete.extend([promotion, repetition, exhaustion]);
        Game::from_position(Rules::from_codes(&complete).unwrap(), position)
    }

    fn bare_king_game(position: Position) -> Game {
        game_with_codes(
            position,
            &[
                RuleCode::L0,
                RuleCode::P0,
                RuleCode::R1,
                RuleCode::E1,
                RuleCode::E3,
            ],
        )
    }

    fn step(from: Square, to: Square) -> Move {
        Move {
            from,
            mid: None,
            to,
            promote: false,
        }
    }

    fn promoting(from: Square, to: Square) -> Move {
        Move {
            from,
            mid: None,
            to,
            promote: true,
        }
    }

    fn win(winner: Color, reason: WinReason) -> Result<GameStatus, GameError> {
        Ok(GameStatus::Finished(GameResult::Win { winner, reason }))
    }

    fn draw(reason: DrawReason) -> Result<GameStatus, GameError> {
        Ok(GameStatus::Finished(GameResult::Draw { reason }))
    }

    // 詰み直前局面(第21条2項)。白の(10,8)→(10,9)の完了時に黒王将の受けが尽きる。
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

    // 王と金の非攻撃的4手周期(第31条R1の反復フィクスチャ)。
    fn gold_cycle_position() -> Position {
        position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(3, 3), piece(Color::Black, PieceKind::GoldGeneral)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
                (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
            ],
        )
    }

    fn gold_cycle() -> [Move; 4] {
        [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
            step(sq(8, 7), sq(8, 8)),
        ]
    }

    // 王将2枚だけの非攻撃的4手周期(R2・R3の拒否フィクスチャ。E2併用で駒枯れを外す)。
    fn king_cycle_position() -> Position {
        position(
            Color::Black,
            &[
                (sq(3, 3), piece(Color::Black, PieceKind::King)),
                (sq(8, 8), piece(Color::White, PieceKind::King)),
            ],
        )
    }

    fn king_cycle() -> [Move; 4] {
        [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
            step(sq(8, 7), sq(8, 8)),
        ]
    }

    // F3(第23条): 手番先手に着手が1つも存在しない局面。王手はかかっていない。
    fn f3_no_move_position() -> Position {
        position(
            Color::White,
            &[
                (sq(0, 11), piece(Color::Black, PieceKind::King)),
                (sq(1, 11), piece(Color::Black, PieceKind::Pawn)),
                (sq(0, 10), piece(Color::Black, PieceKind::Lance)),
                (sq(1, 10), piece(Color::Black, PieceKind::Lance)),
                (sq(10, 0), piece(Color::White, PieceKind::King)),
            ],
        )
    }

    // ---------------------------------------------------------------
    // 第20条　王駒
    // ---------------------------------------------------------------

    #[test]
    fn article_20_1_game_starts_ongoing_with_one_royal_each() {
        // D3-020-01: 開始時は各対局者の王駒が王将・玉将の1枚だけで、終局していない。
        let game = Game::new(Rules::ENGINE_DEFAULT);

        assert_eq!(game.status(), GameStatus::Ongoing);
        assert_eq!(game.result(), None);
        for color in Color::ALL {
            assert_eq!(game.position().royal_pieces(color).popcount(), 1);
        }
    }

    #[test]
    fn article_20_2_promoted_elephant_becomes_a_royal_prince() {
        // D3-020-02/D3-020-04: 成った醉象は太子=王駒であり、王将を取られても
        // 太子が残る限り継続し、2枚目の王駒の捕獲で初めて終局する。
        let mut game = game(position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(4, 7), piece(Color::Black, PieceKind::DrunkElephant)),
                (sq(9, 2), piece(Color::Black, PieceKind::Pawn)),
                (sq(11, 0), piece(Color::White, PieceKind::Rook)),
                (sq(4, 11), piece(Color::White, PieceKind::Rook)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));

        // 醉象が敵陣(段8)へ入り太子へ成る(第18条1項、第20条2項)。
        assert_eq!(
            game.play(promoting(sq(4, 7), sq(4, 8))),
            Ok(GameStatus::Ongoing)
        );
        // 王将を取られても太子が残るため終局せず、勝敗理由も記録されない。
        assert_eq!(
            game.play(step(sq(11, 0), sq(0, 0))),
            Ok(GameStatus::Ongoing)
        );
        assert_eq!(game.result(), None);
        assert!(!game.legal_moves().is_empty());
        assert_eq!(game.play(step(sq(9, 2), sq(9, 3))), Ok(GameStatus::Ongoing));
        // 最後の王駒である太子の捕獲で第21条1項の勝ちになる。
        assert_eq!(
            game.play(step(sq(4, 11), sq(4, 8))),
            win(Color::White, WinReason::RoyalCapture)
        );

        // 境界: 成る前の醉象は王駒ではなく、王将の捕獲だけで直ちに終局する。
        let mut unpromoted = game_with_codes(
            position(
                Color::White,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(4, 7), piece(Color::Black, PieceKind::DrunkElephant)),
                    (sq(11, 0), piece(Color::White, PieceKind::Rook)),
                    (sq(11, 11), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R1],
        );
        assert_eq!(
            unpromoted.play(step(sq(11, 0), sq(0, 0))),
            win(Color::White, WinReason::RoyalCapture)
        );
    }

    #[test]
    fn article_20_3_capturing_the_prince_alone_continues_the_game() {
        // D3-020-03: 王将が残る側は太子を取られても敗北しない。太子喪失直後に
        // 王手がかかっていても、対局は継続し合法手も通常どおり生成される。
        let mut game = game(position(
            Color::White,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(5, 5), prince(Color::Black)),
                (sq(5, 11), piece(Color::White, PieceKind::Rook)),
                (sq(11, 0), piece(Color::White, PieceKind::Rook)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));

        // (11,0)の飛車が黒王将へ王手をかけたまま、太子だけを取る。
        assert_eq!(
            game.play(step(sq(5, 11), sq(5, 5))),
            Ok(GameStatus::Ongoing)
        );
        assert_eq!(game.result(), None);
        assert!(!game.legal_moves().is_empty());
    }

    #[test]
    fn plan_referee_from_position_defers_adjudication_to_the_first_move() {
        // D3-020-05: 任意局面からの構築では自動裁定せず、開始局面を履歴の
        // 第1回として記録する(adjudication-refactor.md「任意局面と履歴」)。
        // 直後の着手完了時からは通常の裁定が働く。
        let missing_royal = position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(5, 5), piece(Color::Black, PieceKind::GoldGeneral)),
                (sq(9, 9), piece(Color::White, PieceKind::Pawn)),
            ],
        );
        let mut no_royal_game = game(missing_royal);
        assert_eq!(no_royal_game.status(), GameStatus::Ongoing);
        assert_eq!(
            no_royal_game.play(step(sq(5, 5), sq(5, 6))),
            win(Color::Black, WinReason::RoyalCapture)
        );

        // 駒枯れ条件(第22条1項)を満たす開始局面も構築時には裁定されない。
        let mut exhausted = game_with_codes(
            position(
                Color::Black,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
                    (sq(11, 11), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R1],
        );
        assert_eq!(exhausted.status(), GameStatus::Ongoing);
        assert_eq!(
            exhausted.play(step(sq(0, 0), sq(0, 1))),
            win(Color::White, WinReason::PieceExhaustion)
        );

        // parse_sfenで得た局面も同じ契約で受け入れ、採用規則をそのまま返す。
        let parsed = parse_sfen("12/12/12/8k3/12/12/12/12/3K8/12/12/12 b").unwrap();
        let rules =
            Rules::from_codes(&[RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E2]).unwrap();
        let mut sfen_game = Game::from_position(rules, parsed);
        assert_eq!(sfen_game.rules(), rules);
        assert_eq!(
            sfen_game.play(step(sq(3, 3), sq(3, 4))),
            Ok(GameStatus::Ongoing)
        );
    }

    // ---------------------------------------------------------------
    // 第21条　王駒による勝敗
    // ---------------------------------------------------------------

    #[test]
    fn article_21_1_capturing_the_last_royal_wins() {
        // D3-021-01: 最後の王駒を取る着手の完了時に、捕獲を理由として終局する。
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
    fn article_21_2_mate_ends_the_game_at_move_completion() {
        // D3-021-02: 3項a・b・cのいずれの合法手もない局面を作る着手の完了時に、
        // 詰みを理由として終局する。
        let (position, mv) = mate_predecessor();
        let mut game = game(position);

        assert_eq!(game.play(mv), win(Color::White, WinReason::Mate));
    }

    #[test]
    fn article_21_3_a_any_escape_prevents_mate() {
        // D3-021-03: 捕獲を回避する着手が1つでもあれば詰みは成立しない。
        // mate_predecessorから(0,11)の飛車を除くと(0,1)への逃げが残る。
        let mut game = game(position(
            Color::White,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(11, 0), piece(Color::White, PieceKind::Rook)),
                (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
                (sq(10, 8), piece(Color::White, PieceKind::King)),
            ],
        ));

        assert_eq!(
            game.play(step(sq(10, 8), sq(10, 9))),
            Ok(GameStatus::Ongoing)
        );
    }

    #[test]
    fn article_21_3_b_and_21_4_checked_side_may_capture_the_last_royal_first() {
        // D3-021-04: 王手放置は合法(第8条)であり、相手の最後の王駒を先に取る
        // 着手が3項bの回避手段になるため詰みではない。
        // D3-021-06: 王手を受けている側が先に取れば、その側の勝ちとなる。
        let mut game = game(position(
            Color::White,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(9, 9), piece(Color::Black, PieceKind::Pawn)),
                (sq(1, 1), piece(Color::White, PieceKind::King)),
                (sq(5, 5), piece(Color::White, PieceKind::Rook)),
            ],
        ));

        // 飛車が(5,0)から段0の王手をかける。黒歩の手は捕獲を回避しないため、
        // 黒の受けは白王将の捕獲だけである。
        assert_eq!(game.play(step(sq(5, 5), sq(5, 0))), Ok(GameStatus::Ongoing));
        assert_eq!(
            game.play(step(sq(0, 0), sq(1, 1))),
            win(Color::Black, WinReason::RoyalCapture)
        );
    }

    #[test]
    fn article_21_3_c_and_31_r1_repetition_win_rescues_mate() {
        // D3-021-05/D3-031-05: 白飛の照準往復(全手が攻撃的着手)で反復履歴を
        // 蓄積すると、黒の非攻撃的な候補手(3,10)→(3,9)だけが4回目の同一局面を
        // 生じさせ、R1裁定(片側継続攻撃側=白の負け)を直ちに成立させる。この
        // 候補手は3項cの回避手段に数えられるため詰みと判定されず、実際に指すと
        // R1裁定で黒の勝ちになる。黒の攻撃連続数(1〜10手目のうち黒の5手は全て
        // 攻撃的)は候補手の評価で0へ戻る必要があり、実着手と仮想着手の遷移の
        // 一致(adjudication-refactor.md「R1の攻撃連続数」)を回帰として固定する。
        let start = position(
            Color::White,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(1, 8), piece(Color::Black, PieceKind::Rook)),
                (sq(3, 9), piece(Color::Black, PieceKind::Rook)),
                (sq(2, 9), piece(Color::Black, PieceKind::Pawn)),
                (sq(0, 5), piece(Color::White, PieceKind::Rook)),
                (sq(2, 1), piece(Color::White, PieceKind::GoldGeneral)),
                (sq(1, 11), piece(Color::White, PieceKind::Pawn)),
                (sq(7, 10), piece(Color::White, PieceKind::Pawn)),
                (sq(11, 0), piece(Color::White, PieceKind::King)),
            ],
        );
        let w_out = step(sq(0, 5), sq(2, 5));
        let w_back = step(sq(2, 5), sq(0, 5));
        let z_out = step(sq(1, 8), sq(1, 2));
        let z_back = step(sq(1, 2), sq(1, 8));
        let y_out = step(sq(3, 9), sq(3, 10));
        let candidate = step(sq(3, 10), sq(3, 9));

        let mut game = game_with_codes(start, &[RuleCode::R1]);
        let plies = [
            w_out, z_out, w_back, z_back, w_out, z_out, w_back, z_back, w_out, y_out, w_back,
        ];
        for (index, mv) in plies.into_iter().enumerate() {
            // 11手目(白の王手)の完了時も、反復による即時勝利が受けに残るため
            // 詰みにならない。
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing), "ply {}", index + 1);
        }
        assert_eq!(
            game.play(candidate),
            win(Color::Black, WinReason::Repetition)
        );

        // 対照: 同じ最終盤面へ履歴なしで到達すると、同じ白の王手が詰みになる。
        let fresh = position(
            Color::White,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(1, 8), piece(Color::Black, PieceKind::Rook)),
                (sq(3, 10), piece(Color::Black, PieceKind::Rook)),
                (sq(2, 9), piece(Color::Black, PieceKind::Pawn)),
                (sq(2, 5), piece(Color::White, PieceKind::Rook)),
                (sq(2, 1), piece(Color::White, PieceKind::GoldGeneral)),
                (sq(1, 11), piece(Color::White, PieceKind::Pawn)),
                (sq(7, 10), piece(Color::White, PieceKind::Pawn)),
                (sq(11, 0), piece(Color::White, PieceKind::King)),
            ],
        );
        let mut fresh_game = game_with_codes(fresh, &[RuleCode::R1]);
        assert_eq!(fresh_game.play(w_back), win(Color::White, WinReason::Mate));
    }

    #[test]
    fn article_21_5_lion_double_capture_of_both_royals_wins() {
        // D3-021-07: 獅子が1手の2段階移動で王将(玉将)と太子の両方を取ると、
        // その着手の終了時に勝利する(第12条4項)。
        let pieces = [
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(5, 5), piece(Color::Black, PieceKind::Lion)),
            (sq(5, 6), piece(Color::White, PieceKind::King)),
            (sq(6, 6), prince(Color::White)),
        ];
        let mut full_capture = game(position(Color::Black, &pieces));
        let double_capture = Move {
            from: sq(5, 5),
            mid: Some(sq(5, 6)),
            to: sq(6, 6),
            promote: false,
        };
        assert_eq!(
            full_capture.play(double_capture),
            win(Color::Black, WinReason::RoyalCapture)
        );

        // 境界: 第1段階の玉将捕獲だけで停止すると太子が残り、終局しない。
        let mut partial = game(position(Color::Black, &pieces));
        assert_eq!(
            partial.play(step(sq(5, 5), sq(5, 6))),
            Ok(GameStatus::Ongoing)
        );
    }

    #[test]
    fn article_21_6_resignation_ends_the_game() {
        // D3-021-08: 投了した側が敗者となる。盤面や履歴は裁定に関与しない。
        let mut game = Game::with_default_rules();
        assert_eq!(
            game.resign(Color::Black),
            win(Color::White, WinReason::Resignation)
        );
        assert_eq!(game.position(), &Position::initial());
        assert_eq!(game.ply_count(), 0);
    }

    #[test]
    fn article_21_7_draw_agreement_ends_the_game() {
        // D3-021-09: 双方の合意で対局は引き分けとして終局する。
        let mut game = Game::with_default_rules();
        assert_eq!(game.agree_draw(), draw(DrawReason::Agreement));
        assert_eq!(game.position(), &Position::initial());
    }

    #[test]
    fn article_21_10_r2_mate_judgement_uses_the_filtered_move_set() {
        // D3-021-10: 詰み判定の1手目(手番側の受け)にはR2禁止フィルタ後の
        // 対局合法手を用いる。唯一の非捕獲の受けが既出局面の再現となる局面は、
        // R1では継続だがR2では詰みになる(game-referee.md 6節)。
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

        let mut r2 = game_with_codes(
            position(Color::White, &pieces),
            &[RuleCode::R2, RuleCode::E2],
        );
        assert_eq!(r2.play(sequence[0]), Ok(GameStatus::Ongoing));
        assert_eq!(r2.play(sequence[1]), Ok(GameStatus::Ongoing));
        assert_eq!(r2.play(sequence[2]), win(Color::White, WinReason::Mate));
    }

    // ---------------------------------------------------------------
    // 第22条　駒枯れ
    // ---------------------------------------------------------------

    #[test]
    fn article_22_1_bare_side_without_a_capture_loses_immediately() {
        // D3-022-01: 条件成立時に裸側の手番でも、余分な駒を取れなければ
        // 余分な駒を持つ側の勝ちとなる。
        let mut game = game_with_codes(
            position(
                Color::Black,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(4, 4), piece(Color::Black, PieceKind::GoldGeneral)),
                    (sq(11, 11), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R1],
        );

        // 黒金の着手完了で条件が成立し、白(裸側)は金を取れないため黒の勝ち。
        assert_eq!(
            game.play(step(sq(4, 4), sq(4, 5))),
            win(Color::Black, WinReason::PieceExhaustion)
        );
    }

    #[test]
    fn plan_referee_5_extra_side_to_move_wins_without_grace() {
        // D3-022-09: 条件成立時に余分な駒を持つ側が手番なら、5項の猶予を
        // 適用せず第22条1項で直ちに裁定する(game-referee.md 5節「猶予の適用条件」)。
        let mut game = game_with_codes(
            position(
                Color::White,
                &[
                    (sq(4, 4), piece(Color::Black, PieceKind::King)),
                    (sq(8, 8), piece(Color::Black, PieceKind::Pawn)),
                    (sq(4, 5), piece(Color::White, PieceKind::Pawn)),
                    (sq(8, 11), piece(Color::White, PieceKind::Rook)),
                    (sq(11, 11), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R1],
        );

        // 白飛が黒歩を取っても非王駒は2枚残り、条件は未成立。
        assert_eq!(
            game.play(step(sq(8, 11), sq(8, 8))),
            Ok(GameStatus::Ongoing)
        );
        // 黒王将が白歩を取った時点で条件が成立し、手番は白(余分側)。猶予なし。
        assert_eq!(
            game.play(step(sq(4, 4), sq(4, 5))),
            win(Color::White, WinReason::PieceExhaustion)
        );
    }

    #[test]
    fn article_22_2_pawn_extra_piece_waits_until_promotion() {
        // D3-022-02: 余分な駒が歩兵なら金将へ成るまで裁定せず対局を継続する。
        // D3-022-10: 非捕獲の成りの時点では猶予を与えず直ちに勝ちとする
        // (game-referee.md 5節「成りの時点の裁定」)。白王将が(3,10)から成歩を
        // 取れる配置でも、成った時点で黒の勝ちが確定する。
        let mut game = game_with_codes(
            position(
                Color::White,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(4, 10), piece(Color::Black, PieceKind::Pawn)),
                    (sq(3, 9), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R1],
        );

        assert_eq!(
            game.play(step(sq(3, 9), sq(3, 10))),
            Ok(GameStatus::Ongoing)
        );
        assert_eq!(
            game.play(promoting(sq(4, 10), sq(4, 11))),
            win(Color::Black, WinReason::PieceExhaustion)
        );
    }

    #[test]
    fn article_22_2_repetition_still_adjudicates_during_the_wait() {
        // D3-022-02境界: 歩兵の成り待機中も反復など他の裁定は通常どおり働く。
        let mut game = game_with_codes(
            position(
                Color::Black,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(4, 10), piece(Color::Black, PieceKind::Pawn)),
                    (sq(3, 7), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R1],
        );
        let cycle = [
            step(sq(0, 0), sq(0, 1)),
            step(sq(3, 7), sq(3, 8)),
            step(sq(0, 1), sq(0, 0)),
            step(sq(3, 8), sq(3, 7)),
        ];

        for ply in 1..=11 {
            assert_eq!(
                game.play(cycle[(ply - 1) % cycle.len()]),
                Ok(GameStatus::Ongoing),
                "ply {ply}"
            );
        }
        assert_eq!(game.play(cycle[3]), draw(DrawReason::Repetition));
    }

    #[test]
    fn article_22_3_go_between_extra_piece_waits_until_promotion() {
        // D3-022-03: 余分な駒が仲人なら醉象へ成った時点で勝ちとなる。
        let mut game = game_with_codes(
            position(
                Color::White,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(4, 7), piece(Color::Black, PieceKind::GoBetween)),
                    (sq(3, 7), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R1],
        );

        assert_eq!(game.play(step(sq(3, 7), sq(3, 8))), Ok(GameStatus::Ongoing));
        assert_eq!(
            game.play(promoting(sq(4, 7), sq(4, 8))),
            win(Color::Black, WinReason::PieceExhaustion)
        );
    }

    #[test]
    fn article_22_4_immobile_pawn_and_lance_are_not_winning_extras() {
        // D3-022-04: 最奥段で移動不能となった歩兵・香車は勝利を成立させる
        // 余分な駒として数えず、裁定を行わず対局を継続する。
        for kind in [PieceKind::Pawn, PieceKind::Lance] {
            let mut game = game_with_codes(
                position(
                    Color::White,
                    &[
                        (sq(0, 0), piece(Color::Black, PieceKind::King)),
                        (sq(4, 11), piece(Color::Black, kind)),
                        (sq(11, 11), piece(Color::White, PieceKind::King)),
                    ],
                ),
                &[RuleCode::R1],
            );

            assert_eq!(
                game.play(step(sq(11, 11), sq(10, 11))),
                Ok(GameStatus::Ongoing),
                "{kind:?}"
            );
        }
    }

    // 第22条5項の猶予直前局面。黒金が白歩を取ると条件が成立し、白王将が
    // (4,7)から金を取り返せる。
    fn grace_predecessor() -> (Position, Move) {
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
    fn articles_22_5_and_22_6_grace_capture_leads_to_a_draw() {
        // D3-022-05/D3-022-06: 裸側が余分な駒を次の1手で取れる場合は裁定を
        // 保留し、取れば駒枯れ不成立。双方王駒のみとなり8項の引き分けになる。
        let (position, establishes_condition) = grace_predecessor();
        let mut game = game_with_codes(position, &[RuleCode::R1]);

        assert_eq!(game.play(establishes_condition), Ok(GameStatus::Ongoing));
        assert_eq!(
            game.play(step(sq(4, 7), sq(5, 6))),
            draw(DrawReason::PieceExhaustion)
        );
    }

    #[test]
    fn article_22_7_declining_the_grace_loses_immediately() {
        // D3-022-07: 猶予の1手で余分な駒を取らなければ、その着手の完了時に
        // 余分な駒を持つ側の勝ちとなる。保留は再付与されない。
        let (position, establishes_condition) = grace_predecessor();
        let mut game = game_with_codes(position, &[RuleCode::R1]);

        assert_eq!(game.play(establishes_condition), Ok(GameStatus::Ongoing));
        assert_eq!(
            game.play(step(sq(4, 7), sq(3, 7))),
            win(Color::Black, WinReason::PieceExhaustion)
        );
    }

    #[test]
    fn article_22_8_bare_royals_draw_but_prince_pairs_continue() {
        // D3-022-08: 双方に王駒1枚ずつだけが残れば自動的に引き分けとする。
        let mut kings = game_with_codes(
            position(
                Color::Black,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(11, 11), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R1],
        );
        assert_eq!(
            kings.play(step(sq(0, 0), sq(0, 1))),
            draw(DrawReason::PieceExhaustion)
        );

        // 境界: 王将と太子が併存する側がある間は第22条の条件自体が成立しない
        // (game-referee.md 5節)。
        let mut with_prince = game_with_codes(
            position(
                Color::Black,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(2, 0), prince(Color::Black)),
                    (sq(11, 11), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R1],
        );
        assert_eq!(
            with_prince.play(step(sq(0, 0), sq(0, 1))),
            Ok(GameStatus::Ongoing)
        );
    }

    #[test]
    fn plan_referee_5_capturing_promotion_that_creates_the_condition_takes_the_grace_path() {
        // D3-022-11: 捕獲を伴う成りで初めて条件が成立した場合は待機局面が
        // 存在しないため、2項ではなく1項・5項の通常の流れで猶予を判定する
        // (game-referee.md 5節但し書き、9節のS1レビュー境界)。
        let capturing_promotion = promoting(sq(5, 9), sq(5, 10));
        let condition_arises = |first_reply: Move| {
            let mut game = game_with_codes(
                position(
                    Color::Black,
                    &[
                        (sq(0, 0), piece(Color::Black, PieceKind::King)),
                        (sq(5, 9), piece(Color::Black, PieceKind::Pawn)),
                        (sq(5, 10), piece(Color::White, PieceKind::GoldGeneral)),
                        (sq(4, 11), piece(Color::White, PieceKind::King)),
                    ],
                ),
                &[RuleCode::R1],
            );
            assert_eq!(game.play(capturing_promotion), Ok(GameStatus::Ongoing));
            game.play(first_reply)
        };

        // 猶予の1手で成歩を取れば駒枯れ不成立、8項により引き分け。
        assert_eq!(
            condition_arises(step(sq(4, 11), sq(5, 10))),
            draw(DrawReason::PieceExhaustion)
        );
        // 取らなければ7項により余分な駒を持つ側の勝ち。
        assert_eq!(
            condition_arises(step(sq(4, 11), sq(3, 11))),
            win(Color::Black, WinReason::PieceExhaustion)
        );
    }

    #[test]
    fn articles_22_2_and_22_4_waiting_pawn_that_becomes_immobile_stops_counting() {
        // D3-022-12: 待機中の歩兵が最奥段で移動不能になると余分な駒として
        // 数えなくなり対局は継続する。その歩兵が取られれば双方王駒のみとなり
        // 8項の引き分けへ進む。
        let mut game = game_with_codes(
            position(
                Color::White,
                &[
                    (sq(7, 1), piece(Color::Black, PieceKind::King)),
                    (sq(8, 1), piece(Color::White, PieceKind::Pawn)),
                    (sq(11, 11), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R1],
        );

        // 白歩が段0(白の最奥段)へ不成のまま到達して移動不能になる。
        assert_eq!(game.play(step(sq(8, 1), sq(8, 0))), Ok(GameStatus::Ongoing));
        assert_eq!(
            game.play(step(sq(7, 1), sq(8, 0))),
            draw(DrawReason::PieceExhaustion)
        );
    }

    // ---------------------------------------------------------------
    // 第23条　合法手がない場合
    // ---------------------------------------------------------------

    #[test]
    fn article_23_1_no_move_at_all_loses_even_without_check() {
        // D3-023-01: 手番側に着手が1つも存在しなければ手番側の負けとなる。
        // F3では王手がかかっていない構成でも成立する(詰みとは独立の敗北条件)。
        let mut game = game(f3_no_move_position());

        assert_eq!(
            game.play(step(sq(10, 0), sq(10, 1))),
            win(Color::White, WinReason::Stalemate)
        );
    }

    #[test]
    fn article_23_2_unsafe_moves_still_count_as_legal_moves() {
        // D3-023-02: 王駒を相手の利きへ移す着手や王手を解消しない着手も
        // 合法手であり、これらが存在すれば第23条は適用されない。
        // 変形a: 香車の前方の駒を後手駒に差し替えると捕獲の着手が残る。
        let mut capture_variant = game(position(
            Color::White,
            &[
                (sq(0, 11), piece(Color::Black, PieceKind::King)),
                (sq(1, 11), piece(Color::White, PieceKind::Pawn)),
                (sq(0, 10), piece(Color::Black, PieceKind::Lance)),
                (sq(1, 10), piece(Color::Black, PieceKind::Lance)),
                (sq(10, 0), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(
            capture_variant.play(step(sq(10, 0), sq(10, 1))),
            Ok(GameStatus::Ongoing)
        );

        // 変形b: 王将を後手の利き(角の照準)へ移す着手も受理される(第8条3項)。
        let mut suicidal_variant = game(position(
            Color::Black,
            &[
                (sq(0, 11), piece(Color::Black, PieceKind::King)),
                (sq(1, 11), piece(Color::Black, PieceKind::Pawn)),
                (sq(1, 10), piece(Color::Black, PieceKind::Lance)),
                (sq(5, 5), piece(Color::White, PieceKind::Bishop)),
                (sq(10, 0), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(
            suicidal_variant.play(step(sq(0, 11), sq(0, 10))),
            Ok(GameStatus::Ongoing)
        );
    }

    #[test]
    fn article_23_3_r2_filtered_out_moves_cause_a_stalemate_loss() {
        // D3-023-03: 局面合法手がすべてR2で禁止されると対局合法手が空になり、
        // 第23条1項により手番側の負けとなる。E1併用でも同じ(D3-032-02)。
        let mut pieces = vec![
            (sq(0, 0), piece(Color::Black, PieceKind::GoBetween)),
            (sq(11, 11), piece(Color::Black, PieceKind::King)),
            (sq(10, 10), piece(Color::Black, PieceKind::Pawn)),
            (sq(10, 11), piece(Color::Black, PieceKind::Pawn)),
            (sq(11, 10), piece(Color::Black, PieceKind::Pawn)),
            (sq(5, 5), piece(Color::White, PieceKind::King)),
        ];
        pieces.extend((2..=11).map(|rank| (sq(0, rank), piece(Color::Black, PieceKind::Pawn))));

        for codes in [
            &[RuleCode::R2, RuleCode::E2][..],
            &[RuleCode::R2, RuleCode::E1, RuleCode::E2][..],
        ] {
            let mut game = game_with_codes(position(Color::White, &pieces), codes);
            assert_eq!(game.play(step(sq(5, 5), sq(5, 4))), Ok(GameStatus::Ongoing));
            assert_eq!(game.play(step(sq(0, 0), sq(0, 1))), Ok(GameStatus::Ongoing));
            assert_eq!(
                game.play(step(sq(5, 4), sq(5, 5))),
                win(Color::White, WinReason::Stalemate),
                "codes={codes:?}"
            );
        }
    }

    // ---------------------------------------------------------------
    // 第24〜25条　同一局面と反復規則の選択
    // ---------------------------------------------------------------

    #[test]
    fn articles_24_1_and_31_r1_f1_cycle_draws_at_ply_12() {
        // D3-024-01/D3-031-01: F1(初期局面の仲人往復)は④⑧⑫の完了時に初期
        // 局面の2〜4回目を生じさせ、全12手が非攻撃的なため⑫で引き分けになる。
        // 対局開始局面は履歴上の第1回として数える。
        let mut game = Game::new(Rules::ENGINE_DEFAULT);
        let f1 = [
            step(sq(3, 4), sq(3, 5)),
            step(sq(8, 7), sq(8, 6)),
            step(sq(3, 5), sq(3, 4)),
            step(sq(8, 6), sq(8, 7)),
        ];

        for ply in 1..=11 {
            assert_eq!(
                game.play(f1[(ply - 1) % f1.len()]),
                Ok(GameStatus::Ongoing),
                "ended at ply {ply}"
            );
        }
        assert_eq!(game.play(f1[3]), draw(DrawReason::Repetition));
        assert_eq!(game.ply_count(), 12);
    }

    #[test]
    fn article_24_1_b_side_to_move_distinguishes_identical_boards() {
        // D3-024-02: 盤面が同じでも手番側が異なる2局面は同一局面ではない。
        // 黒金の往復2周(4手)と白王将の三角移動(3手)で、7手目に開始盤面が
        // 白手番として再現される。R2はこれを既出局面の再現として禁止しない。
        let mut game = game_with_codes(
            position(
                Color::Black,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(3, 3), piece(Color::Black, PieceKind::GoldGeneral)),
                    (sq(8, 8), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R2, RuleCode::E2],
        );

        let plies = [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(7, 8)),
            step(sq(3, 4), sq(3, 3)),
            step(sq(7, 8), sq(7, 7)),
            step(sq(3, 3), sq(3, 4)),
            step(sq(7, 7), sq(8, 8)),
            step(sq(3, 4), sq(3, 3)),
        ];
        for (index, mv) in plies.into_iter().enumerate() {
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing), "ply {}", index + 1);
        }
    }

    #[test]
    fn article_24_1_c_lion_trigger_state_distinguishes_positions() {
        // D3-024-03: 先獅子による直後の捕獲禁止の有無が異なる2局面は同一局面
        // ではない。先獅子あり局面の既出は、後日の先獅子なし同一盤面の再現を
        // 禁止しない。
        let mut game = game_with_codes(
            position(
                Color::Black,
                &[
                    (sq(11, 11), piece(Color::Black, PieceKind::King)),
                    (sq(5, 4), piece(Color::Black, PieceKind::Pawn)),
                    (sq(8, 8), piece(Color::Black, PieceKind::Lion)),
                    (sq(8, 7), piece(Color::Black, PieceKind::GoldGeneral)),
                    (sq(2, 2), piece(Color::Black, PieceKind::GoldGeneral)),
                    (sq(5, 5), piece(Color::White, PieceKind::Lion)),
                    (sq(0, 0), piece(Color::White, PieceKind::Rook)),
                    (sq(11, 0), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R2],
        );

        // 黒歩が白獅子を取る。残る黒獅子(8,8)には金(8,7)の足があり、先獅子の
        // 捕獲禁止状態(第15条)を持つ局面が既出として記録される。
        assert_eq!(game.play(step(sq(5, 4), sq(5, 5))), Ok(GameStatus::Ongoing));
        assert_eq!(game.play(step(sq(0, 0), sq(0, 1))), Ok(GameStatus::Ongoing));
        assert_eq!(game.play(step(sq(2, 2), sq(2, 3))), Ok(GameStatus::Ongoing));
        assert_eq!(game.play(step(sq(0, 1), sq(0, 0))), Ok(GameStatus::Ongoing));
        // 同じ盤面へ先獅子なしで戻る着手は、同一局面の再現ではないため受理される。
        assert_eq!(game.play(step(sq(2, 3), sq(2, 2))), Ok(GameStatus::Ongoing));
        // 対照: 先獅子と無関係な既出局面(2手目完了時)の再現は禁止される。
        let forbidden = step(sq(0, 0), sq(0, 1));
        assert_eq!(
            game.play(forbidden),
            Err(GameError::IllegalMove {
                mv: forbidden,
                cause: IllegalMoveCause::Repetition,
            })
        );
    }

    #[test]
    fn article_24_1_occurrences_accumulate_across_different_paths() {
        // D3-024-05: 同一性は局面に基づき到達手順に依存しない。異なる経路
        // (黒金2枚を交互に往復)による出現も合算され、4回目で裁定される。
        let mut game = game_with_codes(
            position(
                Color::Black,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(3, 3), piece(Color::Black, PieceKind::GoldGeneral)),
                    (sq(5, 3), piece(Color::Black, PieceKind::GoldGeneral)),
                    (sq(11, 11), piece(Color::White, PieceKind::King)),
                    (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
                ],
            ),
            &[RuleCode::R1],
        );

        let cycle_a = [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
            step(sq(8, 7), sq(8, 8)),
        ];
        let cycle_b = [
            step(sq(5, 3), sq(5, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(5, 4), sq(5, 3)),
            step(sq(8, 7), sq(8, 8)),
        ];
        let plies: Vec<Move> = cycle_a
            .into_iter()
            .chain(cycle_b)
            .chain(cycle_a.into_iter().take(3))
            .collect();
        for (index, mv) in plies.into_iter().enumerate() {
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing), "ply {}", index + 1);
        }
        assert_eq!(game.play(cycle_a[3]), draw(DrawReason::Repetition));
    }

    #[test]
    fn article_25_2_repetition_rule_is_mandatory_and_exclusive() {
        // D3-025-01: 反復規則はR1・R2・R3のいずれか1つの明示が必須であり、
        // 未指定は明示的なエラー、2つ以上の同時指定は競合として拒否される。
        assert_eq!(
            Rules::from_codes(&[RuleCode::L0, RuleCode::P0, RuleCode::E0]),
            Err(RulesError::Missing(RuleGroup::Repetition))
        );

        for pair in [
            [RuleCode::R1, RuleCode::R2],
            [RuleCode::R1, RuleCode::R3],
            [RuleCode::R2, RuleCode::R3],
        ] {
            let codes = [RuleCode::L0, RuleCode::P0, pair[0], pair[1], RuleCode::E0];
            assert!(matches!(
                Rules::from_codes(&codes),
                Err(RulesError::Conflicting { .. })
            ));
        }

        for repetition in [RuleCode::R1, RuleCode::R2, RuleCode::R3] {
            let codes = [RuleCode::L0, RuleCode::P0, repetition, RuleCode::E0];
            let game = Game::new(Rules::from_codes(&codes).unwrap());
            assert_eq!(game.status(), GameStatus::Ongoing);
        }
        assert_eq!(
            Game::new(Rules::ENGINE_DEFAULT).status(),
            GameStatus::Ongoing
        );
    }

    // ---------------------------------------------------------------
    // 第26〜27条　不合法な着手とその効果
    // ---------------------------------------------------------------

    #[test]
    fn article_26_1_moving_the_opponents_piece_is_rejected() {
        // D3-026-01: 手番側が所有しない駒を動かす入力は着手として成立しない。
        let mut game = Game::with_default_rules();
        let white_pawn_move = step(sq(0, 8), sq(0, 7));

        assert!(matches!(
            game.play(white_pawn_move),
            Err(GameError::IllegalMove {
                cause: IllegalMoveCause::Movement,
                ..
            })
        ));
        assert_eq!(game.position(), &Position::initial());
        assert_eq!(game.ply_count(), 0);

        // 自分の駒を動かす合法手はそのまま受理される。
        let legal = game.legal_moves()[0];
        assert_eq!(game.play(legal), Ok(GameStatus::Ongoing));
    }

    #[test]
    fn article_26_12_no_moves_are_accepted_after_the_game_ends() {
        // D3-026-02: 終局済みの対局への着手入力は理由によらず拒否され、
        // 記録済みの結果は変化しない。終局後の対局合法手は空である。
        let mut agreed = Game::with_default_rules();
        let legal = agreed.legal_moves()[0];
        agreed.agree_draw().unwrap();
        assert_eq!(agreed.play(legal), Err(GameError::GameAlreadyOver));
        assert_eq!(
            agreed.result(),
            Some(GameResult::Draw {
                reason: DrawReason::Agreement,
            })
        );
        assert!(agreed.legal_moves().is_empty());
        assert_eq!(agreed.ply_count(), 0);

        let mut captured = game(position(
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
            captured.play(step(sq(5, 5), sq(5, 8))),
            Ok(GameStatus::Finished(result))
        );
        assert_eq!(
            captured.play(step(sq(0, 0), sq(0, 1))),
            Err(GameError::GameAlreadyOver)
        );
        assert_eq!(captured.result(), Some(result));
        assert!(captured.legal_moves().is_empty());
    }

    #[test]
    fn article_27_4_r2_r3_reject_before_acceptance_while_r1_adjudicates_after() {
        // D3-026-03/D3-027-02: 同じ着手列でも、R1は⑫を受理してから裁定し、
        // R2・R3は禁止対象の着手を受理前に不合法として拒否する。拒否後は
        // 別の着手で対局を継続できる。
        // D3-031-06: R2は2回目の再現から直ちに禁止する。
        // D3-031-10: R3は2回目・3回目の再現を合法とし4回目の出現だけを禁止する。
        let cycle = king_cycle();
        let alternative = step(sq(8, 7), sq(9, 7));

        let mut r1 = game_with_codes(king_cycle_position(), &[RuleCode::R1, RuleCode::E2]);
        for ply in 1..=11 {
            assert_eq!(
                r1.play(cycle[(ply - 1) % cycle.len()]),
                Ok(GameStatus::Ongoing),
                "R1 ply {ply}"
            );
        }
        assert_eq!(r1.play(cycle[3]), draw(DrawReason::Repetition));

        let mut r2 = game_with_codes(king_cycle_position(), &[RuleCode::R2, RuleCode::E2]);
        for mv in cycle.into_iter().take(3) {
            assert_eq!(r2.play(mv), Ok(GameStatus::Ongoing));
        }
        assert_eq!(
            r2.play(cycle[3]),
            Err(GameError::IllegalMove {
                mv: cycle[3],
                cause: IllegalMoveCause::Repetition,
            })
        );
        assert_eq!(r2.status(), GameStatus::Ongoing);
        assert_eq!(r2.play(alternative), Ok(GameStatus::Ongoing));

        let mut r3 = game_with_codes(king_cycle_position(), &[RuleCode::R3, RuleCode::E2]);
        for ply in 1..=11 {
            assert_eq!(
                r3.play(cycle[(ply - 1) % cycle.len()]),
                Ok(GameStatus::Ongoing),
                "R3 ply {ply}"
            );
        }
        assert_eq!(
            r3.play(cycle[3]),
            Err(GameError::IllegalMove {
                mv: cycle[3],
                cause: IllegalMoveCause::Repetition,
            })
        );
        assert_eq!(r3.status(), GameStatus::Ongoing);
        assert_eq!(r3.play(alternative), Ok(GameStatus::Ongoing));
    }

    #[test]
    fn article_27_1_rejected_moves_leave_the_game_state_unchanged() {
        // D3-027-01: 不合法な着手の拒否の前後で、局面・手番・手数・探索局面
        // キー履歴・対局合法手集合・対局状態がすべて不変である。
        let assert_rejection_is_pure = |game: &mut Game, rejected: Move| {
            let position_before = game.position().clone();
            let ply_before = game.ply_count();
            let keys_before = game.search_key_history().to_vec();
            let legal_before: HashSet<Move> = game.legal_moves().into_iter().collect();

            assert!(game.play(rejected).is_err());

            assert_eq!(game.position(), &position_before);
            assert_eq!(game.ply_count(), ply_before);
            assert_eq!(game.search_key_history(), keys_before.as_slice());
            assert_eq!(
                game.legal_moves().into_iter().collect::<HashSet<_>>(),
                legal_before
            );
            assert_eq!(game.status(), GameStatus::Ongoing);
        };

        // 駒の動きに反する入力(第26条2号)。
        let mut movement = Game::with_default_rules();
        assert_rejection_is_pure(&mut movement, step(sq(5, 5), sq(5, 6)));
        assert_eq!(
            movement.play(movement.legal_moves()[0]),
            Ok(GameStatus::Ongoing)
        );

        // R2が禁止する反復着手(第26条11号)。
        let cycle = king_cycle();
        let mut r2 = game_with_codes(king_cycle_position(), &[RuleCode::R2, RuleCode::E2]);
        for mv in cycle.into_iter().take(3) {
            assert_eq!(r2.play(mv), Ok(GameStatus::Ongoing));
        }
        assert_rejection_is_pure(&mut r2, cycle[3]);
        assert_eq!(r2.play(step(sq(8, 7), sq(9, 7))), Ok(GameStatus::Ongoing));

        // R3が禁止する4回目の出現(第26条11号)。
        let mut r3 = game_with_codes(king_cycle_position(), &[RuleCode::R3, RuleCode::E2]);
        for ply in 1..=11 {
            assert_eq!(
                r3.play(cycle[(ply - 1) % cycle.len()]),
                Ok(GameStatus::Ongoing)
            );
        }
        assert_rejection_is_pure(&mut r3, cycle[3]);
        assert_eq!(r3.play(step(sq(8, 7), sq(9, 7))), Ok(GameStatus::Ongoing));
    }

    // ---------------------------------------------------------------
    // 第31条　反復に関するローカルルール
    // ---------------------------------------------------------------

    #[test]
    fn article_31_r1_sole_continuous_checker_loses() {
        // D3-031-02: 反復区間を通じて一方(黒)のすべての着手だけが攻撃的着手
        // (連続王手)なら、4回目の同一局面の出現時に攻撃側の負けとなる。
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

        for ply in 1..=11 {
            assert_eq!(
                game.play(cycle[(ply - 1) % cycle.len()]),
                Ok(GameStatus::Ongoing),
                "ended at ply {ply}"
            );
        }
        assert_eq!(
            game.play(cycle[3]),
            win(Color::White, WinReason::Repetition)
        );
    }

    #[test]
    fn article_31_r1_capture_threats_count_and_standing_threats_do_not() {
        // D3-031-03: 攻撃的着手は王手に限らず、動かした駒が到達升から相手の
        // いずれかの駒を直ちに捕獲できる状態にする着手を含む。一方、動かさな
        // かった駒による既存の捕獲脅威は含めない。
        // (i) 黒飛が両往復升から白歩を照準し続ける: 黒が継続攻撃側となり負け。
        let mut threatening = game(position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(3, 3), piece(Color::Black, PieceKind::Rook)),
                (sq(3, 9), piece(Color::White, PieceKind::Pawn)),
                (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));
        let cycle = [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
            step(sq(8, 7), sq(8, 8)),
        ];
        for ply in 1..=11 {
            assert_eq!(
                threatening.play(cycle[(ply - 1) % cycle.len()]),
                Ok(GameStatus::Ongoing),
                "ended at ply {ply}"
            );
        }
        assert_eq!(
            threatening.play(cycle[3]),
            win(Color::White, WinReason::Repetition)
        );

        // (ii) 据え置きの黒飛が白歩を照準したまま、黒は金だけを往復する:
        // 既存の脅威は数えず双方非攻撃的なので引き分けになる。
        let mut standing = game(position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(3, 3), piece(Color::Black, PieceKind::Rook)),
                (sq(5, 5), piece(Color::Black, PieceKind::GoldGeneral)),
                (sq(3, 9), piece(Color::White, PieceKind::Pawn)),
                (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));
        let gold_cycle = [
            step(sq(5, 5), sq(5, 6)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(5, 6), sq(5, 5)),
            step(sq(8, 7), sq(8, 8)),
        ];
        for ply in 1..=11 {
            assert_eq!(
                standing.play(gold_cycle[(ply - 1) % gold_cycle.len()]),
                Ok(GameStatus::Ongoing),
                "ended at ply {ply}"
            );
        }
        assert_eq!(standing.play(gold_cycle[3]), draw(DrawReason::Repetition));
    }

    #[test]
    fn article_31_r1_mutual_perpetual_attacks_draw() {
        // D3-031-04: 双方のすべての着手が攻撃的着手なら、4回目の同一局面の
        // 出現時点で引き分けとなる。黒奔王は連続王手、白飛は黒歩の照準継続。
        let mut game = game(position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(3, 3), piece(Color::Black, PieceKind::FreeKing)),
                (sq(8, 0), piece(Color::Black, PieceKind::Pawn)),
                (sq(3, 10), piece(Color::White, PieceKind::King)),
                (sq(8, 8), piece(Color::White, PieceKind::Rook)),
            ],
        ));
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
        }
        assert_eq!(game.play(cycle[3]), draw(DrawReason::Repetition));
    }

    #[test]
    fn article_31_r2_applies_to_the_checked_side_but_never_to_captures() {
        // D3-031-07: 王手を受けている側にもR2の禁止は適用される(適用除外の
        // 不採用)。D3-031-08: 捕獲を含む着手は既出局面を再現し得ず、決して
        // 禁止されない(game-referee.md 6節の補題)。
        let mut game = game_with_codes(
            position(
                Color::White,
                &[
                    (sq(0, 1), piece(Color::Black, PieceKind::King)),
                    (sq(1, 0), piece(Color::White, PieceKind::King)),
                    (sq(10, 9), piece(Color::White, PieceKind::GoBetween)),
                ],
            ),
            &[RuleCode::R2, RuleCode::E2],
        );
        for mv in [
            step(sq(10, 9), sq(10, 8)),
            step(sq(0, 1), sq(0, 0)),
            step(sq(10, 8), sq(10, 9)),
        ] {
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        }

        // 白王将の利きの中で王手を受けたままでも、非捕獲の戻りは禁止される。
        let forbidden_noncapture = step(sq(0, 0), sq(0, 1));
        assert_eq!(
            game.play(forbidden_noncapture),
            Err(GameError::IllegalMove {
                mv: forbidden_noncapture,
                cause: IllegalMoveCause::Repetition,
            })
        );
        // 捕獲の着手は履歴と独立に合法であり、王駒捕獲の勝ちが成立する。
        assert_eq!(
            game.play(step(sq(0, 0), sq(1, 0))),
            win(Color::Black, WinReason::RoyalCapture)
        );
    }

    // ---------------------------------------------------------------
    // 第32条　終局に関するローカルルール
    // ---------------------------------------------------------------

    #[test]
    fn article_32_e1_defers_the_win_to_actual_royal_capture() {
        // D3-032-01: E1採用時は詰み局面でも終局せず、王手放置を含む合法手を
        // 指し続けられる。実際に最後の王駒が取られた着手の完了時に第21条1項で
        // 終局する。
        let (position, mv) = mate_predecessor();
        let mut game = game_with_codes(position, &[RuleCode::R1, RuleCode::E1, RuleCode::E2]);

        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        assert_eq!(game.result(), None);
        assert_eq!(game.play(step(sq(0, 0), sq(0, 1))), Ok(GameStatus::Ongoing));
        assert_eq!(
            game.play(step(sq(0, 11), sq(0, 1))),
            win(Color::White, WinReason::RoyalCapture)
        );
    }

    #[test]
    fn article_32_e1_leaves_other_adjudications_active() {
        // D3-032-02: E1が無効化するのは詰み裁定だけであり、駒枯れと合法手なし
        // の裁定はE1採用下でも通常どおり働く。
        let mut exhaustion = game_with_codes(
            position(
                Color::Black,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(4, 4), piece(Color::Black, PieceKind::GoldGeneral)),
                    (sq(11, 11), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R1, RuleCode::E1],
        );
        assert_eq!(
            exhaustion.play(step(sq(4, 4), sq(4, 5))),
            win(Color::Black, WinReason::PieceExhaustion)
        );

        let mut stalemate = game_with_codes(f3_no_move_position(), &[RuleCode::R1, RuleCode::E1]);
        assert_eq!(
            stalemate.play(step(sq(10, 0), sq(10, 1))),
            win(Color::White, WinReason::Stalemate)
        );
    }

    #[test]
    fn article_32_e2_disables_all_piece_exhaustion_adjudication() {
        // D3-032-03: E2採用時は第22条の裁定を一切行わず対局を継続する。
        let e2_game = |position| game_with_codes(position, &[RuleCode::R1, RuleCode::E2]);

        // 双方王駒のみ(8項相当)でも自動引き分けにしない。
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

        // 猶予が始まるはずの局面(第22条5項)でも裁定しない。
        let (grace_position, establishes_condition) = grace_predecessor();
        let mut condition = e2_game(grace_position);
        assert_eq!(
            condition.play(establishes_condition),
            Ok(GameStatus::Ongoing)
        );
        assert_eq!(
            condition.play(step(sq(4, 7), sq(3, 7))),
            Ok(GameStatus::Ongoing)
        );

        // 即時勝ちになるはずの局面(第22条1項)でも裁定しない。
        let mut immediate = e2_game(position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(4, 4), piece(Color::Black, PieceKind::GoldGeneral)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(
            immediate.play(step(sq(4, 4), sq(4, 5))),
            Ok(GameStatus::Ongoing)
        );
    }

    #[test]
    fn article_32_e3_enough_effective_pieces_adjudicate_the_bare_king_loss() {
        // D3-032-04: (a)王駒1枚以上を含む価値ある駒2枚以上、(b)裸玉側の王手
        // なし、(c)3枚以上または全て非隣接、がすべて成立すると裸玉側の負け。
        // 価値ある駒が3枚以上あれば隣接の有無は問わない。
        let mut adjacent_three = bare_king_game(position(
            Color::Black,
            &[
                (sq(5, 4), piece(Color::Black, PieceKind::King)),
                (sq(6, 5), piece(Color::White, PieceKind::GoldGeneral)),
                (sq(9, 0), piece(Color::White, PieceKind::Bishop)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(
            adjacent_three.play(step(sq(5, 4), sq(5, 5))),
            win(Color::White, WinReason::BareKing)
        );

        // 2枚でも裸玉に隣接していなければ(c)が成立して負けになる(D3-032-05境界)。
        let mut distant_two = bare_king_game(position(
            Color::Black,
            &[
                (sq(5, 4), piece(Color::Black, PieceKind::King)),
                (sq(9, 9), piece(Color::White, PieceKind::Rook)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(
            distant_two.play(step(sq(5, 4), sq(5, 5))),
            win(Color::White, WinReason::BareKing)
        );

        // 境界(a): 歩兵は価値ある駒でないため(第3条11項)、王将+歩兵では
        // 2枚に届かず裁定しない。
        let mut king_and_pawn = bare_king_game(position(
            Color::Black,
            &[
                (sq(5, 4), piece(Color::Black, PieceKind::King)),
                (sq(8, 8), piece(Color::White, PieceKind::Pawn)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(
            king_and_pawn.play(step(sq(5, 4), sq(5, 5))),
            Ok(GameStatus::Ongoing)
        );
    }

    #[test]
    fn article_32_e3_two_pieces_with_adjacency_defer_the_loss() {
        // D3-032-05: 価値ある駒が2枚でその一方が裸玉に隣接していれば(c)が
        // 不成立となり裁定を保留する。以後の着手完了時に隣接が解消されれば
        // 改めて成立する。
        let mut game = bare_king_game(position(
            Color::Black,
            &[
                (sq(5, 4), piece(Color::Black, PieceKind::King)),
                (sq(6, 5), piece(Color::White, PieceKind::GoldGeneral)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));

        assert_eq!(game.play(step(sq(5, 4), sq(5, 5))), Ok(GameStatus::Ongoing));
        // 白金が離れて隣接が解消された着手の完了時に、裸玉側の負けが確定する。
        assert_eq!(
            game.play(step(sq(6, 5), sq(7, 5))),
            win(Color::White, WinReason::BareKing)
        );
    }

    #[test]
    fn article_32_e3_bare_side_giving_check_defers_the_loss() {
        // D3-032-06: 裸玉側が相手王駒へ王手をかけている間は(b)が不成立となり
        // 裁定を保留する。王手が解消された後の着手完了時に改めて判定する。
        let mut game = bare_king_game(position(
            Color::Black,
            &[
                (sq(5, 4), piece(Color::Black, PieceKind::King)),
                (sq(0, 10), piece(Color::White, PieceKind::Rook)),
                (sq(6, 6), piece(Color::White, PieceKind::King)),
                (sq(10, 0), piece(Color::White, PieceKind::Rook)),
            ],
        ));

        // 黒王将が白玉将に隣接して王手をかけるため保留される。
        assert_eq!(game.play(step(sq(5, 4), sq(5, 5))), Ok(GameStatus::Ongoing));
        // 白玉将が王手を外れた着手の完了時に裁定が成立する。
        assert_eq!(
            game.play(step(sq(6, 6), sq(7, 7))),
            win(Color::White, WinReason::BareKing)
        );
    }

    #[test]
    fn article_32_e3_immobile_pawns_and_lances_are_excluded_from_counts() {
        // D3-032-07: 最奥段で移動不能となった歩兵・香車は、裸玉側の判定からも
        // 相手側の価値ある駒の数えからも除外する。
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
            win(Color::White, WinReason::BareKing)
        );

        // 相手側の死に香車を数えると3枚で(c)成立になるが、除外により2枚+隣接
        // として保留される。
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
    }

    #[test]
    fn article_32_e3_double_bare_kings_draw_unless_checked() {
        // D3-032-08: 双方が裸玉(死に駒を除く)で、いずれの王駒にも王手が
        // かかっていなければ引き分け。王手がかかっている間は裁定せず継続する。
        let mut unchecked = bare_king_game(position(
            Color::Black,
            &[
                (sq(5, 4), piece(Color::Black, PieceKind::King)),
                (sq(3, 11), piece(Color::Black, PieceKind::Pawn)),
                (sq(3, 0), piece(Color::White, PieceKind::Lance)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(
            unchecked.play(step(sq(5, 4), sq(5, 5))),
            draw(DrawReason::BareKing)
        );

        let mut checked = bare_king_game(position(
            Color::Black,
            &[
                (sq(5, 4), piece(Color::Black, PieceKind::King)),
                (sq(6, 6), piece(Color::White, PieceKind::King)),
            ],
        ));
        assert_eq!(
            checked.play(step(sq(5, 4), sq(5, 5))),
            Ok(GameStatus::Ongoing)
        );
    }

    #[test]
    fn article_33_9_e2_and_e3_conflict_while_e1_composes() {
        // D3-032-09: E1はE2またはE3と併用できるが、E2とE3は同時に採用できず
        // 有効な規則セットとして扱われない。
        assert!(matches!(
            Rules::from_codes(&[
                RuleCode::L0,
                RuleCode::P0,
                RuleCode::R1,
                RuleCode::E2,
                RuleCode::E3,
            ]),
            Err(RulesError::Conflicting { .. })
        ));
        for codes in [
            &[
                RuleCode::L0,
                RuleCode::P0,
                RuleCode::R1,
                RuleCode::E1,
                RuleCode::E2,
            ][..],
            &[
                RuleCode::L0,
                RuleCode::P0,
                RuleCode::R1,
                RuleCode::E1,
                RuleCode::E3,
            ][..],
        ] {
            assert_eq!(
                Game::new(Rules::from_codes(codes).unwrap()).status(),
                GameStatus::Ongoing
            );
        }
    }

    // ---------------------------------------------------------------
    // 横断的性質
    // ---------------------------------------------------------------

    #[test]
    fn plan_referee_7_all_rule_combinations_report_the_firing_adjudication() {
        // D3-PRP-01: 反復規則{R1,R2,R3}×終局例外{なし,E1,E2,E1+E2,E3,E1+E3}の
        // 有効な全組合せで、王駒捕獲は常に働き、詰みの有無はE1が、駒枯れ系は
        // E2・E3・標準のいずれか1つが、反復の裁定・フィルタは反復規則が決める。
        let exception_sets: [&[RuleCode]; 6] = [
            &[],
            &[RuleCode::E1],
            &[RuleCode::E2],
            &[RuleCode::E1, RuleCode::E2],
            &[RuleCode::E3],
            &[RuleCode::E1, RuleCode::E3],
        ];
        for repetition in [RuleCode::R1, RuleCode::R2, RuleCode::R3] {
            for exceptions in exception_sets {
                let mut codes = vec![RuleCode::L0, RuleCode::P0, repetition];
                if exceptions.contains(&RuleCode::E1) {
                    codes.push(RuleCode::E1);
                }
                codes.push(if exceptions.contains(&RuleCode::E3) {
                    RuleCode::E3
                } else if exceptions.contains(&RuleCode::E2) {
                    RuleCode::E2
                } else {
                    RuleCode::E0
                });
                let rules = Rules::from_codes(&codes).unwrap();

                // 王駒捕獲は全組合せで同じ結果になる。
                let mut capture = Game::from_position(
                    rules,
                    position(
                        Color::Black,
                        &[
                            (sq(0, 0), piece(Color::Black, PieceKind::King)),
                            (sq(5, 5), piece(Color::Black, PieceKind::Rook)),
                            (sq(5, 8), piece(Color::White, PieceKind::King)),
                        ],
                    ),
                );
                assert_eq!(
                    capture.play(step(sq(5, 5), sq(5, 8))),
                    win(Color::Black, WinReason::RoyalCapture),
                    "codes={codes:?}"
                );

                // 詰み局面: E3は裸玉裁定が先に成立し、E1は詰みだけを無効化する。
                let (mate_position, mate_move) = mate_predecessor();
                let mut mate = Game::from_position(rules, mate_position);
                let expected_mate = if rules.exhaustion == ExhaustionRule::E3 {
                    win(Color::White, WinReason::BareKing)
                } else if rules.e1 {
                    Ok(GameStatus::Ongoing)
                } else {
                    win(Color::White, WinReason::Mate)
                };
                assert_eq!(mate.play(mate_move), expected_mate, "codes={codes:?}");

                // 駒枯れ局面: 標準は第22条、E2は継続、E3は裸玉裁定に置き換わる。
                let mut exhaustion = Game::from_position(
                    rules,
                    position(
                        Color::Black,
                        &[
                            (sq(4, 4), piece(Color::Black, PieceKind::King)),
                            (sq(4, 5), piece(Color::White, PieceKind::Pawn)),
                            (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
                            (sq(11, 11), piece(Color::White, PieceKind::King)),
                        ],
                    ),
                );
                let expected_exhaustion = if rules.exhaustion == ExhaustionRule::E3 {
                    win(Color::White, WinReason::BareKing)
                } else if rules.exhaustion == ExhaustionRule::E2 {
                    Ok(GameStatus::Ongoing)
                } else {
                    win(Color::White, WinReason::PieceExhaustion)
                };
                assert_eq!(
                    exhaustion.play(step(sq(4, 4), sq(4, 5))),
                    expected_exhaustion,
                    "codes={codes:?}"
                );

                // 反復列: R1は受理後の裁定、R2は2回目、R3は4回目の再現を拒否する。
                let mut repeated = Game::from_position(rules, gold_cycle_position());
                let cycle = gold_cycle();
                let accepted_plies = match repetition {
                    RuleCode::R1 | RuleCode::R3 => 11,
                    RuleCode::R2 => 3,
                    _ => unreachable!(),
                };
                for ply in 0..accepted_plies {
                    assert_eq!(
                        repeated.play(cycle[ply % cycle.len()]),
                        Ok(GameStatus::Ongoing),
                        "codes={codes:?}, ply={}",
                        ply + 1
                    );
                }
                let terminal_move = cycle[accepted_plies % cycle.len()];
                let expected_repetition = match repetition {
                    RuleCode::R1 => draw(DrawReason::Repetition),
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

    #[test]
    fn plan_referee_7_piece_exhaustion_evaluates_before_mate() {
        // D3-PRP-02: 駒枯れと詰みが同一着手で同時に成立する場合、勝者はどちら
        // でも余分な駒を持つ側で一致し、駒枯れを先に評価するため終局理由は
        // 駒枯れとなる(game-referee.md 7節)。
        let mut game = game_with_codes(
            position(
                Color::White,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(2, 1), piece(Color::White, PieceKind::King)),
                    (sq(5, 5), piece(Color::White, PieceKind::Rook)),
                ],
            ),
            &[RuleCode::R1],
        );

        // 飛車の(0,5)への移動は黒王将の詰みを作ると同時に、黒が飛車を取れない
        // 駒枯れの勝ち条件も満たす。
        assert_eq!(
            game.play(step(sq(5, 5), sq(0, 5))),
            win(Color::White, WinReason::PieceExhaustion)
        );
    }

    // 終局理由が採用規則で発動し得る裁定かを検査する(D3-PRP-01)。
    fn assert_result_reason_allowed(rules: Rules, result: GameResult, name: &str) {
        let allowed = match result {
            GameResult::Win { reason, .. } => match reason {
                WinReason::RoyalCapture | WinReason::Stalemate => true,
                WinReason::Repetition => rules.repetition == RepetitionRule::R1,
                WinReason::Mate => !rules.e1,
                WinReason::PieceExhaustion => rules.exhaustion == ExhaustionRule::E0,
                WinReason::BareKing => rules.exhaustion == ExhaustionRule::E3,
                WinReason::Resignation => false,
            },
            GameResult::Draw { reason } => match reason {
                DrawReason::Repetition => rules.repetition == RepetitionRule::R1,
                DrawReason::PieceExhaustion => rules.exhaustion == ExhaustionRule::E0,
                DrawReason::BareKing => rules.exhaustion == ExhaustionRule::E3,
                DrawReason::Agreement => false,
            },
        };
        assert!(allowed, "rule_set={name}: unexpected result {result:?}");
    }

    #[test]
    fn representative_rule_sets_random_self_play_reaches_valid_terminations() {
        // D3-PRP-01横断: 代表規則セット群の決定的シードのランダム自己対局で、
        // 対局合法手はすべて受理され、終局理由は発動し得る裁定に限られる。
        const PLY_CAP: u32 = 1_500;
        const RULE_SETS: [(&str, &[RuleCode], u64); 9] = [
            (
                "L1+L2+P3+R1+E1",
                &[
                    RuleCode::L1,
                    RuleCode::L2,
                    RuleCode::P0,
                    RuleCode::P3,
                    RuleCode::R1,
                    RuleCode::E1,
                    RuleCode::E0,
                ],
                0x5255_4c45_4741_4d01,
            ),
            (
                "L3+P4+R1",
                &[
                    RuleCode::L0,
                    RuleCode::L3,
                    RuleCode::P0,
                    RuleCode::P4,
                    RuleCode::R1,
                    RuleCode::E0,
                ],
                0x5255_4c45_4741_4d02,
            ),
            (
                "P1+R1",
                &[RuleCode::L0, RuleCode::P1, RuleCode::R1, RuleCode::E0],
                0x5255_4c45_4741_4d03,
            ),
            (
                "P2+R1",
                &[RuleCode::L0, RuleCode::P2, RuleCode::R1, RuleCode::E0],
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
                    RuleCode::P0,
                    RuleCode::P3,
                    RuleCode::R1,
                    RuleCode::E1,
                    RuleCode::E3,
                ],
                0x5255_4c45_4741_4d07,
            ),
            (
                "L1+L3+P5+P6+R2+E1+E2",
                &[
                    RuleCode::L1,
                    RuleCode::L3,
                    RuleCode::P0,
                    RuleCode::P5,
                    RuleCode::P6,
                    RuleCode::R2,
                    RuleCode::E1,
                    RuleCode::E2,
                ],
                0x5255_4c45_4741_4d08,
            ),
            (
                "L4+P2+P5+P6+R2+E1+E2",
                &[
                    RuleCode::L0,
                    RuleCode::L4,
                    RuleCode::P2,
                    RuleCode::P5,
                    RuleCode::P6,
                    RuleCode::R2,
                    RuleCode::E1,
                    RuleCode::E2,
                ],
                0x5255_4c45_4741_4d09,
            ),
        ];

        for (rule_set_name, codes, seed) in RULE_SETS {
            let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
            let mut game = Game::new(Rules::from_codes(codes).unwrap());

            for _ in 0..PLY_CAP {
                // 継続中の対局には対局合法手が必ず残る(第23条により空なら終局済み)。
                let moves = game.legal_moves();
                assert!(!moves.is_empty(), "rule_set={rule_set_name}");
                let selected = moves[(rng.next() as usize) % moves.len()];
                let status = game.play(selected).unwrap_or_else(|error| {
                    panic!("rule_set={rule_set_name}: unexpected rejection: {error}")
                });
                if let GameStatus::Finished(result) = status {
                    assert_result_reason_allowed(game.rules(), result, rule_set_name);
                    break;
                }
            }

            assert!(
                matches!(game.status(), GameStatus::Finished(_)) || game.ply_count() == PLY_CAP,
                "random game ended prematurely: rule_set={rule_set_name}, ply={}",
                game.ply_count()
            );
        }
    }

    #[test]
    fn deterministic_random_r2_self_play_never_forbids_captures_and_terminates() {
        // D3-031-08性質: R2のランダム自己対局で拒否される着手は決して捕獲・
        // 成りを含まない(補題)。D3-PRP-01: R2では反復を理由とする終局がない。
        const GAMES_PER_RULE_SET: usize = 3;
        const PLY_CAP: u32 = 1_500;
        const SEED: u64 = 0x5232_5f53_4f41_4b21;

        let mut rng = XorShift64::new(NonZeroU64::new(SEED).unwrap());
        for codes in [
            &[RuleCode::L0, RuleCode::P0, RuleCode::R2, RuleCode::E0][..],
            &[
                RuleCode::L0,
                RuleCode::P0,
                RuleCode::R2,
                RuleCode::E1,
                RuleCode::E2,
            ][..],
        ] {
            for game_index in 0..GAMES_PER_RULE_SET {
                let mut game = Game::new(Rules::from_codes(codes).unwrap());
                let generator = MoveGenerator::new(game.rules().moves);
                let mut terminated = false;

                'game: for _ in 0..PLY_CAP {
                    // 局面合法手から選び、R2の受理前拒否(第27条4項)を観測する。
                    let mut moves = Vec::new();
                    generator.generate_moves(game.position(), &mut moves);
                    assert!(!moves.is_empty(), "codes={codes:?}");
                    let start = (rng.next() as usize) % moves.len();
                    let mut played = None;
                    for offset in 0..moves.len() {
                        let selected = moves[(start + offset) % moves.len()];
                        match game.play(selected) {
                            Ok(status) => {
                                played = Some(status);
                                break;
                            }
                            Err(GameError::IllegalMove {
                                mv,
                                cause: IllegalMoveCause::Repetition,
                            }) => {
                                assert_eq!(mv, selected);
                                assert!(!mv.promote, "R2 rejected a promotion: {mv:?}");
                                assert!(
                                    game.position()
                                        .captured_squares(mv)
                                        .into_iter()
                                        .all(|capture| capture.is_none()),
                                    "R2 rejected a capture: {mv:?}"
                                );
                            }
                            Err(error) => panic!("unexpected self-play error: {error}"),
                        }
                    }
                    let status = played
                        .expect("Article 23 must end the game before all moves are forbidden");
                    if let GameStatus::Finished(result) = status {
                        assert_result_reason_allowed(game.rules(), result, "R2");
                        terminated = true;
                        break 'game;
                    }
                }

                assert!(
                    terminated,
                    "R2 random game {game_index} with {codes:?} exceeded the {PLY_CAP}-ply cap"
                );
            }
        }
    }

    #[test]
    fn deterministic_random_default_rules_self_play_reaches_a_terminal_result() {
        // D3-PRP-01横断: エンジン既定規則の決定的ランダム自己対局は手数上限内に
        // 終局し、終局理由は既定規則で発動し得る裁定に限られる。
        const GAME_COUNT: usize = 4;
        const PLY_CAP: u32 = 1_500;
        const SEED: u64 = 0x4741_4d45_5f53_4f41;

        let mut rng = XorShift64::new(NonZeroU64::new(SEED).unwrap());
        for game_index in 0..GAME_COUNT {
            let mut game = Game::with_default_rules();
            let mut terminated = false;

            for _ in 0..PLY_CAP {
                let moves = game.legal_moves();
                assert!(!moves.is_empty(), "ongoing game {game_index} has no move");
                let selected = moves[(rng.next() as usize) % moves.len()];
                if let GameStatus::Finished(result) = game.play(selected).unwrap() {
                    assert_result_reason_allowed(game.rules(), result, "engine-default");
                    terminated = true;
                    break;
                }
            }

            assert!(
                terminated,
                "random game {game_index} exceeded the {PLY_CAP}-ply cap"
            );
        }
    }
}
