//! 学習データの検査と内容の表示。

use super::cli::InspectArguments;
use minase::datagen::{invalid_data, provenance::hex, training_error};
use minase::training::records::{Outcome, Reader, Record};
use minase::{Color, Position, to_sfen};
use std::{collections::BTreeSet, fs::File, io};

/// 学習データ検査時の集計。
#[derive(Default)]
struct InspectionSummary {
    /// レコードに現れた対局番号。
    game_numbers: BTreeSet<u32>,
    /// 負け局面数。
    losses: u64,
    /// 引き分け局面数。
    draws: u64,
    /// 勝ち局面数。
    wins: u64,
    /// 先手番局面数。
    black_to_move: u64,
    /// 後手番局面数。
    white_to_move: u64,
    /// 探索値の最小値。
    minimum_score: Option<i16>,
    /// 探索値の最大値。
    maximum_score: Option<i16>,
    /// 探索値の合計。
    score_sum: i128,
}

impl InspectionSummary {
    /// 検証済みレコードを集計へ加える。
    fn record(&mut self, record: &Record) {
        self.game_numbers.insert(record.game_number());
        match record.outcome() {
            Outcome::Loss => self.losses += 1,
            Outcome::Draw => self.draws += 1,
            Outcome::Win => self.wins += 1,
        }
        match record.side_to_move() {
            Color::Black => self.black_to_move += 1,
            Color::White => self.white_to_move += 1,
        }
        self.minimum_score = Some(
            self.minimum_score
                .map_or(record.score(), |score| score.min(record.score())),
        );
        self.maximum_score = Some(
            self.maximum_score
                .map_or(record.score(), |score| score.max(record.score())),
        );
        self.score_sum += i128::from(record.score());
    }
}

/// 学習データを全件復号し、ヘッダと内容の要約を表示する。
pub(super) fn inspect(arguments: &InspectArguments) -> io::Result<()> {
    let file = File::open(&arguments.path)?;
    let mut reader = Reader::new(file).map_err(training_error)?;
    let header = reader.header();
    println!("header:");
    println!("magic: MNSD");
    println!(
        "format_version: {}",
        minase::training::records::FORMAT_VERSION
    );
    println!("record_length: {}", minase::training::records::RECORD_LEN);
    println!("rule_set: {}", header.rule_set());
    println!("generation_commit: {}", header.generation_commit());
    println!("network_checksum: {}", hex(header.network_checksum()));
    println!("teacher_nodes: {}", header.teacher_nodes());
    println!("seed: {}", header.seed());
    println!("record_count: {}", header.record_count());
    let record_count = header.record_count();
    let mut summary = InspectionSummary::default();

    for index in 1..=record_count {
        let record = match reader.read_record() {
            Ok(Some(record)) => record,
            Ok(None) => {
                eprintln!("record {index}: missing record");
                return Err(invalid_data(format!("record {index} is missing")));
            }
            Err(error) => {
                eprintln!("record {index}: {error}");
                return Err(training_error(error));
            }
        };
        let position = match record.to_position() {
            Ok(position) => position,
            Err(error) => {
                eprintln!("record {index}: {record:?}");
                eprintln!("record {index}: {error}");
                return Err(training_error(error));
            }
        };
        if index <= arguments.dump {
            print_record(index, &record, &position);
        }
        summary.record(&record);
    }

    println!("summary:");
    println!("records: {record_count}");
    println!("games: {}", summary.game_numbers.len());
    println!("outcome_loss: {}", summary.losses);
    println!("outcome_draw: {}", summary.draws);
    println!("outcome_win: {}", summary.wins);
    match (summary.minimum_score, summary.maximum_score) {
        (Some(minimum), Some(maximum)) => {
            println!("score_minimum: {minimum}");
            println!("score_maximum: {maximum}");
            println!(
                "score_average: {:.6}",
                summary.score_sum as f64 / record_count as f64
            );
        }
        _ => {
            println!("score_minimum: n/a");
            println!("score_maximum: n/a");
            println!("score_average: n/a");
        }
    }
    println!("black_to_move: {}", summary.black_to_move);
    println!("white_to_move: {}", summary.white_to_move);
    Ok(())
}

/// 検証済みレコードをSFENと各欄で表示する。
fn print_record(index: u64, record: &Record, position: &Position) {
    println!("record: {index}");
    println!("sfen: {}", to_sfen(position));
    println!("side_to_move: {:?}", record.side_to_move());
    match record.lion_square() {
        Some(square) => println!("lion_square: {}", square.dense_index()),
        None => println!("lion_square: none"),
    }
    println!(
        "lion_by_kirin_promotion: {}",
        record.lion_by_kirin_promotion()
    );
    println!("score: {}", record.score());
    println!("outcome: {:?}", record.outcome());
    println!("game_number: {}", record.game_number());
    println!("ply: {}", record.ply());
}
