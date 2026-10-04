//! SPSAの実行と係数の生成・適用の引数。

use super::engine::tuning_spec;
use clap::{ArgGroup, Parser, Subcommand};
use minase::harness::{
    PlayerSpec, SearchLimit, parse_positive_u32, parse_positive_u64, parse_search_limit,
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "spsa_runner", subcommand_negates_reqs = true, args_conflicts_with_subcommands = true,
    group(ArgGroup::new("operation").required(true).args(["run_dir", "resume"])))]
pub(super) struct Arguments {
    /// 新規に作る実行ディレクトリ。
    #[arg(long)]
    pub(super) run_dir: Option<PathBuf>,
    /// 同じ実行条件で再開するディレクトリ。
    #[arg(long)]
    pub(super) resume: Option<PathBuf>,
    /// 全対局と摂動の基本シード。
    #[arg(long, required = true)]
    pub(super) seed: Option<u64>,
    /// tuningビルドのcommit指定または起動コマンド。
    #[arg(long, required = true, value_parser = tuning_spec)]
    pub(super) engine: Option<PlayerSpec>,
    /// 6欄の係数ファイル。
    #[arg(long, required = true)]
    pub(super) params: Option<PathBuf>,
    /// 両エンジンと審判に適用する規則。
    #[arg(long, required = true)]
    pub(super) rules: Option<String>,
    /// 両エンジンの思考制限。
    #[arg(long, required = true, value_parser = parse_search_limit)]
    pub(super) each: Option<SearchLimit>,
    /// 同時に対局させるペアの数。
    #[arg(long, required = true)]
    pub(super) concurrency: Option<std::num::NonZeroUsize>,
    /// 開始時に固定する総反復数。
    #[arg(long, required = true, value_parser = parse_positive_u64)]
    pub(super) iterations: Option<u64>,
    /// 1反復のペア数。
    #[arg(long, required = true)]
    pub(super) pairs_per_iteration: Option<std::num::NonZeroUsize>,
    /// 1局の手数上限。
    #[arg(long, default_value = "4096", value_parser = parse_positive_u32)]
    pub(super) max_ply: u32,
    /// 1回の応答期限を秒で指定する。
    #[arg(long, default_value = "120", value_parser = parse_positive_u64)]
    pub(super) response_timeout: u64,
    #[command(subcommand)]
    pub(super) command: Option<Command>,
}

#[derive(Subcommand)]
pub(super) enum Command {
    /// 完了した調整結果を係数表の既定値へ反映する。
    Apply {
        /// 完了したセッションの実行ディレクトリ。
        #[arg(long)]
        run_dir: PathBuf,
        /// 書き換える係数表のソースファイル。
        #[arg(long)]
        source: PathBuf,
    },
    /// USIのTune_宣言から係数ファイルを標準出力へ生成する。
    Params {
        #[arg(long, value_parser = tuning_spec)]
        engine: PlayerSpec,
        #[arg(long)]
        rules: String,
    },
}
