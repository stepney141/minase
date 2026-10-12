//! minaseの評価関数の学習器の開発用コマンド。
//! NNUEの学習と検証はnnueサブコマンドの下に置く。

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "minase-trainer",
    about = "minaseの評価関数の学習器の開発用コマンド"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// NNUEの学習と検証を行う。
    #[command(subcommand)]
    Nnue(NnueCommand),
    /// 鏡映共有PSTを学習する。
    #[command(subcommand)]
    Pst(PstCommand),
    /// 固定PSTのFM補正を学習し、縮小と倍率の焼き込みを行う。
    #[command(subcommand)]
    Fm(minase_trainer::fm::Command),
    /// PSTとFMを同時学習する。
    #[command(subcommand)]
    Joint(JointCommand),
    /// 評価分布、駒除去、合法手の評価差を診断する。
    #[command(subcommand)]
    Diag(minase_trainer::diagnostics::Command),
}

#[derive(Subcommand)]
enum JointCommand {
    /// 鏡映共有PSTとFMを2群のAdamWで学習する。
    Train(minase_trainer::joint::Args),
}

#[derive(Subcommand)]
enum PstCommand {
    /// CPUで学習し、最良PSTと学習記録を保存する。
    Train(minase_trainer::pst::Args),
}

#[derive(Subcommand)]
enum NnueCommand {
    /// CUDAまたはCPUで学習し、各エポックの検証、チェックポイントの保存、最良モデルの書き出しを行う。
    Train(minase_trainer::nnue::train::Args),
    /// GPUの前進計算、全勾配、更新、検証をCPU参照と照合する。
    CheckEquivalence {
        #[command(flatten)]
        training: minase_trainer::nnue::options::TrainingOptions,
        /// 入力の特徴集合を指定する。
        #[arg(long, value_enum)]
        feature_set: minase_trainer::nnue::data::InputFeatureSet,
        /// 隠れ層の幅を2以上512以下の偶数で指定する。
        #[arg(long)]
        width: usize,
        /// バッチの局面数を指定する。
        #[arg(long)]
        batch: usize,
        /// モデルの初期化と合成入力の生成に使う乱数シードを指定する。
        #[arg(long)]
        seed: u64,
    },
    /// 指定デバイスで更新と供給の時間を測り、CUDAでは転送とメモリも測る。
    BenchStep(minase_trainer::nnue::gpu::bench::Args),
}

fn main() -> std::process::ExitCode {
    let result = match Cli::parse().command {
        Command::Joint(JointCommand::Train(args)) => minase_trainer::joint::train(args)
            .map(|summary| {
                println!(
                    "status: {}; updates: {}; best epoch: {:?}",
                    summary.status, summary.total_steps, summary.best_epoch
                )
            })
            .map_err(|e| e as Box<dyn std::error::Error>),
        Command::Nnue(command) => run_nnue(command),
        Command::Fm(command) => {
            minase_trainer::fm::run(command).map_err(|e| e as Box<dyn std::error::Error>)
        }
        Command::Pst(PstCommand::Train(args)) => minase_trainer::pst::run(args)
            .map(|summary| {
                println!(
                    "best epoch: {}; stopped_by: {:?}; status: {}",
                    summary.best_epoch.expect("completed training"),
                    summary.stopped_by,
                    summary.status
                );
            })
            .map_err(Into::into),
        Command::Diag(command) => minase_trainer::diagnostics::run(command),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("minase-trainer: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run_nnue(command: NnueCommand) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        NnueCommand::Train(args) => minase_trainer::nnue::train::run(args).map(|summary| {
            println!("best epoch: {}; stopped_by: {:?}; status: {}",
                summary.best_epoch.expect("successful training has a best epoch"), summary.stopped_by, summary.status);
        }),
        NnueCommand::CheckEquivalence { feature_set, width, batch, seed, training } => {
            minase_trainer::nnue::gpu::equivalence::run(feature_set, width, batch, seed, training)
                .map(|r| println!("一致: {feature_set:?}, 幅{width}, バッチ{batch}, GPU={}, 比較数={}, 最大絶対誤差={:.3e}, 最大相対誤差={:.3e}, 許容誤差=1e-6+1e-5×|CPU|, 全勾配、1更新と連続3更新の全状態、反復ビット一致", r.device_name, r.comparisons, r.max_absolute_error, r.max_relative_error))
                .map_err(Into::into)
        }
        NnueCommand::BenchStep(args) => minase_trainer::nnue::gpu::bench::run(args)
            .map(|r| {
                println!("デバイス={}、{:?}、幅{}、シード{}、{}更新、{}局面/バッチ、{}スレッドで測定しました。", r.device_name, r.feature_set, r.width, r.seed, r.steps, r.batch, r.threads);
                println!("1更新は平均 {:.3} ms、1バッチの転送は平均 {:.3} ms、入力転送量は {} bytes/バッチでした。", r.compute_ms, r.transfer_ms, r.transfer_bytes);
                println!("ワーカーの供給速度は {:.0} 局面/s、全体の経過時間は {:.3} s、実効速度は {:.0} 局面/sでした。", r.worker_positions_per_second, r.elapsed_seconds, r.steps as f64 * r.batch as f64 / r.elapsed_seconds);
                if let (Some(used), Some(total), Some(allocated)) = (r.gpu_used_bytes, r.gpu_total_bytes, r.gpu_allocation_bytes) {
                    println!("GPUメモリ使用量は {used} / {total} bytes、入力面と更新器の確保前後の差は {allocated} bytesでした。");
                }
                println!("平均損失は {:.8}、補助損失は {:.8} でした。計測用の設定として埋め草を除く全行を観測済みとし、出力と教師値に共通の K={} を使いました。", r.losses.total(), r.losses.auxiliary, r.k);
                if let Some(profile) = r.profile {
                    println!("{profile}");
                }
            }),
    }
}
