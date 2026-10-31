//! 学習データの検査と内容の表示。

use super::cli::InspectArguments;
use minase::datagen::{data_error, hex, invalid_data};
use minase::training::records::{
    Entry, Exclusion, FORMAT_VERSION, GameRecord, Origin, Outcome, Policy, Reader, Value,
};
use minase_core::{Color, GameResult, Position, notation::usi, to_sfen};
use std::{collections::BTreeMap, fs::File, io};

/// 全項目の内訳と、価値学習に使う局面の集計。
#[derive(Default)]
struct InspectionSummary {
    game_origins: [u64; 4],
    origins: [u64; 4],
    exclusions: [u64; 4],
    no_value: u64,
    policies: [u64; 3],
    candidate_counts: BTreeMap<usize, u64>,
    eligible_positions: u64,
    losses: u64,
    draws: u64,
    wins: u64,
    black_to_move: u64,
    white_to_move: u64,
    minimum_score: Option<i32>,
    maximum_score: Option<i32>,
    score_sum: i128,
}

impl InspectionSummary {
    /// 全項目を数え、合成以外で除外なし・価値ありの局面を学習対象として数える。
    fn record(&mut self, game: &GameRecord, entry: &Entry, position: &Position) {
        self.exclusions[entry.exclusion() as usize] += 1;
        if entry.value() == Value::None {
            self.no_value += 1;
        }
        match entry.policy() {
            Policy::None => self.policies[0] += 1,
            Policy::PlayedMove => self.policies[1] += 1,
            Policy::Candidates(candidates) => {
                self.policies[2] += 1;
                *self.candidate_counts.entry(candidates.len()).or_default() += 1;
            }
        }
        if entry.exclusion() != Exclusion::None || game.origin() == Origin::Synthetic {
            return;
        }
        let score = match entry.value() {
            Value::None => return,
            Value::AlphaBeta(score) => i32::from(score),
            Value::Mcts(score) => i32::from(score),
        };
        self.eligible_positions += 1;
        self.origins[game.origin() as usize] += 1;
        match game.outcome() {
            Outcome::Game(GameResult::Win { winner, .. }) | Outcome::MateInThree { winner } => {
                if winner == position.side_to_move() {
                    self.wins += 1;
                } else {
                    self.losses += 1;
                }
            }
            Outcome::Game(GameResult::Draw { .. }) => self.draws += 1,
            Outcome::NotFromGame => unreachable!("synthetic entries were excluded"),
        }
        match position.side_to_move() {
            Color::Black => self.black_to_move += 1,
            Color::White => self.white_to_move += 1,
        }
        self.minimum_score = Some(self.minimum_score.map_or(score, |v| v.min(score)));
        self.maximum_score = Some(self.maximum_score.map_or(score, |v| v.max(score)));
        self.score_sum += i128::from(score);
    }
}

/// 学習データを全対局復号し、ヘッダと内容の要約を表示する。
pub(super) fn inspect(arguments: &InspectArguments) -> io::Result<()> {
    let mut reader = Reader::new(File::open(&arguments.path)?).map_err(data_error)?;
    let header = reader.header();
    println!("header:");
    println!("magic: MNSD");
    println!("format_version: {FORMAT_VERSION}");
    println!("rule_set: {}", header.rule_set());
    println!("generation_commit: {}", header.generation_commit());
    println!("network_checksum: {}", hex(header.network_checksum()));
    println!("teacher_kind: {:?}", header.teacher_kind());
    println!("teacher_search_budget: {}", header.teacher_search_budget());
    println!("seed: {}", header.seed());
    println!("game_count: {}", header.game_count());
    println!("entry_count: {}", header.entry_count());
    let game_count = header.game_count();
    let entry_count = header.entry_count();
    let mut summary = InspectionSummary::default();
    for index in 1..=game_count {
        let decoded = match reader.read_game() {
            Ok(Some(game)) => game,
            Ok(None) => {
                return Err(invalid_data(format!(
                    "game at file index {index} is missing"
                )));
            }
            Err(error) => {
                eprintln!("game at file index {index}: {error}");
                return Err(data_error(error));
            }
        };
        let game = decoded.record();
        summary.game_origins[game.origin() as usize] += 1;
        for (k, (entry, position)) in game.entries().iter().zip(decoded.positions()).enumerate() {
            if index <= arguments.dump {
                print_entry(k, game, entry, position);
            }
            summary.record(game, entry, position);
        }
    }
    println!("summary:");
    println!("games: {game_count}");
    println!("entries: {entry_count}");
    for (code, count) in summary.game_origins.iter().enumerate() {
        println!("games_origin_{code}: {count}");
    }
    for (code, name) in ["none", "mate_band", "tactical", "repetition"]
        .iter()
        .enumerate()
    {
        println!("exclusion_{name}: {}", summary.exclusions[code]);
    }
    println!("value_none: {}", summary.no_value);
    for (code, name) in ["none", "played", "candidates"].iter().enumerate() {
        println!("policy_{name}: {}", summary.policies[code]);
    }
    for (count, entries) in &summary.candidate_counts {
        println!("policy_candidate_count_{count}: {entries}");
    }
    println!("eligible_positions: {}", summary.eligible_positions);
    for (code, count) in summary.origins.iter().enumerate() {
        println!("origin_{code}: {count}");
    }
    println!("outcome_loss: {}", summary.losses);
    println!("outcome_draw: {}", summary.draws);
    println!("outcome_win: {}", summary.wins);
    match (summary.minimum_score, summary.maximum_score) {
        (Some(minimum), Some(maximum)) => {
            println!("score_minimum: {minimum}");
            println!("score_maximum: {maximum}");
            println!(
                "score_average: {:.6}",
                summary.score_sum as f64 / summary.eligible_positions as f64
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

/// 復元した局面と、その局面の教師情報を指定順で表示する。
fn print_entry(k: usize, game: &GameRecord, entry: &Entry, position: &Position) {
    println!("entry: {} {k}", game.game_number());
    println!("sfen: {}", to_sfen(position));
    println!("side_to_move: {:?}", position.side_to_move());
    match position.lion_capture_square() {
        Some(square) => println!("lion_square: {}", square.dense_index()),
        None => println!("lion_square: none"),
    }
    println!(
        "lion_by_kirin_promotion: {}",
        position
            .lion_taken_by_non_lion()
            .is_some_and(|t| t.by_kirin_promotion)
    );
    match entry.value() {
        Value::None => println!("value: none"),
        Value::AlphaBeta(value) => println!("value: {value}"),
        Value::Mcts(value) => println!("value: {value}"),
    }
    println!(
        "exclusion: {}",
        match entry.exclusion() {
            Exclusion::None => "none",
            Exclusion::MateBand => "mate_band",
            Exclusion::Tactical => "tactical",
            Exclusion::Repetition => "repetition",
        }
    );
    match entry.policy() {
        Policy::None => println!("policy: none"),
        Policy::PlayedMove => println!("policy: played"),
        Policy::Candidates(candidates) => {
            print!("policy: {}", candidates.len());
            for (mv, probability) in candidates {
                print!(" {}:{probability}", usi::text_generated(position, *mv));
            }
            println!();
        }
    }
    if let Some(visits) = entry.visits() {
        println!("root_visits: {visits}");
    }
    match entry.played_move() {
        Some(mv) => println!("played_move: {}", usi::text_generated(position, mv)),
        None => println!("played_move: none"),
    }
    let outcome = match game.outcome() {
        Outcome::NotFromGame => "none",
        Outcome::Game(GameResult::Draw { .. }) => "Draw",
        Outcome::Game(GameResult::Win { winner, .. }) | Outcome::MateInThree { winner }
            if winner == position.side_to_move() =>
        {
            "Win"
        }
        Outcome::Game(GameResult::Win { .. }) | Outcome::MateInThree { .. } => "Loss",
    };
    println!("outcome: {outcome}");
    println!("game_number: {}", game.game_number());
    println!("ply: {}", usize::from(game.start_ply()) + k);
    println!("origin: {:?}", game.origin());
}
