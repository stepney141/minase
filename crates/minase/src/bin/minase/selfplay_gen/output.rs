//! NNUE教師生成器の塊の確定、再開、および本への組み直し。
//!
//! 出力先には、生成条件を持つ`config.json`、塊ごとの`chunk-00000.mnsd`と
//! `chunk-00000.json`、全塊を組み直した`teacher.mnsd`、処理中だけ保持する
//! `.selfplay_generate.lock`を置く。
//! 一時ファイルは最終名に`.tmp`を付けた名前で作る。
//!
//! `config.json`は、規則セット、生成コミット、ネットのチェックサム、基本シード、
//! 総対局数、塊の局数、探索ノード数、ランダム着手の上限、手数上限、置換表の容量、
//! 開始局面一覧のSHA-256、および未コミットの変更の有無を持つ。
//! 同時対局数と入力ファイルのパスは保存しない。
//! 塊の集計JSONは、塊番号、先頭の対局番号、および[`Statistics`]の全欄を持つ。
//!
//! 塊は、MNSDの件数を確定して書き込みを完了し、ファイルを同期して最終名へ改名し、
//! ディレクトリを同期した後に集計JSONを原子的に保存して確定する。
//! 再開時は集計JSONがある塊を再利用し、欠けた塊の対局だけを番号順に生成する。
//! 全塊が確定した後は各MNSDを並列に復号し、ヘッダ、対局番号の範囲、および局数を
//! 検査してから、本のヘッダと塊の本体を番号順に連結する。
//!
//! 要約の加算可能な欄は塊の集計から求める。
//! 得点分布、保存した対局の除外件数、および重複局面数は、組み直し前の復号から求める。
//! 毎秒の欄は、その実行で生成して確定した塊の局面数だけを実行時間で割る。

use super::{
    opening::Opening,
    play::{PlaySettings, play_game, play_game_from_opening},
    statistics::{format_rate_percent, score_mean_and_std, score_percentile},
};
use minase::datagen::output::{DirectoryLock, atomic_json, commit_files};
use minase::datagen::statistics::{RecordedStatistics, SCORE_VALUE_COUNT, Statistics, score_index};
use minase::datagen::{data_error, hex};
use minase::eval::Nnue;
use minase::search::{TranspositionTable, TranspositionTableError};
use minase::training::records::{
    self, Exclusion, HEADER_LEN, Header, Reader, TeacherKind, Value, Writer,
};
use minase_core::{Rules, repetition_key};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fmt,
    fs::{self, File},
    io::{self, BufReader, BufWriter, Seek, SeekFrom, Write},
    num::{NonZeroU16, NonZeroU32, NonZeroUsize},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const LOCK_NAME: &str = ".selfplay_generate.lock";

/// 再開時に保存して照合する生成条件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Config {
    rules: String,
    generation_commit: String,
    network_checksum: String,
    seed: u64,
    games: NonZeroU32,
    chunk_games: NonZeroU32,
    nodes: NonZeroU32,
    random_moves: u8,
    max_ply: NonZeroU16,
    hash_mb: NonZeroUsize,
    openings_sha256: Option<String>,
    dirty: bool,
}

impl Config {
    fn differences(&self, saved: &Self) -> Vec<String> {
        [
            (self.rules != saved.rules, "rules"),
            (
                self.generation_commit != saved.generation_commit,
                "generation_commit",
            ),
            (
                self.network_checksum != saved.network_checksum,
                "network_checksum",
            ),
            (self.seed != saved.seed, "seed"),
            (self.games != saved.games, "games"),
            (self.chunk_games != saved.chunk_games, "chunk_games"),
            (self.nodes != saved.nodes, "nodes"),
            (self.random_moves != saved.random_moves, "random_moves"),
            (self.max_ply != saved.max_ply, "max_ply"),
            (self.hash_mb != saved.hash_mb, "hash_mb"),
            (
                self.openings_sha256 != saved.openings_sha256,
                "openings_sha256",
            ),
        ]
        .into_iter()
        .filter(|(different, _)| *different)
        .map(|(_, name)| name.to_owned())
        .collect()
    }

    fn chunk_count(&self) -> u32 {
        self.games.get().div_ceil(self.chunk_games.get())
    }

    fn game_range(&self, chunk: u32) -> std::ops::Range<u64> {
        let first = u64::from(chunk) * u64::from(self.chunk_games.get()) + 1;
        first..(first + u64::from(self.chunk_games.get())).min(u64::from(self.games.get()) + 1)
    }
}

/// 外側で確定した生成条件とMNSDヘッダ。
#[derive(Clone)]
pub(super) struct GenerationConditions {
    config: Config,
    rules: Rules,
    header: Header,
}

impl GenerationConditions {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        rules: Rules,
        generation_commit: String,
        network_checksum: [u8; 32],
        seed: u64,
        games: NonZeroU32,
        chunk_games: NonZeroU32,
        nodes: NonZeroU32,
        random_moves: u8,
        max_ply: NonZeroU16,
        hash_mb: NonZeroUsize,
        openings_sha256: Option<String>,
        dirty: bool,
    ) -> Result<Self, OutputError> {
        let rule_set = rules.to_string();
        let header = Header::new(
            rule_set.clone(),
            generation_commit.clone(),
            network_checksum,
            TeacherKind::AlphaBeta,
            u64::from(nodes.get()),
            seed,
        )?;
        Ok(Self {
            config: Config {
                rules: rule_set,
                generation_commit,
                network_checksum: hex(&network_checksum),
                seed,
                games,
                chunk_games,
                nodes,
                random_moves,
                max_ply,
                hash_mb,
                openings_sha256,
                dirty,
            },
            rules,
            header,
        })
    }
}

/// 出力先の状態、生成結果、または外部入力が契約に違反した原因。
#[derive(Debug)]
pub(super) enum OutputError {
    NonemptyDirectory(PathBuf),
    Locked {
        path: PathBuf,
        source: io::Error,
    },
    ConfigMismatch(Vec<String>),
    DirtyResume,
    OpeningCountMismatch {
        expected: u32,
        actual: usize,
    },
    OpeningRandomMoves,
    MissingChunkFile {
        chunk: u32,
        path: PathBuf,
    },
    HeaderMismatch {
        chunk: u32,
    },
    GameOutOfRange {
        chunk: u32,
        game: u32,
        first: u64,
        end: u64,
    },
    ChunkGameCountMismatch {
        chunk: u32,
        saved: u64,
        discarded: u64,
        expected: u64,
    },
    UnexpectedGame(u32),
    IncompleteChunks,
    WorkerPanic(u32),
    DecoderPanic(usize),
    StatisticsOverflow,
    TranspositionTable(TranspositionTableError),
    Records(records::Error),
    Json(serde_json::Error),
    Io(io::Error),
}

impl fmt::Display for OutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonemptyDirectory(path) => write!(
                f,
                "output directory is not empty and has no config.json: {}",
                path.display()
            ),
            Self::Locked { path, source } => write!(
                f,
                "cannot lock output directory {}: {source}",
                path.display()
            ),
            Self::ConfigMismatch(fields) => {
                write!(f, "generation conditions differ: {}", fields.join(", "))
            }
            Self::DirtyResume => f.write_str(
                "cannot resume committed chunks when either generation has uncommitted changes",
            ),
            Self::OpeningCountMismatch { expected, actual } => write!(
                f,
                "opening count differs from games: expected {expected}, got {actual}"
            ),
            Self::OpeningRandomMoves => {
                f.write_str("opening-list generation requires random_moves 0")
            }
            Self::MissingChunkFile { chunk, path } => write!(
                f,
                "committed chunk {chunk} is missing MNSD file {}",
                path.display()
            ),
            Self::HeaderMismatch { chunk } => {
                write!(
                    f,
                    "chunk {chunk} MNSD header differs from generation conditions"
                )
            }
            Self::GameOutOfRange {
                chunk,
                game,
                first,
                end,
            } => write!(
                f,
                "chunk {chunk} contains game {game} outside {first}..{end}"
            ),
            Self::ChunkGameCountMismatch {
                chunk,
                saved,
                discarded,
                expected,
            } => write!(
                f,
                "chunk {chunk} has {saved} saved and {discarded} discarded games, expected {expected} total"
            ),
            Self::UnexpectedGame(game) => {
                write!(f, "unexpected or duplicate completed game: {game}")
            }
            Self::IncompleteChunks => {
                f.write_str("workers finished before all chunks were committed")
            }
            Self::WorkerPanic(game) => write!(f, "self-play worker panicked in game {game}"),
            Self::DecoderPanic(worker) => {
                write!(f, "chunk decoder worker {worker} panicked")
            }
            Self::StatisticsOverflow => f.write_str("generation statistics count overflow"),
            Self::TranspositionTable(error) => error.fmt(f),
            Self::Records(error) => error.fmt(f),
            Self::Json(error) => write!(f, "invalid JSON: {error}"),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for OutputError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Locked { source, .. } | Self::Io(source) => Some(source),
            Self::TranspositionTable(source) => Some(source),
            Self::Records(source) => Some(source),
            Self::Json(source) => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for OutputError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for OutputError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<records::Error> for OutputError {
    fn from(error: records::Error) -> Self {
        Self::Records(error)
    }
}

impl From<TranspositionTableError> for OutputError {
    fn from(error: TranspositionTableError) -> Self {
        Self::TranspositionTable(error)
    }
}

impl From<minase::datagen::output::Error> for OutputError {
    fn from(error: minase::datagen::output::Error) -> Self {
        match error {
            minase::datagen::output::Error::Locked { path, source } => {
                Self::Locked { path, source }
            }
            minase::datagen::output::Error::Json(error) => Self::Json(error),
            minase::datagen::output::Error::Io(error) => Self::Io(error),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ChunkSummary {
    chunk: u32,
    first_game: u32,
    #[serde(flatten)]
    statistics: Statistics,
}

/// 1回の起動で得た本全体の集計と、その起動だけの進捗。
pub(super) struct RunReport {
    pub(super) statistics: Statistics,
    pub(super) recorded: RecordedStatistics,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) generated_games: u64,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) committed_games: u64,
    pub(super) chunks_reused: u32,
    generated_statistics: Statistics,
    elapsed: Duration,
}

impl RunReport {
    pub(super) fn print(&self, conditions: &GenerationConditions, concurrency: NonZeroUsize) {
        let config = &conditions.config;
        let stats = &self.statistics;
        let recorded = &self.recorded;
        let games = f64::from(config.games.get());
        let elapsed_seconds = self.elapsed.as_secs_f64();
        let per_second = |count: u64| {
            if elapsed_seconds == 0.0 {
                0.0
            } else {
                count as f64 / elapsed_seconds
            }
        };

        println!("summary:");
        println!("seed: {}", config.seed);
        println!("games: {}", config.games);
        println!("nodes: {}", config.nodes);
        println!("concurrency: {concurrency}");
        println!("max_ply: {}", config.max_ply);
        println!("hash_mb: {}", config.hash_mb);
        println!("random_moves_max: {}", config.random_moves);
        println!("rules: {}", config.rules);
        println!("commit: {}", config.generation_commit);
        println!("games_completed: {}", config.games);
        println!("games_discarded_max_ply: {}", stats.discarded_games);
        println!("win_reason_royal_capture: {}", stats.royal_capture_wins);
        println!("win_reason_mate: {}", stats.mate_wins);
        println!("win_reason_stalemate: {}", stats.stalemate_wins);
        println!("win_reason_repetition: {}", stats.repetition_wins);
        println!(
            "win_reason_piece_exhaustion: {}",
            stats.piece_exhaustion_wins
        );
        println!("win_reason_bare_king: {}", stats.bare_king_wins);
        println!("win_reason_resignation: {}", stats.resignation_wins);
        println!("draw_reason_repetition: {}", stats.repetition_draws);
        println!(
            "draw_reason_piece_exhaustion: {}",
            stats.piece_exhaustion_draws
        );
        println!("draw_reason_bare_king: {}", stats.bare_king_draws);
        println!("draw_reason_agreement: {}", stats.agreement_draws);
        println!("black_wins: {}", stats.black_wins);
        println!("white_wins: {}", stats.white_wins);
        println!("draws: {}", stats.draws);
        println!("injections_planned: {}", stats.planned_injections);
        println!("injections_performed: {}", stats.performed_injections);
        for (index, count) in stats.injection_offset_histogram.iter().enumerate() {
            let first = index * 10;
            println!("injection_offset_histogram_{first}_{}: {count}", first + 9);
        }
        println!("searched_positions: {}", stats.searched_positions);
        println!("recordable_positions: {}", stats.recordable_positions);
        println!("recorded_positions: {}", stats.recorded_positions);
        println!("excluded_mate_band: {}", stats.excluded_mate_band);
        println!(
            "excluded_mate_band_rate_percent: {}",
            format_rate_percent(stats.excluded_mate_band, stats.recordable_positions)
        );
        println!("excluded_tactical: {}", stats.excluded_tactical);
        println!(
            "excluded_tactical_rate_percent: {}",
            format_rate_percent(stats.excluded_tactical, stats.recordable_positions)
        );
        println!("excluded_repetition: {}", stats.excluded_repetition);
        println!(
            "excluded_repetition_rate_percent: {}",
            format_rate_percent(stats.excluded_repetition, stats.recordable_positions)
        );
        println!(
            "excluded_mate_band_in_saved_games: {}",
            recorded.exclusions[1]
        );
        println!(
            "excluded_tactical_in_saved_games: {}",
            recorded.exclusions[2]
        );
        println!(
            "excluded_repetition_in_saved_games: {}",
            recorded.exclusions[3]
        );
        match score_mean_and_std(&recorded.score_frequencies) {
            Some((mean, standard_deviation)) => {
                println!("score_mean: {mean:.6}");
                println!("score_std: {standard_deviation:.6}");
                for percentile in [1, 5, 25, 50, 75, 95, 99] {
                    match score_percentile(&recorded.score_frequencies, percentile) {
                        Some(score) => println!("score_p{percentile:02}: {score}"),
                        None => println!("score_p{percentile:02}: n/a"),
                    }
                }
            }
            None => {
                println!("score_mean: n/a");
                println!("score_std: n/a");
                for percentile in [1, 5, 25, 50, 75, 95, 99] {
                    println!("score_p{percentile:02}: n/a");
                }
            }
        }
        println!("duplicate_positions: {}", recorded.duplicate_positions);
        if stats.recorded_positions == 0 {
            println!("duplicate_rate_percent: n/a");
        } else {
            println!(
                "duplicate_rate_percent: {:.6}",
                recorded.duplicate_positions as f64 * 100.0 / stats.recorded_positions as f64
            );
        }
        println!("average_total_ply: {:.6}", stats.total_plies as f64 / games);
        println!(
            "average_searched_ply: {:.6}",
            stats.searched_plies as f64 / games
        );
        println!("elapsed_seconds: {elapsed_seconds:.6}");
        println!(
            "searched_positions_per_second: {:.6}",
            per_second(self.generated_statistics.searched_positions)
        );
        println!(
            "recorded_positions_per_second: {:.6}",
            per_second(self.generated_statistics.recorded_positions)
        );
        println!("searched_nodes_total: {}", stats.searched_nodes);
        println!("chunks_reused: {}", self.chunks_reused);
    }
}

/// 確定済みの塊を再利用し、欠けた塊を生成して本を組み直す。
pub(super) fn generate_to_directory(
    directory: &Path,
    conditions: &GenerationConditions,
    nnue: &Nnue,
    openings: Option<&[Opening]>,
    concurrency: NonZeroUsize,
) -> io::Result<RunReport> {
    generate_impl(directory, conditions, nnue, openings, concurrency).map_err(data_error)
}

fn generate_impl(
    directory: &Path,
    conditions: &GenerationConditions,
    nnue: &Nnue,
    openings: Option<&[Opening]>,
    concurrency: NonZeroUsize,
) -> Result<RunReport, OutputError> {
    if let Some(openings) = openings {
        let expected = usize::try_from(conditions.config.games.get())
            .map_err(|_| OutputError::StatisticsOverflow)?;
        if openings.len() != expected {
            return Err(OutputError::OpeningCountMismatch {
                expected: conditions.config.games.get(),
                actual: openings.len(),
            });
        }
        if conditions.config.random_moves != 0 {
            return Err(OutputError::OpeningRandomMoves);
        }
    }
    let started = Instant::now();
    let (_lock, mut completed) = open_output(directory, conditions)?;
    let chunks_reused =
        u32::try_from(completed.len()).map_err(|_| OutputError::StatisticsOverflow)?;
    let generated = if completed.len() == conditions.config.chunk_count() as usize {
        GeneratedRun::default()
    } else {
        generate_chunks(
            directory,
            conditions,
            nnue,
            openings,
            concurrency,
            &mut completed,
            started,
        )?
    };
    if completed.len() != conditions.config.chunk_count() as usize {
        return Err(OutputError::IncompleteChunks);
    }
    let (statistics, recorded) = reassemble(directory, conditions, &completed, concurrency)?;
    Ok(RunReport {
        statistics,
        recorded,
        generated_games: generated.finished_games,
        committed_games: generated.committed_games,
        chunks_reused,
        generated_statistics: generated.statistics,
        elapsed: started.elapsed(),
    })
}

fn open_output(
    directory: &Path,
    conditions: &GenerationConditions,
) -> Result<(DirectoryLock, BTreeMap<u32, ChunkSummary>), OutputError> {
    if !directory.try_exists()? {
        fs::create_dir(directory)?;
    }
    let lock = DirectoryLock::acquire(directory, LOCK_NAME)?;
    let config_path = directory.join("config.json");
    let saved = if config_path.try_exists()? {
        let saved: Config = serde_json::from_reader(BufReader::new(File::open(&config_path)?))?;
        let differences = conditions.config.differences(&saved);
        if !differences.is_empty() {
            return Err(OutputError::ConfigMismatch(differences));
        }
        Some(saved)
    } else {
        for entry in fs::read_dir(directory)? {
            let name = entry?.file_name();
            if name != LOCK_NAME && name != "config.json.tmp" {
                return Err(OutputError::NonemptyDirectory(directory.to_owned()));
            }
        }
        atomic_json(directory, "config.json", &conditions.config)?;
        None
    };

    let mut completed = BTreeMap::new();
    for chunk in 0..conditions.config.chunk_count() {
        let summary_path = directory.join(chunk_summary_name(chunk));
        if !summary_path.try_exists()? {
            continue;
        }
        let mnsd_path = directory.join(chunk_mnsd_name(chunk));
        if !mnsd_path.try_exists()? {
            return Err(OutputError::MissingChunkFile {
                chunk,
                path: mnsd_path,
            });
        }
        let summary: ChunkSummary =
            serde_json::from_reader(BufReader::new(File::open(summary_path)?))?;
        completed.insert(chunk, summary);
    }

    if let Some(saved) = saved
        && (saved.dirty || conditions.config.dirty)
    {
        if completed.is_empty() {
            atomic_json(directory, "config.json", &conditions.config)?;
        } else {
            return Err(OutputError::DirtyResume);
        }
    }
    Ok((lock, completed))
}

#[derive(Default)]
struct GeneratedRun {
    finished_games: u64,
    committed_games: u64,
    statistics: Statistics,
}

struct StopWorkersOnDrop<'a>(&'a AtomicBool);

impl Drop for StopWorkersOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[allow(clippy::too_many_arguments)]
fn generate_chunks(
    directory: &Path,
    conditions: &GenerationConditions,
    nnue: &Nnue,
    openings: Option<&[Opening]>,
    concurrency: NonZeroUsize,
    completed: &mut BTreeMap<u32, ChunkSummary>,
    started: Instant,
) -> Result<GeneratedRun, OutputError> {
    let numbers = (0..conditions.config.chunk_count())
        .filter(|chunk| !completed.contains_key(chunk))
        .flat_map(|chunk| {
            conditions
                .config
                .game_range(chunk)
                .map(|number| number as u32)
        })
        .collect::<Vec<_>>();
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let (sender, receiver) =
        mpsc::channel::<Result<minase::datagen::game::CompletedGame, OutputError>>();
    let play_settings = PlaySettings {
        base_seed: conditions.config.seed,
        nodes: conditions.config.nodes.get(),
        random_moves: conditions.config.random_moves,
        max_ply: conditions.config.max_ply.get(),
    };
    let result = thread::scope(|scope| -> Result<GeneratedRun, OutputError> {
        for _ in 0..concurrency.get() {
            let sender = sender.clone();
            let next = &next;
            let stop = &stop;
            let numbers = &numbers;
            scope.spawn(move || {
                let mut table = match TranspositionTable::new(conditions.config.hash_mb.get()) {
                    Ok(table) => table,
                    Err(error) => {
                        let _ = sender.send(Err(error.into()));
                        return;
                    }
                };
                loop {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&game_number) = numbers.get(index) else {
                        break;
                    };
                    let played =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match openings {
                            Some(items) => {
                                let opening = game_number
                                    .checked_sub(1)
                                    .and_then(|number| usize::try_from(number).ok())
                                    .and_then(|index| items.get(index))
                                    .ok_or(OutputError::UnexpectedGame(game_number))?;
                                play_game_from_opening(
                                    nnue,
                                    conditions.rules,
                                    game_number,
                                    play_settings,
                                    &mut table,
                                    Some(opening),
                                )
                                .map_err(OutputError::from)
                            }
                            None => play_game(
                                nnue,
                                conditions.rules,
                                game_number,
                                play_settings,
                                &mut table,
                            )
                            .map_err(OutputError::from),
                        }));
                    let message = match played {
                        Ok(result) => result,
                        Err(_) => Err(OutputError::WorkerPanic(game_number)),
                    };
                    let failed = message.is_err();
                    if sender.send(message).is_err() || failed {
                        break;
                    }
                }
            });
        }
        drop(sender);
        let _stop_workers = StopWorkersOnDrop(&stop);

        let mut active = BTreeMap::new();
        let mut generated = GeneratedRun::default();
        let mut last_progress = started;
        for message in receiver {
            let game = match message {
                Ok(game) => game,
                Err(error) => {
                    stop.store(true, Ordering::Release);
                    return Err(error);
                }
            };
            generated.finished_games = generated
                .finished_games
                .checked_add(1)
                .ok_or(OutputError::StatisticsOverflow)?;
            let chunk = game
                .game_number
                .checked_sub(1)
                .ok_or(OutputError::UnexpectedGame(game.game_number))?
                / conditions.config.chunk_games.get();
            if chunk >= conditions.config.chunk_count() || completed.contains_key(&chunk) {
                stop.store(true, Ordering::Release);
                return Err(OutputError::UnexpectedGame(game.game_number));
            }
            if let std::collections::btree_map::Entry::Vacant(entry) = active.entry(chunk) {
                entry.insert(ChunkWriter::new(directory, conditions, chunk)?);
            }
            let game_number = game.game_number;
            let complete = active
                .get_mut(&chunk)
                .ok_or(OutputError::UnexpectedGame(game_number))?
                .push(game)?;
            if complete {
                let writer = active
                    .remove(&chunk)
                    .ok_or(OutputError::UnexpectedGame(game_number))?;
                let summary = writer.commit(directory)?;
                let range = conditions.config.game_range(chunk);
                let chunk_games = range.end - range.start;
                generated.committed_games = generated
                    .committed_games
                    .checked_add(chunk_games)
                    .ok_or(OutputError::StatisticsOverflow)?;
                if !generated.statistics.checked_merge(&summary.statistics) {
                    return Err(OutputError::StatisticsOverflow);
                }
                completed.insert(chunk, summary);
                print_progress(
                    started,
                    generated.finished_games,
                    generated.committed_games,
                    completed.len(),
                    conditions.config.chunk_count(),
                )?;
                last_progress = Instant::now();
            } else if last_progress.elapsed() >= Duration::from_secs(10) {
                print_progress(
                    started,
                    generated.finished_games,
                    generated.committed_games,
                    completed.len(),
                    conditions.config.chunk_count(),
                )?;
                last_progress = Instant::now();
            }
        }
        if completed.len() != conditions.config.chunk_count() as usize {
            return Err(OutputError::IncompleteChunks);
        }
        Ok(generated)
    });
    if result.is_err() {
        cleanup_chunk_temporaries(directory, conditions.config.chunk_count())?;
    }
    result
}

fn print_progress(
    started: Instant,
    finished_games: u64,
    committed_games: u64,
    completed_chunks: usize,
    chunk_count: u32,
) -> Result<(), OutputError> {
    writeln!(
        io::stderr().lock(),
        "progress: elapsed_seconds={:.3} finished_games={finished_games} committed_games={committed_games} chunks={completed_chunks}/{chunk_count}",
        started.elapsed().as_secs_f64()
    )?;
    Ok(())
}

fn cleanup_chunk_temporaries(directory: &Path, chunk_count: u32) -> Result<(), OutputError> {
    for chunk in 0..chunk_count {
        let path = directory.join(format!("{}.tmp", chunk_mnsd_name(chunk)));
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

struct ChunkWriter {
    summary: ChunkSummary,
    next: u64,
    end: u64,
    pending: BTreeMap<u32, minase::datagen::game::CompletedGame>,
    writer: Writer<BufWriter<File>>,
}

impl ChunkWriter {
    fn new(
        directory: &Path,
        conditions: &GenerationConditions,
        chunk: u32,
    ) -> Result<Self, OutputError> {
        let range = conditions.config.game_range(chunk);
        let first_game = u32::try_from(range.start).map_err(|_| OutputError::StatisticsOverflow)?;
        let temporary = directory.join(format!("{}.tmp", chunk_mnsd_name(chunk)));
        Ok(Self {
            summary: ChunkSummary {
                chunk,
                first_game,
                statistics: Statistics::default(),
            },
            next: range.start,
            end: range.end,
            pending: BTreeMap::new(),
            writer: Writer::new(
                BufWriter::new(File::create(temporary)?),
                conditions.header.clone(),
            )?,
        })
    }

    fn push(&mut self, game: minase::datagen::game::CompletedGame) -> Result<bool, OutputError> {
        let number = game.game_number;
        if u64::from(number) < self.next
            || u64::from(number) >= self.end
            || self.pending.contains_key(&number)
        {
            return Err(OutputError::UnexpectedGame(number));
        }
        self.pending.insert(number, game);
        while self.next < self.end {
            let number = u32::try_from(self.next).map_err(|_| OutputError::StatisticsOverflow)?;
            let Some(game) = self.pending.remove(&number) else {
                break;
            };
            if let Some(record) = &game.record {
                self.writer.write_game(record)?;
            }
            if !self.summary.statistics.checked_merge(&game.stats) {
                return Err(OutputError::StatisticsOverflow);
            }
            self.next += 1;
        }
        Ok(self.next == self.end)
    }

    fn commit(self, directory: &Path) -> Result<ChunkSummary, OutputError> {
        let chunk = self.summary.chunk;
        let mut writer = self.writer.finish()?;
        writer.flush()?;
        drop(writer);
        let temporary = directory.join(format!("{}.tmp", chunk_mnsd_name(chunk)));
        let final_path = directory.join(chunk_mnsd_name(chunk));
        commit_files(directory, &[(&temporary, &final_path)])?;
        atomic_json(directory, &chunk_summary_name(chunk), &self.summary)?;
        Ok(self.summary)
    }
}

struct DecodedChunk {
    game_count: u64,
    entry_count: u64,
}

struct DecodedWorker {
    chunks: Vec<DecodedChunk>,
    exclusions: [u64; 4],
    score_frequencies: Vec<u64>,
    search_keys: Vec<u64>,
}

impl DecodedWorker {
    fn new() -> Self {
        Self {
            chunks: Vec::new(),
            exclusions: [0; 4],
            score_frequencies: vec![0; SCORE_VALUE_COUNT],
            search_keys: Vec::new(),
        }
    }
}

fn reassemble(
    directory: &Path,
    conditions: &GenerationConditions,
    completed: &BTreeMap<u32, ChunkSummary>,
    concurrency: NonZeroUsize,
) -> Result<(Statistics, RecordedStatistics), OutputError> {
    let chunk_count = usize::try_from(conditions.config.chunk_count())
        .map_err(|_| OutputError::StatisticsOverflow)?;
    let worker_count = concurrency.get().min(chunk_count);
    let next_chunk = AtomicUsize::new(0);
    let decoded = thread::scope(|scope| -> Result<Vec<DecodedWorker>, OutputError> {
        let mut handles = Vec::with_capacity(worker_count);
        for worker in 0..worker_count {
            let next_chunk = &next_chunk;
            let handle = thread::Builder::new()
                .name(format!("selfplay-decode-{worker}"))
                .spawn_scoped(scope, move || -> Result<DecodedWorker, OutputError> {
                    let mut decoded = DecodedWorker::new();
                    loop {
                        let index = next_chunk.fetch_add(1, Ordering::Relaxed);
                        if index >= chunk_count {
                            break;
                        }
                        let chunk =
                            u32::try_from(index).map_err(|_| OutputError::StatisticsOverflow)?;
                        let summary = completed.get(&chunk).ok_or(OutputError::IncompleteChunks)?;
                        let path = directory.join(chunk_mnsd_name(chunk));
                        let range = conditions.config.game_range(chunk);
                        let counts = decode_chunk(
                            chunk,
                            &path,
                            summary,
                            &conditions.header,
                            range,
                            &mut decoded,
                        )?;
                        decoded.chunks.push(counts);
                    }
                    Ok(decoded)
                })?;
            handles.push((worker, handle));
        }
        let mut decoded = Vec::with_capacity(handles.len());
        for (worker, handle) in handles {
            match handle.join() {
                Ok(result) => decoded.push(result?),
                Err(_) => return Err(OutputError::DecoderPanic(worker)),
            }
        }
        Ok(decoded)
    })?;

    let mut statistics = Statistics::default();
    for chunk in 0..conditions.config.chunk_count() {
        let summary = completed.get(&chunk).ok_or(OutputError::IncompleteChunks)?;
        if !statistics.checked_merge(&summary.statistics) {
            return Err(OutputError::StatisticsOverflow);
        }
    }

    let mut game_count = 0_u64;
    let mut entry_count = 0_u64;
    let mut exclusions = [0_u64; 4];
    let mut score_frequencies = vec![0_u64; SCORE_VALUE_COUNT];
    let mut search_keys = HashSet::new();
    let mut recorded_positions = 0_u64;
    for worker in decoded {
        for chunk in worker.chunks {
            game_count = game_count
                .checked_add(chunk.game_count)
                .ok_or(OutputError::StatisticsOverflow)?;
            entry_count = entry_count
                .checked_add(chunk.entry_count)
                .ok_or(OutputError::StatisticsOverflow)?;
        }
        for (total, count) in exclusions.iter_mut().zip(worker.exclusions) {
            *total = total
                .checked_add(count)
                .ok_or(OutputError::StatisticsOverflow)?;
        }
        for (total, count) in score_frequencies.iter_mut().zip(worker.score_frequencies) {
            *total = total
                .checked_add(count)
                .ok_or(OutputError::StatisticsOverflow)?;
        }
        for key in worker.search_keys {
            recorded_positions = recorded_positions
                .checked_add(1)
                .ok_or(OutputError::StatisticsOverflow)?;
            search_keys.insert(key);
        }
    }
    let unique_positions =
        u64::try_from(search_keys.len()).map_err(|_| OutputError::StatisticsOverflow)?;
    let duplicate_positions = recorded_positions
        .checked_sub(unique_positions)
        .ok_or(OutputError::StatisticsOverflow)?;
    write_teacher(directory, conditions, game_count, entry_count)?;
    Ok((
        statistics,
        RecordedStatistics {
            exclusions,
            score_frequencies,
            search_keys,
            duplicate_positions,
        },
    ))
}

fn decode_chunk(
    chunk: u32,
    path: &Path,
    summary: &ChunkSummary,
    expected_header: &Header,
    range: std::ops::Range<u64>,
    worker: &mut DecodedWorker,
) -> Result<DecodedChunk, OutputError> {
    let mut reader = Reader::new(BufReader::new(File::open(path)?))?;
    let header = reader.header().clone();
    if header.encode_with_counts(0, 0) != expected_header.encode_with_counts(0, 0) {
        return Err(OutputError::HeaderMismatch { chunk });
    }
    let mut game_count = 0_u64;
    while let Some(decoded) = reader.read_game()? {
        let game_number = decoded.record().game_number();
        if !range.contains(&u64::from(game_number)) {
            return Err(OutputError::GameOutOfRange {
                chunk,
                game: game_number,
                first: range.start,
                end: range.end,
            });
        }
        game_count = game_count
            .checked_add(1)
            .ok_or(OutputError::StatisticsOverflow)?;
        for (entry, position) in decoded.record().entries().iter().zip(decoded.positions()) {
            let exclusion = entry.exclusion();
            if exclusion != Exclusion::None {
                let count = worker
                    .exclusions
                    .get_mut(exclusion as usize)
                    .ok_or(OutputError::StatisticsOverflow)?;
                *count = count
                    .checked_add(1)
                    .ok_or(OutputError::StatisticsOverflow)?;
            }
            if let Value::AlphaBeta(score) = entry.value()
                && exclusion == Exclusion::None
            {
                let frequency = worker
                    .score_frequencies
                    .get_mut(score_index(score))
                    .ok_or(OutputError::StatisticsOverflow)?;
                *frequency = frequency
                    .checked_add(1)
                    .ok_or(OutputError::StatisticsOverflow)?;
                worker
                    .search_keys
                    .push(repetition_key(header.rules().repetition, position));
            }
        }
    }
    let expected_games = range.end - range.start;
    let actual_games = game_count
        .checked_add(summary.statistics.discarded_games)
        .ok_or(OutputError::ChunkGameCountMismatch {
            chunk,
            saved: game_count,
            discarded: summary.statistics.discarded_games,
            expected: expected_games,
        })?;
    if actual_games != expected_games {
        return Err(OutputError::ChunkGameCountMismatch {
            chunk,
            saved: game_count,
            discarded: summary.statistics.discarded_games,
            expected: expected_games,
        });
    }
    Ok(DecodedChunk {
        game_count,
        entry_count: header.entry_count(),
    })
}

fn write_teacher(
    directory: &Path,
    conditions: &GenerationConditions,
    game_count: u64,
    entry_count: u64,
) -> Result<(), OutputError> {
    let temporary = directory.join("teacher.mnsd.tmp");
    let final_path = directory.join("teacher.mnsd");
    let mut writer = BufWriter::new(File::create(&temporary)?);
    writer.write_all(
        &conditions
            .header
            .encode_with_counts(game_count, entry_count),
    )?;
    for chunk in 0..conditions.config.chunk_count() {
        let mut source = File::open(directory.join(chunk_mnsd_name(chunk)))?;
        source.seek(SeekFrom::Start(
            u64::try_from(HEADER_LEN).map_err(|_| OutputError::StatisticsOverflow)?,
        ))?;
        io::copy(&mut source, &mut writer)?;
    }
    writer.flush()?;
    drop(writer);
    commit_files(directory, &[(&temporary, &final_path)])?;
    Ok(())
}

fn chunk_mnsd_name(chunk: u32) -> String {
    format!("chunk-{chunk:05}.mnsd")
}

fn chunk_summary_name(chunk: u32) -> String {
    format!("chunk-{chunk:05}.json")
}

#[cfg(test)]
mod tests;
