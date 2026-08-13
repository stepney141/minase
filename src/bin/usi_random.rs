//! 校正用のランダム着手USIエンジン。真のelo差0の対戦カードを提供する。

use std::io::{self, BufRead, Cursor, Write};

use minase::core::rules::parse_rule_set;
use minase::notation::usi;
use minase::protocol::{Engine, EngineLifecycle, Protocol, UsiProtocol};
use minase::rng::XorShift64;

/// 既定の規則セット。
const DEFAULT_RULE_SET: &str = "R1";
/// 既定の乱数シード。
const DEFAULT_SEED: u64 = 1;

/// 合法手から一様ランダムに選ぶ最小のUSIエンジン。
struct RandomUsi {
    /// 局面と規則を管理する状態機械。
    engine: Engine,
    /// `position`等の解釈を委譲するUSIアダプター。
    protocol: UsiProtocol,
    /// 着手選択に使う擬似乱数生成器。
    rng: XorShift64,
}

impl RandomUsi {
    /// 既定の規則セットとシードで構築する。
    fn new() -> Self {
        let codes = parse_rule_set(DEFAULT_RULE_SET)
            .expect("the default rule set must be valid and executable");
        let engine = Engine::new(codes).expect("the default rule set must construct an engine");
        let protocol = UsiProtocol::new(&engine);
        Self {
            engine,
            protocol,
            rng: XorShift64::new(DEFAULT_SEED),
        }
    }

    /// USIコマンドを入力終了または`quit`まで同期処理する。
    fn run(&mut self, input: &mut dyn BufRead, output: &mut dyn Write) -> io::Result<()> {
        let mut line = String::new();
        loop {
            line.clear();
            if input.read_line(&mut line)? == 0 {
                break;
            }
            if !self.handle_line(line.trim_end(), output)? {
                break;
            }
        }
        Ok(())
    }

    /// 1コマンドを処理し、セッションを続けるかどうかを返す。
    fn handle_line(&mut self, line: &str, output: &mut dyn Write) -> io::Result<bool> {
        let tokens: Vec<_> = line.split_whitespace().collect();
        let Some(command) = tokens.first().copied() else {
            return Ok(true);
        };

        let keep_running = match command {
            "usi" => {
                self.write_handshake(output)?;
                true
            }
            "isready" => {
                writeln!(output, "readyok")?;
                true
            }
            "setoption" => {
                self.handle_setoption(line, &tokens[1..], output)?;
                true
            }
            "usinewgame" | "position" => {
                self.handle_standard_line(line, output)?;
                true
            }
            "go" => {
                self.handle_go(output)?;
                true
            }
            "quit" => false,
            _ => true,
        };
        output.flush()?;
        Ok(keep_running)
    }

    /// 1行をUSIアダプターへそのまま委譲する。
    fn handle_standard_line(&mut self, line: &str, output: &mut dyn Write) -> io::Result<()> {
        let mut input = Cursor::new(line.as_bytes());
        self.protocol.run(&mut self.engine, &mut input, output)
    }

    /// `usi`への応答としてエンジン名とoption宣言を出力する。
    fn write_handshake(&self, output: &mut dyn Write) -> io::Result<()> {
        writeln!(output, "id name minase-usi-random")?;
        writeln!(output, "id author stepney141")?;
        writeln!(
            output,
            "option name RuleSet type string default {DEFAULT_RULE_SET}"
        )?;
        writeln!(
            output,
            "option name Seed type string default {DEFAULT_SEED}"
        )?;
        writeln!(output, "usiok")
    }

    /// `setoption`を処理する。RuleSetはUSIアダプターへ委譲し、Seedは
    /// 乱数生成器を作り直す。
    fn handle_setoption(
        &mut self,
        line: &str,
        tokens: &[&str],
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let Some(name) = token_after(tokens, "name") else {
            return Ok(());
        };
        if name.eq_ignore_ascii_case("RuleSet") {
            self.handle_standard_line(line, output)?;
            return Ok(());
        }
        if !name.eq_ignore_ascii_case("Seed") {
            return Ok(());
        }

        let Some(value) = token_after(tokens, "value") else {
            return write_error(output, "Seed requires a value");
        };
        let Ok(seed) = value.parse::<u64>() else {
            return write_error(output, "Seed must be a positive integer");
        };
        if seed == 0 {
            return write_error(output, "Seed must be a positive integer");
        }
        self.rng = XorShift64::new(seed);
        Ok(())
    }

    /// `go`への応答として合法手から一様ランダムに1手選んで返す。
    /// 探索制限の引数は無視する。
    fn handle_go(&mut self, output: &mut dyn Write) -> io::Result<()> {
        if self.engine.lifecycle() != EngineLifecycle::InGame {
            return write_error(output, "go requires an active game");
        }

        let game = self.engine.game();
        let moves = game.legal_moves();
        if moves.is_empty() {
            return write_error(output, "go requires at least one legal move");
        }
        let best_move = moves[self.rng.index(moves.len())];
        writeln!(output, "bestmove {}", usi::text(game.position(), best_move))
    }
}

/// 指定キーの直後のトークンを返す。
fn token_after<'a>(tokens: &'a [&str], key: &str) -> Option<&'a str> {
    tokens
        .iter()
        .position(|token| token.eq_ignore_ascii_case(key))
        .and_then(|index| tokens.get(index + 1).copied())
}

/// エラーを`info string`行として出力する。
fn write_error(output: &mut dyn Write, message: &str) -> io::Result<()> {
    writeln!(output, "info string error: {message}")
}

/// 標準入出力でランダムエンジンのセッションを実行する。
fn main() -> io::Result<()> {
    let mut engine = RandomUsi::new();
    let stdin = io::stdin();
    let stdout = io::stdout();
    engine.run(&mut stdin.lock(), &mut stdout.lock())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn run(input: &str) -> String {
        let mut engine = RandomUsi::new();
        let mut input = Cursor::new(input.as_bytes());
        let mut output = Vec::new();
        engine.run(&mut input, &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn handshake_and_readiness_match_the_complete_transcript() {
        assert_eq!(
            run("usi\nisready\nquit\n"),
            concat!(
                "id name minase-usi-random\n",
                "id author stepney141\n",
                "option name RuleSet type string default R1\n",
                "option name Seed type string default 1\n",
                "usiok\n",
                "readyok\n",
            )
        );
    }

    #[test]
    fn identical_seed_and_commands_produce_identical_bestmove_sequences() {
        let input = concat!(
            "setoption name Seed value 42\n",
            "position startpos\n",
            "go ignored arguments\n",
            "position startpos moves 6i6h\n",
            "go depth 99\n",
            "quit\n",
        );
        let first = run(input);
        let second = run(input);
        let mut expected_engine = RandomUsi::new();
        let mut ignored = Vec::new();
        expected_engine
            .handle_standard_line("position startpos", &mut ignored)
            .unwrap();
        let game = expected_engine.engine.game();
        let moves = game.legal_moves();
        let mut expected_rng = XorShift64::new(42);
        let expected = usi::text(game.position(), moves[expected_rng.index(moves.len())]);

        assert_eq!(first, second);
        assert_eq!(
            first.lines().next(),
            Some(format!("bestmove {expected}").as_str())
        );
        assert_eq!(
            first
                .lines()
                .filter(|line| line.starts_with("bestmove "))
                .count(),
            2
        );
    }

    #[test]
    fn seed_zero_is_rejected() {
        assert_eq!(
            run("setoption name Seed value 0\nquit\n"),
            "info string error: Seed must be a positive integer\n"
        );
    }

    #[test]
    fn go_rejects_inactive_and_finished_games_without_bestmove() {
        let input = concat!(
            "go\n",
            "position sfen 12/12/12/12/12/12/12/12/12/12/12/12 b - 1\n",
            "go anything\n",
            "position sfen 12/12/12/5k6/12/12/5R6/12/12/12/12/K11 b - 1 ",
            "moves 7g7d\n",
            "go nodes 1\n",
            "quit\n",
        );

        assert_eq!(
            run(input),
            concat!(
                "info string error: go requires an active game\n",
                "info string error: go requires at least one legal move\n",
                "info string error: go requires an active game\n",
            )
        );
    }

    #[test]
    fn ruleset_and_usinewgame_use_the_existing_usi_path() {
        let mut engine = RandomUsi::new();
        let mut input = Cursor::new(
            concat!(
                "setoption name RuleSet value lishogi\n",
                "usinewgame\n",
                "position startpos\n",
                "go\n",
                "quit\n",
            )
            .as_bytes(),
        );
        let mut output = Vec::new();

        engine.run(&mut input, &mut output).unwrap();

        assert!(String::from_utf8(output).unwrap().starts_with("bestmove "));
        assert_eq!(
            engine.engine.active_rule_codes(),
            parse_rule_set("lishogi").unwrap()
        );
    }
}
