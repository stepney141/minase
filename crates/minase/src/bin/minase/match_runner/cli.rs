//! 対局測定のコマンドライン引数と既定値。

use clap::{ArgGroup, Parser, Subcommand};
use minase::harness::*;
use minase::stats::GSPRT_H1_ELO;
use minase_core::RuleCode;
use minase_core::rules::parse_rule_set;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// 1局を打ち切る手数上限の既定値。
const DEFAULT_MAX_PLY: u32 = 4096;

/// GSPRTの暴走保険となる実行ペア数上限の既定値。
const DEFAULT_MAX_PAIRS: u64 = 100_000;

/// 1回のエンジン応答を待つ秒数の既定値。
const DEFAULT_RESPONSE_TIMEOUT_SECONDS: u64 = 120;

/// バイナリ対戦ハーネスのコマンドライン引数。
#[derive(Parser)]
#[command(
    name = "match_runner",
    group(
        ArgGroup::new("run_operation")
            .required(true)
            .multiple(false)
            .args(["run_dir", "resume"])
    )
)]
pub(crate) struct Arguments {
    /// 新規実行を作る、まだ存在しないディレクトリ。
    /// `--run-dir`と[`Self::resume`]のどちらか一方が必須であり、記録を残さない実行経路はない。
    #[arg(long)]
    pub(super) run_dir: Option<PathBuf>,
    /// 保存済み実験を再開するディレクトリ。新規作成との排他指定は[`Self::run_dir`]に従う。
    #[arg(long, conflicts_with_all = ["seed", "rules", "max_ply", "candidate", "baseline", "candidate_args", "baseline_args", "candidate_option", "baseline_option", "each", "candidate_limit", "baseline_limit", "candidate_hash", "baseline_hash", "response_timeout", "concurrency", "ponder"])]
    pub(super) resume: Option<PathBuf>,
    /// 再開時の目標ペア数。直前の目標より大きい値を指定する。
    #[arg(long, requires = "resume", conflicts_with = "run_dir", value_parser = parse_positive_u64)]
    pub(super) target_pairs: Option<u64>,
    /// 全ペアの乱数列を派生させる基本シード。
    #[arg(long)]
    pub(super) seed: Option<u64>,
    /// 採用するローカルルールコード列または規則セット名。
    #[arg(
        long,
        default_value = "engine-default",
        value_parser = parse_rule_set_argument
    )]
    pub(super) rules: RuleSetArgument,
    /// 1局を打ち切る手数上限。
    #[arg(long, default_value_t = DEFAULT_MAX_PLY, value_parser = parse_positive_u32)]
    pub(super) max_ply: u32,
    /// 候補側の起動コマンド（パスと空白区切りの引数）または`random`。
    #[arg(long, default_value = "random", value_parser = parse_player_spec)]
    pub(super) candidate: PlayerSpec,
    /// 基準側の起動コマンド（パスと空白区切りの引数）または`random`。
    #[arg(long, default_value = "random", value_parser = parse_player_spec)]
    pub(super) baseline: PlayerSpec,
    /// 候補側のcommit:エンジンへ追加する、空白区切りの起動引数。
    #[arg(long, allow_hyphen_values = true)]
    pub(super) candidate_args: Option<String>,
    /// 基準側のcommit:エンジンへ追加する、空白区切りの起動引数。
    #[arg(long, allow_hyphen_values = true)]
    pub(super) baseline_args: Option<String>,
    /// 各側に送る追加のUSIオプション。
    #[command(flatten)]
    pub(super) options: Box<UsiOptions>,
    /// 両エンジンに適用する既定の思考制限。
    #[arg(long, default_value = "depth=1", value_parser = parse_search_limit)]
    pub(super) each: SearchLimit,
    /// 候補側だけに適用する思考制限。
    #[arg(long, value_parser = parse_search_limit)]
    pub(super) candidate_limit: Option<SearchLimit>,
    /// 候補側だけに適用する置換表容量(MB)。省略時はエンジンの既定値。
    #[arg(long, value_name = "MB")]
    pub(super) candidate_hash: Option<u64>,
    /// 基準側だけに適用する思考制限。
    #[arg(long, value_parser = parse_search_limit)]
    pub(super) baseline_limit: Option<SearchLimit>,
    /// 基準側だけに適用する置換表容量(MB)。省略時はエンジンの既定値。
    #[arg(long, value_name = "MB")]
    pub(super) baseline_hash: Option<u64>,
    /// 1回のエンジン応答を待つ秒数。
    #[arg(
        long,
        default_value_t = DEFAULT_RESPONSE_TIMEOUT_SECONDS,
        value_parser = parse_positive_u64
    )]
    pub(super) response_timeout: u64,
    /// 同時に実行するペア数。省略時は物理コア数から自動計算する。
    #[arg(long, value_parser = parse_positive_usize)]
    pub(super) concurrency: Option<usize>,
    /// USIの予想手に従って両エンジンの先読みを進行する。
    #[arg(long)]
    pub(super) ponder: bool,
    /// 実行する統計モード。
    #[command(subcommand)]
    pub(super) mode: Option<Mode>,
}

/// 候補と基準の追加USIオプションの引数。
#[derive(clap::Args)]
pub(super) struct UsiOptions {
    /// 候補側に送るUSIオプション。名前=値の形式で繰り返し指定できる。
    #[arg(long, value_name = "NAME=VALUE", value_parser = parse_usi_option)]
    pub(super) candidate_option: Vec<(String, String)>,
    /// 基準側に送るUSIオプション。名前=値の形式で繰り返し指定できる。
    #[arg(long, value_name = "NAME=VALUE", value_parser = parse_usi_option)]
    pub(super) baseline_option: Vec<(String, String)>,
}

impl Arguments {
    /// 各側のUSIオプション名の重複を、ASCIIの大小文字を区別せず拒否する。
    pub(super) fn validate_options(&self) -> Result<(), clap::Error> {
        for (side, options) in [
            ("candidate", &self.options.candidate_option),
            ("baseline", &self.options.baseline_option),
        ] {
            let mut names = std::collections::HashSet::new();
            for (name, _) in options {
                if !names.insert(name.to_ascii_lowercase()) {
                    return Err(crate::subcommand_command(&["match", "run"]).error(
                        clap::error::ErrorKind::ValueValidation,
                        format!("duplicate --{side}-option name '{name}'"),
                    ));
                }
            }
        }
        Ok(())
    }

    /// 追加引数は、その側がコミット指定である場合だけ受理する。
    pub(super) fn validate_startup_args(&self) -> Result<(), clap::Error> {
        for (name, spec, args) in [
            ("candidate", &self.candidate, &self.candidate_args),
            ("baseline", &self.baseline, &self.baseline_args),
        ] {
            if args.is_some() && !matches!(spec.kind, PlayerKind::Commit(_)) {
                return Err(crate::subcommand_command(&["match", "run"]).error(
                    clap::error::ErrorKind::ValueValidation,
                    format!("--{name}-args requires --{name} commit:<revision>"),
                ));
            }
        }
        Ok(())
    }

    /// 共通の思考制限を各側の明示指定で上書きする。
    pub(super) fn search_limits(&self) -> (SearchLimit, SearchLimit) {
        (
            self.candidate_limit.unwrap_or(self.each),
            self.baseline_limit.unwrap_or(self.each),
        )
    }

    /// 再開では仮説の照合だけをサブコマンドとして許可する。
    pub(super) fn validate_resume_mode(&self) -> Result<(), clap::Error> {
        if self.resume.is_some() && self.mode.as_ref().is_some_and(|mode| {
            !matches!(mode, Mode::Gsprt { max_pairs: None, elo0, elo1 } if elo0.is_some() || elo1.is_some())
        }) {
            return Err(crate::subcommand_command(&["match", "run"]).error(
                clap::error::ErrorKind::ArgumentConflict,
                "--resume only accepts gsprt hypothesis checks; use --target-pairs to extend a run"));
        }
        Ok(())
    }

    /// `--ponder`を両エンジンがUSIかつ時間制御の場合だけ受理する。
    /// 満たさない条件はプロセスの起動前に拒否する。
    pub(super) fn validate_ponder(&self) -> Result<(), String> {
        if self.ponder {
            if !matches!(
                self.candidate_limit.unwrap_or(self.each),
                SearchLimit::Time(_)
            ) || !matches!(
                self.baseline_limit.unwrap_or(self.each),
                SearchLimit::Time(_)
            ) {
                return Err("--ponder requires time= limits for both engines".to_owned());
            }
            if matches!(self.candidate.kind, PlayerKind::Cecp { .. })
                || matches!(self.baseline.kind, PlayerKind::Cecp { .. })
            {
                return Err("--ponder does not support cecp: engines".to_owned());
            }
        }
        Ok(())
    }
}

/// 最初の`=`でUSIオプションを分け、改行、空の名前、および測定用の予約名を拒否する。
fn parse_usi_option(input: &str) -> Result<(String, String), String> {
    if input.contains(['\r', '\n']) {
        return Err("USI option names and values must not contain newlines".to_owned());
    }
    let (name, value) = input
        .split_once('=')
        .ok_or_else(|| "USI option must have the form NAME=VALUE".to_owned())?;
    if name.trim().is_empty() {
        return Err("USI option name must not be empty".to_owned());
    }
    if ["RuleSet", "USI_Hash", "ResignValue", "Seed", "Threads"]
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
    {
        return Err(format!(
            "USI option '{name}' is reserved by the match harness"
        ));
    }
    Ok((name.to_owned(), value.to_owned()))
}

/// 対局結果の集計方法。
#[derive(Subcommand)]
pub(super) enum Mode {
    /// ペンタノミアルGSPRTでH0またはH1を逐次判定する。
    Gsprt {
        /// 判定を保留して停止する実行ペア数の上限。新規実行の既定値は100000。
        #[arg(long, value_parser = parse_positive_u64)]
        max_pairs: Option<u64>,
        /// H0のElo。新規実行の既定値は0。再開時は保存済みの値と照合する。
        #[arg(long, value_name = "ELO", allow_hyphen_values = true, value_parser = parse_finite_elo)]
        elo0: Option<f64>,
        /// H1のElo。新規実行の既定値は10。再開時は保存済みの値と照合する。
        #[arg(long, value_name = "ELO", allow_hyphen_values = true, value_parser = parse_finite_elo)]
        elo1: Option<f64>,
    },
    /// 固定ペア数からEloと95%信頼区間を推定する。
    Elo {
        /// 実行するペア数。
        #[arg(long, value_parser = parse_positive_u64)]
        pairs: u64,
    },
}

impl Mode {
    /// 新規実行の既定値を補い、仮説の順序を検証する。
    pub(super) fn new_run_settings(&self) -> Result<(u64, super::storage::ManifestMode), String> {
        use super::storage::ManifestMode;
        match self {
            Self::Gsprt {
                max_pairs,
                elo0,
                elo1,
            } => {
                let h0_elo = elo0.unwrap_or(0.0);
                let h1_elo = elo1.unwrap_or(GSPRT_H1_ELO);
                if h0_elo >= h1_elo {
                    return Err("--elo0 must be less than --elo1".to_owned());
                }
                Ok((
                    max_pairs.unwrap_or(DEFAULT_MAX_PAIRS),
                    ManifestMode::Gsprt {
                        h0_elo,
                        h1_elo,
                        alpha: 0.05,
                        beta: 0.05,
                    },
                ))
            }
            Self::Elo { pairs } => Ok((*pairs, ManifestMode::Elo)),
        }
    }
}

/// 仮説として指定できる有限のEloを解析する。
fn parse_finite_elo(text: &str) -> Result<f64, String> {
    let value = text
        .parse::<f64>()
        .map_err(|error| format!("invalid Elo '{text}': {error}"))?;
    if !value.is_finite() {
        return Err("Elo must be finite".to_owned());
    }
    Ok(value)
}

/// `--rules`の入力原文と解析済みコード列。
#[derive(Clone)]
pub(super) struct RuleSetArgument {
    /// 両エンジンへ渡す入力原文。
    pub(super) source: String,
    /// 審判層と測定記録に使う解析済みコード列。
    pub(super) codes: Vec<RuleCode>,
}

/// `--rules`の値を規則セット名またはコード列として解析する。
pub(super) fn parse_rule_set_argument(input: &str) -> Result<RuleSetArgument, String> {
    parse_rule_set(input)
        .map(|codes| RuleSetArgument {
            source: input.to_owned(),
            codes,
        })
        .map_err(|error| error.to_string())
}

/// 0より大きい`usize`を解析する。
fn parse_positive_usize(text: &str) -> Result<usize, String> {
    let value = text
        .parse::<usize>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if value == 0 {
        return Err("value must be at least 1".to_owned());
    }
    Ok(value)
}

/// 現在時刻から基本シードを生成する。
pub(super) fn time_seed() -> Result<u64, std::time::SystemTimeError> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(nanos as u64 ^ (nanos >> 64) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 設計書「対局ハーネスにUSIオプションを指定する」: 最初の=だけで分け、各側の列を保つ。
    #[test]
    fn usi_options_preserve_values_and_order_for_each_side() {
        let args = Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "unused",
            "--candidate-option",
            "NnBatchSize=64",
            "--candidate-option",
            "Output File= path=a b ",
            "--candidate-option",
            "Label=",
            "--baseline-option",
            "NnBatchSize=256",
        ])
        .unwrap();
        args.validate_options().unwrap();
        assert_eq!(
            args.options.candidate_option,
            vec![
                ("NnBatchSize".to_owned(), "64".to_owned()),
                ("Output File".to_owned(), " path=a b ".to_owned()),
                ("Label".to_owned(), String::new()),
            ]
        );
        assert_eq!(
            args.options.baseline_option,
            vec![("NnBatchSize".to_owned(), "256".to_owned())]
        );
    }

    #[test]
    fn usi_options_reject_reserved_names_newlines_and_missing_separators() {
        for flag in ["--candidate-option", "--baseline-option"] {
            for input in [
                "RuleSet=R1",
                "rUlEsEt=R1",
                "USI_Hash=64",
                "usi_hash=64",
                "ResignValue=0",
                "resignvalue=0",
                "Seed=1",
                "SEED=1",
                "Threads=1",
                "threads=1",
                "NnBatchSize",
                "",
                "=64",
                " =64",
                "Nn\nBatchSize=64",
                "Nn\rBatchSize=64",
                "Label=a\nb",
                "Label=a\rb",
            ] {
                let error =
                    Arguments::try_parse_from(["match_runner", "--run-dir", "unused", flag, input])
                        .err()
                        .unwrap();
                assert_eq!(
                    error.kind(),
                    clap::error::ErrorKind::ValueValidation,
                    "{flag} {input:?}"
                );
            }
        }
    }

    #[test]
    fn usi_options_reject_duplicate_names_with_ascii_case_folding() {
        for flag in ["--candidate-option", "--baseline-option"] {
            for duplicate in ["NnBatchSize=64", "nnbATCHsIZE=256"] {
                let args = Arguments::try_parse_from([
                    "match_runner",
                    "--run-dir",
                    "unused",
                    flag,
                    "NnBatchSize=64",
                    flag,
                    duplicate,
                ])
                .unwrap();
                assert_eq!(
                    args.validate_options().unwrap_err().kind(),
                    clap::error::ErrorKind::ValueValidation
                );
            }
        }
    }

    #[test]
    fn search_limits_resolve_each_and_independent_overrides() {
        for (extra, candidate, baseline) in [
            (vec![], 4, 4),
            (vec!["--candidate-limit", "depth=2"], 2, 4),
            (vec!["--baseline-limit", "depth=1"], 4, 1),
            (
                vec![
                    "--candidate-limit",
                    "depth=2",
                    "--baseline-limit",
                    "depth=1",
                ],
                2,
                1,
            ),
        ] {
            let args = Arguments::try_parse_from(
                ["match_runner", "--run-dir", "unused", "--each", "depth=4"]
                    .into_iter()
                    .chain(extra),
            )
            .unwrap();
            assert_eq!(
                args.search_limits(),
                (
                    SearchLimit::Fixed {
                        depth: Some(candidate),
                        nodes: None
                    },
                    SearchLimit::Fixed {
                        depth: Some(baseline),
                        nodes: None
                    }
                )
            );
        }
    }

    #[test]
    fn resume_mode_rejects_new_run_budgets_and_bare_subcommands() {
        for extra in [
            vec!["elo", "--pairs", "1"],
            vec!["gsprt"],
            vec!["gsprt", "--max-pairs", "1"],
        ] {
            let args = Arguments::try_parse_from(
                ["match_runner", "--resume", "unused"]
                    .into_iter()
                    .chain(extra),
            )
            .unwrap();
            assert_eq!(
                args.validate_resume_mode().unwrap_err().kind(),
                clap::error::ErrorKind::ArgumentConflict
            );
        }
        for extra in [
            vec![],
            vec!["gsprt", "--elo0", "20"],
            vec!["gsprt", "--elo1", "30"],
        ] {
            let args = Arguments::try_parse_from(
                ["match_runner", "--resume", "unused"]
                    .into_iter()
                    .chain(extra),
            )
            .unwrap();
            args.validate_resume_mode().unwrap();
        }
    }

    // docs/guides/sprt.mdに従い、文書化された既定値
    // (--response-timeout 120秒、--max-ply 4096、--max-pairs 100,000)を
    // 文書の明文値リテラルで固定する。文書が変わらない限り実装定数の変更は
    // 逸脱である。
    #[test]
    fn documented_defaults_match_sprt_md() {
        let arguments = Arguments::try_parse_from(["match_runner", "--run-dir", "run", "gsprt"])
            .expect("the documented default invocation must be accepted");
        assert_eq!(arguments.candidate_hash, None);
        assert_eq!(arguments.baseline_hash, None);
        assert_eq!(arguments.response_timeout, 120);
        assert_eq!(arguments.max_ply, 4096);
        assert_eq!(arguments.concurrency, None);
        assert!(matches!(
            arguments.mode,
            Some(Mode::Gsprt {
                max_pairs: None,
                elo0: None,
                elo1: None
            })
        ));
        let (target, mode) = arguments.mode.unwrap().new_run_settings().unwrap();
        assert_eq!(target, 100_000);
        assert_eq!(
            serde_json::to_string(&mode).unwrap(),
            r#"{"kind":"gsprt","h0_elo":0.0,"h1_elo":10.0,"alpha":0.05,"beta":0.05}"#
        );
    }

    #[test]
    fn gsprt_hypotheses_accept_finite_values_and_require_increasing_order() {
        for (args, expected) in [
            (vec!["--elo0", "-5", "--elo1", "5"], Some((-5.0, 5.0))),
            (vec!["--elo0", "-2.5"], Some((-2.5, 10.0))),
            (vec!["--elo1", "2.5"], Some((0.0, 2.5))),
            (vec!["--elo0", "5", "--elo1", "5"], None),
            (vec!["--elo0", "5", "--elo1", "-5"], None),
            (vec!["--elo0", "10"], None),
            (vec!["--elo1", "0"], None),
        ] {
            let arguments = Arguments::try_parse_from(
                ["match_runner", "--run-dir", "run", "gsprt"]
                    .into_iter()
                    .chain(args),
            )
            .unwrap();
            let settings = arguments.mode.unwrap().new_run_settings();
            if let Some((h0, h1)) = expected {
                let (_, mode) = settings.unwrap();
                assert_eq!(
                    serde_json::to_value(mode).unwrap(),
                    serde_json::json!({
                        "kind": "gsprt", "h0_elo": h0, "h1_elo": h1, "alpha": 0.05, "beta": 0.05
                    })
                );
            } else {
                assert!(settings.is_err());
            }
        }
        for flag in ["--elo0", "--elo1"] {
            for value in ["NaN", "inf", "-inf", "1e309", "-1e309"] {
                assert!(
                    Arguments::try_parse_from([
                        "match_runner",
                        "--run-dir",
                        "run",
                        "gsprt",
                        flag,
                        value
                    ])
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn explicit_concurrency_remains_a_positive_override() {
        let arguments = Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "run",
            "--concurrency",
            "4",
            "gsprt",
        ])
        .unwrap();
        assert_eq!(arguments.concurrency, Some(4));
        assert!(
            Arguments::try_parse_from([
                "match_runner",
                "--run-dir",
                "run",
                "--concurrency",
                "0",
                "gsprt",
            ])
            .is_err()
        );
    }

    // [`Arguments::run_dir`]と[`Arguments::resume`]は、記録の新規作成と再開を排他的に要求する。
    #[test]
    fn run_directory_operation_is_required_and_exclusive() {
        assert!(Arguments::try_parse_from(["match_runner", "gsprt"]).is_err());
        assert!(
            Arguments::try_parse_from([
                "match_runner",
                "--run-dir",
                "new",
                "--resume",
                "old",
                "gsprt",
            ])
            .is_err()
        );
        assert!(Arguments::try_parse_from(["match_runner", "--resume", "old"]).is_ok());
    }

    #[test]
    fn resume_condition_arguments_are_clap_conflicts() {
        for args in [
            vec!["--seed", "42"],
            vec!["--candidate", "random"],
            vec!["--baseline", "random"],
            vec!["--each", "depth=1"],
            vec!["--rules", "engine-default"],
            vec!["--max-ply", "16"],
            vec!["--response-timeout", "120"],
            vec!["--concurrency", "1"],
            vec!["--ponder"],
            vec!["--candidate-limit", "depth=1"],
            vec!["--baseline-limit", "depth=1"],
            vec!["--candidate-hash", "256"],
            vec!["--baseline-hash", "256"],
            vec!["--candidate-args", "--search mcts"],
            vec!["--baseline-args", "--search alphabeta"],
            vec!["--candidate-option", "NnBatchSize=64"],
            vec!["--baseline-option", "NnBatchSize=64"],
        ] {
            let error = Arguments::try_parse_from(
                ["match_runner", "--resume", "old"]
                    .into_iter()
                    .chain(args.iter().copied()),
            )
            .err()
            .unwrap();
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "{args:?}"
            );
            assert_eq!(error.exit_code(), 2, "{args:?}");
        }
        let error = Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "new",
            "--target-pairs",
            "2",
            "elo",
            "--pairs",
            "1",
        ])
        .err()
        .unwrap();
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn startup_arguments_are_accepted_only_for_the_corresponding_commit_engine() {
        for side in ["candidate", "baseline"] {
            for spec in [
                "commit:HEAD",
                "random",
                "unavailable-engine",
                "cecp:unavailable-engine",
            ] {
                for value in ["--search mcts --nn-model net.onnx", ""] {
                    let arguments = Arguments::try_parse_from([
                        "match_runner",
                        "--run-dir",
                        "unused",
                        &format!("--{side}"),
                        spec,
                        &format!("--{side}-args"),
                        value,
                        "elo",
                        "--pairs",
                        "1",
                    ])
                    .unwrap();
                    let result = arguments.validate_startup_args();
                    if spec == "commit:HEAD" {
                        result.unwrap();
                    } else {
                        assert_eq!(
                            result.unwrap_err().kind(),
                            clap::error::ErrorKind::ValueValidation
                        );
                    }
                }
            }
        }
    }

    // 外部入力は型の最大値を超えた時点で拒否する。
    #[test]
    fn numeric_arguments_reject_overflow() {
        for args in [
            vec!["--seed", "18446744073709551616"],
            vec!["--max-ply", "4294967296"],
            vec!["--candidate-hash", "18446744073709551616"],
            vec!["--baseline-hash", "18446744073709551616"],
            vec!["--response-timeout", "18446744073709551616"],
            vec!["--concurrency", "18446744073709551616"],
            vec!["gsprt", "--max-pairs", "18446744073709551616"],
            vec!["elo", "--pairs", "18446744073709551616"],
        ] {
            let error = Arguments::try_parse_from(
                ["match_runner", "--run-dir", "new"]
                    .into_iter()
                    .chain(args.iter().copied()),
            )
            .err()
            .unwrap();
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::ValueValidation,
                "{args:?}"
            );
        }
        let error = Arguments::try_parse_from([
            "match_runner",
            "--resume",
            "old",
            "--target-pairs",
            "18446744073709551616",
        ])
        .err()
        .unwrap();
        assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
    }

    // RULES.md第33条: 共通parserへの接続と指定原文の保持を検査する。
    // docs/guides/sprt.md: --rules省略時にはengine-defaultを使う。
    #[test]
    fn rules_presets_resolve_per_article_33() {
        let default = Arguments::try_parse_from(["match_runner", "--run-dir", "run", "gsprt"])
            .expect("omitting --rules must fall back to engine-default");
        assert_eq!(default.rules.source, "engine-default");
        assert_eq!(
            default.rules.codes,
            vec![RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E0]
        );

        let lishogi = Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "run",
            "--rules",
            "LISHOGI",
            "gsprt",
        ])
        .expect("preset names must match case-insensitively");
        assert_eq!(lishogi.rules.source, "LISHOGI");
        assert_eq!(
            lishogi.rules.codes,
            vec![
                RuleCode::L1,
                RuleCode::L2,
                RuleCode::P0,
                RuleCode::P3,
                RuleCode::R1,
                RuleCode::E1,
                RuleCode::E3
            ]
        );
        assert!(
            Arguments::try_parse_from([
                "match_runner",
                "--run-dir",
                "run",
                "--rules",
                "lishogi,P1",
                "gsprt",
            ])
            .is_err()
        );
    }

    // [`Arguments::validate_ponder`]の先読み条件を検査する。
    #[test]
    fn ponder_arguments_require_two_usi_time_controls() {
        for extra in [
            vec![],
            vec!["--candidate-limit", "time=2000+10"],
            vec!["--concurrency", "3"],
        ] {
            let mut args = vec![
                "match_runner",
                "--run-dir",
                "unused",
                "--each",
                "time=1000+10",
                "--ponder",
            ];
            args.extend(extra);
            args.push("gsprt");
            let parsed = Arguments::try_parse_from(args).unwrap();
            assert!(parsed.ponder);
            assert!(parsed.validate_ponder().is_ok());
        }
        for extra in [
            vec!["--each", "depth=4"],
            vec!["--each", "nodes=1000"],
            vec!["--each", "movetime=300,timemargin=100"],
            vec!["--candidate-limit", "movetime=300,timemargin=100"],
            vec!["--baseline-limit", "movetime=300,timemargin=100"],
            vec!["--baseline-limit", "depth=1"],
            vec!["--candidate-limit", "nodes=1"],
            vec!["--baseline", "cecp:engine"],
            vec!["--candidate", "cecp:engine"],
        ] {
            let mut args = vec!["match_runner", "--run-dir", "unused", "--ponder"];
            if extra[0] != "--each" {
                args.extend(["--each", "time=1000+10"]);
            }
            args.extend(extra);
            args.push("gsprt");
            assert!(
                Arguments::try_parse_from(args)
                    .unwrap()
                    .validate_ponder()
                    .is_err()
            );
        }
        assert!(
            Arguments::try_parse_from([
                "match_runner",
                "--run-dir",
                "unused",
                "--ponder",
                "true",
                "gsprt"
            ])
            .is_err()
        );
        let plain =
            Arguments::try_parse_from(["match_runner", "--run-dir", "unused", "gsprt"]).unwrap();
        assert!(!plain.ponder);
    }
}
