//! 既定値を持たない自己対局の生成条件と実行条件。

use std::{num::NonZeroUsize, path::PathBuf};

use clap::ValueEnum;
use minase::datagen::git::validate_commit_hash;
use minase::nn::{ExecutionProvider, tensorrt_cache::cache_root};

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
    /// サイクルで生成する総対局数。
    #[arg(long, required = true, value_parser = clap::value_parser!(u32).range(1..))]
    pub(super) games: u32,
    /// 1つの塊の対局数。最後の塊は端数を許す。
    #[arg(long, required = true, value_parser = clap::value_parser!(u32).range(1..))]
    pub(super) chunk_games: u32,
    /// 根の評価後に行う1手のシミュレーション数。
    #[arg(long, required = true, value_parser = clap::value_parser!(u32).range(1..))]
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
