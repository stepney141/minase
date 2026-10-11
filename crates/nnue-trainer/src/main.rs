//! minaseのNNUE学習器の開発用コマンド。dump-batchはCPUだけで動く。

mod dump;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "nnue-trainer", about = "minase NNUE trainer development tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write batch indices and teacher scores for comparison with Python.
    DumpBatch(dump::Args),
}

fn main() -> std::process::ExitCode {
    let result = match Cli::parse().command {
        Command::DumpBatch(args) => dump::run(args),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("nnue-trainer: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
