//! 中将棋エンジン本体の実行ファイル。プロトコルと規則を指定して起動する。

use std::error::Error;
use std::io::{self, BufRead};
use std::process;
use std::sync::mpsc;
use std::thread;

use clap::{Parser, ValueEnum};
use minase::RuleCode;
use minase::core::rules::parse_rule_set;
use minase::protocol::{CecpProtocol, Engine, Protocol, UsiProtocol};

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
}

/// 解析済みの`--rules`引数。
#[derive(Clone)]
struct RuleSetArgument(Vec<RuleCode>);

/// `--rules`の値を規則セット名またはコード列として解析する。
fn parse_rule_set_argument(input: &str) -> Result<RuleSetArgument, String> {
    parse_rule_set(input).map(RuleSetArgument)
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
    if let Err(error) = run() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

/// 指定プロトコルでセッションを実行する。
///
/// USIは探索中もコマンドを受けるため、標準入力をreader threadで読んで
/// チャネル経由で処理する。CECPは同期入出力で実行する。
fn run() -> Result<(), Box<dyn Error>> {
    let arguments = Arguments::parse();
    let mut engine = Engine::new(arguments.rules.0)
        .map_err(|reason| format!("invalid --rules value: {reason}"))?;

    match arguments.protocol {
        ProtocolKind::Usi => {
            let mut protocol = UsiProtocol::new(&engine);
            let (sender, receiver) = mpsc::channel();
            thread::spawn(move || {
                let stdin = io::stdin();
                let mut input = stdin.lock();
                let mut line = String::new();
                loop {
                    line.clear();
                    match input.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => {
                            if sender.send(Ok(line.trim_end().to_owned())).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = sender.send(Err(error));
                            break;
                        }
                    }
                }
            });
            let stdout = io::stdout();
            protocol.run_channel(&mut engine, &receiver, &mut stdout.lock())?;
            Ok(())
        }
        ProtocolKind::Cecp => {
            let mut protocol = CecpProtocol::new(&engine);
            let stdin = io::stdin();
            let stdout = io::stdout();
            protocol.run(&mut engine, &mut stdin.lock(), &mut stdout.lock())?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_argument_accepts_preset_and_codes_but_rejects_their_combination() {
        let engine_default =
            Arguments::try_parse_from(["minase", "--protocol", "usi", "--rules", "engine-default"])
                .unwrap();
        let preset =
            Arguments::try_parse_from(["minase", "--protocol", "usi", "--rules", "lishogi"])
                .unwrap();
        let codes = Arguments::try_parse_from([
            "minase",
            "--protocol",
            "usi",
            "--rules",
            "L1,L2,P3,R1,E1,E3",
        ])
        .unwrap();
        let error = match Arguments::try_parse_from([
            "minase",
            "--protocol",
            "usi",
            "--rules",
            "lishogi,P1",
        ]) {
            Ok(_) => panic!("a preset combined with a rule code must be rejected"),
            Err(error) => error,
        };

        assert_eq!(engine_default.rules.0, [RuleCode::R1]);
        assert_eq!(preset.rules.0, codes.rules.0);
        assert!(
            error
                .to_string()
                .contains("preset 'lishogi' must be specified alone")
        );
    }
}
