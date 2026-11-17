//! minaseの評価関数の学習器を呼び出す開発用コマンド。
//! サブコマンドと学習器の対応、および共有範囲は[`minase_trainer`]に従う。

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
    /// 教師データの勝率尺度を較正する。
    #[command(subcommand)]
    Data(DataCommand),
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
enum DataCommand {
    /// 訓練集合から分類ごとに教師の勝率尺度を推定し、JSONへ保存する。
    Calibrate(minase_trainer::data::CalibrateArgs),
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
    /// 手書き評価v0を線形の経路だけで表したMNUEへ書き出す。
    ExportHandcrafted(minase_trainer::nnue::handcrafted::Args),
    /// 構造を選んで学習する。
    #[command(subcommand)]
    Train(TrainCommand),
    /// CPUとCUDAの計算を照合する。
    #[command(subcommand)]
    CheckEquivalence(CheckCommand),
    /// 更新の時間を計測する。
    #[command(subcommand)]
    BenchStep(BenchCommand),
}
#[derive(Subcommand)]
enum TrainCommand {
    Layerstack(Box<minase_trainer::nnue::train::Args>),
    Simple(Box<minase_trainer::nnue::train::Args>),
}
#[derive(Subcommand)]
enum CheckCommand {
    Layerstack(minase_trainer::nnue::layerstack::commands::CheckArgs),
    Simple(minase_trainer::nnue::simple::commands::CheckArgs),
}
#[derive(Subcommand)]
enum BenchCommand {
    Layerstack(minase_trainer::nnue::layerstack::commands::BenchArgs),
    Simple(minase_trainer::nnue::simple::commands::BenchArgs),
}

fn main() -> std::process::ExitCode {
    let result = match Cli::parse().command {
        Command::Data(DataCommand::Calibrate(args)) => minase_trainer::data::calibrate(args),
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
        NnueCommand::ExportHandcrafted(args) => {
            minase_trainer::nnue::handcrafted::run(args).map_err(Into::into)
        }
        NnueCommand::Train(TrainCommand::Layerstack(args)) => {
            minase_trainer::nnue::layerstack::trainer::run(*args).map(|summary| {
                eprintln!(
                    "superbatches: {}; updates: {}; stopped_by: {}; status: {}",
                    summary.completed_superbatches,
                    summary.completed_steps,
                    summary.stopped_by,
                    summary.status
                );
            })
        }
        NnueCommand::CheckEquivalence(CheckCommand::Layerstack(args)) => {
            let report = minase_trainer::nnue::layerstack::commands::check(args)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        NnueCommand::BenchStep(BenchCommand::Layerstack(args)) => {
            let report = minase_trainer::nnue::layerstack::commands::bench(args)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        NnueCommand::Train(TrainCommand::Simple(args)) => {
            minase_trainer::nnue::simple::trainer::run(*args).map(|summary| {
                eprintln!(
                    "superbatches: {}; updates: {}; stopped_by: {}; status: {}",
                    summary.completed_superbatches,
                    summary.completed_steps,
                    summary.stopped_by,
                    summary.status
                );
            })
        }
        NnueCommand::CheckEquivalence(CheckCommand::Simple(args)) => {
            let report = minase_trainer::nnue::simple::commands::check(args)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        NnueCommand::BenchStep(BenchCommand::Simple(args)) => {
            let report = minase_trainer::nnue::simple::commands::bench(args)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
    }
}
