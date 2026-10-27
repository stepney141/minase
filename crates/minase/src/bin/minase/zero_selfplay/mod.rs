//! Gumbel AlphaZeroによる自己対局を、再開可能な塊ごとにMNSD版2へ保存する。
//!
//! 各局の葉をスレッドごとにまとめて推論し、着手ごとに新しい探索木を使う。
//! 投了した対局は別ファイルへ保存し、最後まで指させた対局で測った誤投了率により
//! サイクル全体の学習ファイルを選ぶ。生成条件と確定済みの塊は再開時にも保持する。

mod cli;
mod error;
mod game;
mod output;
mod worker;

pub(super) use cli::Arguments;
use error::Error;
use minase::nn::{Network, OnnxEvaluator};
use minase::training::records;
use minase_core::{Rules, rules::parse_rule_set};

/// 引き延ばす戦略を学習させないための手数上限1,000手。
/// 到達しても未終局なら破棄する。`docs/plans/alphazero-selfplay.md`の
/// 「対局の進め方と手数の上限」に従う。
const MAX_PLY: u32 = 1_000;

/// 勝敗の決まった終盤の生成費用を抑える投了閾値1%。
/// 選んだ手の期待得点を使う理由は`docs/plans/alphazero-selfplay.md`の
/// 「投了」と「着手時に補った決定」にある。
const RESIGN_THRESHOLD: f32 = 0.01;

/// 誤った勝敗を教師にしないための誤投了率の上限5%。
/// 厳密に超えたサイクルの投了ファイルを除く。
/// `docs/plans/alphazero-selfplay.md`の「投了」に従う。
const FALSE_RESIGNATION_LIMIT: f64 = 0.05;

/// 1局の探索木の容量16 MiB。
/// 32回の探索の見積もり約100 KBに余裕を持たせ、容量不足は設定の誤りとして扱う。
/// `docs/plans/alphazero-selfplay.md`の「着手時に補った決定」に従う。
const TREE_CAPACITY: usize = 16 * 1024 * 1024;

/// コマンドを実行し、失敗の説明を標準エラーへ出して非0終了する。
pub(super) fn main(arguments: Arguments) {
    if let Err(error) = run(arguments) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run(arguments: Arguments) -> Result<(), Error> {
    let network = Network::read(&arguments.nn_model)?;
    let rules = engine_default_rules()?;
    let config = output::Config {
        rules: rules.to_string(),
        generation_commit: arguments.generation_commit,
        network_sha256: minase::datagen::hex(network.sha256()),
        seed: arguments.seed,
        games: arguments.games,
        chunk_games: arguments.chunk_games,
        simulations: arguments.simulations,
        playthrough_percent: arguments.playthrough_percent,
        max_ply: MAX_PLY,
    };
    let report = output::generate(
        &arguments.output_dir,
        &config,
        network.sha256(),
        arguments.threads,
        arguments.games_per_thread,
        || {
            let provider = arguments.nn_backend.provider()?;
            Ok(Box::new(OnnxEvaluator::new(
                &network,
                &provider,
                arguments.games_per_thread,
            )?))
        },
    )?;
    report.print(&config)
}

/// 生成器が使う規則を、公開の規則解析APIから構築する。
fn engine_default_rules() -> Result<Rules, Error> {
    let codes = parse_rule_set("engine-default").map_err(records::Error::RuleSetParse)?;
    Ok(Rules::from_codes(&codes).map_err(records::Error::Rules)?)
}

#[cfg(test)]
mod tests;
