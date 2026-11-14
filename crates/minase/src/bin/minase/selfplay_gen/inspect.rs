//! 学習データの検査と内容の表示。

use super::cli::InspectArguments;
use minase::datagen::{data_error, hex, invalid_data};
use minase::training::records::{
    Entry, Exclusion, FORMAT_VERSION, GameRecord, Origin, Outcome, Policy, Reader, Value,
};
use minase_core::{Color, GameResult, Position, notation::usi, to_sfen};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Write},
};

/// 全項目の内訳と、価値学習に使う局面の集計。
#[derive(Default)]
struct InspectionSummary {
    game_origins: [u64; 4],
    origins: [u64; 4],
    exclusions: [u64; 5],
    evaluation_adjudication_games: u64,
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
            Outcome::Game(GameResult::Win { winner, .. })
            | Outcome::MateInThree { winner }
            | Outcome::EvaluationAdjudication { winner } => {
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
    inspect_to(arguments, &mut io::stdout().lock())
}

fn inspect_to(arguments: &InspectArguments, output: &mut impl Write) -> io::Result<()> {
    let mut reader = Reader::new(File::open(&arguments.path)?).map_err(data_error)?;
    let header = reader.header();
    writeln!(output, "header:")?;
    writeln!(output, "magic: MNSD")?;
    writeln!(output, "format_version: {FORMAT_VERSION}")?;
    writeln!(output, "rule_set: {}", header.rule_set())?;
    writeln!(output, "generation_commit: {}", header.generation_commit())?;
    writeln!(
        output,
        "network_checksum: {}",
        hex(header.network_checksum())
    )?;
    writeln!(output, "teacher_kind: {:?}", header.teacher_kind())?;
    writeln!(
        output,
        "teacher_search_budget: {}",
        header.teacher_search_budget()
    )?;
    writeln!(output, "seed: {}", header.seed())?;
    writeln!(output, "game_count: {}", header.game_count())?;
    writeln!(output, "entry_count: {}", header.entry_count())?;
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
        summary.evaluation_adjudication_games += u64::from(matches!(
            game.outcome(),
            Outcome::EvaluationAdjudication { .. }
        ));
        for (k, (entry, position)) in game.entries().iter().zip(decoded.positions()).enumerate() {
            if index <= arguments.dump {
                print_entry(k, game, entry, position, output)?;
            }
            summary.record(game, entry, position);
        }
    }
    writeln!(output, "summary:")?;
    writeln!(output, "games: {game_count}")?;
    writeln!(output, "entries: {entry_count}")?;
    writeln!(
        output,
        "games_evaluation_adjudication: {}",
        summary.evaluation_adjudication_games
    )?;
    for (code, count) in summary.game_origins.iter().enumerate() {
        writeln!(output, "games_origin_{code}: {count}")?;
    }
    for (code, name) in ["none", "mate_band", "tactical", "repetition", "check"]
        .iter()
        .enumerate()
    {
        writeln!(output, "exclusion_{name}: {}", summary.exclusions[code])?;
    }
    writeln!(output, "value_none: {}", summary.no_value)?;
    for (code, name) in ["none", "played", "candidates"].iter().enumerate() {
        writeln!(output, "policy_{name}: {}", summary.policies[code])?;
    }
    for (count, entries) in &summary.candidate_counts {
        writeln!(output, "policy_candidate_count_{count}: {entries}")?;
    }
    writeln!(output, "eligible_positions: {}", summary.eligible_positions)?;
    for (code, count) in summary.origins.iter().enumerate() {
        writeln!(output, "origin_{code}: {count}")?;
    }
    writeln!(output, "outcome_loss: {}", summary.losses)?;
    writeln!(output, "outcome_draw: {}", summary.draws)?;
    writeln!(output, "outcome_win: {}", summary.wins)?;
    match (summary.minimum_score, summary.maximum_score) {
        (Some(minimum), Some(maximum)) => {
            writeln!(output, "score_minimum: {minimum}")?;
            writeln!(output, "score_maximum: {maximum}")?;
            writeln!(
                output,
                "score_average: {:.6}",
                summary.score_sum as f64 / summary.eligible_positions as f64
            )?;
        }
        _ => {
            writeln!(output, "score_minimum: n/a")?;
            writeln!(output, "score_maximum: n/a")?;
            writeln!(output, "score_average: n/a")?;
        }
    }
    writeln!(output, "black_to_move: {}", summary.black_to_move)?;
    writeln!(output, "white_to_move: {}", summary.white_to_move)?;
    Ok(())
}

/// 復元した局面と、その局面の教師情報を指定順で表示する。
fn print_entry(
    k: usize,
    game: &GameRecord,
    entry: &Entry,
    position: &Position,
    output: &mut impl Write,
) -> io::Result<()> {
    writeln!(output, "entry: {} {k}", game.game_number())?;
    writeln!(output, "sfen: {}", to_sfen(position))?;
    writeln!(output, "side_to_move: {:?}", position.side_to_move())?;
    match position.lion_capture_square() {
        Some(square) => writeln!(output, "lion_square: {}", square.dense_index())?,
        None => writeln!(output, "lion_square: none")?,
    }
    writeln!(
        output,
        "lion_by_kirin_promotion: {}",
        position
            .lion_taken_by_non_lion()
            .is_some_and(|t| t.by_kirin_promotion)
    )?;
    match entry.value() {
        Value::None => writeln!(output, "value: none")?,
        Value::AlphaBeta(value) => writeln!(output, "value: {value}")?,
        Value::Mcts(value) => writeln!(output, "value: {value}")?,
    }
    writeln!(
        output,
        "exclusion: {}",
        match entry.exclusion() {
            Exclusion::None => "none",
            Exclusion::MateBand => "mate_band",
            Exclusion::Tactical => "tactical",
            Exclusion::Repetition => "repetition",
            Exclusion::Check => "check",
        }
    )?;
    match entry.policy() {
        Policy::None => writeln!(output, "policy: none")?,
        Policy::PlayedMove => writeln!(output, "policy: played")?,
        Policy::Candidates(candidates) => {
            write!(output, "policy: {}", candidates.len())?;
            for (mv, probability) in candidates {
                write!(
                    output,
                    " {}:{probability}",
                    usi::text_generated(position, *mv)
                )?;
            }
            writeln!(output)?;
        }
    }
    if let Some(visits) = entry.visits() {
        writeln!(output, "root_visits: {visits}")?;
    }
    match entry.played_move() {
        Some(mv) => writeln!(output, "played_move: {}", usi::text_generated(position, mv))?,
        None => writeln!(output, "played_move: none")?,
    }
    let outcome = match game.outcome() {
        Outcome::NotFromGame => "none",
        Outcome::Game(GameResult::Draw { .. }) => "Draw",
        Outcome::Game(GameResult::Win { winner, .. })
        | Outcome::MateInThree { winner }
        | Outcome::EvaluationAdjudication { winner }
            if winner == position.side_to_move() =>
        {
            "Win"
        }
        Outcome::Game(GameResult::Win { .. })
        | Outcome::MateInThree { .. }
        | Outcome::EvaluationAdjudication { .. } => "Loss",
    };
    writeln!(output, "outcome: {outcome}")?;
    writeln!(output, "game_number: {}", game.game_number())?;
    writeln!(output, "ply: {}", usize::from(game.start_ply()) + k)?;
    writeln!(output, "origin: {:?}", game.origin())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selfplay_gen::test_support::TemporaryDirectory;
    use minase::training::records::{Header, TeacherKind, Writer};
    use minase_core::{Game, Rules};

    #[test]
    fn inspect_reports_check_exclusions_and_evaluation_adjudication_for_both_winners() {
        let directory = TemporaryDirectory::new();
        let path = directory.0.join("teacher.mnsd");
        let header = Header::new(
            "lishogi".into(),
            "a".repeat(40),
            [0; 32],
            TeacherKind::AlphaBeta,
            100,
            7,
        )
        .unwrap();
        let mut writer = Writer::new(File::create(&path).unwrap(), header).unwrap();
        for (number, winner) in Color::ALL.into_iter().enumerate() {
            let mut game = Game::new(Rules::LISHOGI);
            let mut entries = Vec::new();
            for exclusion in [Exclusion::Check, Exclusion::None] {
                let mv = game.legal_moves()[0];
                entries.push(
                    Entry::new(
                        Some(mv),
                        Value::AlphaBeta(100),
                        exclusion,
                        Policy::PlayedMove,
                        None,
                    )
                    .unwrap(),
                );
                game.play(mv).unwrap();
            }
            entries
                .push(Entry::new(None, Value::None, Exclusion::None, Policy::None, None).unwrap());
            writer
                .write_game(
                    &GameRecord::new(
                        number as u32,
                        Origin::RandomSelfplay,
                        Outcome::EvaluationAdjudication { winner },
                        0,
                        Position::initial(),
                        entries,
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        writer.finish().unwrap();
        let mut output = Vec::new();
        inspect_to(&InspectArguments { path, dump: 2 }, &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        for line in [
            "format_version: 3",
            "games_evaluation_adjudication: 2",
            "exclusion_check: 2",
            "eligible_positions: 2",
            "outcome_loss: 1",
            "outcome_win: 1",
            "exclusion: check",
        ] {
            assert!(
                text.lines().any(|actual| actual == line),
                "missing {line}: {text}"
            );
        }
    }
}
