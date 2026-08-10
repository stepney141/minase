use std::error::Error;
use std::io;
use std::process;

use clap::{Parser, ValueEnum};
use minase::RuleCode;
use minase::protocol::{Engine, Protocol, UsiProtocol};

/// 中将棋エンジンのプロトコル入口。
#[derive(Parser)]
#[command(name = "minase")]
struct Arguments {
    /// 使用する通信プロトコル。
    #[arg(long, value_enum, required = true)]
    protocol: ProtocolKind,
    /// 採用するローカルルールコード列。
    #[arg(long, value_delimiter = ',', required = true)]
    rules: Vec<RuleCode>,
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
    let mut engine = Engine::new(arguments.rules.clone())
        .map_err(|reason| format!("invalid --rules value: {reason}"))?;

    match arguments.protocol {
        ProtocolKind::Usi => {
            let mut protocol = UsiProtocol::new(&engine);
            let stdin = io::stdin();
            let stdout = io::stdout();
            protocol.run(&mut engine, &mut stdin.lock(), &mut stdout.lock())?;
            Ok(())
        }
        ProtocolKind::Cecp => Err("CECP protocol is not implemented (planned for phase 5)".into()),
    }
}
