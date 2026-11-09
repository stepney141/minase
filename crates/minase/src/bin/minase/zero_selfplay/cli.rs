//! 既定値を持たない自己対局の生成条件と実行条件。

use std::{num::NonZeroUsize, path::PathBuf};

use clap::ValueEnum;
use minase::datagen::git::validate_commit_hash;
use minase::nn::{ExecutionProvider, tensorrt_cache::cache_root};
use serde::{Deserialize, Serialize};

use super::Error;

/// AlphaZero型の自己対局生成器の起動引数。
#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// 生成条件、塊、および集計を保存するディレクトリ。
    #[arg(long, required = true)]
    pub(super) output_dir: PathBuf,
    /// 方策と価値を評価するONNX形式のネット。
    #[arg(long, required = true)]
    pub(super) nn_model: PathBuf,
    /// 推論に使う実行プロバイダ。
    #[arg(long, required = true, value_enum)]
    pub(super) nn_backend: Backend,
    /// 生成器の40桁のコミットハッシュ。
    #[arg(long, required = true, value_parser = parse_commit)]
    pub(super) generation_commit: String,
    /// サイクルの基本シード。
    #[arg(long, required = true)]
    pub(super) seed: u64,
    /// 自己対局を始める局面の種類。
    #[arg(long, required = true, value_enum)]
    pub(super) start_position: StartPosition,
    /// サイクルで生成する対局数の上限。
    #[arg(long, required = true, value_parser = clap::value_parser!(u32).range(1..))]
    pub(super) games: u32,
    /// 投了ファイルの除外前に数える訓練局面数の目標。1以上とし、塊0からの累計で判定する。
    #[arg(long, required = true, value_parser = clap::value_parser!(u64).range(1..))]
    pub(super) target_positions: u64,
    /// 1つの塊の対局数。最後の塊は端数を許す。
    #[arg(long, required = true, value_parser = clap::value_parser!(u32).range(1..))]
    pub(super) chunk_games: u32,
    /// 完全探索のシミュレーション数。4以上の4の倍数。速い探索はこの4分の1。
    #[arg(long, required = true, value_parser = parse_simulations)]
    pub(super) simulations: u32,
    /// 最後まで指させる対局の割合。100なら投了を無効にする。
    #[arg(long, required = true, value_parser = clap::value_parser!(u8).range(1..=100))]
    pub(super) playthrough_percent: u8,
    /// 推論器を1つずつ所有するスレッド数。
    #[arg(long, required = true)]
    pub(super) threads: NonZeroUsize,
    /// 各スレッドの同時対局数と推論器の最大バッチ。
    #[arg(long, required = true)]
    pub(super) games_per_thread: NonZeroUsize,
}

/// 自己対局の開始局面と序盤の作り方。
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum StartPosition {
    ShuffledSetup,
    RandomOpening,
}

impl std::fmt::Display for StartPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ShuffledSetup => "shuffled-setup",
            Self::RandomOpening => "random-opening",
        })
    }
}

/// 明示的に選ぶ推論の実行プロバイダ。
#[derive(Clone, Copy, ValueEnum)]
pub(super) enum Backend {
    #[value(name = "tensorrt")]
    TensorRt,
    Cuda,
    Cpu,
}

impl Backend {
    pub(super) fn provider(self) -> Result<ExecutionProvider, Error> {
        Ok(match self {
            Self::TensorRt => ExecutionProvider::TensorRt {
                cache_root: cache_root(std::env::var_os("XDG_CACHE_HOME").as_deref())
                    .ok_or(Error::CacheRootUnset)?,
            },
            Self::Cuda => ExecutionProvider::Cuda,
            Self::Cpu => ExecutionProvider::Cpu,
        })
    }
}

fn parse_commit(text: &str) -> Result<String, std::io::Error> {
    validate_commit_hash(text)?;
    Ok(text.to_owned())
}

fn parse_simulations(text: &str) -> Result<u32, String> {
    let simulations = text.parse::<u32>().map_err(|error| error.to_string())?;
    if simulations < 4 || !simulations.is_multiple_of(4) {
        return Err("simulations must be a multiple of 4 and at least 4".into());
    }
    Ok(simulations)
}
