//! NNUEの分布、駒除去、合法手の評価差をCPUで診断する。

mod network;
mod nnue_distribution;
mod nnue_moves;
mod nnue_removal;
mod sample;
mod statistics;

use std::{fs, io::Write, path::Path};

use clap::Subcommand;
use serde::Serialize;

pub use nnue_distribution::DistributionArgs;
pub use nnue_moves::MovesArgs;
pub use nnue_removal::RemovalArgs;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// CPUだけで実行する評価関数の診断。
#[derive(Subcommand)]
pub enum Command {
    /// 全局面の標本でMNPT版3とMNUE版5の評価分布を比較する。
    NnueDistribution(DistributionArgs),
    /// 検証集合で実数と整数の駒除去差分を検査する。
    NnueRemoval(RemovalArgs),
    /// 全合法手の評価差をNNUE、PST＋FM、PSTのみで比較する。
    NnueMoves(MovesArgs),
}

/// 診断報告を保存する。不合格の報告も保存してからエラーを返す。
pub fn run(command: Command) -> Result<()> {
    let path = match &command {
        Command::NnueDistribution(args) => &args.output,
        Command::NnueRemoval(args) => &args.output,
        Command::NnueMoves(args) => &args.output,
    };
    if path.try_exists()? {
        return Err(format!("{}: diagnostic output already exists", path.display()).into());
    }
    match command {
        Command::NnueDistribution(args) => {
            let report = nnue_distribution::run(&args)?;
            save(&args.output, &report, None)
        }
        Command::NnueRemoval(args) => {
            let report = nnue_removal::run(&args)?;
            save(&args.output, &report, report.error.as_deref())
        }
        Command::NnueMoves(args) => {
            let report = nnue_moves::run(&args)?;
            save(&args.output, &report, report.error.as_deref())
        }
    }
}

fn save(path: &Path, report: &impl Serialize, error: Option<&str>) -> Result<()> {
    let mut json = serde_json::to_vec_pretty(report)?;
    json.push(b'\n');
    if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    // 検査後に別プロセスが作成した場合も上書きしない。
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&json)?;
    file.flush()?;
    if let Some(error) = error {
        return Err(error.into());
    }
    Ok(())
}
