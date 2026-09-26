//! ランダム対局の検証ハーネス。合法手適用の不変条件を長時間対局で検査する。

mod cli;
mod game;
mod seed;
mod summary;
mod text;

use std::{process, time::Instant};

use clap::{CommandFactory, Parser, error::ErrorKind};
use minase::Rules;

use cli::Arguments;
use game::run_game;
use seed::time_seed;
use summary::{Summary, report_game};
use text::rules_text;

/// 1局を打ち切る手数上限の既定値。
const DEFAULT_MAX_PLY: u32 = 4096;

/// 引数を検証し、指定局数のランダム対局を実行して集計を出力する。
fn main() {
    let arguments = Arguments::parse();
    let rules = match Rules::from_codes(&arguments.rules.0) {
        Ok(rules) => rules,
        Err(error) => Arguments::command()
            .error(ErrorKind::ValueValidation, error.to_string())
            .exit(),
    };
    let base_seed = match arguments.seed {
        Some(seed) => seed,
        None => match time_seed() {
            Ok(seed) => seed,
            Err(error) => {
                eprintln!("failed to generate a seed from the current time: {error}");
                process::exit(1);
            }
        },
    };
    let rules_text = rules_text(&arguments.rules.0);
    println!("rules: {rules_text}");
    println!("seed: {base_seed}");

    let start = Instant::now();
    let mut summary = Summary::default();
    let mut execute = |game_number| {
        let completed = run_game(
            rules,
            &rules_text,
            base_seed,
            game_number,
            arguments.max_ply,
            arguments.verify_all,
        );
        report_game(game_number, completed, arguments.verbose, &mut summary);
    };

    match arguments.game {
        Some(game_number) => execute(game_number),
        None => {
            for game_number in 1..=arguments.games {
                execute(game_number);
            }
        }
    }
    summary.print(start.elapsed());
}
