//! 自己対局データ生成器の引数と既定値の解釈。

use super::{DEFAULT_HASH_MB, DEFAULT_MAX_PLY, DEFAULT_NODES};
use clap::{Parser, Subcommand};
use minase::datagen::provenance::ProvenanceArguments;
use std::{num::NonZeroUsize, path::PathBuf};

/// 自己対局データ生成器のコマンドライン引数。
#[derive(Parser)]
#[command(name = "selfplay_gen")]
pub(super) struct Arguments {
    /// 実行する操作。
    #[command(subcommand)]
    pub(super) command: Operation,
}

/// 自己対局データに対する操作。
#[derive(Subcommand)]
pub(super) enum Operation {
    /// 自己対局から学習データを生成する。
    Generate(GenerateArguments),
    /// 学習データの形式と全局面を検査する。
    Inspect(InspectArguments),
    /// 保存済みの局面を探索し、MNRSへ付け直す。
    Rescore(RescoreArguments),
    /// 既存MNSDに明示的な来歴を付ける。
    Provenance(ProvenanceArguments),
}

/// 付け直しの引数。探索の並列化は局面単位で行う。
#[derive(clap::Args)]
pub(super) struct RescoreArguments {
    /// 読み取り専用で開く元のMNSD。
    #[arg(long)]
    pub(super) input: PathBuf,
    /// 昇順で重複のない対象番号の一覧。
    #[arg(long)]
    pub(super) targets: PathBuf,
    /// 新規作成または再開するMNRS。
    #[arg(long)]
    pub(super) output: PathBuf,
    /// 教師のMNPT。省略時は埋め込み重み。
    #[arg(long)]
    pub(super) pst: Option<PathBuf>,
    /// 各局面のノード上限。
    #[arg(long, value_parser = parse_positive_u32)]
    pub(super) nodes: u32,
    /// 各ワーカーの置換表容量(MB)。
    #[arg(long, default_value_t = DEFAULT_HASH_MB, value_parser = parse_positive_usize)]
    pub(super) hash_mb: NonZeroUsize,
    /// 並行して探索する局面数。
    #[arg(long, default_value = "1", value_parser = parse_positive_usize)]
    pub(super) concurrency: NonZeroUsize,
    /// 変更のある作業ツリーでの実行を許可する。
    #[arg(long)]
    pub(super) allow_dirty: bool,
}

/// `generate`サブコマンドの引数。
#[derive(clap::Args)]
pub(super) struct GenerateArguments {
    /// 新規作成する出力ファイル。
    #[arg(long, required = true)]
    pub(super) output: PathBuf,
    /// 生成する対局数。
    #[arg(long, required_unless_present = "openings", conflicts_with = "openings", value_parser = parse_positive_u32)]
    pub(super) games: Option<u32>,
    /// 実戦開始の局面一覧。1行につき1局を生成する。
    #[arg(long)]
    pub(super) openings: Option<PathBuf>,
    /// 対局シードの派生元。
    #[arg(long, required = true)]
    pub(super) seed: u64,
    /// 1探索のノード上限。
    #[arg(long, default_value_t = DEFAULT_NODES, value_parser = parse_positive_u32)]
    pub(super) nodes: u32,
    /// 1局に注入するランダム着手の上限回数。
    #[arg(long, required = true, value_parser = clap::value_parser!(u8).range(0..=80))]
    pub(super) random_moves: u8,
    /// 同時に走らせる自己対局数。
    #[arg(long, default_value = "1", value_parser = parse_positive_usize)]
    pub(super) concurrency: NonZeroUsize,
    /// 1局の手数上限。
    #[arg(long, default_value_t = DEFAULT_MAX_PLY, value_parser = parse_positive_u16)]
    pub(super) max_ply: u16,
    /// ワーカーごとの置換表容量(MB)。
    #[arg(long, default_value_t = DEFAULT_HASH_MB, value_parser = parse_positive_usize)]
    pub(super) hash_mb: NonZeroUsize,
    /// 変更のある作業ツリーからの生成を許可する。
    #[arg(long)]
    pub(super) allow_dirty: bool,
}

/// `inspect`サブコマンドの引数。
#[derive(clap::Args)]
pub(super) struct InspectArguments {
    /// 検査する学習データファイル。
    pub(super) path: PathBuf,
    /// 内容を表示する先頭レコード数。
    #[arg(long, default_value_t = 0)]
    pub(super) dump: u64,
}

/// 0より大きい`u32`を解析する。
fn parse_positive_u32(text: &str) -> Result<u32, String> {
    let value = text
        .parse::<u32>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if value == 0 {
        Err("value must be greater than zero".to_owned())
    } else {
        Ok(value)
    }
}

/// 0より大きい`u16`を解析する。
fn parse_positive_u16(text: &str) -> Result<u16, String> {
    let value = text
        .parse::<u16>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if value == 0 {
        Err("value must be greater than zero".to_owned())
    } else {
        Ok(value)
    }
}

/// 0より大きい`usize`を解析する。
fn parse_positive_usize(text: &str) -> Result<NonZeroUsize, String> {
    let value = text
        .parse::<usize>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    NonZeroUsize::new(value).ok_or_else(|| "value must be greater than zero".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CLIはランダム着手上限の0と80だけを境界値として受理する。
    #[test]
    fn random_moves_accepts_only_zero_through_eighty() {
        let arguments = |value: Option<&str>| {
            let mut input = vec![
                "selfplay_gen",
                "generate",
                "--output",
                "unused.bin",
                "--games",
                "1",
                "--seed",
                "1",
            ];
            if let Some(value) = value {
                input.extend(["--random-moves", value]);
            }
            Arguments::try_parse_from(input)
        };

        assert!(arguments(Some("0")).is_ok());
        assert!(arguments(Some("80")).is_ok());
        assert!(arguments(Some("81")).is_err());
        assert!(arguments(Some("-1")).is_err());
        assert!(arguments(None).is_err());
    }
}
