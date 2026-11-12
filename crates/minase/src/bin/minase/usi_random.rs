//! 校正用のランダム着手USIエンジン。真のelo差0の対戦カードを提供する。

use std::io::{self, BufRead, Cursor, Write};
use std::num::{NonZeroU64, NonZeroUsize};

use minase::protocol::{EngineLifecycle, Protocol, RuleSetNotSet, UsiProtocol};
use minase_core::MoveGenerator;
use minase_core::notation::usi;
use minase_core::rng::XorShift64;

/// 既定の乱数シード。
const DEFAULT_SEED: NonZeroU64 = NonZeroU64::new(1).unwrap();

/// 合法手から一様ランダムに選ぶ最小のUSIエンジン。
struct RandomUsi {
    /// `position`等の解釈を委譲するUSIアダプター。
    protocol: UsiProtocol,
    /// 着手選択に使う擬似乱数生成器。
    rng: XorShift64,
}

impl RandomUsi {
    /// 規則未設定のセッションと既定のシードで構築する。
    fn new() -> Self {
        Self {
            protocol: UsiProtocol::new(),
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
                if self.protocol.engine().is_err() {
                    write_error(output, &RuleSetNotSet.to_string())?;
                    output.flush()?;
                    return Err(RuleSetNotSet.into());
                }
                writeln!(output, "readyok")?;
                true
            }
            "setoption" => {
                self.handle_setoption(line, &tokens[1..], output)?;
                true
            }
            "usinewgame" | "position" | "gameover" | "moves" | "state" | "d" | "eval" | "tt" => {
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
        self.protocol.run(&mut input, output)
    }

    /// `usi`への応答としてエンジン名とoption宣言を出力する。
    fn write_handshake(&self, output: &mut dyn Write) -> io::Result<()> {
        writeln!(output, "id name minase-usi-random")?;
        writeln!(output, "id author stepney141")?;
        writeln!(output, "option name RuleSet type string default <empty>")?;
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
        let Some(seed) = value.parse::<u64>().ok().and_then(NonZeroU64::new) else {
            return write_error(output, "Seed must be a positive integer");
        };
        self.rng = XorShift64::new(seed);
        Ok(())
    }

    /// `go`への応答として合法手から一様ランダムに1手選んで返す。
    /// 探索制限の引数は無視する。
    fn handle_go(&mut self, output: &mut dyn Write) -> io::Result<()> {
        let engine = match self.protocol.engine() {
            Ok(engine) => engine,
            Err(error) => return write_error(output, &error.to_string()),
        };
        if engine.lifecycle() != EngineLifecycle::InGame {
            return write_error(output, "go requires an active game");
        }

        let game = engine.game();
        let moves = game.legal_moves();
        if moves.is_empty() {
            return write_error(output, "go requires at least one legal move");
        }
        let best_move = moves[self
            .rng
            .index(NonZeroUsize::new(moves.len()).expect("legal moves were checked as non-empty"))];
        let bestmove = usi::text(
            game.position(),
            best_move,
            &MoveGenerator::new(game.rules().moves),
        )
        .expect("a move returned by legal_moves must be renderable");
        writeln!(output, "bestmove {bestmove}")
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
pub(super) fn main() -> io::Result<()> {
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
        let input = format!("setoption name RuleSet value JCSA\n{input}");
        let mut input = Cursor::new(input.as_bytes());
        let mut output = Vec::new();
        engine.run(&mut input, &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn isready_without_ruleset_fails_without_readyok() {
        let mut engine = RandomUsi::new();
        let mut output = Vec::new();
        let error = engine
            .run(&mut Cursor::new("isready\n"), &mut output)
            .unwrap_err();
        use std::error::Error;
        assert!(error.source().unwrap().is::<RuleSetNotSet>());
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "info string error: required option RuleSet is not set\n"
        );
    }

    #[test]
    fn commands_without_ruleset_reject_and_keep_the_session_alive() {
        let mut engine = RandomUsi::new();
        let mut output = Vec::new();
        engine
            .run(
                &mut Cursor::new(concat!(
                    "position startpos\ngo\ngameover win\n",
                    "setoption name RuleSet\nsetoption name RuleSet value <empty>\n",
                    "setoption name RuleSet value JCSA\nisready\n",
                )),
                &mut output,
            )
            .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            concat!(
                "info string error: required option RuleSet is not set\n",
                "info string error: required option RuleSet is not set\n",
                "info string error: required option RuleSet is not set\n",
                "info string error: RuleSet requires a value\n",
                "info string error: RuleSet requires a value\n",
                "readyok\n",
            )
        );
    }

    #[test]
    fn handshake_declares_ruleset_and_seed_defaults() {
        let mut engine = RandomUsi::new();
        let mut output = Vec::new();
        engine
            .run(
                &mut Cursor::new("usi\nsetoption name RuleSet value JCSA\nisready\nquit\n"),
                &mut output,
            )
            .unwrap();
        let output = String::from_utf8(output).unwrap();
        let lines: Vec<_> = output.lines().collect();
        assert!(lines.iter().any(|line| line.starts_with("id name ")));
        assert!(lines.iter().any(|line| line.starts_with("id author ")));
        assert!(lines.contains(&"option name RuleSet type string default <empty>"));
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("option name Seed type string default "))
        );
        assert!(lines.contains(&"usiok"));
        assert!(lines.contains(&"readyok"));
    }

    #[test]
    fn identical_seeds_reproduce_identical_outputs() {
        // docs/guides/sprt.md: 同じ入力とシードは応答列を再現し、Seed指定は着手選択へ反映される。
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
        assert_eq!(first, second);

        // goごとにbestmoveがちょうど1行ずつ返る。
        let moves: Vec<_> = first
            .lines()
            .filter(|line| line.starts_with("bestmove "))
            .collect();
        assert_eq!(moves.len(), 2);
        let other_seed = run(&input.replace("Seed value 42", "Seed value 43"));
        let other_moves: Vec<_> = other_seed
            .lines()
            .filter(|line| line.starts_with("bestmove "))
            .collect();
        assert_ne!(moves, other_moves, "Seed must affect the response sequence");
    }

    #[test]
    fn seed_zero_is_rejected() {
        // XorShift64は非零シードを前提とするため、規範文書に明文がないシード0の拒否を実装契約とする。
        assert_eq!(
            run("setoption name Seed value 0\nquit\n"),
            "info string error: Seed must be a positive integer\n"
        );
    }

    #[test]
    fn go_outside_an_active_game_returns_no_bestmove() {
        // 校正用エンジンにも主エンジンと同じライフサイクル契約を適用することを実装契約とする。
        use minase_core::{
            Color::{Black, White},
            PieceKind::{King, Rook},
            test_util::{position, sq},
            to_sfen,
        };
        let empty = position(Black, &[]);
        let capture = position(
            Black,
            &[
                (sq(5, 8), White, King),
                (sq(5, 5), Black, Rook),
                (sq(0, 0), Black, King),
            ],
        );
        let input = format!(
            "go\nposition sfen {} - 1\ngo anything\nposition sfen {} - 1 moves 7g7d\ngo nodes 1\nquit\n",
            to_sfen(&empty),
            to_sfen(&capture)
        );

        assert_eq!(
            run(&input),
            concat!(
                "info string error: go requires an active game\n",
                "info string error: go requires at least one legal move\n",
                "info string error: go requires an active game\n",
            )
        );
    }

    #[test]
    fn ruleset_delegates_to_the_shared_usi_grammar() {
        // [`minase_core::rules::parse_rule_set`]の値文法への接続を検査する。
        // プリセット併記の専用エラーがUSI経路からそのまま返ることで委譲を観測する。
        let output = run(concat!(
            "setoption name RuleSet value lishogi,P1\n",
            "setoption name RuleSet value lishogi\n",
            "usinewgame\n",
            "position startpos\n",
            "go\n",
            "quit\n",
        ));
        let lines: Vec<_> = output.lines().collect();

        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("info string error: "));
        assert!(lines[0].contains("lishogi"));
        assert!(lines[1].starts_with("bestmove "));
    }
}
