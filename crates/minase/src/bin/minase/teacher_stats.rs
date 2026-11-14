//! 教師データから手数の分位点、終局理由、各局面の合法手数を集計し、JSONを標準出力へ書く。
//! `minase_zero run-cycles`が各サイクルの終わりに呼び、生成した対局の分布を記録する。

use clap::Parser;
use minase::training::records::{self, Outcome, Reader};
use minase_core::{DrawReason, GameResult, MoveGenerator, WinReason};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Write};
use std::path::PathBuf;

/// 集計する教師データファイル。
#[derive(Parser)]
pub(super) struct Arguments {
    /// 教師データ形式MNSDの入力ファイル。
    #[arg(required = true, num_args = 1..)]
    pub(super) files: Vec<PathBuf>,
}

#[derive(Debug)]
enum Error {
    Open {
        path: PathBuf,
        source: io::Error,
    },
    Read {
        path: PathBuf,
        source: records::Error,
    },
    NoGames(Vec<PathBuf>),
    Json(serde_json::Error),
    Write(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Read { path, source } => write!(f, "{}: {source}", path.display()),
            Self::NoGames(paths) => {
                f.write_str("入力ファイルに対局がない:")?;
                for path in paths {
                    write!(f, " {}", path.display())?;
                }
                Ok(())
            }
            Self::Json(error) => write!(f, "cannot write JSON: {error}"),
            Self::Write(error) => write!(f, "cannot write stdout: {error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open { source, .. } | Self::Write(source) => Some(source),
            Self::Read { source, .. } => Some(source),
            Self::Json(source) => Some(source),
            Self::NoGames(_) => None,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Write(error)
    }
}

#[derive(Default)]
struct Histogram {
    bins: BTreeMap<usize, u64>,
    count: u64,
    sum: u64,
}

impl Histogram {
    fn record(&mut self, value: usize) {
        *self.bins.entry(value).or_insert(0) += 1;
        self.count += 1;
        self.sum += value as u64;
    }

    fn mean(&self) -> f64 {
        self.sum as f64 / self.count as f64
    }

    // 分位点は、累積件数がceil(p × 件数 / 100)に達する最初の値。
    fn percentile(&self, percent: u64) -> usize {
        let rank = (u128::from(self.count) * u128::from(percent)).div_ceil(100);
        let mut cumulative = 0_u128;
        for (&value, &count) in &self.bins {
            cumulative += u128::from(count);
            if cumulative >= rank {
                return value;
            }
        }
        unreachable!("空でない分布の分位点は必ず存在する")
    }

    fn summary(&self) -> Value {
        json!({
            "mean": self.mean(),
            "p50": self.percentile(50),
            "p90": self.percentile(90),
            "p99": self.percentile(99),
            "max": self.bins.last_key_value().expect("分布は空でない").0,
        })
    }
}

fn end_reason(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::NotFromGame => "not_from_game",
        Outcome::MateInThree { .. } => "win_mate_in_three",
        Outcome::EvaluationAdjudication { .. } => "win_evaluation_adjudication",
        Outcome::Game(GameResult::Win { reason, .. }) => match reason {
            WinReason::RoyalCapture => "win_royal_capture",
            WinReason::Repetition => "win_repetition",
            WinReason::PieceExhaustion => "win_piece_exhaustion",
            WinReason::BareKing => "win_bare_king",
            WinReason::Stalemate => "win_no_legal_move",
            WinReason::Mate => "win_mate",
            WinReason::Resignation => "win_resignation",
        },
        Outcome::Game(GameResult::Draw { reason }) => match reason {
            DrawReason::Repetition => "draw_repetition",
            DrawReason::PieceExhaustion => "draw_piece_exhaustion",
            DrawReason::BareKing => "draw_bare_king",
            DrawReason::Agreement => "draw_agreement",
        },
    }
}

fn summarize(paths: &[PathBuf]) -> Result<Value, Error> {
    let mut lengths = Histogram::default();
    let mut legal_counts = Histogram::default();
    let mut reasons: BTreeMap<&str, u64> = [
        "not_from_game",
        "win_royal_capture",
        "win_repetition",
        "win_piece_exhaustion",
        "win_bare_king",
        "win_no_legal_move",
        "win_mate",
        "win_resignation",
        "win_mate_in_three",
        "win_evaluation_adjudication",
        "draw_repetition",
        "draw_piece_exhaustion",
        "draw_bare_king",
        "draw_agreement",
    ]
    .into_iter()
    .map(|reason| (reason, 0))
    .collect();
    let mut moves = Vec::new();
    for path in paths {
        let file = File::open(path).map_err(|source| Error::Open {
            path: path.clone(),
            source,
        })?;
        let mut reader = Reader::new(BufReader::new(file)).map_err(|source| Error::Read {
            path: path.clone(),
            source,
        })?;
        let generator = MoveGenerator::new(reader.header().rules().moves);
        while let Some(game) = reader.read_game().map_err(|source| Error::Read {
            path: path.clone(),
            source,
        })? {
            let record = game.record();
            lengths.record(usize::from(record.start_ply()) + record.entries().len() - 1);
            *reasons
                .get_mut(end_reason(record.outcome()))
                .expect("定義済みの終局理由") += 1;
            for position in game.positions() {
                moves.clear();
                generator.generate_moves(position, &mut moves);
                legal_counts.record(moves.len());
            }
        }
    }
    if lengths.count == 0 {
        return Err(Error::NoGames(paths.to_vec()));
    }
    let mut game_length = lengths.summary();
    game_length["p95"] = json!(lengths.percentile(95));
    let over_1000: u64 = lengths.bins.range(1001..).map(|(_, &count)| count).sum();
    let bytes_per_position = 11.0 + 4.0 * legal_counts.mean();
    let output = json!({
        "files": paths.len(),
        "games": lengths.count,
        "positions": legal_counts.count,
        "game_length_plies": game_length,
        "games_over_1000_plies_fraction": over_1000 as f64 / lengths.count as f64,
        "end_reasons": reasons,
        "legal_moves": legal_counts.summary(),
        "bytes_per_position": bytes_per_position,
        "gb_per_million_positions": bytes_per_position * 1_000_000.0 / 1_000_000_000.0,
        "percentile_method": "nearest_rank",
        "legal_moves_method": "MoveGenerator::generate_moves (without repetition history)",
    });
    Ok(output)
}

fn run(arguments: Arguments) -> Result<(), Error> {
    let output = summarize(&arguments.files)?;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    serde_json::to_writer_pretty(&mut out, &output)?;
    writeln!(out)?;
    Ok(())
}

/// コマンドを実行し、失敗の説明を標準エラーへ出して非0終了する。
pub(super) fn main(arguments: Arguments) {
    if let Err(error) = run(arguments) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests;
