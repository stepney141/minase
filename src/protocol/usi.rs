//! Universal Shogi Interfaceの同期アダプター。

use std::io::{self, BufRead, Write};

use crate::core::game::{DrawReason, Game, GameResult, GameStatus, WinReason};
use crate::core::mv::Move;
use crate::core::piece::Color;
use crate::core::position::Position;
use crate::core::rules::parse_rule_set;
use crate::notation::sfen::{SetupPosition, parse_extended_sfen, to_sfen};
use crate::notation::usi;
use crate::search::{self, SearchConfig, TranspositionTable};

use super::Protocol;
use super::engine::{
    Engine, EngineCommand, EngineLifecycle, EngineReply, RejectReason, canonical_rules_text,
};

/// lishogi系拡張を含むUSIプロトコル。
pub struct UsiProtocol {
    startup_rules_text: String,
    transposition_table: Option<TranspositionTable>,
}

impl UsiProtocol {
    /// エンジンの起動時active規則をoption宣言の正準default値として保持する。
    ///
    /// セッション開始前に構築することで、宣言値と状態機械の起動時規則を
    /// 異なる値から作れないようにする。
    pub fn new(engine: &Engine) -> Self {
        Self {
            startup_rules_text: canonical_rules_text(engine.active_rule_codes()),
            transposition_table: None,
        }
    }

    fn handle_line(
        &mut self,
        engine: &mut Engine,
        line: &str,
        output: &mut dyn Write,
    ) -> io::Result<bool> {
        let tokens: Vec<_> = line.split_whitespace().collect();
        let Some(command) = tokens.first().copied() else {
            return Ok(true);
        };

        match command {
            "usi" => self.write_handshake(output)?,
            "isready" => writeln!(output, "readyok")?,
            "setoption" => self.handle_setoption(engine, &tokens[1..], output)?,
            "usinewgame" => self.apply_silent(engine, EngineCommand::NewGame, output)?,
            "position" => self.handle_position(engine, &tokens[1..], output)?,
            "gameover" => self.apply_silent(engine, EngineCommand::EndGame, output)?,
            "moves" => self.handle_moves(engine, output)?,
            "state" => self.handle_state(engine, output)?,
            "go" if tokens[1..].contains(&"mate") => {
                writeln!(output, "checkmate notimplemented")?;
            }
            "go" => self.handle_go(engine, &tokens[1..], output)?,
            "quit" => {
                let _ = engine.handle(EngineCommand::Quit);
                return Ok(false);
            }
            _ => {}
        }
        output.flush()?;
        Ok(true)
    }

    fn handle_go(
        &mut self,
        engine: &Engine,
        tokens: &[&str],
        output: &mut dyn Write,
    ) -> io::Result<()> {
        if engine.lifecycle() != EngineLifecycle::InGame {
            return write_error(output, "go requires an active game");
        }
        let config = match parse_go_config(tokens) {
            Ok(config) => config,
            Err(error) => return write_error(output, &error),
        };
        let game = engine.game();
        let root_moves = game.legal_moves();
        if root_moves.is_empty() {
            return write_error(output, "go requires at least one legal move");
        }
        let transposition_table = self
            .transposition_table
            .get_or_insert_with(TranspositionTable::default);
        let result = search::search(
            game.position(),
            engine.active_rules(),
            &root_moves,
            game.search_key_history(),
            &config,
            transposition_table,
        );
        writeln!(
            output,
            "bestmove {}",
            usi::text(game.position(), result.best_move)
        )
    }

    fn write_handshake(&self, output: &mut dyn Write) -> io::Result<()> {
        writeln!(output, "id name minase {}", env!("CARGO_PKG_VERSION"))?;
        writeln!(output, "id author stepney141")?;
        writeln!(
            output,
            "option name RuleSet type string default {}",
            self.startup_rules_text
        )?;
        writeln!(
            output,
            "option name USI_Variant type string default chushogi"
        )?;
        writeln!(output, "usiok")
    }

    fn handle_setoption(
        &mut self,
        engine: &mut Engine,
        tokens: &[&str],
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let name = token_after(tokens, "name");
        let value = token_after(tokens, "value");
        let Some(name) = name else {
            return Ok(());
        };

        if name.eq_ignore_ascii_case("RuleSet") {
            let Some(value) = value else {
                return write_error(output, "RuleSet requires a value");
            };
            let codes = match parse_rule_set(value) {
                Ok(codes) => codes,
                Err(error) => return write_error(output, &error),
            };
            self.apply_silent(engine, EngineCommand::SetRules(codes), output)
        } else if name.eq_ignore_ascii_case("USI_Variant") {
            match value {
                Some(value) if value.eq_ignore_ascii_case("chushogi") => Ok(()),
                Some(value) => {
                    write_error(output, &format!("unsupported USI_Variant value '{value}'"))
                }
                None => write_error(output, "USI_Variant requires a value"),
            }
        } else {
            Ok(())
        }
    }

    fn handle_position(
        &self,
        engine: &mut Engine,
        tokens: &[&str],
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let Some(kind) = tokens.first().copied() else {
            return write_error(output, "position requires startpos or sfen");
        };
        let moves_index = tokens.iter().position(|token| *token == "moves");
        let move_tokens = moves_index.map(|index| &tokens[index + 1..]).unwrap_or(&[]);
        let rules = engine.position_rules();
        let setup = match kind {
            "startpos" => SetupPosition {
                position: Position::initial(),
                lion_capture: None,
                next_move_number: 1,
            },
            "sfen" => {
                let end = moves_index.unwrap_or(tokens.len());
                match parse_sfen_fields(&tokens[1..end], rules) {
                    Ok(setup) => setup,
                    Err(error) => return write_error(output, &error.to_string()),
                }
            }
            _ => return Ok(()),
        };

        let parsed = match parse_moves(&setup, rules, move_tokens) {
            Ok(parsed) => parsed,
            Err(error) => return write_error(output, &error),
        };
        match engine.handle(EngineCommand::SetPosition {
            setup,
            moves: parsed.moves,
        }) {
            EngineReply::Accepted { .. } => Ok(()),
            EngineReply::Rejected(reason) => write_error(
                output,
                &position_reject_reason_text(&reason, parsed.first_rejected_text.as_deref()),
            ),
        }
    }

    fn handle_moves(&self, engine: &Engine, output: &mut dyn Write) -> io::Result<()> {
        if engine.lifecycle() != EngineLifecycle::InGame {
            return write_error(output, "moves requires an active game");
        }

        let game = engine.game();
        write!(output, "moves")?;
        for mv in game.legal_moves() {
            write!(output, " {}", usi::text(game.position(), mv))?;
        }
        writeln!(output)
    }

    fn handle_state(&self, engine: &Engine, output: &mut dyn Write) -> io::Result<()> {
        if engine.lifecycle() == EngineLifecycle::AwaitingStart {
            return write_error(output, "state requires an active or finished game");
        }

        let status = match state_status_text(engine.status()) {
            Ok(status) => status,
            Err(error) => return write_error(output, error),
        };
        writeln!(
            output,
            "state rules {} board {} status {status}",
            canonical_rules_text(engine.active_rule_codes()),
            to_sfen(engine.game().position()),
        )
    }

    fn apply_silent(
        &mut self,
        engine: &mut Engine,
        command: EngineCommand,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let clears_transposition_table = matches!(
            &command,
            EngineCommand::NewGame | EngineCommand::SetRules(_)
        );
        match engine.handle(command) {
            EngineReply::Accepted { .. } => {
                if clears_transposition_table
                    && let Some(transposition_table) = &mut self.transposition_table
                {
                    transposition_table.clear();
                }
                Ok(())
            }
            EngineReply::Rejected(reason) => write_error(output, &reason.to_string()),
        }
    }
}

fn parse_go_config(tokens: &[&str]) -> Result<SearchConfig, String> {
    if tokens.is_empty() {
        return Err("go requires depth or nodes".to_owned());
    }

    let mut depth = None;
    let mut nodes = None;
    let mut index = 0;
    while index < tokens.len() {
        let name = tokens[index];
        let value = tokens.get(index + 1).copied();
        match name {
            "depth" => {
                if depth.is_some() {
                    return Err("go depth must be specified once".to_owned());
                }
                let parsed = value
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|&value| value > 0)
                    .ok_or_else(|| "go depth must be a positive integer".to_owned())?;
                if parsed > search::MAX_PLY {
                    return Err(format!("go depth must not exceed {}", search::MAX_PLY));
                }
                depth = Some(parsed);
            }
            "nodes" => {
                if nodes.is_some() {
                    return Err("go nodes must be specified once".to_owned());
                }
                nodes = Some(
                    value
                        .and_then(|value| value.parse::<u64>().ok())
                        .filter(|&value| value > 0)
                        .ok_or_else(|| "go nodes must be a positive integer".to_owned())?,
                );
            }
            unsupported => return Err(format!("unsupported go argument '{unsupported}'")),
        }
        index += 2;
    }

    Ok(SearchConfig {
        depth: depth.unwrap_or(search::MAX_PLY),
        nodes,
    })
}

fn parse_sfen_fields(
    fields: &[&str],
    rules: crate::Rules,
) -> Result<SetupPosition, crate::notation::sfen::SfenError> {
    let four_field_text = fields.iter().take(4).copied().collect::<Vec<_>>().join(" ");
    let four_field_setup = parse_extended_sfen(&four_field_text, rules)?;
    let Some(fifth) = fields.get(4) else {
        return Ok(four_field_setup);
    };

    let five_field_text = fields.iter().take(5).copied().collect::<Vec<_>>().join(" ");
    match parse_extended_sfen(&five_field_text, rules) {
        Ok(setup) => Ok(setup),
        Err(error) if looks_like_promotion_deferred(fifth) => Err(error),
        Err(_) => Ok(four_field_setup),
    }
}

fn looks_like_promotion_deferred(field: &str) -> bool {
    field == "-" || field.contains(',') || field.as_bytes().first().is_some_and(u8::is_ascii_digit)
}

impl Protocol for UsiProtocol {
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
            if !self.handle_line(engine, line.trim_end(), output)? {
                break;
            }
        }
        Ok(())
    }
}

fn token_after<'a>(tokens: &'a [&str], key: &str) -> Option<&'a str> {
    tokens
        .iter()
        .position(|token| token.eq_ignore_ascii_case(key))
        .and_then(|index| tokens.get(index + 1).copied())
}

fn parse_moves(
    setup: &SetupPosition,
    rules: crate::Rules,
    move_tokens: &[&str],
) -> Result<ParsedMoves, String> {
    let mut position = setup.position.clone();
    position
        .set_lion_capture(setup.lion_capture)
        .map_err(|error| error.to_string())?;
    let mut game = Game::from_position(rules, position).map_err(|error| error.to_string())?;
    let mut moves = Vec::with_capacity(move_tokens.len());

    for &text in move_tokens {
        let mv = usi::parse(game.position(), text).map_err(|error| error.to_string())?;
        moves.push(mv);
        if game.play(mv).is_err() {
            return Ok(ParsedMoves {
                moves,
                first_rejected_text: Some(text.to_owned()),
            });
        }
    }
    Ok(ParsedMoves {
        moves,
        first_rejected_text: None,
    })
}

struct ParsedMoves {
    moves: Vec<Move>,
    first_rejected_text: Option<String>,
}

fn position_reject_reason_text(reason: &RejectReason, rejected_text: Option<&str>) -> String {
    if let (RejectReason::IllegalMove { cause, .. }, Some(text)) = (reason, rejected_text) {
        format!("illegal move '{text}': {cause}")
    } else {
        reason.to_string()
    }
}

fn state_status_text(status: GameStatus) -> Result<String, &'static str> {
    match status {
        GameStatus::Ongoing => Ok("ongoing".to_owned()),
        GameStatus::Finished(GameResult::Win { winner, reason }) => {
            let winner = match winner {
                Color::Black => "black",
                Color::White => "white",
            };
            let reason = match reason {
                WinReason::RoyalCapture => "royal-capture",
                WinReason::Repetition => "repetition",
                WinReason::PieceExhaustion => "piece-exhaustion",
                WinReason::BareKing => "bare-king",
                WinReason::Stalemate => "stalemate",
                WinReason::Mate => "mate",
                WinReason::Resignation => return Err("state cannot represent resignation"),
            };
            Ok(format!("win {winner} {reason}"))
        }
        GameStatus::Finished(GameResult::Draw { reason }) => {
            let reason = match reason {
                DrawReason::Repetition => "repetition",
                DrawReason::PieceExhaustion => "piece-exhaustion",
                DrawReason::BareKing => "bare-king",
                DrawReason::Agreement => return Err("state cannot represent agreement"),
            };
            Ok(format!("draw {reason}"))
        }
    }
}

fn write_error(output: &mut dyn Write, message: &str) -> io::Result<()> {
    writeln!(output, "info string error: {message}")
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::io::Cursor;

    use super::*;
    use crate::core::piece::Color;
    use crate::core::rules::RuleCode;
    use crate::protocol::engine::EngineLifecycle;
    use crate::{GameResult, GameStatus, Rules, WinReason};

    const INITIAL_SFEN: &str = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b - 1";

    fn engine(codes: &[RuleCode]) -> Engine {
        Engine::new(codes.to_vec()).unwrap()
    }

    fn run(protocol: &mut UsiProtocol, engine: &mut Engine, input: &str) -> String {
        let mut input = Cursor::new(input.as_bytes());
        let mut output = Vec::new();
        protocol.run(engine, &mut input, &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn handshake_and_readiness_match_the_complete_transcript() {
        let startup = [RuleCode::E2, RuleCode::R1, RuleCode::L1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        assert_eq!(
            run(&mut protocol, &mut engine, "usi\nisready\nquit\n"),
            concat!(
                "id name minase ",
                env!("CARGO_PKG_VERSION"),
                "\n",
                "id author stepney141\n",
                "option name RuleSet type string default L1,R1,E2\n",
                "option name USI_Variant type string default chushogi\n",
                "usiok\n",
                "readyok\n",
            )
        );
    }

    #[test]
    fn ruleset_accepts_mixed_case_and_invalid_values_preserve_pending() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let input = concat!(
            "setoption name RuleSet value p3,r2,e2\n",
            "setoption name RuleSet value R2,ZZ\n",
            "setoption name RuleSet value R2,R2\n",
            "setoption name RuleSet value P3,E2\n",
            "quit\n",
        );

        assert_eq!(
            run(&mut protocol, &mut engine, input),
            concat!(
                "info string error: unknown rule code 'ZZ'\n",
                "info string error: duplicate rule code: R2\n",
                "info string error: missing repetition rule\n",
            )
        );
        assert_eq!(
            engine.pending_rule_codes(),
            &[RuleCode::P3, RuleCode::R2, RuleCode::E2]
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R1]);
    }

    #[test]
    fn ruleset_preset_is_accepted_and_combination_error_preserves_pending() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let input = concat!(
            "setoption name RuleSet value lishogi\n",
            "setoption name RuleSet value lishogi,P1\n",
            "quit\n",
        );

        assert_eq!(
            run(&mut protocol, &mut engine, input),
            "info string error: rule set preset 'lishogi' must be specified alone\n"
        );
        assert_eq!(
            engine.pending_rule_codes(),
            &[
                RuleCode::L1,
                RuleCode::L2,
                RuleCode::P3,
                RuleCode::R1,
                RuleCode::E1,
                RuleCode::E3,
            ]
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R1]);
    }

    #[test]
    fn variant_accepts_chushogi_case_insensitively_and_rejects_other_values() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                concat!(
                    "setoption name USI_Variant value CHUSHOGI\n",
                    "setoption name USI_Variant value shogi\n",
                    "quit\n",
                )
            ),
            "info string error: unsupported USI_Variant value 'shogi'\n"
        );
    }

    #[test]
    fn engine_default_preset_sets_r1_as_pending_rules() {
        let startup = [RuleCode::R2];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "setoption name RuleSet value engine-default\nquit\n",
            ),
            ""
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R2]);
        assert_eq!(engine.pending_rule_codes(), &[RuleCode::R1]);
    }

    #[test]
    fn startpos_script_applies_two_stage_jitto_promotion_and_igui_moves() {
        let startup = [RuleCode::R1, RuleCode::E2];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        // 20手目は2段階移動、26手目はじっと、31手目は成り、36手目は居喰い。
        let moves = "6i6h 6c7e 10l11k 4a4b 10i10h 7e5e 6h6g 7c6c 7j5h 5e5f 9j11h 8c7c 7i7h 10d10e 6j11e 9c11e 5h4g 8b9c 1i1h 5f6g7h 8j6j 7h8i7i 4g3g 9c10d 11h9f 7i8j7i 9f9g 7i8k 3g2g 8k7k8j 9g6d+ 5a4a 3l2k 8j10l 9h9g 10l10k10l";

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("position startpos moves {moves}\nquit\n")
            ),
            ""
        );
        assert_eq!(engine.lifecycle(), EngineLifecycle::InGame);
        assert_eq!(engine.game().ply_count(), 36);
        assert_eq!(engine.status(), GameStatus::Ongoing);
    }

    #[test]
    fn moves_matches_the_legal_move_notation_set() {
        let startup = [RuleCode::R1, RuleCode::E2];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        let output = run(&mut protocol, &mut engine, "position startpos\nmoves\n");
        let mut fields = output.split_whitespace();
        assert_eq!(fields.next(), Some("moves"));
        let actual: HashSet<_> = fields.map(str::to_owned).collect();
        let expected: HashSet<_> = engine
            .game()
            .legal_moves()
            .into_iter()
            .map(|mv| usi::text(engine.game().position(), mv))
            .collect();

        assert_eq!(actual, expected);
    }

    #[test]
    fn queries_report_explicit_errors_before_the_game_starts() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        assert_eq!(
            run(&mut protocol, &mut engine, "moves\nstate\nquit\n"),
            concat!(
                "info string error: moves requires an active game\n",
                "info string error: state requires an active or finished game\n",
            )
        );
    }

    #[test]
    fn state_reports_exact_ongoing_and_finished_lines() {
        let startup = [RuleCode::E2, RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        assert_eq!(
            run(&mut protocol, &mut engine, "position startpos\nstate\n"),
            concat!(
                "state rules R1,E2 board ",
                "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/",
                "3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/",
                "A1B1TOXT1B1A/LFCSGKEGSCFL b status ongoing\n",
            )
        );

        let finished = concat!(
            "position sfen 12/12/12/5k6/12/12/5R6/12/12/12/12/K11 b - 1 ",
            "moves 7g7d\nstate\n",
        );
        assert_eq!(
            run(&mut protocol, &mut engine, finished),
            concat!(
                "state rules R1,E2 board ",
                "12/12/12/5R6/12/12/12/12/12/12/12/K11 w ",
                "status win black royal-capture\n",
            )
        );
    }

    #[test]
    fn state_uses_the_next_games_active_rules_after_gameover() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let input = concat!(
            "position startpos\n",
            "gameover draw\n",
            "state\n",
            "setoption name RuleSet value E2,R2,L1\n",
            "position startpos\n",
            "state\n",
        );

        assert_eq!(
            run(&mut protocol, &mut engine, input),
            concat!(
                "info string error: state requires an active or finished game\n",
                "state rules L1,R2,E2 board ",
                "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/",
                "3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/",
                "A1B1TOXT1B1A/LFCSGKEGSCFL b status ongoing\n",
            )
        );
    }

    #[test]
    fn state_status_uses_the_contract_vocabulary() {
        let win_reasons = [
            (WinReason::RoyalCapture, "royal-capture"),
            (WinReason::Repetition, "repetition"),
            (WinReason::PieceExhaustion, "piece-exhaustion"),
            (WinReason::BareKing, "bare-king"),
            (WinReason::Stalemate, "stalemate"),
            (WinReason::Mate, "mate"),
        ];
        for (reason, text) in win_reasons {
            assert_eq!(
                state_status_text(GameStatus::Finished(GameResult::Win {
                    winner: Color::White,
                    reason,
                })),
                Ok(format!("win white {text}"))
            );
        }

        let draw_reasons = [
            (DrawReason::Repetition, "repetition"),
            (DrawReason::PieceExhaustion, "piece-exhaustion"),
            (DrawReason::BareKing, "bare-king"),
        ];
        for (reason, text) in draw_reasons {
            assert_eq!(
                state_status_text(GameStatus::Finished(GameResult::Draw { reason })),
                Ok(format!("draw {text}"))
            );
        }

        assert_eq!(
            state_status_text(GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::Resignation,
            })),
            Err("state cannot represent resignation")
        );
        assert_eq!(
            state_status_text(GameStatus::Finished(GameResult::Draw {
                reason: DrawReason::Agreement,
            })),
            Err("state cannot represent agreement")
        );
    }

    #[test]
    fn position_accepts_four_and_five_field_sfen_and_named_lion_capture_square() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let five_fields = "12/12/7S4/12/12/12/12/12/12/4s7/12/12 b - 17 8j,5c";
        let named_capture = "12/12/12/12/12/12/12/12/12/12/12/12 b 7f 9999";
        let input = format!(
            "position sfen {INITIAL_SFEN}\ngameover draw\nsetoption name RuleSet value P1,R1\nposition sfen {five_fields}\ngameover win\nposition sfen {named_capture}\nquit\n"
        );

        assert_eq!(run(&mut protocol, &mut engine, &input), "");
        assert_eq!(engine.lifecycle(), EngineLifecycle::InGame);
        assert_eq!(
            engine
                .game()
                .position()
                .lion_taken_by_non_lion()
                .map(|trigger| trigger.square),
            Some(crate::test_util::sq(5, 6))
        );

        // 5欄局面自体も独立にcommitし、P1保留集合を検査する。
        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("gameover lose\nposition sfen {five_fields}\n")
            ),
            ""
        );
        let deferred = engine.game().position().promotion_deferred();
        assert!(deferred.contains(crate::test_util::sq(4, 2)));
        assert!(deferred.contains(crate::test_util::sq(7, 9)));
    }

    #[test]
    fn illegal_move_in_position_rejects_the_whole_replacement_as_repetition() {
        let startup = [RuleCode::R2, RuleCode::E2];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let base = "12/12/12/8k3/12/12/12/12/3K8/12/12/12 b - 1";
        let first_three = "9i9h 4d4e 9h9i";
        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("position sfen {base} moves {first_three}\n")
            ),
            ""
        );
        let position_before = engine.game().position().clone();

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("position sfen {base} moves {first_three} 4e4d\nquit\n")
            ),
            "info string error: illegal move '4e4d': forbidden repetition\n"
        );
        assert_eq!(engine.game().position(), &position_before);
        assert_eq!(engine.game().ply_count(), 3);
        assert_eq!(engine.status(), GameStatus::Ongoing);
    }

    #[test]
    fn go_depth_one_returns_a_legal_move_without_applying_it() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let position_before = engine.game().position().clone();

        let output = run(
            &mut protocol,
            &mut engine,
            "position startpos\ngo depth 1\nquit\n",
        );
        let move_text = output
            .strip_prefix("bestmove ")
            .and_then(|text| text.strip_suffix('\n'))
            .expect("go must return exactly one bestmove line");
        let best_move = usi::parse(engine.game().position(), move_text).unwrap();

        assert!(engine.game().legal_moves().contains(&best_move));
        assert_eq!(engine.game().position(), &position_before);
        assert_eq!(engine.game().ply_count(), 0);
    }

    #[test]
    fn go_rejects_inactive_and_finished_games_without_bestmove() {
        let startup = [RuleCode::R1, RuleCode::E2];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let input = concat!(
            "go depth 1\n",
            "position sfen 12/12/12/12/12/12/12/12/12/12/12/12 b - 1\n",
            "go depth 1\n",
            "position sfen 12/12/12/5k6/12/12/5R6/12/12/12/12/K11 b - 1 ",
            "moves 7g7d\n",
            "go nodes 1\n",
            "quit\n",
        );

        assert_eq!(
            run(&mut protocol, &mut engine, input),
            concat!(
                "info string error: go requires an active game\n",
                "info string error: go requires at least one legal move\n",
                "info string error: go requires an active game\n",
            )
        );
    }

    #[test]
    fn go_rejects_missing_unsupported_and_invalid_limits() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let input = concat!(
            "position startpos\n",
            "go\n",
            "go movetime 1000\n",
            "go depth 0\n",
            "go nodes nope\n",
            "go depth 257\n",
            "quit\n",
        );

        assert_eq!(
            run(&mut protocol, &mut engine, input),
            concat!(
                "info string error: go requires depth or nodes\n",
                "info string error: unsupported go argument 'movetime'\n",
                "info string error: go depth must be a positive integer\n",
                "info string error: go nodes must be a positive integer\n",
                "info string error: go depth must not exceed 256\n",
            )
        );
    }

    #[test]
    fn go_accepts_nodes_and_combined_limits_and_keeps_go_mate_behavior() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let output = run(
            &mut protocol,
            &mut engine,
            concat!(
                "position startpos\n",
                "go nodes 1\n",
                "go depth 1 nodes 1\n",
                "go ignored mate 1000\n",
                "quit\n",
            ),
        );
        let lines: Vec<_> = output.lines().collect();

        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("bestmove "));
        assert!(lines[1].starts_with("bestmove "));
        assert_eq!(lines[2], "checkmate notimplemented");
    }

    #[test]
    fn rules_latch_changes_only_the_next_game_for_position_and_usinewgame() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "position startpos\nsetoption name RuleSet value R2,E2\n"
            ),
            ""
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R1]);
        assert_eq!(engine.pending_rule_codes(), &[RuleCode::R2, RuleCode::E2]);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "position startpos\ngameover draw\nposition startpos\n"
            ),
            ""
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R2, RuleCode::E2]);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                concat!(
                    "gameover lose\n",
                    "setoption name RuleSet value R1,E2\n",
                    "usinewgame\n",
                    "position startpos\n",
                    "quit\n",
                )
            ),
            ""
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R1, RuleCode::E2]);
        assert_eq!(engine.lifecycle(), EngineLifecycle::InGame);
    }

    #[test]
    fn royal_capture_finishes_engine_without_usi_result_output() {
        let startup = [RuleCode::R1, RuleCode::E2];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let input = "position sfen 12/12/12/5k6/12/12/5R6/12/12/12/12/K11 b - 1 moves 7g7d\nquit\n";

        assert_eq!(run(&mut protocol, &mut engine, input), "");
        assert_eq!(
            engine.status(),
            GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::RoyalCapture,
            })
        );
        assert_eq!(engine.lifecycle(), EngineLifecycle::Finished);
    }

    #[test]
    fn movement_error_in_the_middle_preserves_the_previous_position() {
        let startup = [RuleCode::R1, RuleCode::E2];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        assert_eq!(run(&mut protocol, &mut engine, "position startpos\n"), "");
        let position_before = engine.game().position().clone();

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "position startpos moves 6i6h 1a1b 6h6g\nquit\n"
            ),
            "info string error: illegal move '1a1b': illegal movement\n"
        );
        assert_eq!(engine.game().position(), &position_before);
        assert_eq!(engine.game().ply_count(), 0);
    }

    #[test]
    fn unknown_commands_options_and_tokens_are_ignored() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let input = format!(
            "unknown command\nsetoption name Unknown value anything\nposition startpos ignored tokens\nposition sfen {INITIAL_SFEN} - ignored tokens\nquit\n"
        );

        assert_eq!(run(&mut protocol, &mut engine, &input), "");
        assert_eq!(engine.game().rules(), Rules::from_codes(&startup).unwrap());
        assert_eq!(engine.lifecycle(), EngineLifecycle::InGame);
    }
}
