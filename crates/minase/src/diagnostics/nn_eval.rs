//! 固定局面集をセッションの推論器で評価し、PyTorchとの照合用JSONを書き出す。
//!
//! 出力JSONの形式の正本は、本モジュールの[`Evaluation`]と[`PositionEvaluation`]である。
//! 学習器の`minase_zero fixture-eval`も同じ形式を出力し、`backend`を`"pytorch"`とする。
//! この形式は`tools/train/zero/nn_compare.py`が読み、推論結果の照合に使う。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{fmt, fs, io};

use crate::nn::{Query, policy_label};
use crate::search::{EvaluationError, Searcher};
use minase_core::notation::sfen::{SetupPosition, to_extended_sfen};
use minase_core::{MoveGenerator, Rules};
use serde::Serialize;

use super::nn_fixture::fixture_positions;

/// 構造体のフィールド順をJSONのキー順として保持する。
#[derive(Serialize)]
struct Evaluation {
    /// ONNXのファイルのSHA-256を小文字の16進数で表す。
    network_sha256: String,
    /// 実行プロバイダ名で、`"tensorrt"`、`"cuda"`、`"cpu"`のいずれかである。
    backend: String,
    /// `NnBatchSize`で指定した、1回の推論にまとめる局面数の上限である。
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

/// 固定局面の推論、直列化、または出力の失敗。
#[derive(Debug)]
pub enum Error {
    /// 準備済みの推論器で評価できない。
    Evaluation(EvaluationError),
    /// 評価値をJSONへ直列化できない。
    Serialize(serde_json::Error),
    /// 指定されたファイルへ書き込めない。
    Write {
        /// 書き込み先。
        path: PathBuf,
        /// 入出力の原因。
        source: io::Error,
    },
    /// 所要時間の行を出力できない。
    Output(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Evaluation(error) => write!(f, "failed to evaluate NN fixtures: {error}"),
            Self::Serialize(error) => write!(f, "failed to serialize NN evaluation: {error}"),
            Self::Write { path, source } => write!(
                f,
                "failed to write NN evaluation to {}: {source}",
                path.display()
            ),
            Self::Output(error) => write!(f, "failed to report NN inference: {error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Evaluation(error) => Some(error),
            Self::Serialize(error) => Some(error),
            Self::Write { source, .. } | Self::Output(source) => Some(source),
        }
    }
}

/// セッションの準備済み推論器で288局面を評価し、指定ファイルへ上書きする。
/// `NnBatchSize`局面ずつ推論し、探索木と設定を変えずに所要時間の行を出す。
pub fn run(
    out: &Path,
    searcher: &mut Searcher,
    output: &mut dyn FnMut(&str) -> io::Result<()>,
) -> Result<(), Error> {
    let (checksum, backend, batch) = match &*searcher {
        Searcher::Mcts(engine) => engine.evaluation_metadata().map_err(Error::Evaluation)?,
        Searcher::AlphaBeta(_) => {
            return Err(Error::Evaluation(EvaluationError::UnsupportedSearch));
        }
    };
    let network_sha256 = checksum.iter().map(|byte| format!("{byte:02x}")).collect();
    let backend = backend.to_owned();
    let positions = fixture_positions();
    let generator = MoveGenerator::new(Rules::JCSA.moves);
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
        searcher
            .evaluate(&queries, &mut logits, &mut values)
            .map_err(Error::Evaluation)?;
        inference_time += start.elapsed();
        let mut logits = logits.iter();
        for (query, &value) in queries.iter().zip(&values) {
            let position = query.position;
            let setup = SetupPosition::new(position.clone(), position.lion_capture_square(), 1)
                .expect("fixed positions and move number are valid");
            let moves = query
                .moves
                .iter()
                .zip(logits.by_ref())
                .map(|(&mv, &logit)| (policy_label(mv, position.side_to_move()), logit))
                .collect();
            results.push(PositionEvaluation {
                sfen: to_extended_sfen(&setup),
                value,
                moves,
            });
        }
    }
    output(&format!(
        "NN inference ({} positions): {:.6} s",
        positions.len(),
        inference_time.as_secs_f64()
    ))
    .map_err(Error::Output)?;
    let evaluation = Evaluation {
        network_sha256,
        backend,
        batch: batch.get(),
        positions: results,
    };
    let mut bytes = serde_json::to_vec(&evaluation).map_err(Error::Serialize)?;
    bytes.push(b'\n');
    fs::write(out, bytes).map_err(|source| Error::Write {
        path: out.to_owned(),
        source,
    })
}
