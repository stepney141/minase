//! 中将棋エンジンと補助ツールの引数を解析し、実行先を振り分ける。

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

mod bench;
mod engine;
mod io_log;
mod lishogi_import;
mod match_report;
mod match_runner;
mod nn_eval;
mod nn_fixture;
mod perft;
mod random_play;
mod selfplay_gen;
mod spsa_runner;
mod teacher_stats;
mod usi_random;
mod zero_selfplay;

/// グローバルアロケータ。1つの実行ファイルに1つだけ置け、エンジンとすべての補助ツールに適用する。
/// mimallocを使う理由は`docs/adr/use-mimalloc-in-executables.md`にある。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// エンジンの起動と補助ツールの呼び出しを排他的に解析する。
///
/// サブコマンドなしはエンジン起動であり、`--protocol`を必須とする。
/// この欄は`Option`だが`required = true`を付け、`subcommand_negates_reqs`で補助ツール時だけ必須を外す。
/// `--protocol`、`--io-log`、`--search`とサブコマンドの併用は拒否する。
/// 探索方式は既定で`alphabeta`とし、`mcts`にはUSIを要求し、ネットは`NnModel`で指定する。
/// 引数なしの起動もエラーである。プロトコルは`usi`か`cecp`で、既定値も自動判別も持たない。
/// 対局ハーネスの`commit:`はコミットを`--protocol usi`で起動するため、この形は互換性の対象である。
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
    /// 入出力のログを新規作成するパス。
    #[arg(long)]
    io_log: Option<PathBuf>,
    /// 起動時に選ぶ探索方式。
    #[arg(long, value_enum, default_value = "alphabeta")]
    search: SearchKind,
    /// 補助ツールの呼び出し。
    #[command(subcommand)]
    command: Option<Command>,
}

/// 対応する通信プロトコル。
#[derive(Clone, Copy, ValueEnum)]
enum ProtocolKind {
    /// lishogi系拡張を含むUSI。
    Usi,
    /// CECP(XBoard)。
    Cecp,
}

/// 起動時に固定する探索方式。
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum SearchKind {
    /// NNUEを用いたαβ探索。
    Alphabeta,
    /// 方策・価値ネットを用いたモンテカルロ木探索。
    Mcts,
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
    /// Gumbel AlphaZeroの自己対局から学習データを生成する。
    ZeroSelfplay(zero_selfplay::Arguments),
    /// 教師データの手数、終局理由、合法手数を集計する。
    TeacherStats(teacher_stats::Arguments),
    /// lishogiの棋譜を取り込む。
    Lishogi(lishogi_import::Arguments),
}

#[derive(Subcommand)]
enum DevCommand {
    /// 固定局面で探索を検証する。
    Bench(bench::Arguments),
    /// ネットの入出力の照合に使う固定局面集を書き出す。
    NnFixture(nn_fixture::Arguments),
    /// 固定局面集をネットで評価し、推論の照合用JSONを書き出す。
    NnEval(nn_eval::Arguments),
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
            arguments.io_log,
            arguments.search,
        ),
        Some(Command::Match(MatchCommand::Run(arguments))) => match_runner::main(arguments),
        Some(Command::Match(MatchCommand::Report(arguments))) => match_report::main(arguments),
        Some(Command::Spsa(arguments)) => spsa_runner::main(arguments),
        Some(Command::Data(DataCommand::Selfplay(arguments))) => selfplay_gen::main(arguments),
        Some(Command::Data(DataCommand::ZeroSelfplay(arguments))) => zero_selfplay::main(arguments),
        Some(Command::Data(DataCommand::TeacherStats(arguments))) => teacher_stats::main(arguments),
        Some(Command::Data(DataCommand::Lishogi(arguments))) => lishogi_import::main(arguments),
        Some(Command::Dev(DevCommand::Bench(arguments))) => bench::main(arguments),
        Some(Command::Dev(DevCommand::NnFixture(arguments))) => nn_fixture::main(arguments),
        Some(Command::Dev(DevCommand::NnEval(arguments))) => nn_eval::main(arguments),
        Some(Command::Dev(DevCommand::Perft(arguments))) => perft::main(arguments),
        Some(Command::Dev(DevCommand::RandomPlay(arguments))) => random_play::main(arguments),
        Some(Command::Dev(DevCommand::UsiRandom)) => {
            if let Err(error) = usi_random::main() {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {

    use super::*;

    fn zero_selfplay_arguments() -> Vec<&'static str> {
        vec![
            "data",
            "zero-selfplay",
            "--output-dir",
            "cycle",
            "--nn-model",
            "net.onnx",
            "--nn-backend",
            "cpu",
            "--generation-commit",
            "0123456789abcdef0123456789abcdef01234567",
            "--seed",
            "1",
            "--games",
            "2",
            "--target-positions",
            "100",
            "--chunk-games",
            "2",
            "--simulations",
            "4",
            "--playthrough-percent",
            "100",
            "--threads",
            "1",
            "--devices",
            "1",
            "--games-per-thread",
            "2",
            "--start-position",
            "random-opening",
        ]
    }

    // zero-selfplayはすべての引数を必須とし、割合・件数・コミットの範囲を検査する。
    #[test]
    fn zero_selfplay_requires_every_option_and_validates_values() {
        let parse =
            |args: Vec<&str>| Arguments::try_parse_from(std::iter::once("minase").chain(args));
        let complete = zero_selfplay_arguments();
        for index in (2..complete.len()).step_by(2) {
            let mut missing = complete.clone();
            missing.drain(index..index + 2);
            let error = parse(missing).err().unwrap();
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::MissingRequiredArgument,
                "{}",
                complete[index]
            );
        }
        for (option, invalid) in [
            ("--playthrough-percent", "0"),
            ("--playthrough-percent", "101"),
            ("--games", "0"),
            ("--target-positions", "0"),
            ("--target-positions", "-1"),
            ("--target-positions", "18446744073709551616"),
            ("--chunk-games", "0"),
            ("--simulations", "0"),
            ("--simulations", "1"),
            ("--simulations", "2"),
            ("--simulations", "3"),
            ("--simulations", "5"),
            ("--simulations", "6"),
            ("--simulations", "4294967295"),
            ("--threads", "0"),
            ("--devices", "0"),
            ("--devices", "-1"),
            ("--devices", "18446744073709551616"),
            ("--games-per-thread", "0"),
            ("--generation-commit", "abc"),
            ("--generation-commit", "あ"),
            (
                "--generation-commit",
                "g123456789abcdef0123456789abcdef01234567",
            ),
            ("--nn-backend", "auto"),
            ("--start-position", "random"),
            ("--start-position", ""),
        ] {
            let mut args = complete.clone();
            let index = args.iter().position(|&arg| arg == option).unwrap();
            args[index + 1] = invalid;
            assert!(parse(args).is_err(), "{option} {invalid}");
        }
        for backend in ["tensorrt", "cuda", "cpu"] {
            for percent in ["1", "100"] {
                let mut args = complete.clone();
                args[7] = backend;
                let index = args
                    .iter()
                    .position(|&arg| arg == "--playthrough-percent")
                    .unwrap();
                args[index + 1] = percent;
                assert!(parse(args).is_ok());
            }
        }
        for target in ["1", "18446744073709551615"] {
            let mut args = complete.clone();
            let index = args
                .iter()
                .position(|&arg| arg == "--target-positions")
                .unwrap();
            args[index + 1] = target;
            assert!(parse(args).is_ok());
        }
    }

    // teacher-statsは1つ以上の入力ファイルを位置引数に取る。
    #[test]
    fn teacher_stats_requires_one_or_more_files() {
        let parse =
            |args: Vec<&str>| Arguments::try_parse_from(std::iter::once("minase").chain(args));
        let error = parse(vec!["data", "teacher-stats"]).err().unwrap();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
        for files in [vec!["first.mnsd"], vec!["first.mnsd", "second.mnsd"]] {
            let mut args = vec!["data", "teacher-stats"];
            args.extend(&files);
            let parsed = parse(args).unwrap();
            let Some(Command::Data(DataCommand::TeacherStats(arguments))) = parsed.command else {
                panic!("expected teacher-stats");
            };
            assert_eq!(
                arguments.files,
                files.into_iter().map(PathBuf::from).collect::<Vec<_>>()
            );
        }
    }

    // [`Arguments`]の契約: 全ツールをエンジン引数なしで起動できる。
    #[test]
    fn subcommands_do_not_require_engine_arguments() {
        for args in [
            vec![
                "match",
                "run",
                "--rules",
                "JCSA",
                "--run-dir",
                "run",
                "elo",
                "--pairs",
                "4",
            ],
            vec!["match", "report", "--run-dir", "run"],
            vec!["data", "selfplay", "inspect", "positions.mnsd"],
            zero_selfplay_arguments(),
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
            vec![
                "dev",
                "nn-eval",
                "--nn-model",
                "net.onnx",
                "--backend",
                "cpu",
                "--batch",
                "7",
                "--out",
                "evaluation.json",
            ],
            vec!["dev", "perft", "1", "--rules", "JCSA"],
            vec!["dev", "random-play", "--rules", "JCSA"],
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
            vec!["--io-log", "io.log"],
            vec!["--search", "alphabeta"],
            vec!["--search", "mcts"],
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
                "JCSA",
                "--each",
                "depth=2",
                "--concurrency",
                "1",
                "--iterations",
                "1",
                "--pairs-per-iteration",
                "1",
            ],
            vec!["params", "--engine", "engine"],
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
    fn cli_requires_explicit_protocol() {
        assert!(Arguments::try_parse_from(["minase"]).is_err());
        assert!(Arguments::try_parse_from(["minase", "--protocol", "xboard"]).is_err());
        for protocol in ["usi", "cecp"] {
            assert!(Arguments::try_parse_from(["minase", "--protocol", protocol]).is_ok());
        }
    }
}
