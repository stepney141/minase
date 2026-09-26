//! 対局測定のコマンドライン引数と既定値。

use clap::{ArgGroup, Parser, Subcommand};
use minase::RuleCode;
use minase::core::rules::parse_rule_set;
use minase::harness::*;
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
pub(super) struct Arguments {
    /// 新しい実験を作成する、まだ存在しない実行ディレクトリ。
    #[arg(long)]
    pub(super) run_dir: Option<PathBuf>,
    /// 保存済みの実験を再開する実行ディレクトリ。
    #[arg(long)]
    pub(super) resume: Option<PathBuf>,
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
    pub(super) mode: Mode,
}

impl Arguments {
    /// 先読みを利用できない対局条件を、プロセス起動前に拒否する。
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

/// 対局結果の集計方法。
#[derive(Subcommand)]
pub(super) enum Mode {
    /// ペンタノミアルGSPRTでH0またはH1を逐次判定する。
    Gsprt {
        /// 判定を保留して停止する実行ペア数の上限。
        #[arg(long, default_value_t = DEFAULT_MAX_PAIRS, value_parser = parse_positive_u64)]
        max_pairs: u64,
    },
    /// 固定ペア数からEloと95%信頼区間を推定する。
    Elo {
        /// 実行するペア数。
        #[arg(long, value_parser = parse_positive_u64)]
        pairs: u64,
    },
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
    use minase::Rules;

    // D8-HARN-08/D8-STAT-06(sprt.md): 文書化された既定値
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
        assert!(matches!(arguments.mode, Mode::Gsprt { max_pairs: 100_000 }));
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

    // match-harness-efficiency.md「実行記録と再開」: 記録なし実行を許さず、
    // 新規作成と再開を同時に指定させない。
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
        assert!(Arguments::try_parse_from(["match_runner", "--resume", "old", "gsprt"]).is_ok());
    }

    // D8-HARN-09(sprt.md測定の種類と標準コマンド節): `--each`がマッチ共通既定、
    // `--candidate-limit`・`--baseline-limit`が当該エンジンだけを上書きする。
    // 対等条件と完了基準ゲートの非対称条件を同じCLIで表現できる。
    #[test]
    fn each_limit_is_shared_and_per_engine_overrides_are_optional() {
        let equal = Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "run",
            "--each",
            "depth=4",
            "gsprt",
        ])
        .expect("the equal-condition invocation must be accepted");
        assert_eq!(
            equal.each,
            SearchLimit::Fixed {
                depth: Some(4),
                nodes: None
            }
        );
        assert_eq!(equal.candidate_limit, None);
        assert_eq!(equal.baseline_limit, None);

        // 完了基準ゲートの形: ベースライン側だけdepth=1へ上書き
        let gate = Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "run",
            "--each",
            "depth=4",
            "--baseline-limit",
            "depth=1",
            "gsprt",
        ])
        .expect("the asymmetric gate invocation must be accepted");
        assert_eq!(
            gate.baseline_limit,
            Some(SearchLimit::Fixed {
                depth: Some(1),
                nodes: None
            })
        );
        assert_eq!(gate.candidate_limit, None);

        // 等価表現: `--each X`と`--candidate-limit X --baseline-limit X`は
        // 同一の制限値を与える
        let explicit = Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "run",
            "--candidate-limit",
            "depth=4",
            "--baseline-limit",
            "depth=4",
            "gsprt",
        ])
        .expect("explicit overrides must be accepted");
        assert_eq!(explicit.candidate_limit, Some(equal.each));
        assert_eq!(explicit.baseline_limit, Some(equal.each));
    }

    // 置換表容量の指定: 両側の容量は独立に指定でき、省略時はNoneを保つ。
    #[test]
    fn hash_options_resolve_independently_for_each_engine() {
        let arguments = Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "run",
            "--candidate",
            "engine",
            "--baseline",
            "cecp:engine",
            "--candidate-hash",
            "300",
            "--baseline-hash",
            "512",
            "gsprt",
        ])
        .unwrap();
        for (spec, hash_mb, expected) in [
            (arguments.candidate, arguments.candidate_hash, 300),
            (arguments.baseline, arguments.baseline_hash, 512),
        ] {
            let player = resolve_player(spec, arguments.each, hash_mb, "R1", Vec::new()).unwrap();
            assert_eq!(player.hash_mb, Some(expected));
        }
    }

    // RULES.md第33条: 共通parserへの接続と指定原文の保持を検査する。
    // sprt.md: --rules省略時にはengine-defaultを使う。
    #[test]
    fn rules_presets_resolve_per_article_33() {
        let default = Arguments::try_parse_from(["match_runner", "--run-dir", "run", "gsprt"])
            .expect("omitting --rules must fall back to engine-default");
        assert_eq!(default.rules.source, "engine-default");
        assert_eq!(
            default.rules.codes,
            Vec::<RuleCode>::from(Rules::ENGINE_DEFAULT)
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
        assert_eq!(lishogi.rules.codes, Vec::<RuleCode>::from(Rules::LISHOGI));
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

    // D8-HARN-21/27（ponder.md「対局ハーネスの対局進行」「同時対局数」）。
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
