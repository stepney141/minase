//! 中将棋エンジンと補助ツールの引数を解析し、実行先を振り分ける。

use std::path::PathBuf;
use std::process::{ExitCode, Termination};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use minase_core::RuleCode;
use minase_core::rules::parse_rule_set;

mod bench;
mod engine;
mod io_log;
mod lishogi_import;
mod match_report;
mod match_runner;
mod perft;
mod pst_probe;
mod random_play;
mod selfplay_gen;
mod spsa_runner;
mod usi_random;

/// グローバルアロケータ。benchの実測（docs/plans/search.md 実施状況）に基づきmimallocを使う。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// 中将棋エンジンの起動と、補助ツールのサブコマンドの入口。
#[derive(Parser)]
#[command(
    name = "minase",
    subcommand_negates_reqs = true,
    args_conflicts_with_subcommands = true
)]
struct Arguments {
    /// 使用する通信プロトコル。
    #[arg(long, value_enum, required = true)]
    protocol: Option<ProtocolKind>,
    /// 採用するローカルルールコード列。
    #[arg(long, required = true, value_parser = parse_rule_set_argument)]
    rules: Option<RuleSetArgument>,
    /// 入出力のログを新規作成するパス。
    #[arg(long)]
    io_log: Option<PathBuf>,
    /// 補助ツールの呼び出し。
    #[command(subcommand)]
    command: Option<Command>,
}

/// 解析済みの`--rules`引数。
#[derive(Clone)]
struct RuleSetArgument(Vec<RuleCode>);

/// `--rules`の値を規則セット名またはコード列として解析する。
fn parse_rule_set_argument(input: &str) -> Result<RuleSetArgument, String> {
    parse_rule_set(input)
        .map(RuleSetArgument)
        .map_err(|error| error.to_string())
}

/// 対応する通信プロトコル。
#[derive(Clone, Copy, ValueEnum)]
enum ProtocolKind {
    /// lishogi系拡張を含むUSI。
    Usi,
    /// CECP(XBoard)。
    Cecp,
}

/// 目的別の補助ツール。
#[derive(Subcommand)]
enum Command {
    /// 対局測定と集計。
    #[command(subcommand)]
    Match(MatchCommand),
    /// 探索係数と時間管理係数の調整。
    Spsa(spsa_runner::Arguments),
    /// 学習データの生成と取り込み。
    #[command(subcommand)]
    Data(DataCommand),
    /// 検証と診断。
    #[command(subcommand)]
    Dev(DevCommand),
}

#[derive(Subcommand)]
#[expect(
    clippy::large_enum_variant,
    reason = "起動時に一度だけ解析する既存の引数型をそのまま保持する"
)]
enum MatchCommand {
    /// ペア対局を実行して測定する。
    Run(match_runner::Arguments),
    /// 保存済みの対局を集計する。
    Report(match_report::Arguments),
}

#[derive(Subcommand)]
enum DataCommand {
    /// 自己対局データを生成・検査する。
    Selfplay(selfplay_gen::Arguments),
    /// lishogiの棋譜を取り込む。
    Lishogi(lishogi_import::Arguments),
}

#[derive(Subcommand)]
enum DevCommand {
    /// 固定局面で探索を検証する。
    Bench(bench::Arguments),
    /// 合法手の経路数を数える。
    Perft(perft::Arguments),
    /// ランダム対局で不変条件を検証する。
    RandomPlay(random_play::Arguments),
    /// 評価関数の診断結果を出力する。
    PstProbe(pst_probe::Arguments),
    /// 校正用のランダム着手エンジンを起動する。
    UsiRandom,
}

/// 解析後の引数エラーにも完全な呼び出し名を付ける。
fn subcommand_command(path: &[&str]) -> clap::Command {
    let mut command = Arguments::command();
    command.build();
    let mut selected = &command;
    for name in path {
        selected = selected.find_subcommand(name).expect("declared subcommand");
    }
    selected.clone()
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
    match arguments.command {
        None => engine::main(
            arguments
                .protocol
                .expect("clap requires --protocol for the engine"),
            arguments
                .rules
                .expect("clap requires --rules for the engine"),
            arguments.io_log,
        ),
        Some(Command::Match(MatchCommand::Run(arguments))) => match_runner::main(arguments),
        Some(Command::Match(MatchCommand::Report(arguments))) => match_report::main(arguments),
        Some(Command::Spsa(arguments)) => spsa_runner::main(arguments),
        Some(Command::Data(DataCommand::Selfplay(arguments))) => selfplay_gen::main(arguments),
        Some(Command::Data(DataCommand::Lishogi(arguments))) => lishogi_import::main(arguments),
        Some(Command::Dev(DevCommand::Bench(arguments))) => bench::main(arguments),
        Some(Command::Dev(DevCommand::Perft(arguments))) => perft::main(arguments),
        Some(Command::Dev(DevCommand::RandomPlay(arguments))) => random_play::main(arguments),
        Some(Command::Dev(DevCommand::PstProbe(arguments))) => return pst_probe::main(arguments),
        Some(Command::Dev(DevCommand::UsiRandom)) => return usi_random::main().report(),
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use minase_core::Rules;

    use super::*;

    // cli-subcommands.md「設計判断」: 全ツールをエンジン引数なしで起動できる。
    #[test]
    fn subcommands_do_not_require_engine_arguments() {
        for args in [
            vec!["match", "run", "--run-dir", "run", "elo", "--pairs", "4"],
            vec!["match", "report", "--run-dir", "run"],
            vec!["data", "selfplay", "inspect", "positions.mnsd"],
            vec![
                "data",
                "lishogi",
                "openings",
                "--input",
                "games.ndjson",
                "--output",
                "openings",
                "--report",
                "report.json",
                "--nodes",
                "200",
            ],
            vec!["dev", "bench"],
            vec!["dev", "perft", "1", "--rules", "engine-default"],
            vec!["dev", "random-play", "--rules", "engine-default"],
            vec![
                "dev",
                "pst-probe",
                "--pst",
                "pst.bin",
                "--positions",
                "positions.mnsd",
            ],
            vec!["dev", "usi-random"],
        ] {
            let parsed = Arguments::try_parse_from(std::iter::once("minase").chain(args));
            assert!(parsed.is_ok(), "{}", parsed.err().unwrap());
        }
    }

    // 同節: 最上位のエンジン引数と補助ツールの併用を拒む。
    #[test]
    fn engine_arguments_conflict_with_subcommands() {
        for args in [
            vec!["--protocol", "usi"],
            vec!["--rules", "engine-default"],
            vec!["--io-log", "io.log"],
            vec!["--protocol", "usi", "--rules", "engine-default"],
        ] {
            assert!(
                Arguments::try_parse_from(
                    std::iter::once("minase")
                        .chain(args)
                        .chain(["dev", "usi-random"])
                )
                .is_err()
            );
        }
    }

    // 同節: SPSAのセッション・params・applyの3つの入口を保つ。
    #[test]
    fn spsa_session_and_nested_commands_parse_from_root() {
        for args in [
            vec![
                "--run-dir",
                "run",
                "--seed",
                "1",
                "--engine",
                "engine",
                "--params",
                "params.txt",
                "--rules",
                "engine-default",
                "--each",
                "depth=2",
                "--concurrency",
                "1",
                "--iterations",
                "1",
                "--pairs-per-iteration",
                "1",
            ],
            vec!["params", "--engine", "engine", "--rules", "engine-default"],
            vec!["apply", "--run-dir", "run", "--source", "params.rs"],
        ] {
            let parsed = Arguments::try_parse_from(["minase", "spsa"].into_iter().chain(args));
            assert!(parsed.is_ok(), "{}", parsed.err().unwrap());
        }
    }

    // 同節: 解析後の検査でも使い方に新しい呼び出し名を表示する。
    #[test]
    fn validation_errors_use_full_subcommand_names() {
        for path in [["match", "run"], ["dev", "random-play"], ["dev", "perft"]] {
            let error = subcommand_command(&path)
                .error(clap::error::ErrorKind::ValueValidation, "invalid arguments");
            assert_eq!(error.exit_code(), 2);
            assert!(
                error
                    .to_string()
                    .contains(&format!("Usage: minase {}", path.join(" ")))
            );
        }
    }

    #[test]
    fn cli_requires_explicit_protocol_and_rules() {
        // PL「モジュールと名称」・完了条件: --protocol usi|cecpと--rulesの明示指定を必須とし、
        // 既定値と自動判別を設けない（D6-CLI-01）。
        assert!(Arguments::try_parse_from(["minase"]).is_err());
        assert!(Arguments::try_parse_from(["minase", "--protocol", "usi"]).is_err());
        assert!(Arguments::try_parse_from(["minase", "--rules", "R1"]).is_err());
        // --protocolの値はusiとcecpの2値のみ。
        assert!(
            Arguments::try_parse_from(["minase", "--protocol", "xboard", "--rules", "R1"]).is_err()
        );
        assert!(
            Arguments::try_parse_from(["minase", "--protocol", "usi", "--rules", "R1"]).is_ok()
        );
        assert!(
            Arguments::try_parse_from(["minase", "--protocol", "cecp", "--rules", "R1"]).is_ok()
        );
    }

    #[test]
    fn rules_argument_shares_the_wire_value_grammar() {
        // PL「規則オプション」（同じ値文法を--rulesにも適用、解析は共通関数parse_rule_set）・
        // R33第6項（lishogi=L1+L2+P0+P3+R1+E1+E3、大小非区別）
        // （D6-CLI-04、D6-CLI-05のminase側接続確認）。
        let parse = |value: &str| {
            Arguments::try_parse_from(["minase", "--protocol", "usi", "--rules", value])
                .map(|arguments| arguments.rules.unwrap().0)
        };

        assert_eq!(
            parse("LISHOGI").unwrap(),
            Vec::<RuleCode>::from(Rules::LISHOGI)
        );
    }
}
