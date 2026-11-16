//! 固定局数の塊を保存し、確定済みの塊からサイクルの集計を作る。
//!
//! `config.json`では規則、生成コミット、ネットの検査和、推論の方式、シード、開始局面、
//! 対局数、塊の局数、探索回数、投了の閾値、および手数上限を保存し、再開時に等価比較する。
//! 開始局面は`initial`か`shuffled-setup`、閾値はf64で比較する。
//! 閾値は実行器がAobaZeroの較正の方式で決め、`--resign-threshold`で渡す。
//! スレッド数、同時対局数、GPUの種類、ネットのパス、および目標局面数は照合しない。
//!
//! 塊kには対局番号kG+1からmin((k+1)G, M)までを番号順に記録する。
//! 各塊は投了も含めて`chunk-NNNNN.mnsd`の1ファイルとし、手数上限の引き分けも残す。
//! 塊0から連続する確定済み塊の`positions`が初めて目標以上になる塊までをサイクルに含める。
//! 全塊で届かなければ全塊を含め、範囲外の確定済み塊は保持する。
//!
//! 書き込み中は`.tmp`を付け、MNSDの件数確定、フラッシュ、同期、改名の後に塊のJSONを確定する。
//! JSONの存在を確定の印とし、再開時はMNSDや保存済みの塊の条件を再検証しない。
//! 未確定の塊は全局を生成し直す。範囲が決まればワーカーを回収し、書きかけの塊を削除する。
//! MNSDの版は3、教師の種類はMCTS、探索規模は指定した探索回数とする。
//! MNSDの項目数には、探索しなかった終局の最終項目も含まれる。
//!
//! 塊とサイクルの両方の集計に、次の欄を保存する。
//!
//! | 欄 | 意味 |
//! |---|---|
//! | `games` | 記録した対局数を表す。 |
//! | `positions` | 探索し、価値と方策を記録した訓練局面数を表す。 |
//! | `searches` | 探索した手番数を表す。 |
//! | `no_resign_games` | 対局番号1、11、21…の投了禁止の対局数を表す。 |
//! | `resign_calibration_games` | 投了禁止の対局番号、結果、両者の値の最小値を対局番号順に持つ。 |
//! | `draw_reason_max_ply` | 手数上限による引き分け数を表す。 |
//! | `mate_in_three_games` | 王手限定の3手以内の勝ちによる打ち切り数を表す。 |
//! | `black_wins` | 先手が勝った対局数を表す。 |
//! | `white_wins` | 後手が勝った対局数を表す。 |
//! | `draws` | 引き分けた対局数を表す。 |
//! | `win_reason_royal_capture` | 相手の最後の王駒の捕獲による勝利数を表す。 |
//! | `win_reason_repetition` | 反復裁定による勝利数を表す。 |
//! | `win_reason_piece_exhaustion` | 駒枯れによる勝利数を表す。 |
//! | `win_reason_bare_king` | 裸玉による勝利数を表す。 |
//! | `win_reason_stalemate` | 相手に合法手がないことによる勝利数を表す。 |
//! | `win_reason_mate` | 相手の最後の王駒の詰みによる勝利数を表す。 |
//! | `win_reason_resignation` | 相手の投了による勝利数を表す。 |
//! | `draw_reason_repetition` | 反復裁定による引き分け数を表す。 |
//! | `draw_reason_piece_exhaustion` | 駒枯れによる引き分け数を表す。 |
//! | `draw_reason_bare_king` | 裸玉による引き分け数を表す。 |
//! | `draw_reason_agreement` | 双方の合意による引き分け数を表す。 |
//! | `plies` | 着手数の合計を表す。 |
//!
//! 塊とサイクルそれぞれの集計には、対象範囲と学習ファイルを示す欄も保存する。
//!
//! | 欄 | 保存先と意味 |
//! |---|---|
//! | `chunk` | 塊の集計で、0から数えた塊の番号を表す。 |
//! | `first_game` | 塊の集計で、最初の対局番号を表す。対局番号は1から数える。 |
//! | `start_position` | サイクルの集計で、開始局面の指定を`initial`か`shuffled-setup`で表す。 |
//! | `target_positions` | サイクルの集計で、目標とする訓練局面数を表す。 |
//! | `chunks` | サイクルの集計で、含めた塊の数を表す。 |
//! | `training_files` | サイクルの集計で、含めた塊のMNSDファイル名を塊の番号順に持つ。 |
//! | `training_positions` | サイクルの集計で、含めた塊の訓練局面数の和を表す。 |
//!
//! `resign_calibration_games`の各項目には、次の欄を保存する。
//! 最小値は、比較に使ったf32の値をf64へ広げて保存する。
//!
//! | 欄 | 意味 |
//! |---|---|
//! | `game` | 1から数えた対局番号を表す。 |
//! | `result` | 先手の勝ちを`black`、後手の勝ちを`white`、引き分けを`draw`で表す。 |
//! | `min_value_black` | 先手の全探索局面での投了判定値の最小値を表す。 |
//! | `min_value_white` | 後手の全探索局面での投了判定値の最小値を表す。 |

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::num::NonZeroUsize;
use std::path::Path;
use std::time::{Duration, Instant};

use minase::datagen::output::{DirectoryLock, atomic_json, commit_files};
use minase::nn::Evaluator;
use minase::training::records::{Header, Outcome, TeacherKind, Writer};
use minase_core::{Color, DrawReason, GameResult, WinReason};
use serde::{Deserialize, Serialize};

use super::Error;
use super::cli::StartPosition;
use super::game::{CompletedGame, ResignCalibrationGame, Settings};
use super::worker::Workers;

/// 出力を決める生成条件。すべての欄を再開時に照合する。
///
/// CPUとTensorRTの16ビット浮動小数点演算では推論の数値が異なるため、
/// `nn_backend`も照合し、同じサイクルに異なる実行プロバイダの結果を混ぜない。
/// 推論の並列数は照合せず、借りた機械を替えて再開できるようGPUの種類も照合しない。
/// `docs/adr/commit-zero-self-play-in-fixed-game-chunks-and-close-the-cycle-at-the-target-positions.md`を参照。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Config {
    pub(super) rules: String,
    pub(super) generation_commit: String,
    pub(super) network_sha256: String,
    pub(super) nn_backend: String,
    pub(super) seed: u64,
    pub(super) start_position: StartPosition,
    pub(super) games: u32,
    pub(super) chunk_games: u32,
    pub(super) simulations: u32,
    pub(super) resign_threshold: f64,
    pub(super) max_ply: u32,
}

impl Config {
    fn validate(&self) -> Result<(), Error> {
        for (invalid, field) in [
            (self.games == 0, "games"),
            (self.chunk_games == 0, "chunk_games"),
            (self.simulations == 0, "simulations"),
            (
                !self.resign_threshold.is_finite() || !(0.0..=1.0).contains(&self.resign_threshold),
                "resign_threshold",
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
            (self.nn_backend != saved.nn_backend, "nn_backend"),
            (self.seed != saved.seed, "seed"),
            (
                self.start_position != saved.start_position,
                "start_position",
            ),
            (self.games != saved.games, "games"),
            (self.chunk_games != saved.chunk_games, "chunk_games"),
            (self.simulations != saved.simulations, "simulations"),
            (
                self.resign_threshold != saved.resign_threshold,
                "resign_threshold",
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

/// 塊とサイクルの双方で加算する件数と、投了禁止の対局の較正の材料。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Counts {
    pub(super) games: u64,
    pub(super) no_resign_games: u64,
    pub(super) resign_calibration_games: Vec<ResignCalibrationGame>,
    pub(super) draw_reason_max_ply: u64,
    pub(super) positions: u64,
    pub(super) searches: u64,
    pub(super) mate_in_three_games: u64,
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
        self.searches += game.searches;
        self.positions += game.searches;
        if let Some(calibration) = &game.calibration {
            self.no_resign_games += 1;
            self.resign_calibration_games.push(calibration.clone());
        }
        let winner = match game.record.outcome() {
            Outcome::Game(GameResult::Win { winner, .. }) | Outcome::MateInThree { winner } => {
                Some(winner)
            }
            _ => None,
        };
        match winner {
            Some(Color::Black) => self.black_wins += 1,
            Some(Color::White) => self.white_wins += 1,
            None => {}
        }
        match game.record.outcome() {
            Outcome::Game(GameResult::Win { reason, .. }) => match reason {
                WinReason::RoyalCapture => self.win_reason_royal_capture += 1,
                WinReason::Repetition => self.win_reason_repetition += 1,
                WinReason::PieceExhaustion => self.win_reason_piece_exhaustion += 1,
                WinReason::BareKing => self.win_reason_bare_king += 1,
                WinReason::Stalemate => self.win_reason_stalemate += 1,
                WinReason::Mate => self.win_reason_mate += 1,
                WinReason::Resignation => self.win_reason_resignation += 1,
            },
            Outcome::Game(GameResult::Draw { reason }) => {
                self.draws += 1;
                match reason {
                    DrawReason::Repetition => self.draw_reason_repetition += 1,
                    DrawReason::PieceExhaustion => self.draw_reason_piece_exhaustion += 1,
                    DrawReason::BareKing => self.draw_reason_bare_king += 1,
                    DrawReason::Agreement => self.draw_reason_agreement += 1,
                }
            }
            Outcome::MateInThree { .. } => self.mate_in_three_games += 1,
            Outcome::MaxPlyDraw => {
                self.draws += 1;
                self.draw_reason_max_ply += 1;
            }
            Outcome::NotFromGame | Outcome::EvaluationAdjudication { .. } => {}
        }
    }

    fn add(&mut self, other: &Self) {
        self.games += other.games;
        self.no_resign_games += other.no_resign_games;
        self.draw_reason_max_ply += other.draw_reason_max_ply;
        self.resign_calibration_games
            .extend_from_slice(&other.resign_calibration_games);
        self.positions += other.positions;
        self.searches += other.searches;
        self.mate_in_three_games += other.mate_in_three_games;
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct ChunkSummary {
    pub(super) chunk: u32,
    pub(super) first_game: u32,
    #[serde(flatten)]
    pub(super) counts: Counts,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Summary {
    pub(super) start_position: StartPosition,
    pub(super) target_positions: u64,
    pub(super) chunks: u32,
    #[serde(flatten)]
    pub(super) counts: Counts,
    pub(super) training_files: Vec<String>,
    pub(super) training_positions: u64,
}

/// 塊の集計だけから、サイクルの件数と学習ファイルを決める。
/// 較正の材料とファイルの一覧は対局番号に対応する順に並べる。
pub(super) fn summarize(
    chunks: &[ChunkSummary],
    target_positions: u64,
    start_position: StartPosition,
) -> Summary {
    let mut counts = Counts::default();
    for chunk in chunks {
        counts.add(&chunk.counts);
    }
    counts
        .resign_calibration_games
        .sort_unstable_by_key(|game| game.game);
    let mut ordered: Vec<_> = chunks.iter().collect();
    ordered.sort_unstable_by_key(|chunk| chunk.chunk);
    let files = ordered
        .into_iter()
        .map(|chunk| games_filename(chunk.chunk))
        .collect();
    let training_positions = counts.positions;
    Summary {
        start_position,
        target_positions,
        chunks: chunks.len() as u32,
        counts,
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
        writeln!(stdout, "start_position: {}", config.start_position)?;
        writeln!(stdout, "rules: {}", config.rules)?;
        writeln!(stdout, "commit: {}", config.generation_commit)?;
        writeln!(stdout, "network_sha256: {}", config.network_sha256)?;
        writeln!(stdout, "simulations: {}", config.simulations)?;
        writeln!(stdout, "resign_threshold: {}", config.resign_threshold)?;
        writeln!(stdout, "max_ply: {}", config.max_ply)?;
        if let serde_json::Value::Object(fields) = serde_json::to_value(&self.summary)? {
            for (name, value) in fields {
                if name == "start_position" {
                    continue;
                }
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

/// 塊0から目標に届く最小の範囲か、目標未達で確定した全塊の数を返す。
/// その範囲に未確定の塊があれば、まだサイクルを締められない。
fn included_chunks(
    completed: &BTreeMap<u32, ChunkSummary>,
    chunk_count: u32,
    target_positions: u64,
) -> Option<u32> {
    let mut positions = 0;
    for chunk in 0..chunk_count {
        let counts = &completed.get(&chunk)?.counts;
        positions += counts.positions;
        if positions >= target_positions {
            return Some(chunk + 1);
        }
    }
    Some(chunk_count)
}

/// 確定済みの塊は読むだけにし、サイクルの範囲がまだ決まらないときだけ推論器を作る。
/// 推論器の生成関数には0から始まるスレッド番号を順に渡し、全推論器を作ってからスレッドを起動する。
pub(super) fn generate(
    directory: &Path,
    config: &Config,
    target_positions: u64,
    checksum: &[u8; 32],
    threads: NonZeroUsize,
    games_per_thread: NonZeroUsize,
    mut make_evaluator: impl FnMut(usize) -> Result<Box<dyn Evaluator + Send>, Error>,
) -> Result<RunReport, Error> {
    let started = Instant::now();
    config.validate()?;
    if target_positions == 0 {
        return Err(Error::InvalidSettings("target_positions"));
    }
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
    for chunk in 0..config.chunk_count() {
        let path = directory.join(format!("chunk-{chunk:05}.json"));
        if path.try_exists()? {
            // JSONの存在が確定の印であり、MNSDや保存済み条件の再検証はしない。
            let summary = serde_json::from_reader(BufReader::new(File::open(path)?))?;
            completed.insert(chunk, summary);
        }
    }
    let mut included = included_chunks(&completed, config.chunk_count(), target_positions);
    let mut generated_games = 0;
    let mut generated_positions = 0;
    if included.is_none() {
        let numbers = (0..config.chunk_count())
            .filter(|chunk| !completed.contains_key(chunk))
            .flat_map(|chunk| config.game_range(chunk).map(|number| number as u32))
            .collect();
        let mut evaluators = Vec::new();
        for thread in 0..threads.get() {
            evaluators.push(make_evaluator(thread)?);
        }
        let settings = Settings {
            rules: header.rules(),
            seed: config.seed,
            start_position: config.start_position,
            simulations: config.simulations,
            resign_threshold: config.resign_threshold,
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
            generated_positions += game.searches;
            if active
                .get_mut(&chunk)
                .expect("the chunk writer was opened above")
                .push(game)?
            {
                let writer = active
                    .remove(&chunk)
                    .expect("the completed chunk has a writer");
                completed.insert(chunk, writer.commit(directory)?);
                included = included_chunks(&completed, config.chunk_count(), target_positions);
                if included.is_some() {
                    workers.stop();
                }
                writeln!(
                    io::stderr().lock(),
                    "chunks: {}/{} games: {generated_games} positions: {generated_positions} elapsed_seconds: {:.3}",
                    completed.len(),
                    config.chunk_count(),
                    started.elapsed().as_secs_f64()
                )?;
                if included.is_some() {
                    break;
                }
            }
        }
        let result = workers.join();
        for writer in active.into_values() {
            writer.discard(directory)?;
        }
        result?;
    }
    let included = included.ok_or(Error::IncompleteChunks)?;
    let chunks: Vec<_> = completed.into_values().take(included as usize).collect();
    let summary = summarize(&chunks, target_positions, config.start_position);
    atomic_json(directory, "summary.json", &summary)?;
    Ok(RunReport {
        summary,
        generated_games,
        generated_positions,
        elapsed: started.elapsed(),
    })
}

fn open_directory(directory: &Path, config: &Config) -> Result<DirectoryLock, Error> {
    if !directory.try_exists()? {
        fs::create_dir(directory)?;
    }
    let config_path = directory.join("config.json");
    if !config_path.try_exists()? {
        for entry in fs::read_dir(directory)? {
            let name = entry?.file_name();
            if name != ".zero_selfplay.lock" && name != "config.json.tmp" {
                return Err(Error::NonemptyDirectory(directory.to_owned()));
            }
        }
    }
    let lock = DirectoryLock::acquire(directory, ".zero_selfplay.lock")?;
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
}

impl ChunkWriter {
    fn new(directory: &Path, config: &Config, header: &Header, chunk: u32) -> Result<Self, Error> {
        let range = config.game_range(chunk);
        let open = || -> Result<_, Error> {
            let path = directory.join(format!("{}.tmp", games_filename(chunk)));
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
            games: open()?,
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
            self.games.write_game(&game.record)?;
            self.summary.counts.add_game(&game);
            self.next += 1;
        }
        Ok(self.next == self.end)
    }

    fn commit(self, directory: &Path) -> Result<ChunkSummary, Error> {
        let mut games = self.games.finish()?;
        games.flush()?;
        drop(games);
        let games_name = games_filename(self.summary.chunk);
        let games_temporary = directory.join(format!("{games_name}.tmp"));
        let games_final = directory.join(games_name);
        commit_files(directory, &[(&games_temporary, &games_final)])?;
        atomic_json(
            directory,
            &format!("chunk-{:05}.json", self.summary.chunk),
            &self.summary,
        )?;
        Ok(self.summary)
    }

    fn discard(self, directory: &Path) -> Result<(), Error> {
        let chunk = self.summary.chunk;
        drop(self);
        fs::remove_file(directory.join(format!("{}.tmp", games_filename(chunk))))?;
        Ok(())
    }
}

fn games_filename(chunk: u32) -> String {
    format!("chunk-{chunk:05}.mnsd")
}

#[cfg(test)]
mod tests;
