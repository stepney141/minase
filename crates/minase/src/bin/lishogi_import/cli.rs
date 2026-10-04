//! lishogi棋譜の変換コマンドの引数。

use clap::{Parser, Subcommand};
use std::{
    num::{NonZeroU32, NonZeroU64, NonZeroUsize},
    path::PathBuf,
};

#[derive(Parser)]
#[command(name = "lishogi_import")]
pub(super) struct Arguments {
    #[command(subcommand)]
    pub(super) command: Operation,
}

#[derive(Subcommand)]
pub(super) enum Operation {
    /// 人間の対局結果を教師とするMNSDを作る。
    Games {
        #[command(flatten)]
        common: Common,
        /// 他の教師ファイルと重ならない、0以外の識別用シード。
        #[arg(long)]
        seed: NonZeroU64,
        /// 変更のある作業ツリーでの生成を許可する。
        #[arg(long)]
        allow_dirty: bool,
    },
    /// 実戦開始の自己対局に使う中盤の局面一覧を作る。
    Openings(Common),
}

#[derive(clap::Args)]
pub(super) struct Common {
    /// 1行1局のゲームJSON。
    #[arg(long)]
    pub(super) input: PathBuf,
    /// 新規作成するMNSDまたは局面一覧。
    #[arg(long)]
    pub(super) output: PathBuf,
    /// 理由ごとの件数と棋譜IDを保存するJSON。
    #[arg(long)]
    pub(super) report: PathBuf,
    /// 局面単独の探索のノード上限。本測定では100000を指定する。
    #[arg(long, default_value = "100000")]
    pub(super) nodes: NonZeroU32,
    /// 並行して処理する対局数。
    #[arg(long, default_value = "1")]
    pub(super) concurrency: NonZeroUsize,
    /// ワーカーごとの置換表容量(MB)。
    #[arg(long, default_value = "16")]
    pub(super) hash_mb: NonZeroUsize,
}
