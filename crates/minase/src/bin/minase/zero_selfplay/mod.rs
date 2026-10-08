//! 強化学習の自己対局生成器。Gumbel探索の自己対局で、方策の目標を持つ学習データMNZTを生成する
//! （docs/plans/alphazero.md「強化学習」「RLの探索はGumbel AlphaZeroにする」「学習データの形式」）。
//!
//! 1プロセスで数百局を同時に進め、各局の探索が評価を必要とした葉を複数局からまとめて1回の推論にかける。
//! 1サイクルの生成は塊に分け、塊ごとにMNZTとサマリを確定させるので、止まっても欠けた塊から再開できる。

mod chunk;
mod cli;
mod generate;
mod play;

pub(super) use cli::Arguments;
use cli::EvaluatorKind;
use generate::generate;
use minase::datagen::data_error;
use minase::nn::evaluator::{Evaluator, UniformEvaluator};
use minase::nn::ort_evaluator::{NETWORK, OrtEvaluator, SessionConfig};
use minase::search::mcts::default_cache_dir;
use sha2::{Digest, Sha256};
use std::{io, process};

/// コマンドを実行し、失敗時は説明を標準エラーへ出して非0終了する。
pub(super) fn main(arguments: Arguments) {
    if let Err(error) = run(&arguments) {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

/// 推論器を選んで生成する。一様評価器のネットの検査和は全零とする。
fn run(arguments: &Arguments) -> io::Result<()> {
    match arguments.evaluator {
        EvaluatorKind::Uniform => generate(
            arguments,
            &|| Ok(Box::new(UniformEvaluator) as Box<dyn Evaluator>),
            [0; 32],
        ),
        EvaluatorKind::Ort => {
            let config = SessionConfig {
                backend: arguments.nn_backend,
                cache_dir: arguments
                    .nn_cache_dir
                    .clone()
                    .unwrap_or_else(default_cache_dir),
                max_batch: arguments.nn_batch_size,
            };
            generate(
                arguments,
                &|| {
                    OrtEvaluator::new(NETWORK, &config)
                        .map(|evaluator| Box::new(evaluator) as Box<dyn Evaluator>)
                        .map_err(data_error)
                },
                Sha256::digest(NETWORK).into(),
            )
        }
    }
    .map(|_| ())
}
