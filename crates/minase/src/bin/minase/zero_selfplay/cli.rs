//! 強化学習の自己対局生成器の引数と既定値。

use clap::ValueEnum;
use minase::nn::ort_evaluator::NnBackend;
use minase_core::Rules;
use minase_core::rules::parse_rule_set;
use std::{
    num::{NonZeroU16, NonZeroU32, NonZeroUsize},
    path::PathBuf,
};

/// 強化学習の自己対局生成器のコマンドライン引数（docs/plans/alphazero.md「強化学習」）。
#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// 塊のMNZTとサマリを置くディレクトリ。
    #[arg(long)]
    pub(super) out_dir: PathBuf,
    /// 生成する対局の総数。
    #[arg(long, value_parser = parse_positive_u32)]
    pub(super) games: NonZeroU32,
    /// 1つの塊の対局数。
    #[arg(long, default_value = "2000", value_parser = parse_positive_u32)]
    pub(super) chunk_games: NonZeroU32,
    /// サイクルの基本シード。塊のシードはここから塊の対局数の2倍ずつ離して派生させる。
    #[arg(long)]
    pub(super) seed: u64,
    /// 採用する規則セット（規則セット名またはコード列）。
    #[arg(long, value_parser = parse_rules)]
    pub(super) rules: Rules,
    /// 同時に進める対局数。
    #[arg(long, default_value = "256", value_parser = parse_positive_usize)]
    pub(super) games_in_flight: NonZeroUsize,
    /// 対局の集合を分けて受け持つスレッドの数。スレッドごとに推論器を1つ持つ。
    #[arg(long, default_value = "2", value_parser = parse_positive_usize)]
    pub(super) threads: NonZeroUsize,
    /// Gumbel探索の1手の探索回数n。
    #[arg(long, default_value = "32", value_parser = parse_positive_u32)]
    pub(super) simulations: NonZeroU32,
    /// 1局の手数の上限。達した対局は学習データに書かない。
    #[arg(long, default_value = "1000", value_parser = parse_positive_u16)]
    pub(super) max_ply: NonZeroU16,
    /// 投了する根の勝率の閾値。手番側の根の勝率がこれを下回ったら投了する。
    #[arg(long, default_value = "0.01", value_parser = parse_unit_f32)]
    pub(super) resign_threshold: f32,
    /// 投了せずに最後まで指させる対局の割合。
    #[arg(long, default_value = "0.2", value_parser = parse_unit_f64)]
    pub(super) resign_playthrough: f64,
    /// 実戦開始の局面一覧。省略時は初期局面から乱択の手順で開始局面を作る。
    #[arg(long)]
    pub(super) openings: Option<PathBuf>,
    /// 推論器。
    #[arg(long, value_enum, default_value = "ort")]
    pub(super) evaluator: EvaluatorKind,
    /// 推論に使うネット（ONNX）のファイル。省略時は埋め込みのネット。`--evaluator uniform`とは併用できない。
    #[arg(long)]
    pub(super) nn_model: Option<PathBuf>,
    /// ortの実行プロバイダ（tensorrt、cuda、cpu）。
    #[arg(long, default_value = "tensorrt", value_parser = parse_backend)]
    pub(super) nn_backend: NnBackend,
    /// TensorRTのエンジンのキャッシュを置く親ディレクトリ。省略時は`NnCacheDir`の既定値。
    #[arg(long)]
    pub(super) nn_cache_dir: Option<PathBuf>,
    /// 1回の推論にまとめる局面数の上限。
    #[arg(long, default_value = "256", value_parser = parse_positive_usize)]
    pub(super) nn_batch_size: NonZeroUsize,
    /// 確定済みの塊を残し、欠けた塊だけを生成し直す。
    #[arg(long)]
    pub(super) resume: bool,
    /// 変更のある作業ツリーからの生成を許可する。
    #[arg(long)]
    pub(super) allow_dirty: bool,
}

/// 推論器の種類。
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub(super) enum EvaluatorKind {
    /// ネット（既定は埋め込みのネット、`--nn-model`で別のファイル）をONNX Runtimeで推論する。
    Ort,
    /// すべての合法手に等しい方策と勝率0.5を返す試験用の評価器。
    Uniform,
}

impl EvaluatorKind {
    /// サマリに表示する名前を返す。
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Ort => "ort",
            Self::Uniform => "uniform",
        }
    }
}

/// 規則セット名またはコード列を解析する。
fn parse_rules(text: &str) -> Result<Rules, String> {
    let codes = parse_rule_set(text).map_err(|error| error.to_string())?;
    Rules::from_codes(&codes).map_err(|error| error.to_string())
}

/// ortの実行プロバイダの名前を解析する。
fn parse_backend(text: &str) -> Result<NnBackend, String> {
    text.parse()
        .map_err(|error: minase::nn::ort_evaluator::UnknownNnBackend| error.to_string())
}

/// 0より大きい`u32`を解析する。
fn parse_positive_u32(text: &str) -> Result<NonZeroU32, String> {
    text.parse::<NonZeroU32>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))
}

/// 0より大きい`u16`を解析する。
fn parse_positive_u16(text: &str) -> Result<NonZeroU16, String> {
    text.parse::<NonZeroU16>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))
}

/// 0より大きい`usize`を解析する。
fn parse_positive_usize(text: &str) -> Result<NonZeroUsize, String> {
    text.parse::<NonZeroUsize>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))
}

/// 0以上1以下の`f32`を解析する。
fn parse_unit_f32(text: &str) -> Result<f32, String> {
    let value = text
        .parse::<f32>()
        .map_err(|error| format!("invalid number '{text}': {error}"))?;
    if (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(format!("value must be in [0, 1], got {text}"))
    }
}

/// 0以上1以下の`f64`を解析する。
fn parse_unit_f64(text: &str) -> Result<f64, String> {
    let value = text
        .parse::<f64>()
        .map_err(|error| format!("invalid number '{text}': {error}"))?;
    if (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(format!("value must be in [0, 1], got {text}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// 試験用に引数だけを解析する最上位のコマンド。
    #[derive(Parser)]
    struct Command {
        #[command(flatten)]
        arguments: Arguments,
    }

    /// 既定値は設計書の値であり、範囲外の割合と0は拒否する。
    #[test]
    fn defaults_follow_the_design_and_ranges_are_checked() {
        let parse = |extra: &[&str]| {
            Command::try_parse_from(
                [
                    "zero-selfplay",
                    "--out-dir",
                    "out",
                    "--games",
                    "10",
                    "--seed",
                    "1",
                    "--rules",
                    "engine-default",
                ]
                .into_iter()
                .chain(extra.iter().copied()),
            )
            .map(|command| command.arguments)
        };
        let arguments = parse(&[]).unwrap();
        assert_eq!(arguments.chunk_games.get(), 2000);
        assert_eq!(arguments.games_in_flight.get(), 256);
        assert_eq!(arguments.threads.get(), 2);
        assert_eq!(arguments.simulations.get(), 32);
        assert_eq!(arguments.max_ply.get(), 1000);
        assert_eq!(arguments.resign_threshold, 0.01);
        assert_eq!(arguments.resign_playthrough, 0.2);
        assert_eq!(arguments.evaluator, EvaluatorKind::Ort);
        assert_eq!(arguments.nn_backend, NnBackend::TensorRt);
        assert_eq!(arguments.nn_batch_size.get(), 256);
        assert_eq!(arguments.nn_model, None);
        assert_eq!(
            parse(&["--nn-model", "net.onnx"]).unwrap().nn_model,
            Some(PathBuf::from("net.onnx"))
        );
        assert!(parse(&["--resign-playthrough", "1.5"]).is_err());
        assert!(parse(&["--resign-threshold", "NaN"]).is_err());
        assert!(parse(&["--simulations", "0"]).is_err());
        assert!(parse(&["--nn-backend", "rocm"]).is_err());
        assert!(parse(&["--evaluator", "uniform"]).is_ok());
    }
}
