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
    /// Compare GPU forward, removal penalties, all gradients and validation with the CPU reference.
    CheckEquivalence {
        #[arg(long, value_enum)]
        feature_set: nnue_trainer::data::InputFeatureSet,
        #[arg(long)]
        width: usize,
        #[arg(long)]
        batch: usize,
        #[arg(long)]
        seed: u64,
    },
}

fn main() -> std::process::ExitCode {
    let result: Result<(), Box<dyn std::error::Error>> = match Cli::parse().command {
        Command::DumpBatch(args) => dump::run(args),
        Command::CheckEquivalence { feature_set, width, batch, seed } => {
            nnue_trainer::gpu::equivalence::run(feature_set, width, batch, seed)
                .map(|r| println!("一致: {feature_set:?}, 幅{width}, バッチ{batch}, GPU={}, 比較数={}, 最大絶対誤差={:.3e}, 最大相対誤差={:.3e}, 許容誤差=1e-6+1e-5×|CPU|, 補助損失の下限・上限を含む全勾配・反復ビット一致", r.device_name, r.comparisons, r.max_absolute_error, r.max_relative_error))
                .map_err(Into::into)
        }
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("nnue-trainer: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
