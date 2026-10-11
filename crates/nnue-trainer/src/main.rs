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
    /// Compare GPU forward, all gradients, updates and validation with the CPU reference.
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
    /// Measure GPU updates, real-data worker throughput, transfers and device memory.
    BenchStep(nnue_trainer::gpu::bench::Args),
}

fn main() -> std::process::ExitCode {
    let result: Result<(), Box<dyn std::error::Error>> = match Cli::parse().command {
        Command::DumpBatch(args) => dump::run(args),
        Command::CheckEquivalence { feature_set, width, batch, seed } => {
            nnue_trainer::gpu::equivalence::run(feature_set, width, batch, seed)
                .map(|r| println!("一致: {feature_set:?}, 幅{width}, バッチ{batch}, GPU={}, 比較数={}, 最大絶対誤差={:.3e}, 最大相対誤差={:.3e}, 許容誤差=1e-6+1e-5×|CPU|, 全勾配、1更新と連続3更新の全状態、反復ビット一致", r.device_name, r.comparisons, r.max_absolute_error, r.max_relative_error))
                .map_err(Into::into)
        }
        Command::BenchStep(args) => nnue_trainer::gpu::bench::run(args)
            .map(|r| {
                println!("GPU={}、{:?}、幅{}、シード{}、{}更新、{}局面/バッチ、{}ワーカーで測定しました。", r.device_name, r.feature_set, r.width, r.seed, r.steps, r.batch, r.workers);
                println!("計算ストリームの1更新は平均 {:.3} ms、転送ストリームの1バッチは平均 {:.3} ms、入力転送量は {} bytes/バッチでした。", r.compute_ms, r.transfer_ms, r.transfer_bytes);
                println!("ワーカーの供給速度は {:.0} 局面/s、全体の経過時間は {:.3} s、実効速度は {:.0} 局面/sでした。", r.worker_positions_per_second, r.elapsed_seconds, r.steps as f64 * r.batch as f64 / r.elapsed_seconds);
                println!("GPUメモリ使用量は {} / {} bytes、入力面と更新器の確保前後の差は {} bytesでした。", r.gpu_used_bytes, r.gpu_total_bytes, r.gpu_allocation_bytes);
                println!("平均損失は {:.8}、補助損失は {:.8} でした。計測用の設定として埋め草を除く全行を観測済みとし、出力と教師値に共通の K={} を使いました。", r.losses.total(), r.losses.auxiliary, r.k);
            })
            .map_err(Into::into),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("nnue-trainer: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
