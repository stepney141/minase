//! プロトコル非依存の対局状態機械。

use core::fmt;

use crate::core::game::{Game, GameError, GameResult, GameStatus, IllegalMoveCause};
use crate::core::mv::Move;
use crate::core::rules::{RuleCode, Rules};
use crate::notation::sfen::SetupPosition;

/// 対局セッションのライフサイクル。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EngineLifecycle {
    /// 規則は確定しているが、対局開始局面はまだ受信していない。
    AwaitingStart,
    /// 対局が進行中である。
    InGame,
    /// エンジン内部の裁定により対局が終了している。
    Finished,
}

/// プロトコルから対局状態機械へ渡すコマンド。
#[allow(clippy::large_enum_variant)] // 確定設計の値型境界を保ち、未要求の間接化を避ける。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EngineCommand {
    /// 次局に適用する規則コード列を検証してpendingへ保存する。
    SetRules(Vec<RuleCode>),
    /// pending規則をcommitし、開始局面の待機状態へ戻す。
    NewGame,
    /// 開始局面と着手列を原子的に適用する。
    SetPosition {
        /// 拡張SFENから得た開始局面。
        setup: SetupPosition,
        /// 開始局面から再適用する着手列。
        moves: Vec<Move>,
    },
    /// 現局へ1手を適用する。
    ApplyMove(Move),
    /// GUI発の終局通知を受け、次局の開始待ちへ戻る。
    EndGame,
    /// プロトコルセッションを終了する。
    Quit,
}

/// 対局状態機械が返す応答。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EngineReply {
    /// コマンドを受理した後の対局状態。
    Accepted {
        /// コマンド処理後の裁定状態。
        status: GameStatus,
        /// この応答で初めて終局した場合だけ存在する裁定結果。
        newly_finished: Option<GameResult>,
    },
    /// 状態を変更せずにコマンドを拒否した。
    Rejected(RejectReason),
}

/// コマンドを拒否した理由。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RejectReason {
    /// 規則コード列が不正、または実行可能な反復規則を含まない。
    InvalidRules(String),
    /// 開始局面またはライフサイクルが着手適用に適さない。
    InvalidPosition(String),
    /// 駒の動きまたは反復禁止規則により着手を適用できない。
    IllegalMove { mv: Move, cause: IllegalMoveCause },
    /// 終局済みの対局へ着手または局面設定が送られた。
    GameAlreadyOver,
}

impl fmt::Display for RejectReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRules(message) | Self::InvalidPosition(message) => {
                formatter.write_str(message)
            }
            Self::IllegalMove { mv, cause } => write!(formatter, "illegal move {mv:?}: {cause}"),
            Self::GameAlreadyOver => formatter.write_str("the game is already over"),
        }
    }
}

impl std::error::Error for RejectReason {}

/// 検証済みの規則コード列と、それが表す規則集合の組。
#[derive(Clone)]
struct RuleSelection {
    /// 正準順の規則コード列。option宣言などの表示に使う。
    codes: Vec<RuleCode>,
    /// コード列が表す規則集合。
    rules: Rules,
}

/// `Game`と次局用規則を所有するプロトコル非依存の状態機械。
pub struct Engine {
    /// 現在保持している対局。
    game: Game,
    /// 現局に適用中の規則。
    active: RuleSelection,
    /// 次局の開始時に適用する規則。
    pending: RuleSelection,
    /// 対局のライフサイクル。
    lifecycle: EngineLifecycle,
}

impl Engine {
    /// 起動時規則をactiveとpendingの両方へ設定して状態機械を構築する。
    ///
    /// 規則コードの重複・矛盾、または実行可能な反復規則の欠落は起動時に
    /// 拒否するため、構築後に規則未確定状態は存在しない。
    pub fn new(startup_codes: Vec<RuleCode>) -> Result<Self, RejectReason> {
        let selection = validate_rules(startup_codes)?;
        let game = Game::new(selection.rules)
            .map_err(|error| RejectReason::InvalidRules(error.to_string()))?;
        Ok(Self {
            game,
            active: selection.clone(),
            pending: selection,
            lifecycle: EngineLifecycle::AwaitingStart,
        })
    }

    /// コマンドを1個処理する。
    ///
    /// `ApplyMove`は`InGame`だけで受理する。`AwaitingStart`では開始局面がなく、
    /// `Finished`では終局済みなので拒否する。`SetPosition`は`AwaitingStart`なら
    /// pending規則をcommitし、`InGame`ならactive規則で現局を原子的に再構成する。
    pub fn handle(&mut self, command: EngineCommand) -> EngineReply {
        match command {
            EngineCommand::SetRules(codes) => match validate_rules(codes) {
                Ok(selection) => {
                    self.pending = selection;
                    self.accepted(None)
                }
                Err(reason) => EngineReply::Rejected(reason),
            },
            EngineCommand::NewGame => self.new_game(),
            EngineCommand::SetPosition { setup, moves } => self.set_position(setup, &moves),
            EngineCommand::ApplyMove(mv) => self.apply_move(mv),
            EngineCommand::EndGame => {
                self.lifecycle = EngineLifecycle::AwaitingStart;
                self.accepted(None)
            }
            EngineCommand::Quit => self.accepted(None),
        }
    }

    #[inline]
    /// 現在のライフサイクルを返す。
    pub const fn lifecycle(&self) -> EngineLifecycle {
        self.lifecycle
    }

    #[inline]
    /// 現在保持している対局の裁定状態を返す。
    pub const fn status(&self) -> GameStatus {
        self.game.status()
    }

    #[inline]
    /// 現在保持している対局を返す。
    pub const fn game(&self) -> &Game {
        &self.game
    }

    #[inline]
    /// 現局に適用中の規則を返す。
    pub fn active_rules(&self) -> Rules {
        self.active.rules
    }

    #[inline]
    /// 次局に適用する規則を返す。
    pub fn pending_rules(&self) -> Rules {
        self.pending.rules
    }

    #[inline]
    /// 現局に適用中の正準規則コード列を返す。
    pub fn active_rule_codes(&self) -> &[RuleCode] {
        &self.active.codes
    }

    #[inline]
    /// 次局に適用する正準規則コード列を返す。
    pub fn pending_rule_codes(&self) -> &[RuleCode] {
        &self.pending.codes
    }

    /// 次の`SetPosition`の解析に適用する規則を返す。
    #[inline]
    pub fn position_rules(&self) -> Rules {
        match self.lifecycle {
            EngineLifecycle::AwaitingStart => self.pending.rules,
            EngineLifecycle::InGame | EngineLifecycle::Finished => self.active.rules,
        }
    }

    /// pending規則をcommitして初期局面の対局を作り、開始待ちへ戻る。
    fn new_game(&mut self) -> EngineReply {
        let Ok(game) = Game::new(self.pending.rules) else {
            return EngineReply::Rejected(RejectReason::InvalidRules(
                "missing repetition rule".to_owned(),
            ));
        };
        self.active = self.pending.clone();
        self.game = game;
        self.lifecycle = EngineLifecycle::AwaitingStart;
        self.accepted(None)
    }

    /// 開始局面と指し手列から対局を再構成する。
    ///
    /// 指し手列の途中で拒否された場合は保持中の対局を変更しない。
    fn set_position(&mut self, setup: SetupPosition, moves: &[Move]) -> EngineReply {
        if self.lifecycle == EngineLifecycle::Finished {
            return EngineReply::Rejected(RejectReason::GameAlreadyOver);
        }

        let selection = match self.lifecycle {
            EngineLifecycle::AwaitingStart => &self.pending,
            EngineLifecycle::InGame => &self.active,
            EngineLifecycle::Finished => unreachable!(),
        };
        let mut position = setup.position.clone();
        if let Err(error) = position.set_lion_capture(setup.lion_capture) {
            return EngineReply::Rejected(RejectReason::InvalidPosition(error.to_string()));
        }
        let Ok(mut game) = Game::from_position(selection.rules, position) else {
            return EngineReply::Rejected(RejectReason::InvalidRules(
                "missing repetition rule".to_owned(),
            ));
        };

        for &mv in moves {
            if let Err(error) = game.play(mv) {
                return EngineReply::Rejected(reject_game_error(error));
            }
        }

        let status = game.status();
        let newly_finished = match status {
            GameStatus::Ongoing => None,
            GameStatus::Finished(result) => Some(result),
        };
        if self.lifecycle == EngineLifecycle::AwaitingStart {
            self.active = self.pending.clone();
        }
        self.game = game;
        self.lifecycle = match status {
            GameStatus::Ongoing => EngineLifecycle::InGame,
            GameStatus::Finished(_) => EngineLifecycle::Finished,
        };
        EngineReply::Accepted {
            status,
            newly_finished,
        }
    }

    /// 対局中の1手を適用し、終局すればライフサイクルを進める。
    fn apply_move(&mut self, mv: Move) -> EngineReply {
        match self.lifecycle {
            EngineLifecycle::AwaitingStart => {
                return EngineReply::Rejected(RejectReason::InvalidPosition(
                    "the game has not started".to_owned(),
                ));
            }
            EngineLifecycle::Finished => {
                return EngineReply::Rejected(RejectReason::GameAlreadyOver);
            }
            EngineLifecycle::InGame => {}
        }

        match self.game.play(mv) {
            Ok(status) => {
                let newly_finished = match status {
                    GameStatus::Ongoing => None,
                    GameStatus::Finished(result) => {
                        self.lifecycle = EngineLifecycle::Finished;
                        Some(result)
                    }
                };
                EngineReply::Accepted {
                    status,
                    newly_finished,
                }
            }
            Err(error) => EngineReply::Rejected(reject_game_error(error)),
        }
    }

    /// 現在の裁定状態を添えた受理応答を作る。
    fn accepted(&self, newly_finished: Option<GameResult>) -> EngineReply {
        EngineReply::Accepted {
            status: self.game.status(),
            newly_finished,
        }
    }
}

/// 規則コードをL、P、R、Eの順と同一カテゴリ内番号順へ正準化する。
pub(crate) fn canonical_rule_codes(codes: &[RuleCode]) -> Vec<RuleCode> {
    RuleCode::ALL
        .into_iter()
        .filter(|code| codes.contains(code))
        .collect()
}

/// 規則コードを正準順のコンマ区切りで返す。
pub(crate) fn canonical_rules_text(codes: &[RuleCode]) -> String {
    canonical_rule_codes(codes)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// 規則コード列の重複・矛盾と実行可能な反復規則の存在を検証する。
fn validate_rules(codes: Vec<RuleCode>) -> Result<RuleSelection, RejectReason> {
    let rules =
        Rules::from_codes(&codes).map_err(|error| RejectReason::InvalidRules(error.to_string()))?;
    Game::new(rules).map_err(|error| RejectReason::InvalidRules(error.to_string()))?;
    Ok(RuleSelection {
        codes: canonical_rule_codes(&codes),
        rules,
    })
}

/// 対局管理層のエラーを拒否理由へ写す。
fn reject_game_error(error: GameError) -> RejectReason {
    match error {
        GameError::GameAlreadyOver => RejectReason::GameAlreadyOver,
        GameError::IllegalMove { mv, cause } => RejectReason::IllegalMove { mv, cause },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::piece::{Color, PieceCode, PieceKind};
    use crate::core::position::{Position, PositionBuilder};
    use crate::test_util::sq;
    use crate::{WinReason, notation::usi};

    fn setup(position: Position) -> SetupPosition {
        SetupPosition {
            position,
            lion_capture: None,
            next_move_number: 1,
        }
    }

    fn step(from: crate::Square, to: crate::Square) -> Move {
        Move {
            from,
            mid: None,
            to,
            promote: false,
        }
    }

    #[test]
    fn failed_awaiting_start_commit_changes_no_engine_state() {
        let mut engine = Engine::new(vec![RuleCode::R1, RuleCode::E2]).unwrap();
        assert!(matches!(
            engine.handle(EngineCommand::SetPosition {
                setup: setup(Position::initial()),
                moves: Vec::new(),
            }),
            EngineReply::Accepted { .. }
        ));
        assert!(matches!(
            engine.handle(EngineCommand::SetRules(vec![RuleCode::R2, RuleCode::E2])),
            EngineReply::Accepted { .. }
        ));
        assert!(matches!(
            engine.handle(EngineCommand::EndGame),
            EngineReply::Accepted { .. }
        ));

        let position_before = engine.game.position().clone();
        let status_before = engine.status();
        let ply_before = engine.game.ply_count();
        let active_before = engine.active.clone();
        let pending_before = engine.pending.clone();
        let lifecycle_before = engine.lifecycle;
        let illegal = step(sq(0, 0), sq(0, 1));

        assert_eq!(
            engine.handle(EngineCommand::SetPosition {
                setup: setup(Position::initial()),
                moves: vec![illegal],
            }),
            EngineReply::Rejected(RejectReason::IllegalMove {
                mv: illegal,
                cause: IllegalMoveCause::Movement,
            })
        );
        assert_eq!(engine.game.position(), &position_before);
        assert_eq!(engine.status(), status_before);
        assert_eq!(engine.game.ply_count(), ply_before);
        assert_eq!(engine.active.rules, active_before.rules);
        assert_eq!(engine.active.codes, active_before.codes);
        assert_eq!(engine.pending.rules, pending_before.rules);
        assert_eq!(engine.pending.codes, pending_before.codes);
        assert_eq!(engine.lifecycle, lifecycle_before);
    }

    #[test]
    fn newly_finished_is_returned_only_by_the_finishing_response() {
        let mut builder = PositionBuilder::new(Color::Black);
        for (square, color, kind) in [
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Rook),
            (sq(5, 8), Color::White, PieceKind::King),
        ] {
            builder.put(square, PieceCode::new(color, kind)).unwrap();
        }
        let position = builder.finish().unwrap();
        let capture = step(sq(5, 5), sq(5, 8));
        assert_eq!(usi::text(&position, capture), "7g7d");
        let result = GameResult::Win {
            winner: Color::Black,
            reason: WinReason::RoyalCapture,
        };
        let mut engine = Engine::new(vec![RuleCode::R1, RuleCode::E2]).unwrap();

        assert_eq!(
            engine.handle(EngineCommand::SetPosition {
                setup: setup(position),
                moves: Vec::new(),
            }),
            EngineReply::Accepted {
                status: GameStatus::Ongoing,
                newly_finished: None,
            }
        );
        assert_eq!(
            engine.handle(EngineCommand::ApplyMove(capture)),
            EngineReply::Accepted {
                status: GameStatus::Finished(result),
                newly_finished: Some(result),
            }
        );
        assert_eq!(
            engine.handle(EngineCommand::ApplyMove(capture)),
            EngineReply::Rejected(RejectReason::GameAlreadyOver)
        );
        assert_eq!(
            engine.handle(EngineCommand::NewGame),
            EngineReply::Accepted {
                status: GameStatus::Ongoing,
                newly_finished: None,
            }
        );
    }

    #[test]
    fn set_rules_validates_repetition_and_preserves_pending_on_failure() {
        let mut engine = Engine::new(vec![RuleCode::R1]).unwrap();
        assert!(matches!(
            engine.handle(EngineCommand::SetRules(vec![RuleCode::P3, RuleCode::R2])),
            EngineReply::Accepted { .. }
        ));
        let pending = engine.pending_rule_codes().to_vec();

        assert!(matches!(
            engine.handle(EngineCommand::SetRules(vec![RuleCode::P3])),
            EngineReply::Rejected(RejectReason::InvalidRules(_))
        ));
        assert_eq!(engine.pending_rule_codes(), pending);
    }

    #[test]
    fn set_position_classifies_r2_rejection_and_preserves_the_previous_state() {
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(sq(3, 3), PieceCode::new(Color::Black, PieceKind::King))
            .unwrap();
        builder
            .put(sq(8, 8), PieceCode::new(Color::White, PieceKind::King))
            .unwrap();
        let position = builder.finish().unwrap();
        let moves = vec![
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
            step(sq(8, 7), sq(8, 8)),
        ];
        let repeated = moves[3];
        let mut engine = Engine::new(vec![RuleCode::R2, RuleCode::E2]).unwrap();
        let position_before = engine.game().position().clone();

        assert_eq!(
            engine.handle(EngineCommand::SetPosition {
                setup: setup(position),
                moves,
            }),
            EngineReply::Rejected(RejectReason::IllegalMove {
                mv: repeated,
                cause: IllegalMoveCause::Repetition,
            })
        );
        assert_eq!(engine.game().position(), &position_before);
        assert_eq!(engine.game().ply_count(), 0);
        assert_eq!(engine.lifecycle(), EngineLifecycle::AwaitingStart);
    }
}
