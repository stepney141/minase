//! 通信方式に依存しない対局状態機械。
//!
//! [`Engine`]は現局用の規則と次局用の規則を分け、次の表に従って遷移する。
//!
//! | コマンド | 受理条件と変更 | 受理後の状態 |
//! | --- | --- | --- |
//! | `NewGame` | 全状態で次局の規則を確定し、初期局面の新しい対局を作る。 | `AwaitingStart`になる。 |
//! | `SetPosition` | `AwaitingStart`なら次局の規則を確定し、`InGame`なら現局の規則で再構成する。`Finished`では`GameAlreadyOver`で拒否する。 | 終局していれば`Finished`、それ以外は`InGame`になる。 |
//! | `ExtendPosition`、`ReplaceLastMove`、`ApplyMove` | `InGame`だけで受理する。`AwaitingStart`では`GameNotStarted`、`Finished`では`GameAlreadyOver`で拒否する。 | 着手で終局した場合だけ`Finished`へ移る。 |
//! | `EndGame` | 全状態で対局を保持し、基底手数と末尾置換用の直前の対局を消す。 | `AwaitingStart`へ戻る。 |
//! | `SetRules` | 全状態で検証済みの次局の規則だけを置き換える。 | 状態を保つ。 |
//! | `Quit` | 状態を変えず受理し、セッションの終了をアダプターに任せる。 | 状態を保つ。 |
//!
//! 拒否したコマンドは規則、対局、およびライフサイクルを一切変えない。
//! 着手列の設定と差分の適用は複製上で全手を試し、成功時だけ交換する。
//! 1手の適用も[`Game::play`]の拒否時に対局を保つ。
//! `newly_finished`は、局面設定または着手で初めて終局した応答だけに裁定結果を持つ。
//!
//! `usinewgame`を送らないLishogi-Botにも対応するため、
//! `AwaitingStart`で最初に受理した`SetPosition`でも次局の規則を確定する。
//! 理由は`docs/adr/commit-pending-rules-on-first-position.md`にある。

use core::fmt;

use minase_core::game::{Game, GameError, GameResult, GameStatus, IllegalMoveCause};
use minase_core::mv::Move;
use minase_core::notation::sfen::SetupPosition;
use minase_core::position::PositionError;
use minase_core::rules::{RuleCode, Rules, RulesError};

/// 対局セッションのライフサイクル。遷移と拒否条件は[`crate::protocol::engine`]の表に従う。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EngineLifecycle {
    /// 規則は確定しているが、対局開始局面はまだ受信していない。
    AwaitingStart,
    /// 対局が進行中である。
    InGame,
    /// エンジン内部の裁定により対局が終了している。
    Finished,
}

/// 対局状態の変更とセッション終了を伝えるコマンド。
///
/// 探索の開始、停止、結果、および`moves`、`state`、`d`などの読み取り専用照会は含めない。
/// 照会はアダプターが[`Engine::game`]などを直接読む。
/// 理由は`docs/adr/call-search-from-protocol-modules.md`にある。
#[allow(clippy::large_enum_variant)] // 確定設計の値型境界を保ち、未要求の間接化を避ける。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EngineCommand {
    /// 受信時に[`Rules::from_codes`]で検証し、次局に適用する規則を置き換える。
    /// 同じコードの重複、同じ排他群の複数指定、および獅子、成り、反復、駒枯れの群の欠落を拒否する。
    /// 不正なら次局の規則を保ったまま[`RejectReason::InvalidRules`]を返す。
    /// 保持するコード列はL、P、R、Eの順、同じ群では番号順へ正準化する。
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
    /// 現局へ着手列を原子的に追加する。
    ExtendPosition {
        /// 現局から追加する着手列。
        moves: Vec<Move>,
    },
    /// 最後の着手列の末尾1手だけを原子的に置き換える。
    ///
    /// 受理した`SetPosition`または空でない`ExtendPosition`の最後の1手を適用する直前の対局を使う。
    /// 空の`ExtendPosition`はこの保存を保ち、`ApplyMove`、`NewGame`、`EndGame`は消す。
    /// 保存した対局の複製へ新しい1手だけを適用し、成功時に現局と交換する。
    /// 保存した起点は残すため、外れが続いても同じ起点から置き換えられる。
    /// 起点がなければ[`RejectReason::NoPreviousMove`]で拒否する。
    /// 先読みが外れ、末尾1手だけが異なる`position`を受けた際の全手再生を省くための操作である。
    /// 理由は`docs/adr/apply-position-by-token-difference.md`にある。
    ReplaceLastMove(Move),
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
    InvalidRules(RulesError),
    /// 開始局面が規則と整合しない。
    InvalidPosition(PositionError),
    /// 開始局面を受信する前に着手が送られた。
    GameNotStarted,
    /// 置き換える直前の着手がない。
    NoPreviousMove,
    /// 駒の動きまたは反復禁止規則により着手を適用できない。
    IllegalMove {
        /// 拒否された着手。
        mv: Move,
        /// 拒否の原因。
        cause: IllegalMoveCause,
    },
    /// 終局済みの対局へ着手または局面設定が送られた。
    GameAlreadyOver,
}

impl fmt::Display for RejectReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRules(error) => error.fmt(formatter),
            Self::InvalidPosition(error) => error.fmt(formatter),
            Self::NoPreviousMove => formatter.write_str("there is no previous move to replace"),
            Self::GameNotStarted => formatter.write_str("the game has not started"),
            Self::IllegalMove { mv, cause } => write!(formatter, "illegal move {mv:?}: {cause}"),
            Self::GameAlreadyOver => formatter.write_str("the game is already over"),
        }
    }
}

impl std::error::Error for RejectReason {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidRules(error) => Some(error),
            Self::InvalidPosition(error) => Some(error),
            Self::GameNotStarted
            | Self::NoPreviousMove
            | Self::IllegalMove { .. }
            | Self::GameAlreadyOver => None,
        }
    }
}

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
    /// [`EngineCommand::ReplaceLastMove`]用に保存する、着手列の末尾を適用する直前の対局。
    before_last_move: Option<Game>,
    /// [`Self::ply`]の基底手数。`SetPosition`で受けたSFENの次の手番号から1を引いた値。
    base_ply: u32,
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
        let game = Game::new(selection.rules);
        Ok(Self {
            game,
            before_last_move: None,
            base_ply: 0,
            active: selection.clone(),
            pending: selection,
            lifecycle: EngineLifecycle::AwaitingStart,
        })
    }

    /// コマンドを1個処理する。
    /// 受理条件、規則の確定、および拒否時の原子性は[`crate::protocol::engine`]のライフサイクル表に従う。
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
            EngineCommand::ExtendPosition { moves } => self.extend_position(&moves),
            EngineCommand::ReplaceLastMove(mv) => self.replace_last_move(mv),
            EngineCommand::ApplyMove(mv) => self.apply_move(mv),
            EngineCommand::EndGame => {
                self.before_last_move = None;
                self.base_ply = 0;
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
    /// [`EngineCommand::ReplaceLastMove`]の解析に使う直前の対局を返す。
    /// 保存と消去の条件は同コマンドの契約に従う。
    pub(crate) fn before_last_move(&self) -> Option<&Game> {
        self.before_last_move.as_ref()
    }

    /// 初形を0とする絶対手数を`base_ply + Game::ply_count()`で返す。
    ///
    /// `base_ply`は`SetPosition`のSFENの次の手番号から1を引いた値である。
    /// `ExtendPosition`と`ReplaceLastMove`は基底手数を保ち、`NewGame`と`EndGame`は基底手数を0に戻す。
    /// USIとCECPはこの値を[`crate::search::ClockLimits`]へ渡し、探索履歴の長さから推測しない。
    /// 理由は`docs/adr/estimate-moves-to-go-from-ply.md`にある。
    pub const fn ply(&self) -> u32 {
        self.base_ply + self.game.ply_count()
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
        let game = Game::new(self.pending.rules);
        self.active = self.pending.clone();
        self.game = game;
        self.before_last_move = None;
        self.base_ply = 0;
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
        let base_ply = setup.next_move_number() - 1;
        let position = match setup.restore_position() {
            Ok(position) => position,
            Err(error) => return EngineReply::Rejected(RejectReason::InvalidPosition(error)),
        };
        let mut game = Game::from_position(selection.rules, position);

        let mut before_last_move = None;
        for (index, &mv) in moves.iter().enumerate() {
            if index + 1 == moves.len() {
                before_last_move = Some(game.clone());
            }
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
        self.before_last_move = before_last_move;
        self.base_ply = base_ply;
        self.lifecycle = match status {
            GameStatus::Ongoing => EngineLifecycle::InGame,
            GameStatus::Finished(_) => EngineLifecycle::Finished,
        };
        EngineReply::Accepted {
            status,
            newly_finished,
        }
    }

    /// 現局の複製へ指し手列を適用し、全手が成功した場合だけ置き換える。
    fn extend_position(&mut self, moves: &[Move]) -> EngineReply {
        match self.lifecycle {
            EngineLifecycle::AwaitingStart => {
                return EngineReply::Rejected(RejectReason::GameNotStarted);
            }
            EngineLifecycle::Finished => {
                return EngineReply::Rejected(RejectReason::GameAlreadyOver);
            }
            EngineLifecycle::InGame => {}
        }

        let mut game = self.game.clone();
        let mut before_last_move = None;
        for (index, &mv) in moves.iter().enumerate() {
            if index + 1 == moves.len() {
                before_last_move = Some(game.clone());
            }
            if let Err(error) = game.play(mv) {
                return EngineReply::Rejected(reject_game_error(error));
            }
        }

        let status = game.status();
        let newly_finished = match status {
            GameStatus::Ongoing => None,
            GameStatus::Finished(result) => Some(result),
        };
        self.game = game;
        if !moves.is_empty() {
            self.before_last_move = before_last_move;
        }
        self.lifecycle = match status {
            GameStatus::Ongoing => EngineLifecycle::InGame,
            GameStatus::Finished(_) => EngineLifecycle::Finished,
        };
        EngineReply::Accepted {
            status,
            newly_finished,
        }
    }

    /// 保存した直前の対局から1手だけを適用し、成功時だけ交換する。
    fn replace_last_move(&mut self, mv: Move) -> EngineReply {
        match self.lifecycle {
            EngineLifecycle::AwaitingStart => {
                return EngineReply::Rejected(RejectReason::GameNotStarted);
            }
            EngineLifecycle::Finished => {
                return EngineReply::Rejected(RejectReason::GameAlreadyOver);
            }
            EngineLifecycle::InGame => {}
        }
        let Some(previous) = &self.before_last_move else {
            return EngineReply::Rejected(RejectReason::NoPreviousMove);
        };
        let mut game = previous.clone();
        let status = match game.play(mv) {
            Ok(status) => status,
            Err(error) => return EngineReply::Rejected(reject_game_error(error)),
        };
        self.game = game;
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

    /// 対局中の1手を適用し、終局すればライフサイクルを進める。
    fn apply_move(&mut self, mv: Move) -> EngineReply {
        match self.lifecycle {
            EngineLifecycle::AwaitingStart => {
                return EngineReply::Rejected(RejectReason::GameNotStarted);
            }
            EngineLifecycle::Finished => {
                return EngineReply::Rejected(RejectReason::GameAlreadyOver);
            }
            EngineLifecycle::InGame => {}
        }

        match self.game.play(mv) {
            Ok(status) => {
                // 1手ずつの適用は置き換えの対象にしないので、古い状態を残さない。
                self.before_last_move = None;
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
    let rules = Rules::from_codes(&codes).map_err(RejectReason::InvalidRules)?;
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
    use minase_core::WinReason;
    use minase_core::piece::{Color, PieceCode, PieceKind};
    use minase_core::position::{Position, PositionBuilder};
    use minase_core::test_util::sq;

    // 本モジュールの検査対象はEngineReply（公開境界）までとする（D6-ENG-07）。
    // 状態の不変・遷移は後続コマンドへの応答差で観測し、内部フィールドへは触れない。

    fn setup(position: Position) -> SetupPosition {
        SetupPosition::new(position, None, 1).unwrap()
    }

    fn step(from: minase_core::Square, to: minase_core::Square) -> Move {
        Move {
            from,
            mid: None,
            to,
            promote: false,
        }
    }

    /// 先手玉(3,3)・後手玉(8,8)の2王局面。反復系の基底に使う。
    fn kings_position() -> Position {
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(
                sq(3, 3),
                PieceCode::new(Color::Black, PieceKind::King).unwrap(),
            )
            .unwrap();
        builder
            .put(
                sq(8, 8),
                PieceCode::new(Color::White, PieceKind::King).unwrap(),
            )
            .unwrap();
        builder.finish().unwrap()
    }

    /// 開始局面へ戻る4手。末尾の1手が同一局面の出現を1回増やす。
    fn kings_cycle() -> Vec<Move> {
        vec![
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
            step(sq(8, 7), sq(8, 8)),
        ]
    }

    /// 先手が飛車で後手の最後の王駒を取る1手前の局面。
    fn royal_position() -> Position {
        let mut builder = PositionBuilder::new(Color::Black);
        for (square, color, kind) in [
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Rook),
            (sq(5, 8), Color::White, PieceKind::King),
        ] {
            builder
                .put(
                    square,
                    PieceCode::new(color, kind).expect("fixture uses an unpromoted-capable kind"),
                )
                .unwrap();
        }
        builder.finish().unwrap()
    }

    /// royal_position上の王駒捕獲手（RULES.md第21条）。
    fn royal_capture() -> Move {
        step(sq(5, 5), sq(5, 8))
    }

    #[test]
    fn failed_commit_is_atomic_and_keeps_active_pending_and_lifecycle() {
        // [`crate::protocol::engine`]のライフサイクル表: commitは複製上の全適用が成功した場合に限りactive規則・Game・
        // ライフサイクルを同時に交換する原子的操作（D6-ENG-01）。
        let mut engine =
            Engine::new(vec![RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E2]).unwrap();
        assert!(matches!(
            engine.handle(EngineCommand::SetRules(vec![
                RuleCode::L0,
                RuleCode::P0,
                RuleCode::R2,
                RuleCode::E2,
            ])),
            EngineReply::Accepted { .. }
        ));

        // 初期配置の(0,0)香車は自駒のある(0,1)へ動けない（RULES.md第26条）。
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
        // ライフサイクルは遷移していないため、AwaitingStartでの着手として拒否される。
        assert!(matches!(
            engine.handle(EngineCommand::ApplyMove(illegal)),
            EngineReply::Rejected(RejectReason::GameNotStarted)
        ));
        // pendingは失敗をまたいで保持され、次のcommitの検証にはpending規則(R2)が使われる。
        // R2は既出局面の再現を禁止する（RULES.md第31条R2、第27条第4項）。
        let moves = kings_cycle();
        let repeated = moves[3];
        assert_eq!(
            engine.handle(EngineCommand::SetPosition {
                setup: setup(kings_position()),
                moves,
            }),
            EngineReply::Rejected(RejectReason::IllegalMove {
                mv: repeated,
                cause: IllegalMoveCause::Repetition,
            })
        );
        // 成功するcommitは通常どおり受理される。
        assert!(matches!(
            engine.handle(EngineCommand::SetPosition {
                setup: setup(Position::initial()),
                moves: Vec::new(),
            }),
            EngineReply::Accepted {
                status: GameStatus::Ongoing,
                ..
            }
        ));
    }

    #[test]
    fn lifecycle_reports_each_finishing_transition_once_and_rejects_invalid_moves() {
        // [`crate::protocol::engine`]のライフサイクル表: newly_finishedはOngoingからFinishedへ遷移した応答だけが持つ。
        // 終局通知は状態遷移イベントであって状態述語ではない（D6-ENG-04）。
        let result = GameResult::Win {
            winner: Color::Black,
            reason: WinReason::RoyalCapture,
        };
        let mut engine =
            Engine::new(vec![RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E2]).unwrap();

        assert_eq!(
            engine.handle(EngineCommand::ApplyMove(royal_capture())),
            EngineReply::Rejected(RejectReason::GameNotStarted)
        );
        assert_eq!(
            engine.handle(EngineCommand::SetPosition {
                setup: setup(royal_position()),
                moves: Vec::new(),
            }),
            EngineReply::Accepted {
                status: GameStatus::Ongoing,
                newly_finished: None,
            }
        );
        assert_eq!(
            engine.handle(EngineCommand::ApplyMove(royal_capture())),
            EngineReply::Accepted {
                status: GameStatus::Finished(result),
                newly_finished: Some(result),
            }
        );
        // 終局後の着手は成立しない（RULES.md第26条第12項、D6-ENG-05のGameAlreadyOver）。
        assert_eq!(
            engine.handle(EngineCommand::ApplyMove(royal_capture())),
            EngineReply::Rejected(RejectReason::GameAlreadyOver)
        );
        assert_eq!(
            engine.handle(EngineCommand::SetPosition {
                setup: setup(Position::initial()),
                moves: Vec::new(),
            }),
            EngineReply::Rejected(RejectReason::GameAlreadyOver)
        );
        // EndGameはAwaitingStartへ戻るだけで、終局通知を再生成しない。
        assert_eq!(
            engine.handle(EngineCommand::EndGame),
            EngineReply::Accepted {
                status: GameStatus::Finished(result),
                newly_finished: None,
            }
        );
        // 終局局面を含む再構成では、その応答が「新たに終局を確定した」応答になる。
        assert_eq!(
            engine.handle(EngineCommand::SetPosition {
                setup: setup(royal_position()),
                moves: vec![royal_capture()],
            }),
            EngineReply::Accepted {
                status: GameStatus::Finished(result),
                newly_finished: Some(result),
            }
        );
        assert_eq!(
            engine.handle(EngineCommand::NewGame),
            EngineReply::Accepted {
                status: GameStatus::Ongoing,
                newly_finished: None,
            }
        );
        assert_eq!(
            engine.handle(EngineCommand::SetPosition {
                setup: setup(Position::initial()),
                moves: Vec::new(),
            }),
            EngineReply::Accepted {
                status: GameStatus::Ongoing,
                newly_finished: None,
            }
        );
        // ApplyMoveの拒否後も、同じ局面の合法手が受理される。
        let illegal = step(sq(0, 0), sq(0, 1));
        assert_eq!(
            engine.handle(EngineCommand::ApplyMove(illegal)),
            EngineReply::Rejected(RejectReason::IllegalMove {
                mv: illegal,
                cause: IllegalMoveCause::Movement,
            })
        );
        assert_eq!(
            engine.handle(EngineCommand::ApplyMove(step(sq(0, 3), sq(0, 4)))),
            EngineReply::Accepted {
                status: GameStatus::Ongoing,
                newly_finished: None,
            }
        );
    }

    #[test]
    fn extend_position_is_atomic_and_preserves_base_ply() {
        let mut engine =
            Engine::new(vec![RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E2]).unwrap();
        assert_eq!(
            engine.handle(EngineCommand::ExtendPosition { moves: Vec::new() }),
            EngineReply::Rejected(RejectReason::GameNotStarted)
        );

        let setup = SetupPosition::new(Position::initial(), None, 41).unwrap();
        assert!(matches!(
            engine.handle(EngineCommand::SetPosition {
                setup,
                moves: Vec::new(),
            }),
            EngineReply::Accepted { .. }
        ));
        let initial = engine.game().position().clone();
        assert_eq!(engine.ply(), 40);
        assert_eq!(
            engine.handle(EngineCommand::ExtendPosition { moves: Vec::new() }),
            EngineReply::Accepted {
                status: GameStatus::Ongoing,
                newly_finished: None,
            }
        );
        assert_eq!(engine.game().position(), &initial);
        assert_eq!(engine.ply(), 40);

        let legal = step(sq(0, 3), sq(0, 4));
        let illegal = step(sq(0, 0), sq(0, 1));
        assert_eq!(
            engine.handle(EngineCommand::ExtendPosition {
                moves: vec![legal, illegal],
            }),
            EngineReply::Rejected(RejectReason::IllegalMove {
                mv: illegal,
                cause: IllegalMoveCause::Movement,
            })
        );
        assert_eq!(engine.game().position(), &initial);
        assert_eq!(engine.ply(), 40);

        assert!(matches!(
            engine.handle(EngineCommand::ExtendPosition { moves: vec![legal] }),
            EngineReply::Accepted {
                status: GameStatus::Ongoing,
                newly_finished: None,
            }
        ));
        assert_eq!(engine.ply(), 41);
    }

    #[test]
    fn extend_position_reports_the_finishing_transition_once() {
        let result = GameResult::Win {
            winner: Color::Black,
            reason: WinReason::RoyalCapture,
        };
        let mut engine =
            Engine::new(vec![RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E2]).unwrap();
        assert!(matches!(
            engine.handle(EngineCommand::SetPosition {
                setup: SetupPosition::new(royal_position(), None, 12).unwrap(),
                moves: Vec::new(),
            }),
            EngineReply::Accepted { .. }
        ));
        assert_eq!(
            engine.handle(EngineCommand::ExtendPosition {
                moves: vec![royal_capture()],
            }),
            EngineReply::Accepted {
                status: GameStatus::Finished(result),
                newly_finished: Some(result),
            }
        );
        assert_eq!(engine.ply(), 12);
        assert_eq!(
            engine.handle(EngineCommand::ExtendPosition { moves: Vec::new() }),
            EngineReply::Rejected(RejectReason::GameAlreadyOver)
        );
    }

    #[test]
    fn set_rules_validates_on_receipt_and_keeps_pending_on_failure() {
        // [`crate::protocol::engine`]のライフサイクル表: SetRulesの検証はfrom_codesで反復規則を含む各群の存在を検査し、
        // commit時ではなく受信時に弾く（D6-ENG-02）。RULES.md第33条第5項（反復規則は常にいずれか1つ）・
        // 第9項（矛盾する組合せは無効）。
        let mut engine =
            Engine::new(vec![RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E0]).unwrap();
        assert!(matches!(
            engine.handle(EngineCommand::SetRules(vec![
                RuleCode::L0,
                RuleCode::P0,
                RuleCode::R2,
                RuleCode::E2,
            ])),
            EngineReply::Accepted { .. }
        ));

        // 反復規則を含まない列・排他違反・重複はいずれも受信時に拒否される。
        for codes in [
            vec![RuleCode::P3],
            vec![RuleCode::R1, RuleCode::R2],
            vec![RuleCode::L1, RuleCode::L1, RuleCode::R1],
        ] {
            assert!(matches!(
                engine.handle(EngineCommand::SetRules(codes)),
                EngineReply::Rejected(RejectReason::InvalidRules(_))
            ));
        }

        // 拒否はpendingを変えない: commitすると直前の正当なpending(R2,E2)が反映され、
        // R2の反復禁止として観測できる。
        assert!(matches!(
            engine.handle(EngineCommand::NewGame),
            EngineReply::Accepted { .. }
        ));
        let moves = kings_cycle();
        let repeated = moves[3];
        assert_eq!(
            engine.handle(EngineCommand::SetPosition {
                setup: setup(kings_position()),
                moves,
            }),
            EngineReply::Rejected(RejectReason::IllegalMove {
                mv: repeated,
                cause: IllegalMoveCause::Repetition,
            })
        );
    }
    // [`Engine::before_last_move`]と`docs/adr/apply-position-by-token-difference.md`（D6-ENG-08、D6-ENG-05）。
    #[test]
    fn ponder_replacement_preserves_history_and_failed_extensions_are_atomic() {
        let mut engine =
            Engine::new(vec![RuleCode::L0, RuleCode::P0, RuleCode::R2, RuleCode::E2]).unwrap();
        let cycle = kings_cycle();
        assert_eq!(
            engine.handle(EngineCommand::ReplaceLastMove(cycle[0])),
            EngineReply::Rejected(RejectReason::GameNotStarted)
        );
        assert!(matches!(
            engine.handle(EngineCommand::SetPosition {
                setup: SetupPosition::new(kings_position(), None, 41).unwrap(),
                moves: vec![],
            }),
            EngineReply::Accepted { .. }
        ));
        assert_eq!(
            engine.handle(EngineCommand::ReplaceLastMove(cycle[0])),
            EngineReply::Rejected(RejectReason::NoPreviousMove)
        );
        assert!(matches!(
            engine.handle(EngineCommand::ExtendPosition {
                moves: cycle[..2].to_vec()
            }),
            EngineReply::Accepted { .. }
        ));
        let alternate = step(sq(8, 8), sq(7, 8));
        assert!(matches!(
            engine.handle(EngineCommand::ReplaceLastMove(alternate)),
            EngineReply::Accepted { .. }
        ));
        assert_eq!(engine.ply(), 42);
        // 途中の1手が成功して末尾で失敗しても、置換に使う履歴まで不変である。
        assert!(matches!(
            engine.handle(EngineCommand::ExtendPosition {
                moves: vec![cycle[2], cycle[2]]
            }),
            EngineReply::Rejected(_)
        ));
        assert!(matches!(
            engine.handle(EngineCommand::ReplaceLastMove(cycle[1])),
            EngineReply::Accepted { .. }
        ));
        assert!(matches!(
            engine.handle(EngineCommand::ExtendPosition {
                moves: vec![cycle[2]]
            }),
            EngineReply::Accepted { .. }
        ));
        assert_eq!(
            engine.handle(EngineCommand::ExtendPosition {
                moves: vec![cycle[3]]
            }),
            EngineReply::Rejected(RejectReason::IllegalMove {
                mv: cycle[3],
                cause: IllegalMoveCause::Repetition
            })
        );
        assert_eq!(engine.ply(), 43);
    }
}
