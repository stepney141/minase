//! Chess Engine Communication Protocolの同期アダプター。

use std::io::{self, BufRead, Write};

use crate::core::game::{DrawReason, GameResult, IllegalMoveCause, WinReason};
use crate::core::piece::Color;
use crate::core::position::Position;
use crate::core::rules::parse_rule_set;
use crate::notation::cecp;
use crate::notation::sfen::{SetupPosition, parse_extended_sfen};

use super::Protocol;
use super::engine::{Engine, EngineCommand, EngineReply, RejectReason, canonical_rules_text};

/// 中将棋用のCECPアダプター。
pub struct CecpProtocol {
    /// feature宣言のRuleSet既定値に使う起動時規則の正準表記。
    startup_rules_text: String,
}

impl CecpProtocol {
    /// エンジンの起動時active規則をfeature宣言の既定値として保持する。
    pub fn new(engine: &Engine) -> Self {
        Self {
            startup_rules_text: canonical_rules_text(engine.active_rule_codes()),
        }
    }

    /// 1コマンドを処理し、セッションを続けるかどうかを返す。
    fn handle_line(
        &self,
        engine: &mut Engine,
        line: &str,
        output: &mut dyn Write,
    ) -> io::Result<bool> {
        let tokens: Vec<_> = line.split_whitespace().collect();
        let Some(command) = tokens.first().copied() else {
            return Ok(true);
        };

        match command {
            "xboard" | "accepted" | "force" => {}
            "protover" => self.write_features(output)?,
            "rejected" => {
                let feature = tokens.get(1).copied().unwrap_or("");
                if matches!(feature, "setboard" | "usermove" | "ping") {
                    writeln!(
                        output,
                        "tellusererror minase requires the {feature} feature"
                    )?;
                    return Ok(false);
                }
            }
            "variant" => {
                let name = tokens.get(1).copied().unwrap_or("");
                if name != "chu" {
                    writeln!(output, "Error (unsupported variant): {name}")?;
                }
            }
            "new" => self.handle_new(engine, output)?,
            "setboard" => self.handle_setboard(engine, &tokens[1..], output)?,
            "usermove" => {
                let move_text = tokens.get(1).copied().unwrap_or("");
                self.handle_usermove(engine, move_text, output)?;
            }
            "ping" => {
                let argument = tokens.get(1).copied().unwrap_or("");
                writeln!(output, "pong {argument}")?;
            }
            "result" => {
                let _ = engine.handle(EngineCommand::EndGame);
            }
            "option" => self.handle_option(engine, line, output)?,
            "quit" => {
                let _ = engine.handle(EngineCommand::Quit);
                return Ok(false);
            }
            "time" | "otim" | "level" | "st" | "sd" | "easy" | "hard" | "post" | "nopost"
            | "random" | "computer" | "name" | "hint" | "draw" => {}
            "undo" | "remove" | "analyze" | "go" => {
                writeln!(output, "Error (command not supported): {command}")?;
            }
            _ => writeln!(output, "Error (unknown command): {command}")?,
        }
        Ok(true)
    }

    /// `protover`への応答としてfeature宣言を出力する。
    ///
    /// setboard・usermove・pingは動作の前提であり、拒否された場合は
    /// セッションを継続しない(handle_lineの`rejected`分岐)。
    fn write_features(&self, output: &mut dyn Write) -> io::Result<()> {
        writeln!(
            output,
            "feature myname=\"minase {}\"",
            env!("CARGO_PKG_VERSION")
        )?;
        writeln!(output, "feature variants=\"chu\"")?;
        writeln!(output, "feature setboard=1")?;
        writeln!(output, "feature usermove=1")?;
        writeln!(output, "feature ping=1")?;
        writeln!(output, "feature colors=0")?;
        writeln!(output, "feature sigint=0")?;
        writeln!(output, "feature sigterm=0")?;
        writeln!(output, "feature analyze=0")?;
        writeln!(output, "feature time=0")?;
        writeln!(output, "feature draw=0")?;
        writeln!(
            output,
            "feature option=\"RuleSet -string {}\"",
            self.startup_rules_text
        )?;
        writeln!(output, "feature done=1")
    }

    /// `new`を処理し、初期局面から対局を開始する。
    fn handle_new(&self, engine: &mut Engine, output: &mut dyn Write) -> io::Result<()> {
        if !matches!(
            engine.handle(EngineCommand::NewGame),
            EngineReply::Accepted { .. }
        ) {
            return writeln!(output, "Error (command not legal now): new");
        }

        let setup = SetupPosition {
            position: Position::initial(),
            lion_capture: None,
            next_move_number: 1,
        };
        if matches!(
            engine.handle(EngineCommand::SetPosition {
                setup,
                moves: Vec::new(),
            }),
            EngineReply::Accepted { .. }
        ) {
            Ok(())
        } else {
            writeln!(output, "Error (command not legal now): new")
        }
    }

    /// `setboard`のFEN風局面を解析して対局を設定する。
    fn handle_setboard(
        &self,
        engine: &mut Engine,
        fields: &[&str],
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let (Some(board), Some(side)) = (fields.first(), fields.get(1)) else {
            return writeln!(output, "tellusererror Illegal position");
        };
        // CECPの手番記号は白=先手でSFENと逆のため、読み替えて委譲する。
        let sfen_side = match *side {
            "w" => "b",
            "b" => "w",
            _ => return writeln!(output, "tellusererror Illegal position"),
        };
        let text = format!("{board} {sfen_side} - 1");
        let setup = match parse_extended_sfen(&text, engine.position_rules()) {
            Ok(setup) => setup,
            Err(_) => return writeln!(output, "tellusererror Illegal position"),
        };

        match engine.handle(EngineCommand::SetPosition {
            setup,
            moves: Vec::new(),
        }) {
            EngineReply::Accepted { .. } => Ok(()),
            EngineReply::Rejected(RejectReason::GameAlreadyOver) => {
                writeln!(output, "Error (command not legal now): setboard")
            }
            EngineReply::Rejected(_) => writeln!(output, "tellusererror Illegal position"),
        }
    }

    /// `usermove`の指し手を適用し、終局すれば結果行を出力する。
    fn handle_usermove(
        &self,
        engine: &mut Engine,
        move_text: &str,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let mv = if move_text == "@@@@" {
            let Some(mv) = representative_jitto(engine) else {
                return writeln!(output, "Illegal move: {move_text}");
            };
            mv
        } else {
            match cecp::parse(engine.game().position(), move_text) {
                Ok(mv) => mv,
                Err(_) => return writeln!(output, "Illegal move: {move_text}"),
            }
        };

        match engine.handle(EngineCommand::ApplyMove(mv)) {
            EngineReply::Accepted {
                newly_finished: Some(result),
                ..
            } => write_result(output, result),
            EngineReply::Accepted { .. } => Ok(()),
            EngineReply::Rejected(RejectReason::IllegalMove {
                cause: IllegalMoveCause::Movement,
                ..
            }) => writeln!(output, "Illegal move: {move_text}"),
            EngineReply::Rejected(RejectReason::IllegalMove {
                cause: IllegalMoveCause::Repetition,
                ..
            }) => writeln!(output, "Illegal move (repetition): {move_text}"),
            EngineReply::Rejected(_) => {
                writeln!(
                    output,
                    "Error (command not legal now): usermove {move_text}"
                )
            }
        }
    }

    /// `option`のRuleSet設定を処理する。他のoption名は黙って無視する。
    fn handle_option(
        &self,
        engine: &mut Engine,
        line: &str,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let Some(remainder) = line.trim_start().strip_prefix("option ") else {
            return Ok(());
        };
        let Some((name, value)) = remainder.split_once('=') else {
            return Ok(());
        };
        if name != "RuleSet" {
            return Ok(());
        }

        let accepted = parse_rule_set(value).is_ok_and(|codes| {
            matches!(
                engine.handle(EngineCommand::SetRules(codes)),
                EngineReply::Accepted { .. }
            )
        });
        if accepted {
            Ok(())
        } else {
            writeln!(output, "Error (invalid option value): {line}")
        }
    }
}

impl Protocol for CecpProtocol {
    fn run(
        &mut self,
        engine: &mut Engine,
        input: &mut dyn BufRead,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let mut line = String::new();
        loop {
            line.clear();
            if input.read_line(&mut line)? == 0 {
                break;
            }
            let keep_running = self.handle_line(engine, line.trim_end(), output)?;
            output.flush()?;
            if !keep_running {
                break;
            }
        }
        Ok(())
    }
}

/// `@@@@`入力へ割り当てる正準じっとを、移動元の内部密番号が最小の合法手から選ぶ。
fn representative_jitto(engine: &Engine) -> Option<crate::Move> {
    engine
        .game()
        .legal_moves()
        .into_iter()
        .filter(|mv| mv.mid.is_none() && mv.to == mv.from && !mv.promote)
        .min_by_key(|mv| mv.from.dense_index())
}

/// 対局結果をCECPの結果行(`1-0 {reason}`など)として出力する。
fn write_result(output: &mut dyn Write, result: GameResult) -> io::Result<()> {
    let (code, reason) = match result {
        GameResult::Win { winner, reason } => {
            let code = match winner {
                Color::Black => "1-0",
                Color::White => "0-1",
            };
            (code, win_reason_text(reason))
        }
        GameResult::Draw { reason } => ("1/2-1/2", draw_reason_text(reason)),
    };
    writeln!(output, "{code} {{{reason}}}")
}

/// 勝利理由を結果行の注記へ変換する。
const fn win_reason_text(reason: WinReason) -> &'static str {
    match reason {
        WinReason::RoyalCapture => "royal capture",
        WinReason::Mate => "checkmate",
        WinReason::Stalemate => "no legal moves",
        WinReason::Repetition => "repetition",
        WinReason::PieceExhaustion => "piece exhaustion",
        WinReason::BareKing => "bare king",
        WinReason::Resignation => "resignation",
    }
}

/// 引き分け理由を結果行の注記へ変換する。
const fn draw_reason_text(reason: DrawReason) -> &'static str {
    match reason {
        DrawReason::Repetition => "repetition",
        DrawReason::PieceExhaustion => "piece exhaustion",
        DrawReason::BareKing => "bare kings",
        DrawReason::Agreement => "agreement",
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::core::piece::{PieceCode, PieceKind};
    use crate::core::position::PositionBuilder;
    use crate::core::rules::RuleCode;
    use crate::protocol::UsiProtocol;
    use crate::protocol::engine::EngineLifecycle;
    use crate::test_util::sq;
    use crate::{GameStatus, Rules, WinReason, to_sfen};

    const INITIAL_BOARD: &str = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL";

    fn engine(codes: &[RuleCode]) -> Engine {
        Engine::new(codes.to_vec()).unwrap()
    }

    fn run(protocol: &mut CecpProtocol, engine: &mut Engine, input: &str) -> String {
        let mut input = Cursor::new(input.as_bytes());
        let mut output = Vec::new();
        protocol.run(engine, &mut input, &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    fn run_usi(protocol: &mut UsiProtocol, engine: &mut Engine, input: &str) -> String {
        let mut input = Cursor::new(input.as_bytes());
        let mut output = Vec::new();
        protocol.run(engine, &mut input, &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    fn board(position: &Position) -> String {
        to_sfen(position).split_once(' ').unwrap().0.to_owned()
    }

    fn position_with(pieces: &[(crate::Square, Color, PieceKind)]) -> Position {
        let mut builder = PositionBuilder::new(Color::Black);
        for &(square, color, kind) in pieces {
            builder.put(square, PieceCode::new(color, kind)).unwrap();
        }
        builder.finish().unwrap()
    }

    #[test]
    fn handshake_matches_the_complete_feature_transcript() {
        let startup = [RuleCode::E2, RuleCode::R1, RuleCode::L1];
        let mut engine = engine(&startup);
        let mut protocol = CecpProtocol::new(&engine);

        assert_eq!(
            run(&mut protocol, &mut engine, "xboard\nprotover 2\nquit\n"),
            concat!(
                "feature myname=\"minase ",
                env!("CARGO_PKG_VERSION"),
                "\"\n",
                "feature variants=\"chu\"\n",
                "feature setboard=1\n",
                "feature usermove=1\n",
                "feature ping=1\n",
                "feature colors=0\n",
                "feature sigint=0\n",
                "feature sigterm=0\n",
                "feature analyze=0\n",
                "feature time=0\n",
                "feature draw=0\n",
                "feature option=\"RuleSet -string L1,R1,E2\"\n",
                "feature done=1\n",
            )
        );
    }

    #[test]
    fn required_feature_rejection_terminates_but_other_rejections_are_ignored() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);
        let input = "rejected draw\naccepted setboard\nping 007\nrejected ping\nprotover 2\n";

        assert_eq!(
            run(&mut protocol, &mut engine, input),
            concat!(
                "pong 007\n",
                "tellusererror minase requires the ping feature\n"
            )
        );
    }

    #[test]
    fn variant_accepts_only_exact_chu() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "variant chu\nvariant shogi\nquit\n"
            ),
            "Error (unsupported variant): shogi\n"
        );
    }

    #[test]
    fn cecp_and_usi_side_to_move_conventions_are_cross_checked() {
        let mut cecp_engine = engine(&[RuleCode::R1]);
        let mut cecp_protocol = CecpProtocol::new(&cecp_engine);
        assert_eq!(
            run(
                &mut cecp_protocol,
                &mut cecp_engine,
                &format!("setboard {INITIAL_BOARD} w\n")
            ),
            ""
        );

        let mut usi_engine = engine(&[RuleCode::R1]);
        let mut usi_protocol = UsiProtocol::new(&usi_engine);
        assert_eq!(
            run_usi(
                &mut usi_protocol,
                &mut usi_engine,
                &format!("position sfen {INITIAL_BOARD} b - 1\n")
            ),
            ""
        );
        assert_eq!(
            cecp_engine.game().position().side_to_move(),
            usi_engine.game().position().side_to_move()
        );
        assert_eq!(cecp_engine.game().position().side_to_move(), Color::Black);

        assert_eq!(
            run(
                &mut cecp_protocol,
                &mut cecp_engine,
                &format!("setboard {INITIAL_BOARD} b\nquit\n")
            ),
            ""
        );
        assert_eq!(cecp_engine.game().position().side_to_move(), Color::White);
    }

    #[test]
    fn new_starts_immediately_and_rules_latch_until_the_next_new() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        assert_eq!(run(&mut protocol, &mut engine, "new\nusermove g4g5\n"), "");
        assert_eq!(engine.game().ply_count(), 1);
        assert_eq!(engine.lifecycle(), EngineLifecycle::InGame);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                concat!(
                    "option RuleSet=R2,E2\n",
                    "option RuleSet=P3,E2\n",
                    "option Unknown=value\n",
                    "option Button\n",
                )
            ),
            "Error (invalid option value): option RuleSet=P3,E2\n"
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R1]);
        assert_eq!(engine.pending_rule_codes(), &[RuleCode::R2, RuleCode::E2]);

        assert_eq!(run(&mut protocol, &mut engine, "new\nquit\n"), "");
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R2, RuleCode::E2]);
        assert_eq!(engine.lifecycle(), EngineLifecycle::InGame);
    }

    #[test]
    fn engine_default_preset_sets_r1_as_pending_rules() {
        let mut engine = engine(&[RuleCode::R2]);
        let mut protocol = CecpProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "option RuleSet=engine-default\nquit\n",
            ),
            ""
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R2]);
        assert_eq!(engine.pending_rule_codes(), &[RuleCode::R1]);
    }

    #[test]
    fn multileg_capture_and_igui_are_applied() {
        let two_stage_position = position_with(&[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(4, 5), Color::Black, PieceKind::Lion),
            (sq(3, 5), Color::White, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ]);
        let mut engine = engine(&[RuleCode::R1, RuleCode::E2]);
        let mut protocol = CecpProtocol::new(&engine);
        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!(
                    "setboard {} w\nusermove e6d6,d6c6\n",
                    board(&two_stage_position)
                )
            ),
            ""
        );
        assert_eq!(
            engine.game().position().piece_at(sq(2, 5)),
            Some(PieceCode::new(Color::Black, PieceKind::Lion))
        );
        assert_eq!(engine.game().position().piece_at(sq(3, 5)), None);

        let igui_position = position_with(&[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Lion),
            (sq(6, 6), Color::White, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ]);
        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!(
                    "setboard {} w\nusermove f6g7,g7f6\nquit\n",
                    board(&igui_position)
                )
            ),
            ""
        );
        assert_eq!(
            engine.game().position().piece_at(sq(5, 5)),
            Some(PieceCode::new(Color::Black, PieceKind::Lion))
        );
        assert_eq!(engine.game().position().piece_at(sq(6, 6)), None);
    }

    #[test]
    fn at_sign_jitto_uses_the_smallest_dense_origin() {
        let first = sq(5, 4);
        let second = sq(4, 5);
        let position = position_with(&[
            (sq(0, 0), Color::Black, PieceKind::King),
            (first, Color::Black, PieceKind::Lion),
            (second, Color::Black, PieceKind::Lion),
            (sq(11, 11), Color::White, PieceKind::King),
        ]);
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);
        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("setboard {} w\n", board(&position))
            ),
            ""
        );
        let candidates: Vec<_> = engine
            .game()
            .legal_moves()
            .into_iter()
            .filter(|mv| mv.mid.is_none() && mv.to == mv.from && !mv.promote)
            .collect();
        assert!(candidates.iter().any(|mv| mv.from == first));
        assert!(candidates.iter().any(|mv| mv.from == second));
        let expected = candidates
            .into_iter()
            .min_by_key(|mv| mv.from.dense_index())
            .unwrap();
        assert_eq!(expected.from, first);
        assert_eq!(representative_jitto(&engine), Some(expected));

        let mut expected_game = engine.game().clone();
        expected_game.play(expected).unwrap();
        assert_eq!(run(&mut protocol, &mut engine, "usermove @@@@\nquit\n"), "");
        assert_eq!(engine.game().ply_count(), 1);
        assert_eq!(engine.game().position(), expected_game.position());
    }

    #[test]
    fn promotion_suffix_is_applied() {
        let position = position_with(&[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(4, 7), Color::Black, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ]);
        let mut engine = engine(&[RuleCode::R1, RuleCode::E2]);
        let mut protocol = CecpProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("setboard {} w\nusermove e8e9+\nquit\n", board(&position))
            ),
            ""
        );
        assert_eq!(
            engine.game().position().piece_at(sq(4, 8)),
            PieceCode::new(Color::Black, PieceKind::Pawn).promote()
        );
    }

    #[test]
    fn movement_and_repetition_illegal_moves_use_distinct_responses() {
        let mut movement_engine = engine(&[RuleCode::R1]);
        let mut movement_protocol = CecpProtocol::new(&movement_engine);
        assert_eq!(
            run(
                &mut movement_protocol,
                &mut movement_engine,
                "new\nusermove a1b1\nquit\n"
            ),
            "Illegal move: a1b1\n"
        );

        let base = "12/12/12/8k3/12/12/12/12/3K8/12/12/12";
        let mut repetition_engine = engine(&[RuleCode::R2, RuleCode::E2]);
        let mut repetition_protocol = CecpProtocol::new(&repetition_engine);
        let input = format!(
            "setboard {base} w\nusermove d4d5\nusermove i9i8\nusermove d5d4\nusermove i8i9\nquit\n"
        );
        assert_eq!(
            run(&mut repetition_protocol, &mut repetition_engine, &input),
            "Illegal move (repetition): i8i9\n"
        );
        assert_eq!(repetition_engine.game().ply_count(), 3);
    }

    #[test]
    fn royal_capture_emits_one_result_and_later_move_only_reports_lifecycle_error() {
        let board = "12/12/12/5k6/12/12/5R6/12/12/12/12/K11";
        let mut engine = engine(&[RuleCode::R1, RuleCode::E2]);
        let mut protocol = CecpProtocol::new(&engine);
        let input = format!("setboard {board} w\nusermove f6f9\nusermove f6f9\nquit\n");

        assert_eq!(
            run(&mut protocol, &mut engine, &input),
            concat!(
                "1-0 {royal capture}\n",
                "Error (command not legal now): usermove f6f9\n",
            )
        );
        assert_eq!(
            engine.status(),
            GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::RoyalCapture,
            })
        );
    }

    #[test]
    fn residual_command_errors_unknown_command_and_ping_match_the_transcript() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                concat!(
                    "time 100\notim 100\nlevel 40 5 0\nst 1\nsd 2\neasy\nhard\n",
                    "post\nnopost\nrandom\ncomputer\nname player\nhint\ndraw\nforce\n",
                    "go\nundo\nremove\nanalyze\nmystery value\nping token-0042\nquit\n",
                )
            ),
            concat!(
                "Error (command not supported): go\n",
                "Error (command not supported): undo\n",
                "Error (command not supported): remove\n",
                "Error (command not supported): analyze\n",
                "Error (unknown command): mystery\n",
                "pong token-0042\n",
            )
        );
    }

    #[test]
    fn invalid_setboard_preserves_state_and_extra_fields_are_ignored() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);
        assert_eq!(run(&mut protocol, &mut engine, "new\n"), "");
        let position_before = engine.game().position().clone();
        let invalid_board = "13/12/12/12/12/12/12/12/12/12/12/12";

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("setboard {INITIAL_BOARD} x\nsetboard {invalid_board} w\nsetboard\n")
            ),
            concat!(
                "tellusererror Illegal position\n",
                "tellusererror Illegal position\n",
                "tellusererror Illegal position\n",
            )
        );
        assert_eq!(engine.game().position(), &position_before);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("setboard {INITIAL_BOARD} b invalid castling fields are ignored\nquit\n")
            ),
            ""
        );
        assert_eq!(engine.game().position().side_to_move(), Color::White);
        assert_eq!(
            engine.game().rules(),
            Rules::from_codes(&[RuleCode::R1]).unwrap()
        );
    }
}
