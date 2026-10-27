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
mod nn_fixture;
mod perft;
mod random_play;
mod selfplay_gen;
mod spsa_runner;
mod usi_random;

/// グローバルアロケータ。1つの実行ファイルに1つだけ置け、エンジンとすべての補助ツールに適用する。
/// mimallocを使う理由は`docs/adr/use-mimalloc-in-executables.md`にある。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// エンジンの起動と補助ツールの呼び出しを排他的に解析する。
///
/// サブコマンドなしはエンジン起動であり、`--protocol`と`--rules`を必須とする。
/// 両欄は`Option`だが`required = true`を付け、`subcommand_negates_reqs`で補助ツール時だけ必須を外す。
/// `--protocol`、`--rules`、`--io-log`とサブコマンドの併用は`args_conflicts_with_subcommands`で拒否する。
/// 引数なしの起動もエラーである。プロトコルは`usi`か`cecp`で、既定値も自動判別も持たない。
/// 対局ハーネスの`commit:`は過去のコミットを`--protocol usi --rules <規則>`で起動するため、この形は互換性の対象である。
/// 理由は`docs/adr/run-developer-tools-as-engine-subcommands.md`にある。
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

/// 補助ツールを目的別に分けるサブコマンド。
///
/// `match`は対局測定、`data`は学習データ、`dev`は検証と診断をまとめる。
/// 係数調整は1ツールなので群を作らず`spsa`とする。複数語の名前はケバブケースである。
/// サブコマンドとその引数は`CONTRIBUTING.md`に従う開発者向けの操作であり、互換性の対象外である。
/// 配置の理由は`docs/adr/run-developer-tools-as-engine-subcommands.md`にある。
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
    /// ネットの入出力の照合に使う固定局面集を書き出す。
    NnFixture(nn_fixture::Arguments),
    /// 合法手の経路数を数える。
    Perft(perft::Arguments),
    /// ランダム対局で不変条件を検証する。
    RandomPlay(random_play::Arguments),
    /// 校正用のランダム着手エンジンを起動する。
    UsiRandom,
}

/// 解析後の引数エラーにも最上位から辿ったサブコマンドの定義を使う。
/// 使い方を`minase <群> <名前>`として表示し、clapの引数エラーは終了コード2で終える。
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
        Some(Command::Dev(DevCommand::NnFixture(arguments))) => nn_fixture::main(arguments),
        Some(Command::Dev(DevCommand::Perft(arguments))) => perft::main(arguments),
        Some(Command::Dev(DevCommand::RandomPlay(arguments))) => random_play::main(arguments),
        Some(Command::Dev(DevCommand::UsiRandom)) => return usi_random::main().report(),
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {

    use super::*;

    // [`Arguments`]の契約: 全ツールをエンジン引数なしで起動できる。
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
            vec!["dev", "nn-fixture", "--out", "positions.jsonl"],
            vec!["dev", "perft", "1", "--rules", "engine-default"],
            vec!["dev", "random-play", "--rules", "engine-default"],
            vec!["dev", "usi-random"],
        ] {
            let parsed = Arguments::try_parse_from(std::iter::once("minase").chain(args));
            assert!(parsed.is_ok(), "{}", parsed.err().unwrap());
        }
    }

    // [`Arguments`]の契約: 最上位のエンジン引数と補助ツールの併用を拒む。
    #[test]
    fn engine_arguments_conflict_with_subcommands() {
        for args in [
            vec!["--protocol", "usi"],
            vec!["--rules", "engine-default"],
            vec!["--io-log", "io.log"],
            vec!["--protocol", "usi", "--rules", "engine-default"],
        ] {
            let error = Arguments::try_parse_from(
                std::iter::once("minase")
                    .chain(args)
                    .chain(["dev", "usi-random"]),
            )
            .err()
            .unwrap();
            assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
        }
    }

    // [`Arguments`]の契約: SPSAのセッション・params・applyの3つの入口を保つ。
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

    // [`Arguments`]の契約: 解析後の検査でも使い方に新しい呼び出し名を表示する。
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
        // [`Arguments`]はエンジンの起動に--protocol usi|cecpと--rulesを必須とし、
        // 既定値と自動判別を設けない。
        assert!(Arguments::try_parse_from(["minase"]).is_err());
        assert!(Arguments::try_parse_from(["minase", "--protocol", "usi"]).is_err());
        assert!(Arguments::try_parse_from(["minase", "--rules", "engine-default"]).is_err());
        // --protocolの値はusiとcecpの2値のみ。
        assert!(
            Arguments::try_parse_from([
                "minase",
                "--protocol",
                "xboard",
                "--rules",
                "engine-default"
            ])
            .is_err()
        );
        assert!(
            Arguments::try_parse_from(["minase", "--protocol", "usi", "--rules", "engine-default"])
                .is_ok()
        );
        assert!(
            Arguments::try_parse_from([
                "minase",
                "--protocol",
                "cecp",
                "--rules",
                "engine-default"
            ])
            .is_ok()
        );
    }

    #[test]
    fn article_33_rules_argument_shares_the_wire_value_grammar() {
        // --rulesはUSIの規則オプションと同じ文法を[`minase_core::rules::parse_rule_set`]で解析する。
        // RULES.md第33条第6項に従い、lishogi=L1+L2+P0+P3+R1+E1+E3で大小文字を区別しない。
        let parse = |value: &str| {
            Arguments::try_parse_from(["minase", "--protocol", "usi", "--rules", value])
                .map(|arguments| arguments.rules.unwrap().0)
        };

        assert_eq!(
            parse("LISHOGI").unwrap(),
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
    }
}
