//! 中将棋エンジン本体の実行ファイル。プロトコルと規則を指定して起動する。

use std::error::Error;
use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;
use std::process;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Instant;

mod io_log;

use io_log::{IoLog, LoggedOutput, read_input};

use clap::{Parser, ValueEnum};
use minase::RuleCode;
use minase::core::rules::parse_rule_set;
use minase::protocol::{CecpProtocol, Engine, UsiProtocol};

/// グローバルアロケータ。benchの実測（docs/plans/search.md 実施状況）に基づきmimallocを使う。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// 中将棋エンジンのプロトコル入口。
#[derive(Parser)]
#[command(name = "minase")]
struct Arguments {
    /// 使用する通信プロトコル。
    #[arg(long, value_enum, required = true)]
    protocol: ProtocolKind,
    /// 採用するローカルルールコード列。
    #[arg(long, required = true, value_parser = parse_rule_set_argument)]
    rules: RuleSetArgument,
    /// 入出力のログを新規作成するパス。
    #[arg(long)]
    io_log: Option<PathBuf>,
}

/// 解析済みの`--rules`引数。
#[derive(Clone)]
struct RuleSetArgument(Vec<RuleCode>);

/// `--rules`の値を規則セット名またはコード列として解析する。
fn parse_rule_set_argument(input: &str) -> Result<RuleSetArgument, String> {
    parse_rule_set(input)
        .map(RuleSetArgument)
        .map_err(|error| error.to_string())
}

/// 対応する通信プロトコル。
#[derive(Clone, Copy, ValueEnum)]
enum ProtocolKind {
    /// lishogi系拡張を含むUSI。
    Usi,
    /// CECP(XBoard)。
    Cecp,
}

/// エラーを標準エラーへ報告して終了コード1で終わる入口。
fn main() {
    let started = Instant::now();
    if let Err(error) = minase::eval::weights() {
        eprintln!("error: embedded evaluation weights are invalid: {error}");
        process::exit(1);
    }
    if let Err(error) = run(started) {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

/// 指定プロトコルでセッションを実行する。
///
/// USIとCECPは探索中もコマンドを受けるため、標準入力を
/// reader threadで読んでチャネル経由で処理する。
fn run(started: Instant) -> Result<(), Box<dyn Error>> {
    let arguments = Arguments::parse();
    let log = arguments
        .io_log
        .map(|path| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map(|file| Arc::new(IoLog::new(started, file)))
        })
        .transpose()?;
    let mut engine = Engine::new(arguments.rules.0)
        .map_err(|reason| format!("invalid --rules value: {reason}"))?;

    let (sender, receiver) = mpsc::channel();
    let input_log = log.clone();
    thread::spawn(move || {
        read_input(io::stdin().lock(), sender, input_log.as_deref());
    });
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut logged_output;
    let output: &mut dyn io::Write = if let Some(log) = log {
        logged_output = LoggedOutput::new(&mut stdout, log);
        &mut logged_output
    } else {
        &mut stdout
    };

    match arguments.protocol {
        ProtocolKind::Usi => {
            let mut protocol = UsiProtocol::new(&engine);
            protocol.run_channel(&mut engine, &receiver, output)?;
            Ok(())
        }
        ProtocolKind::Cecp => {
            let mut protocol = CecpProtocol::new(&engine);
            protocol.run_channel(&mut engine, &receiver, output)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use minase::Rules;

    use super::*;

    #[test]
    fn cli_requires_explicit_protocol_and_rules() {
        // PL「モジュールと名称」・完了条件: --protocol usi|cecpと--rulesの明示指定を必須とし、
        // 既定値と自動判別を設けない（D6-CLI-01）。
        assert!(Arguments::try_parse_from(["minase"]).is_err());
        assert!(Arguments::try_parse_from(["minase", "--protocol", "usi"]).is_err());
        assert!(Arguments::try_parse_from(["minase", "--rules", "R1"]).is_err());
        // --protocolの値はusiとcecpの2値のみ。
        assert!(
            Arguments::try_parse_from(["minase", "--protocol", "xboard", "--rules", "R1"]).is_err()
        );
        assert!(
            Arguments::try_parse_from(["minase", "--protocol", "usi", "--rules", "R1"]).is_ok()
        );
        assert!(
            Arguments::try_parse_from(["minase", "--protocol", "cecp", "--rules", "R1"]).is_ok()
        );
    }

    /// debugging-tools.md「入出力のログの形式」: --io-logは省略可能で、指定時はパスを要する。
    #[test]
    fn cli_io_log_is_optional_and_accepts_a_path() {
        for protocol in ["usi", "cecp"] {
            let base = [
                "minase",
                "--protocol",
                protocol,
                "--rules",
                "engine-default",
            ];
            assert!(Arguments::try_parse_from(base).unwrap().io_log.is_none());
            let arguments = Arguments::try_parse_from(
                base.into_iter().chain(["--io-log", "logs/session log.txt"]),
            )
            .unwrap();
            assert_eq!(
                arguments.io_log,
                Some(PathBuf::from("logs/session log.txt"))
            );
            assert!(Arguments::try_parse_from(base.into_iter().chain(["--io-log"])).is_err());
        }
    }

    #[test]
    fn rules_argument_shares_the_wire_value_grammar() {
        // PL「規則オプション」（同じ値文法を--rulesにも適用、解析は共通関数parse_rule_set）・
        // R33第5・6項（engine-default=L0+P0+R1+E0、
        // lishogi=L1+L2+P0+P3+R1+E1+E3、大小非区別・併記拒否）
        // （D6-CLI-02〜04、D6-CLI-05のminase側接続確認）。
        let parse = |value: &str| {
            Arguments::try_parse_from(["minase", "--protocol", "usi", "--rules", value])
                .map(|arguments| arguments.rules.0)
        };

        assert_eq!(
            parse("LISHOGI").unwrap(),
            Vec::<RuleCode>::from(Rules::LISHOGI)
        );
        assert!(parse("lishogi,P1").is_err());

        // 4群のいずれかを欠く列は値文法としては解析できるが、エンジン構築時に拒否される
        // ため、不正値での起動成功はあり得ない（D6-CLI-02境界）。
        assert!(Engine::new(parse("L1,P0,E1,E0").unwrap()).is_err());
    }
}
