use std::error::Error;
use std::io;
use std::process;

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

#[derive(Clone)]
struct RuleSetArgument(Vec<RuleCode>);

fn parse_rule_set_argument(input: &str) -> Result<RuleSetArgument, String> {
    parse_rule_set(input).map(RuleSetArgument)
}

#[derive(Clone, Copy, ValueEnum)]
enum ProtocolKind {
    Usi,
    Cecp,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let arguments = Arguments::parse();
    let mut engine = Engine::new(arguments.rules.0)
        .map_err(|reason| format!("invalid --rules value: {reason}"))?;

    match arguments.protocol {
        ProtocolKind::Usi => {
            let mut protocol = UsiProtocol::new(&engine);
            let stdin = io::stdin();
            let stdout = io::stdout();
            protocol.run(&mut engine, &mut stdin.lock(), &mut stdout.lock())?;
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

        assert_eq!(preset.rules.0, codes.rules.0);
        assert!(
            error
                .to_string()
                .contains("preset 'lishogi' must be specified alone")
        );
    }
}
