//! 固定局面集をONNX Runtimeで評価し、PyTorchとの照合用JSONを書き出す。
//!
//! 出力JSONの形式の正本は、本モジュールの[`Evaluation`]と[`PositionEvaluation`]である。
//! `tools/train/zero/nn_torch_reference.py`も同じ形式を出力し、`backend`を`"pytorch"`とする。
//! この形式は`tools/train/zero/nn_compare.py`が読み、推論結果の照合に使う。

use std::fs;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::process;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};
use minase::nn::{
    Evaluator, ExecutionProvider, Network, NnError, OnnxEvaluator, Query, policy_label,
};
use minase_core::notation::sfen::{SetupPosition, to_extended_sfen};
use minase_core::{MoveGenerator, Rules};
use serde::Serialize;

use super::nn_fixture::fixture_positions;

/// 推論のネット、実行プロバイダ、バッチ、および出力先。
#[derive(Parser)]
pub(super) struct Arguments {
    /// 評価するONNXのファイル。
    #[arg(long)]
    nn_model: PathBuf,
    /// 推論に使う実行プロバイダ。
    #[arg(long, value_enum)]
    backend: Backend,
    /// 1回の依頼にまとめる局面数。1以上。
    #[arg(long)]
    batch: NonZeroUsize,
    /// JSONの出力先。既存のファイルは上書きする。
    #[arg(long)]
    out: PathBuf,
    /// TensorRTのエンジンのキャッシュを置くディレクトリ。TensorRTのときだけ必須。
    #[arg(long, required_if_eq("backend", "tensorrt"))]
    cache_dir: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
enum Backend {
    #[value(name = "tensorrt")]
    TensorRt,
    Cuda,
    Cpu,
}

impl Arguments {
    /// 引数の組合せを検査し、GPUやONNX Runtimeに触れる前にプロバイダを確定する。
    fn execution_provider(&self) -> Result<ExecutionProvider, clap::Error> {
        match (&self.backend, &self.cache_dir) {
            (Backend::TensorRt, Some(cache_root)) => Ok(ExecutionProvider::TensorRt {
                cache_root: cache_root.clone(),
            }),
            (Backend::TensorRt, None) => Err(super::subcommand_command(&["dev", "nn-eval"]).error(
                clap::error::ErrorKind::MissingRequiredArgument,
                "--backend tensorrt requires --cache-dir",
            )),
            (_, Some(_)) => Err(super::subcommand_command(&["dev", "nn-eval"]).error(
                clap::error::ErrorKind::ArgumentConflict,
                "--cache-dir requires --backend tensorrt",
            )),
            (Backend::Cuda, None) => Ok(ExecutionProvider::Cuda),
            (Backend::Cpu, None) => Ok(ExecutionProvider::Cpu),
        }
    }
}

/// 構造体のフィールド順をJSONのキー順として保持する。
#[derive(Serialize)]
struct Evaluation {
    /// ONNXのファイルのSHA-256を小文字の16進数で表す。
    network_sha256: String,
    /// 実行プロバイダ名で、`"tensorrt"`、`"cuda"`、`"cpu"`のいずれかである。
    backend: &'static str,
    /// `--batch`で指定した、1回の推論にまとめる局面数の上限である。
    batch: usize,
    /// 局面ごとの結果を固定局面集と同じ順に並べる。
    positions: Vec<PositionEvaluation>,
}

#[derive(Serialize)]
struct PositionEvaluation {
    /// 固定局面集の`sfen`と同じ拡張SFENである。
    sfen: String,
    /// 手番側の勝率を0〜1で表す。
    value: f32,
    /// 固定局面集の`moves`と同じ順に`[ラベル, logit]`を並べる。
    moves: Vec<(usize, f32)>,
}

fn evaluate(
    network: &Network,
    provider: &ExecutionProvider,
    batch: NonZeroUsize,
) -> Result<Evaluation, NnError> {
    let start = Instant::now();
    let mut evaluator = OnnxEvaluator::new(network, provider, batch)?;
    eprintln!(
        "NN session creation: {:.6} s",
        start.elapsed().as_secs_f64()
    );

    let positions = fixture_positions();
    let generator = MoveGenerator::new(Rules::ENGINE_DEFAULT.moves);
    let moves: Vec<Vec<_>> = positions
        .iter()
        .map(|position| {
            let mut moves = Vec::new();
            generator.generate_moves(position, &mut moves);
            moves
        })
        .collect();
    let mut results = Vec::with_capacity(positions.len());
    let mut logits = Vec::new();
    let mut values = Vec::new();
    let mut inference_time = Duration::ZERO;
    for (positions, moves) in positions.chunks(batch.get()).zip(moves.chunks(batch.get())) {
        let queries: Vec<_> = positions
            .iter()
            .zip(moves)
            .map(|(position, moves)| Query { position, moves })
            .collect();
        let start = Instant::now();
        evaluator.evaluate(&queries, &mut logits, &mut values)?;
        inference_time += start.elapsed();
        let mut offset = 0;
        for (query, &value) in queries.iter().zip(&values) {
            let position = query.position;
            let setup = SetupPosition::new(position.clone(), position.lion_capture_square(), 1)
                .expect("fixed positions and move number are valid");
            let moves = query
                .moves
                .iter()
                .zip(&logits[offset..offset + query.moves.len()])
                .map(|(&mv, &logit)| (policy_label(mv, position.side_to_move()), logit))
                .collect();
            results.push(PositionEvaluation {
                sfen: to_extended_sfen(&setup),
                value,
                moves,
            });
            offset += query.moves.len();
        }
    }
    eprintln!(
        "NN inference ({} positions): {:.6} s",
        positions.len(),
        inference_time.as_secs_f64()
    );
    Ok(Evaluation {
        network_sha256: network
            .sha256()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        backend: provider.name(),
        batch: batch.get(),
        positions: results,
    })
}

/// 引数を検査して固定局面集を評価し、失敗は標準エラーへ報告する。
pub(super) fn main(arguments: Arguments) {
    let provider = arguments
        .execution_provider()
        .unwrap_or_else(|error| error.exit());
    let result = Network::read(&arguments.nn_model)
        .and_then(|network| evaluate(&network, &provider, arguments.batch));
    let evaluation = match result {
        Ok(evaluation) => evaluation,
        Err(error) => {
            eprintln!("failed to evaluate NN fixtures: {error}");
            process::exit(1);
        }
    };
    let mut bytes = match serde_json::to_vec(&evaluation) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("failed to serialize NN evaluation: {error}");
            process::exit(1);
        }
    };
    bytes.push(b'\n');
    if let Err(error) = fs::write(&arguments.out, bytes) {
        eprintln!(
            "failed to write NN evaluation to {}: {error}",
            arguments.out.display()
        );
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    fn parse_provider(
        backend: &str,
        batch: &str,
        cache: &[&str],
    ) -> Result<ExecutionProvider, clap::Error> {
        let mut args = vec![
            "minase",
            "dev",
            "nn-eval",
            "--nn-model",
            "net.onnx",
            "--backend",
            backend,
            "--batch",
            batch,
            "--out",
            "evaluation.json",
        ];
        args.extend_from_slice(cache);
        let arguments = crate::Arguments::try_parse_from(args)?;
        let Some(crate::Command::Dev(crate::DevCommand::NnEval(arguments))) = arguments.command
        else {
            panic!("expected nn-eval arguments");
        };
        arguments.execution_provider()
    }

    // [`Arguments`]の契約: TensorRTだけがキャッシュの指定を必須とし、バッチ0を拒む。
    #[test]
    fn arguments_require_cache_only_for_tensorrt_and_positive_batch() {
        let cache = ["--cache-dir", "cache"];
        assert!(matches!(
            parse_provider("tensorrt", "7", &cache),
            Ok(ExecutionProvider::TensorRt { .. })
        ));
        assert!(matches!(
            parse_provider("cpu", "1", &[]),
            Ok(ExecutionProvider::Cpu)
        ));
        assert!(matches!(
            parse_provider("cuda", "256", &[]),
            Ok(ExecutionProvider::Cuda)
        ));
        for (backend, batch, cache, kind) in [
            ("tensorrt", "7", &[][..], ErrorKind::MissingRequiredArgument),
            ("cpu", "7", &cache[..], ErrorKind::ArgumentConflict),
            ("cuda", "7", &cache[..], ErrorKind::ArgumentConflict),
            ("cpu", "0", &[][..], ErrorKind::ValueValidation),
        ] {
            let Err(error) = parse_provider(backend, batch, cache) else {
                panic!("expected invalid arguments for {backend}, {batch}, {cache:?}");
            };
            assert_eq!(error.kind(), kind);
        }
    }
}
