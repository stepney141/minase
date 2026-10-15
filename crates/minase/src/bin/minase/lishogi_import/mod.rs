//! lishogiのゲームNDJSONを教師データまたは実戦開始の局面一覧へ変換する。
//!
//! # 開始局面一覧
//!
//! `data lishogi openings`はUTF-8で、各行を`<棋譜ID> <経過手数> <拡張SFEN>`としてLFで終える。
//! 棋譜IDは空白を含まない空でない文字列、経過手数は0起算のu32で65,535未満とする。
//! SFENは局面を文字列化した表記で、拡張SFENは[`minase_core::notation::sfen::to_extended_sfen`]に従い、
//! 次の着手番号を経過手数+1として先獅子の状態も保存する。
//! 見出し行、空行、コメント行は書かない。
//! 読み手の`selfplay_gen/opening.rs`の`read_openings`は全行を検査し、
//! 両側の王駒と合法手があり、成りを保留した駒がない局面だけを受け付ける。

mod cli;
mod error;
mod filter;
mod input;
mod label;
mod output;
mod replay;

pub(super) use cli::Arguments;
use cli::{Common, Operation};
use error::ImportError;
use filter::{Report, metadata_exclusion};
use input::time_control;
use label::{GameDetails, Job, process_job};
use minase::{
    datagen::{self, data_error},
    search::TranspositionTable,
    training::records::{Header, TeacherKind, Writer},
};
use output::{create, write_json};
use replay::replay;
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::{self, BufRead, BufReader, Write},
    num::NonZeroU64,
    thread,
};

fn run(common: &Common, generation: Option<(NonZeroU64, bool)>) -> io::Result<()> {
    let openings = generation.is_none();
    let mut report = Report::new(common.nodes.get());
    let mut seen = HashSet::new();
    let mut jobs = Vec::new();
    for (index, line) in BufReader::new(File::open(&common.input)?)
        .lines()
        .enumerate()
    {
        let input = input::parse_line(line, index + 1)?;
        if let Some(reason) = metadata_exclusion(&input, openings, &mut seen) {
            report.exclude(reason, input.id);
            continue;
        }
        match replay(&input, openings) {
            Err(reason) => report.exclude(reason, input.id),
            Ok(replay) => {
                if let Some(ply) = replay.truncated {
                    report.truncated_illegal.insert(input.id.clone(), ply);
                }
                report.accepted_ids.push(input.id.clone());
                let number = u32::try_from(jobs.len() + 1).map_err(data_error)?;
                jobs.push(Job { input, number });
            }
        }
    }
    report.accepted = jobs.len();
    let commit = datagen::git::git_output(&["rev-parse", "HEAD"])?;
    datagen::git::validate_commit_hash(&commit)?;
    if let Some((_, false)) = generation
        && !datagen::git::git_output(&["status", "--porcelain"])?.is_empty()
    {
        return Err(data_error(ImportError::DirtyTree));
    }
    // 作成したファイルだけを回収し、既存の成果物は上書きしない。
    let mut created = Vec::new();
    let result = (|| {
        let output = create(&common.output)?;
        created.push(common.output.clone());
        let (mut text_output, mut writer) = if let Some((seed, _)) = generation {
            let nnue = minase::eval::weights().map_err(data_error)?;
            // MNSDの規則セット名は selfplay_gen と同じく規則コードの列挙で書く。
            let header = Header::new(
                minase_core::Rules::from_codes(
                    &minase_core::rules::parse_rule_set("engine-default").map_err(data_error)?,
                )
                .map_err(data_error)?
                .to_string(),
                commit.clone(),
                *nnue.checksum(),
                TeacherKind::AlphaBeta,
                u64::from(common.nodes.get()),
                seed.get(),
            )
            .map_err(data_error)?;
            (None, Some(Writer::new(output, header).map_err(data_error)?))
        } else {
            (Some(output), None)
        };
        let mut details = BTreeMap::new();
        // バッチの大きさをワーカー数に抑え、順序待ちの局面を全対局分保持しない。
        for batch in jobs.chunks(common.concurrency.get()) {
            let results = thread::scope(|scope| {
                let handles = batch
                    .iter()
                    .map(|job| {
                        scope.spawn(move || {
                            let mut table = TranspositionTable::new(common.hash_mb.get())
                                .map_err(data_error)?;
                            process_job(job, common, openings, &mut table)
                        })
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .map_err(|_| data_error(ImportError::WorkerPanic))
                            .and_then(|r| r)
                    })
                    .collect::<Vec<_>>()
            });
            let mut completed = Vec::new();
            for (job, result) in batch.iter().zip(results) {
                let mut result = result?;
                if let Some(output) = &mut text_output {
                    report.candidates += result.candidates;
                    report.excluded_score += result.score_exclusions;
                    report.excluded_no_legal_moves += result.no_legal_moves;
                    report.retained += result.openings.len();
                    let mut plies = Vec::new();
                    for (ply, sfen) in result.openings {
                        writeln!(output, "{} {ply} {sfen}", job.input.id)?;
                        plies.push(ply);
                    }
                    report.retained_plies.insert(job.input.id.clone(), plies);
                }
                if !openings {
                    let stats = &result.completed.stats;
                    let players = job.input.players.as_ref();
                    details.insert(
                        job.input.id.clone(),
                        GameDetails {
                            game: job.number,
                            sente_rating: players
                                .and_then(|p| p.sente.as_ref())
                                .and_then(|p| p.rating),
                            gote_rating: players
                                .and_then(|p| p.gote.as_ref())
                                .and_then(|p| p.rating),
                            speed: time_control(&job.input),
                            clock: job.input.clock.clone(),
                            days_per_turn: job.input.days_per_turn.clone(),
                            status: job.input.status.clone(),
                            winner: job.input.winner.clone(),
                            plies: job.input.moves.split_whitespace().count(),
                            recorded_positions: result.completed.records.len(),
                            excluded_positions: BTreeMap::from([
                                ("mate_band", stats.excluded_mate_band),
                                ("tactical", stats.excluded_tactical),
                                ("repetition", stats.excluded_repetition),
                            ]),
                        },
                    );
                }
                // バッチ内の統合番号だけを詰める。GameRecordの対局番号は受理順のまま。
                result.completed.game_number =
                    u32::try_from(completed.len() + 1).map_err(data_error)?;
                completed.push(Ok(result.completed));
            }
            if let Some(writer) = &mut writer {
                datagen::game::merge_completed_games(
                    completed,
                    writer,
                    u32::try_from(batch.len()).map_err(data_error)?,
                    |_, _| {},
                )?;
            }
        }
        if let Some(writer) = writer {
            writer.finish().map_err(data_error)?.sync_all()?;
        }
        if let Some(mut output) = text_output {
            output.flush()?;
        }
        if !openings {
            report.games = Some(details);
        }
        write_json(&common.report, &report)?;
        println!("accepted: {}", report.accepted);
        for (reason, entry) in &report.excluded {
            println!(
                "{}: {}",
                serde_json::to_string(reason)
                    .map_err(data_error)?
                    .trim_matches('"'),
                entry.count
            );
        }
        if openings {
            println!(
                "candidates: {}\nexcluded_score: {}\nretained: {}",
                report.candidates, report.excluded_score, report.retained
            );
        }
        Ok(())
    })();
    if result.is_err() {
        for path in created {
            fs::remove_file(path)?;
        }
    }
    result
}

pub(super) fn main(arguments: Arguments) {
    let result = match arguments.command {
        Operation::Games {
            common,
            seed,
            allow_dirty,
        } => run(&common, Some((seed, allow_dirty))),
        Operation::Openings(common) => run(&common, None),
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
