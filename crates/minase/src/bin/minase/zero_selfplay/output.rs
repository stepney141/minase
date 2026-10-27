//! 出力先の排他、塊の順序付き保存、再開、および投了の較正結果の集計。

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::num::NonZeroUsize;
use std::path::Path;
use std::time::{Duration, Instant};

use fs2::FileExt;
use minase::nn::Evaluator;
use minase::training::records::{Header, TeacherKind, Writer};
use minase_core::{Color, DrawReason, GameResult, WinReason};
use serde::{Deserialize, Serialize};

use super::game::{CompletedGame, Settings, false_resignation};
use super::worker::Workers;
use super::{Error, FALSE_RESIGNATION_LIMIT};

/// 出力を決める生成条件。推論の並列数は再開時の照合に含めない。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Config {
    pub(super) rules: String,
    pub(super) generation_commit: String,
    pub(super) network_sha256: String,
    pub(super) seed: u64,
    pub(super) games: u32,
    pub(super) chunk_games: u32,
    pub(super) simulations: u32,
    pub(super) playthrough_percent: u8,
    pub(super) max_ply: u32,
}

impl Config {
    fn validate(&self) -> Result<(), Error> {
        for (invalid, field) in [
            (self.games == 0, "games"),
            (self.chunk_games == 0, "chunk_games"),
            (self.simulations == 0, "simulations"),
            (
                !(1..=100).contains(&self.playthrough_percent),
                "playthrough_percent",
            ),
            (
                !(1..=u32::from(u16::MAX)).contains(&self.max_ply),
                "max_ply",
            ),
        ] {
            if invalid {
                return Err(Error::InvalidSettings(field));
            }
        }
        Ok(())
    }

    fn differences(&self, saved: &Self) -> Vec<String> {
        [
            (self.rules != saved.rules, "rules"),
            (
                self.generation_commit != saved.generation_commit,
                "generation_commit",
            ),
            (
                self.network_sha256 != saved.network_sha256,
                "network_sha256",
            ),
            (self.seed != saved.seed, "seed"),
            (self.games != saved.games, "games"),
            (self.chunk_games != saved.chunk_games, "chunk_games"),
            (self.simulations != saved.simulations, "simulations"),
            (
                self.playthrough_percent != saved.playthrough_percent,
                "playthrough_percent",
            ),
            (self.max_ply != saved.max_ply, "max_ply"),
        ]
        .into_iter()
        .filter(|(different, _)| *different)
        .map(|(_, field)| field.to_owned())
        .collect()
    }

    fn chunk_count(&self) -> u32 {
        self.games.div_ceil(self.chunk_games)
    }

    fn game_range(&self, chunk: u32) -> std::ops::Range<u64> {
        let first = u64::from(chunk) * u64::from(self.chunk_games) + 1;
        first..(first + u64::from(self.chunk_games)).min(u64::from(self.games) + 1)
    }
}

/// 塊とサイクルの双方で加算する件数。局面数には破棄した対局を含めない。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Counts {
    pub(super) games: u64,
    pub(super) discarded_max_ply: u64,
    pub(super) resigned_games: u64,
    pub(super) games_file_games: u64,
    pub(super) positions: u64,
    pub(super) resigned_positions: u64,
    pub(super) playthrough_games: u64,
    pub(super) resign_triggered_playthrough_games: u64,
    pub(super) false_resignations: u64,
    pub(super) black_wins: u64,
    pub(super) white_wins: u64,
    pub(super) draws: u64,
    pub(super) win_reason_royal_capture: u64,
    pub(super) win_reason_repetition: u64,
    pub(super) win_reason_piece_exhaustion: u64,
    pub(super) win_reason_bare_king: u64,
    pub(super) win_reason_stalemate: u64,
    pub(super) win_reason_mate: u64,
    pub(super) win_reason_resignation: u64,
    pub(super) draw_reason_repetition: u64,
    pub(super) draw_reason_piece_exhaustion: u64,
    pub(super) draw_reason_bare_king: u64,
    pub(super) draw_reason_agreement: u64,
    pub(super) plies: u64,
}

impl Counts {
    fn add_game(&mut self, game: &CompletedGame) {
        self.games += 1;
        self.plies += u64::from(game.plies);
        self.playthrough_games += u64::from(game.playthrough);
        self.resign_triggered_playthrough_games +=
            u64::from(game.playthrough && game.first_trigger.is_some());
        self.false_resignations += u64::from(false_resignation(
            game.playthrough,
            game.first_trigger,
            game.result,
        ));
        if game.record.is_none() {
            self.discarded_max_ply += 1;
        } else if game.resigned {
            self.resigned_games += 1;
            self.resigned_positions += game.positions;
        } else {
            self.games_file_games += 1;
            self.positions += game.positions;
        }
        match game.result {
            Some(GameResult::Win { winner, reason }) => {
                match winner {
                    Color::Black => self.black_wins += 1,
                    Color::White => self.white_wins += 1,
                }
                match reason {
                    WinReason::RoyalCapture => self.win_reason_royal_capture += 1,
                    WinReason::Repetition => self.win_reason_repetition += 1,
                    WinReason::PieceExhaustion => self.win_reason_piece_exhaustion += 1,
                    WinReason::BareKing => self.win_reason_bare_king += 1,
                    WinReason::Stalemate => self.win_reason_stalemate += 1,
                    WinReason::Mate => self.win_reason_mate += 1,
                    WinReason::Resignation => self.win_reason_resignation += 1,
                }
            }
            Some(GameResult::Draw { reason }) => {
                self.draws += 1;
                match reason {
                    DrawReason::Repetition => self.draw_reason_repetition += 1,
                    DrawReason::PieceExhaustion => self.draw_reason_piece_exhaustion += 1,
                    DrawReason::BareKing => self.draw_reason_bare_king += 1,
                    DrawReason::Agreement => self.draw_reason_agreement += 1,
                }
            }
            None => {}
        }
    }

    fn add(&mut self, other: &Self) {
        self.games += other.games;
        self.discarded_max_ply += other.discarded_max_ply;
        self.resigned_games += other.resigned_games;
        self.games_file_games += other.games_file_games;
        self.positions += other.positions;
        self.resigned_positions += other.resigned_positions;
        self.playthrough_games += other.playthrough_games;
        self.resign_triggered_playthrough_games += other.resign_triggered_playthrough_games;
        self.false_resignations += other.false_resignations;
        self.black_wins += other.black_wins;
        self.white_wins += other.white_wins;
        self.draws += other.draws;
        self.win_reason_royal_capture += other.win_reason_royal_capture;
        self.win_reason_repetition += other.win_reason_repetition;
        self.win_reason_piece_exhaustion += other.win_reason_piece_exhaustion;
        self.win_reason_bare_king += other.win_reason_bare_king;
        self.win_reason_stalemate += other.win_reason_stalemate;
        self.win_reason_mate += other.win_reason_mate;
        self.win_reason_resignation += other.win_reason_resignation;
        self.draw_reason_repetition += other.draw_reason_repetition;
        self.draw_reason_piece_exhaustion += other.draw_reason_piece_exhaustion;
        self.draw_reason_bare_king += other.draw_reason_bare_king;
        self.draw_reason_agreement += other.draw_reason_agreement;
        self.plies += other.plies;
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ChunkSummary {
    pub(super) chunk: u32,
    pub(super) first_game: u32,
    #[serde(flatten)]
    pub(super) counts: Counts,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Summary {
    #[serde(flatten)]
    pub(super) counts: Counts,
    pub(super) false_resignation_rate: Option<f64>,
    pub(super) resigned_games_excluded: bool,
    pub(super) training_files: Vec<String>,
    pub(super) training_positions: u64,
}

/// 塊の集計だけから、サイクルの件数と学習ファイルを決める。
/// 誤投了率の分母が0なら率は未定義とし、投了ファイルを除外しない。
pub(super) fn summarize(chunks: &[ChunkSummary]) -> Summary {
    let mut counts = Counts::default();
    for chunk in chunks {
        counts.add(&chunk.counts);
    }
    let rate = (counts.resign_triggered_playthrough_games > 0).then(|| {
        counts.false_resignations as f64 / counts.resign_triggered_playthrough_games as f64
    });
    let excluded = rate.is_some_and(|rate| rate > FALSE_RESIGNATION_LIMIT);
    let mut ordered: Vec<_> = chunks.iter().collect();
    ordered.sort_unstable_by_key(|chunk| chunk.chunk);
    let mut files = Vec::new();
    for chunk in ordered {
        files.push(games_filename(chunk.chunk, false));
        if !excluded {
            files.push(games_filename(chunk.chunk, true));
        }
    }
    let training_positions = counts.positions
        + if excluded {
            0
        } else {
            counts.resigned_positions
        };
    Summary {
        counts,
        false_resignation_rate: rate,
        resigned_games_excluded: excluded,
        training_files: files,
        training_positions,
    }
}

pub(super) struct RunReport {
    pub(super) summary: Summary,
    pub(super) generated_games: u64,
    pub(super) generated_positions: u64,
    elapsed: Duration,
}

impl RunReport {
    pub(super) fn print(&self, config: &Config) -> Result<(), Error> {
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "seed: {}", config.seed)?;
        writeln!(stdout, "rules: {}", config.rules)?;
        writeln!(stdout, "commit: {}", config.generation_commit)?;
        writeln!(stdout, "network_sha256: {}", config.network_sha256)?;
        writeln!(stdout, "simulations: {}", config.simulations)?;
        writeln!(stdout, "max_ply: {}", config.max_ply)?;
        if let serde_json::Value::Object(fields) = serde_json::to_value(&self.summary)? {
            for (name, value) in fields {
                writeln!(stdout, "{name}: {value}")?;
            }
        }
        let seconds = self.elapsed.as_secs_f64();
        writeln!(stdout, "generated_games: {}", self.generated_games)?;
        writeln!(stdout, "generated_positions: {}", self.generated_positions)?;
        writeln!(stdout, "elapsed_seconds: {seconds:.6}")?;
        writeln!(
            stdout,
            "positions_per_second: {:.6}",
            self.generated_positions as f64 / seconds
        )?;
        Ok(())
    }
}

/// 確定済みの塊は読むだけにし、未確定の塊があるときだけ推論器を作る。
pub(super) fn generate(
    directory: &Path,
    config: &Config,
    checksum: &[u8; 32],
    threads: NonZeroUsize,
    games_per_thread: NonZeroUsize,
    mut make_evaluator: impl FnMut() -> Result<Box<dyn Evaluator + Send>, Error>,
) -> Result<RunReport, Error> {
    let started = Instant::now();
    config.validate()?;
    let header = Header::new(
        config.rules.clone(),
        config.generation_commit.clone(),
        *checksum,
        TeacherKind::Mcts,
        u64::from(config.simulations),
        config.seed,
    )?;
    let _lock = open_directory(directory, config)?;
    let mut completed = BTreeMap::new();
    let mut numbers = Vec::new();
    for chunk in 0..config.chunk_count() {
        let path = directory.join(format!("chunk-{chunk:05}.json"));
        if path.try_exists()? {
            // JSONの存在が確定の印であり、MNSDや保存済み条件の再検証はしない。
            let summary = serde_json::from_reader(BufReader::new(File::open(path)?))?;
            completed.insert(chunk, summary);
        } else {
            numbers.extend(config.game_range(chunk).map(|number| number as u32));
        }
    }
    let mut generated_games = 0;
    let mut generated_positions = 0;
    if !numbers.is_empty() {
        let mut evaluators = Vec::new();
        for _ in 0..threads.get() {
            evaluators.push(make_evaluator()?);
        }
        let settings = Settings {
            rules: header.rules(),
            seed: config.seed,
            simulations: config.simulations,
            playthrough_percent: config.playthrough_percent,
            max_ply: config.max_ply,
        };
        let mut workers = Workers::start(numbers, settings, games_per_thread, evaluators)?;
        let mut active = BTreeMap::new();
        while let Ok(game) = workers.results.recv() {
            let chunk = game
                .number
                .checked_sub(1)
                .ok_or(Error::UnexpectedGame(game.number))?
                / config.chunk_games;
            if chunk >= config.chunk_count() || completed.contains_key(&chunk) {
                return Err(Error::UnexpectedGame(game.number));
            }
            if let std::collections::btree_map::Entry::Vacant(entry) = active.entry(chunk) {
                entry.insert(ChunkWriter::new(directory, config, &header, chunk)?);
            }
            generated_games += 1;
            if game.record.is_some() {
                generated_positions += game.positions;
            }
            if active
                .get_mut(&chunk)
                .expect("the chunk writer was opened above")
                .push(game)?
            {
                let writer = active
                    .remove(&chunk)
                    .expect("the completed chunk has a writer");
                completed.insert(chunk, writer.commit(directory)?);
                writeln!(
                    io::stderr().lock(),
                    "chunks: {}/{} games: {generated_games} positions: {generated_positions} elapsed_seconds: {:.3}",
                    completed.len(),
                    config.chunk_count(),
                    started.elapsed().as_secs_f64()
                )?;
            }
        }
        workers.join()?;
    }
    if completed.len() != config.chunk_count() as usize {
        return Err(Error::IncompleteChunks);
    }
    let chunks: Vec<_> = completed.into_values().collect();
    let summary = summarize(&chunks);
    atomic_json(directory, "summary.json", &summary)?;
    Ok(RunReport {
        summary,
        generated_games,
        generated_positions,
        elapsed: started.elapsed(),
    })
}

/// ファイルの存在ではなく、OSの排他ロックを生存中保持する。
struct DirectoryLock(File);

impl Drop for DirectoryLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

fn open_directory(directory: &Path, config: &Config) -> Result<DirectoryLock, Error> {
    if !directory.try_exists()? {
        fs::create_dir(directory)?;
    }
    let config_path = directory.join("config.json");
    if !config_path.try_exists()? && fs::read_dir(directory)?.next().transpose()?.is_some() {
        return Err(Error::NonemptyDirectory(directory.to_owned()));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(".zero_selfplay.lock"))?;
    file.try_lock_exclusive().map_err(|source| Error::Locked {
        path: directory.to_owned(),
        source,
    })?;
    let lock = DirectoryLock(file);
    if config_path.try_exists()? {
        let saved: Config = serde_json::from_reader(BufReader::new(File::open(config_path)?))?;
        let differences = config.differences(&saved);
        if !differences.is_empty() {
            return Err(Error::ConfigMismatch(differences));
        }
    } else {
        atomic_json(directory, "config.json", config)?;
    }
    Ok(lock)
}

/// 次の番号まで連続して届いた局だけを書き、順序待ちの局だけを保持する。
struct ChunkWriter {
    summary: ChunkSummary,
    next: u64,
    end: u64,
    pending: BTreeMap<u32, CompletedGame>,
    games: Writer<BufWriter<File>>,
    resigned: Writer<BufWriter<File>>,
}

impl ChunkWriter {
    fn new(directory: &Path, config: &Config, header: &Header, chunk: u32) -> Result<Self, Error> {
        let range = config.game_range(chunk);
        let open = |resigned| -> Result<_, Error> {
            let path = directory.join(format!("{}.tmp", games_filename(chunk, resigned)));
            Ok(Writer::new(
                BufWriter::new(File::create(path)?),
                header.clone(),
            )?)
        };
        Ok(Self {
            summary: ChunkSummary {
                chunk,
                first_game: range.start as u32,
                counts: Counts::default(),
            },
            next: range.start,
            end: range.end,
            pending: BTreeMap::new(),
            games: open(false)?,
            resigned: open(true)?,
        })
    }

    fn push(&mut self, game: CompletedGame) -> Result<bool, Error> {
        let number = game.number;
        if u64::from(number) < self.next
            || u64::from(number) >= self.end
            || self.pending.contains_key(&number)
        {
            return Err(Error::UnexpectedGame(number));
        }
        self.pending.insert(number, game);
        while self.next < self.end {
            let Some(game) = self.pending.remove(&(self.next as u32)) else {
                break;
            };
            if let Some(record) = &game.record {
                if game.resigned {
                    self.resigned.write_game(record)?;
                } else {
                    self.games.write_game(record)?;
                }
            }
            self.summary.counts.add_game(&game);
            self.next += 1;
        }
        Ok(self.next == self.end)
    }

    fn commit(self, directory: &Path) -> Result<ChunkSummary, Error> {
        let mut games = self.games.finish()?;
        let mut resigned = self.resigned.finish()?;
        games.flush()?;
        resigned.flush()?;
        games.get_ref().sync_all()?;
        resigned.get_ref().sync_all()?;
        drop((games, resigned));
        for resigned in [false, true] {
            let name = games_filename(self.summary.chunk, resigned);
            fs::rename(directory.join(format!("{name}.tmp")), directory.join(name))?;
        }
        File::open(directory)?.sync_all()?;
        atomic_json(
            directory,
            &format!("chunk-{:05}.json", self.summary.chunk),
            &self.summary,
        )?;
        Ok(self.summary)
    }
}

fn games_filename(chunk: u32, resigned: bool) -> String {
    if resigned {
        format!("chunk-{chunk:05}.resigned.mnsd")
    } else {
        format!("chunk-{chunk:05}.mnsd")
    }
}

fn atomic_json(directory: &Path, name: &str, value: &impl Serialize) -> Result<(), Error> {
    let temporary = directory.join(format!("{name}.tmp"));
    let mut writer = BufWriter::new(File::create(&temporary)?);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    drop(writer);
    fs::rename(temporary, directory.join(name))?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests;
