//! lishogiのゲームNDJSONを教師データまたは実戦開始の局面一覧へ変換する。

// 編集範囲をバイナリ2本に限定し、既存の順序付き書き出しと来歴実装を共有する。
// 別バイナリのCLIなど、この変換器が使わない部分だけは未使用を許す。
#[allow(dead_code)]
#[path = "selfplay_gen.rs"]
mod selfplay;

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::path::{Path, PathBuf};
use std::thread;

use clap::{Parser, Subcommand};
use minase::eval::provenance::{GameOrigin, ResultOrigin, SearchCondition, StartOrigin};
use minase::eval::training_data::{Header, Outcome, Record, Writer, best_move_is_tactical};
use minase::notation::{
    sfen::{SetupPosition, to_extended_sfen},
    usi,
};
use minase::search::{DEFAULT_THREADS, SearchLimits, SearchSnapshot, TranspositionTable, search};
use minase::{Color, Game, GameResult, MoveGenerator, Position, Rules};
use selfplay::{CompletedGame, CompletedRecord, Statistics, data_error};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Parser)]
#[command(name = "lishogi_import")]
struct Arguments {
    #[command(subcommand)]
    command: Operation,
}

#[derive(Subcommand)]
enum Operation {
    /// 人間の対局結果を教師とするMNSDを作る。
    Games {
        #[command(flatten)]
        common: Common,
        /// 他の教師ファイルと重ならない、0以外の識別用シード。
        #[arg(long)]
        seed: NonZeroU64,
        /// 変更のある作業ツリーでの生成を許可する。
        #[arg(long)]
        allow_dirty: bool,
    },
    /// 実戦開始の自己対局に使う中盤の局面一覧を作る。
    Openings(Common),
}

#[derive(clap::Args)]
struct Common {
    /// 1行1局のゲームJSON。
    #[arg(long)]
    input: PathBuf,
    /// 新規作成するMNSDまたは局面一覧。
    #[arg(long)]
    output: PathBuf,
    /// 理由ごとの件数と棋譜IDを保存するJSON。
    #[arg(long)]
    report: PathBuf,
    /// 局面単独の探索のノード上限。本測定では100000を指定する。
    #[arg(long, default_value = "100000")]
    nodes: NonZeroU32,
    /// 並行して処理する対局数。
    #[arg(long, default_value = "1")]
    concurrency: NonZeroUsize,
    /// ワーカーごとの置換表容量(MB)。
    #[arg(long, default_value = "16")]
    hash_mb: NonZeroUsize,
}

#[derive(Debug)]
enum ImportError {
    ReadLine {
        line: usize,
        source: io::Error,
    },
    JsonLine {
        line: usize,
        source: serde_json::Error,
    },
    InvalidId {
        line: usize,
    },
    DirtyTree,
    WorkerPanic,
}
impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadLine { line, source } => write!(f, "NDJSON line {line}: {source}"),
            Self::JsonLine { line, source } => write!(f, "NDJSON line {line}: {source}"),
            Self::InvalidId { line } => write!(
                f,
                "NDJSON line {line}: id must be nonempty and contain no whitespace"
            ),
            Self::DirtyTree => {
                f.write_str("the working tree is dirty; commit changes or pass --allow-dirty")
            }
            Self::WorkerPanic => f.write_str("import worker panicked"),
        }
    }
}
impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::JsonLine { source, .. } => Some(source),
            Self::ReadLine { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InputGame {
    id: String,
    variant: Option<String>,
    #[serde(default, deserialize_with = "present_field")]
    initial_sfen: Option<Value>,
    players: Option<Players>,
    rated: Option<bool>,
    speed: Option<String>,
    status: Option<String>,
    winner: Option<String>,
    moves: String,
    clock: Option<Value>,
    days_per_turn: Option<Value>,
}
/// 持ち時間の区分を返す。lishogiの書き出しは`speed`欄を省くことが多いので、
/// `clock`（リアルタイム）または`daysPerTurn`（通信対局）からも導く。どちらもなければ`None`。
fn time_control(game: &InputGame) -> Option<String> {
    if let Some(speed) = game.speed.as_ref().filter(|s| !s.is_empty()) {
        return Some(speed.clone());
    }
    if game.clock.as_ref().is_some_and(Value::is_object) {
        return Some("realTime".to_owned());
    }
    if game.days_per_turn.as_ref().is_some_and(|v| !v.is_null()) {
        return Some("correspondence".to_owned());
    }
    None
}

/// nullを含め、欄が存在すること自体を保持する。
fn present_field<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}
#[derive(Deserialize)]
struct Players {
    sente: Option<Player>,
    gote: Option<Player>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Player {
    user: Option<User>,
    ai_level: Option<Value>,
    rating: Option<i32>,
    provisional: Option<bool>,
}
#[derive(Deserialize)]
struct User {
    title: Option<String>,
}

/// 最初に該当した理由だけに集計する。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum Exclusion {
    Variant,
    InitialSfen,
    Bot,
    Unrated,
    MissingSpeed,
    Rating,
    StatusOutoftime,
    StatusTimeout,
    StatusAborted,
    StatusDraw,
    StatusRepetition,
    StatusPerpetualCheck,
    StatusBareKing,
    StatusCreated,
    StatusStarted,
    StatusMate,
    StatusStalemate,
    StatusCheat,
    StatusNoStart,
    StatusUnknownFinish,
    UnknownStatus,
    Winner,
    Duplicate,
    IllegalDefaultRules,
    WinnerMismatch,
    ResignerMismatch,
    DeferredPromotion,
}
impl std::fmt::Display for Exclusion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Exclusion {}
impl Exclusion {
    const ALL: [Self; 27] = [
        Self::Variant,
        Self::InitialSfen,
        Self::Bot,
        Self::Unrated,
        Self::MissingSpeed,
        Self::Rating,
        Self::StatusOutoftime,
        Self::StatusTimeout,
        Self::StatusAborted,
        Self::StatusDraw,
        Self::StatusRepetition,
        Self::StatusPerpetualCheck,
        Self::StatusBareKing,
        Self::StatusCreated,
        Self::StatusStarted,
        Self::StatusMate,
        Self::StatusStalemate,
        Self::StatusCheat,
        Self::StatusNoStart,
        Self::StatusUnknownFinish,
        Self::UnknownStatus,
        Self::Winner,
        Self::Duplicate,
        Self::IllegalDefaultRules,
        Self::WinnerMismatch,
        Self::ResignerMismatch,
        Self::DeferredPromotion,
    ];
}
#[derive(Default, Serialize)]
struct Excluded {
    count: usize,
    ids: Vec<String>,
}
#[derive(Serialize)]
struct Report {
    accepted: usize,
    accepted_ids: Vec<String>,
    excluded: BTreeMap<Exclusion, Excluded>,
    candidates: usize,
    excluded_score: usize,
    excluded_no_legal_moves: usize,
    retained: usize,
    retained_plies: BTreeMap<String, Vec<u32>>,
    truncated_illegal: BTreeMap<String, u32>,
    nodes: u32,
    start_origin_note: &'static str,
}
impl Report {
    fn new(nodes: u32) -> Self {
        Self {
            accepted: 0,
            accepted_ids: Vec::new(),
            excluded: Exclusion::ALL
                .into_iter()
                .map(|r| (r, Excluded::default()))
                .collect(),
            candidates: 0,
            excluded_score: 0,
            excluded_no_legal_moves: 0,
            retained: 0,
            retained_plies: BTreeMap::new(),
            truncated_illegal: BTreeMap::new(),
            nodes,
            start_origin_note: "Human games begin at the standard initial position; games provenance uses start_origin=random under the existing enum contract.",
        }
    }
    fn exclude(&mut self, reason: Exclusion, id: String) {
        let entry = self.excluded.entry(reason).or_default();
        entry.count += 1;
        entry.ids.push(id);
    }
}

fn metadata_exclusion(
    game: &InputGame,
    openings: bool,
    seen: &mut HashSet<String>,
) -> Option<Exclusion> {
    use Exclusion::*;
    if game.variant.as_deref() != Some("chushogi") {
        return Some(Variant);
    }
    if game.initial_sfen.is_some() {
        return Some(InitialSfen);
    }
    let players = game
        .players
        .as_ref()
        .and_then(|p| Some([p.sente.as_ref()?, p.gote.as_ref()?]));
    let Some(players) = players else {
        return Some(Bot);
    };
    if players.iter().any(|p| {
        p.ai_level.is_some()
            || p.user
                .as_ref()
                .is_none_or(|u| u.title.as_deref() == Some("BOT"))
    }) {
        return Some(Bot);
    }
    if !openings {
        if game.rated != Some(true) {
            return Some(Unrated);
        }
        if time_control(game).is_none() {
            return Some(MissingSpeed);
        }
        if players
            .iter()
            .any(|p| p.rating.is_none() || p.provisional == Some(true))
        {
            return Some(Rating);
        }
        let status = match game.status.as_deref() {
            Some("resign" | "royalsLost") => None,
            Some("outoftime") => Some(StatusOutoftime),
            Some("timeout") => Some(StatusTimeout),
            Some("aborted") => Some(StatusAborted),
            Some("draw") => Some(StatusDraw),
            Some("repetition") => Some(StatusRepetition),
            Some("perpetualCheck") => Some(StatusPerpetualCheck),
            Some("bareKing") => Some(StatusBareKing),
            Some("created") => Some(StatusCreated),
            Some("started") => Some(StatusStarted),
            Some("mate") => Some(StatusMate),
            Some("stalemate") => Some(StatusStalemate),
            Some("cheat") => Some(StatusCheat),
            Some("noStart") => Some(StatusNoStart),
            Some("unknownFinish") => Some(StatusUnknownFinish),
            _ => Some(UnknownStatus),
        };
        if status.is_some() {
            return status;
        }
        if winner(game).is_none() {
            return Some(Winner);
        }
    }
    if !seen.insert(game.id.clone()) {
        return Some(Duplicate);
    }
    None
}
fn winner(game: &InputGame) -> Option<Color> {
    match game.winner.as_deref() {
        Some("sente") => Some(Color::Black),
        Some("gote") => Some(Color::White),
        _ => None,
    }
}
struct ReplayPosition {
    position: Position,
    ply: u16,
    repeated: bool,
}
struct Replay {
    positions: Vec<ReplayPosition>,
    truncated: Option<u32>,
}

/// 対局全体の規則と勝者を照合してから、探索対象を返す。
fn replay(input: &InputGame, openings: bool) -> Result<Replay, Exclusion> {
    let mut game = Game::new(Rules::ENGINE_DEFAULT);
    let mut positions = Vec::new();
    let mut truncated = None;
    let mut moves = input.moves.split_whitespace();
    loop {
        if !openings && !game.position().promotion_deferred().is_empty() {
            return Err(Exclusion::DeferredPromotion);
        }
        if game.result().is_some() {
            break;
        }
        let ply = game.ply_count();
        if !openings
            || (ply >= 40
                && ply.is_multiple_of(20)
                && game.position().occupied().popcount() >= 47
                && game.position().promotion_deferred().is_empty())
        {
            positions.push(ReplayPosition {
                position: game.position().clone(),
                ply: u16::try_from(ply).map_err(|_| Exclusion::IllegalDefaultRules)?,
                repeated: selfplay::current_position_is_repeated(&game),
            });
        }
        let Some(text) = moves.next() else {
            break;
        };
        let legal = usi::parse(game.position(), text)
            .ok()
            .and_then(|mv| game.play(mv).ok());
        if legal.is_none() {
            if !openings {
                return Err(Exclusion::IllegalDefaultRules);
            }
            truncated = Some(ply + 1);
            break;
        }
    }
    if !openings {
        match game.result() {
            Some(GameResult::Win { winner: actual, .. }) if Some(actual) == winner(input) => {}
            Some(_) => return Err(Exclusion::WinnerMismatch),
            None if input.status.as_deref() == Some("royalsLost") => {
                return Err(Exclusion::WinnerMismatch);
            }
            None if Some(game.position().side_to_move()) == winner(input) => {
                return Err(Exclusion::ResignerMismatch);
            }
            None => {}
        }
    }
    Ok(Replay {
        positions,
        truncated,
    })
}

#[derive(Serialize)]
struct GameDetails {
    game: u32,
    sente_rating: Option<i32>,
    gote_rating: Option<i32>,
    speed: Option<String>,
    clock: Option<Value>,
    days_per_turn: Option<Value>,
    status: Option<String>,
    winner: Option<String>,
    plies: usize,
    recorded_positions: usize,
    excluded_positions: BTreeMap<&'static str, u64>,
}
struct Job {
    input: InputGame,
    number: u32,
}
struct Processed {
    completed: CompletedGame,
    openings: Vec<(u32, String)>,
    score_exclusions: usize,
    no_legal_moves: usize,
    candidates: usize,
}
fn process_job(
    job: &Job,
    common: &Common,
    openings: bool,
    table: &mut TranspositionTable,
) -> io::Result<Processed> {
    let pst = minase::eval::weights().map_err(data_error)?;
    let limits = SearchLimits::new(None, Some(u64::from(common.nodes.get())), None, None)
        .map_err(data_error)?;
    let mut records = Vec::new();
    let mut stats = Statistics::default();
    let mut retained = Vec::new();
    let mut score_exclusions = 0;
    let mut no_legal_moves = 0;
    let replay = replay(&job.input, openings).map_err(data_error)?;
    for candidate in &replay.positions {
        let position = &candidate.position;
        let game = Game::from_position(Rules::ENGINE_DEFAULT, position.clone());
        if openings && game.legal_moves().is_empty() {
            no_legal_moves += 1;
            continue;
        }
        let snapshot = SearchSnapshot::from_game(&game).map_err(data_error)?;
        table.clear();
        let result =
            search(&pst, &snapshot, &limits, DEFAULT_THREADS, table).map_err(data_error)?;
        if openings {
            if result.score.unsigned_abs() > 500 {
                score_exclusions += 1;
                continue;
            }
            let setup = SetupPosition::new(
                position.clone(),
                position.lion_capture_square(),
                u32::from(candidate.ply) + 1,
            )
            .map_err(data_error)?;
            retained.push((u32::from(candidate.ply), to_extended_sfen(&setup)));
        } else if result.score.unsigned_abs() >= selfplay::MATE_BAND_START {
            stats.excluded_mate_band += 1;
        } else if best_move_is_tactical(
            position,
            &MoveGenerator::new(Rules::ENGINE_DEFAULT.moves),
            result.best_move,
        )
        .map_err(data_error)?
        {
            stats.excluded_tactical += 1;
        } else if candidate.repeated {
            stats.excluded_repetition += 1;
        } else {
            let outcome = if winner(&job.input) == Some(position.side_to_move()) {
                Outcome::Win
            } else {
                Outcome::Loss
            };
            records.push(CompletedRecord {
                record: Record::from_position(
                    position,
                    i16::try_from(result.score).map_err(data_error)?,
                    outcome,
                    job.number,
                    candidate.ply,
                ),
                search_key: position.zobrist() ^ position.rights_zobrist(),
            });
        }
    }
    stats.recorded_positions = records.len() as u64;
    Ok(Processed {
        completed: CompletedGame {
            game_number: job.number,
            records,
            stats,
        },
        openings: retained,
        score_exclusions,
        no_legal_moves,
        candidates: replay.positions.len(),
    })
}
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut path = path.as_os_str().to_owned();
    path.push(suffix);
    path.into()
}
fn create(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}
fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let mut file = create(path)?;
    let result = serde_json::to_writer_pretty(&mut file, value)
        .map_err(data_error)
        .and_then(|()| file.flush());
    if result.is_err() {
        fs::remove_file(path)?;
    }
    result
}

fn run(common: &Common, generation: Option<(NonZeroU64, bool)>) -> io::Result<()> {
    let openings = generation.is_none();
    let mut report = Report::new(common.nodes.get());
    let mut seen = HashSet::new();
    let mut jobs = Vec::new();
    for (index, line) in BufReader::new(File::open(&common.input)?)
        .lines()
        .enumerate()
    {
        let line = line.map_err(|source| {
            data_error(ImportError::ReadLine {
                line: index + 1,
                source,
            })
        })?;
        let input: InputGame = serde_json::from_str(&line).map_err(|source| {
            data_error(ImportError::JsonLine {
                line: index + 1,
                source,
            })
        })?;
        if input.id.is_empty() || input.id.chars().any(char::is_whitespace) {
            return Err(data_error(ImportError::InvalidId { line: index + 1 }));
        }
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
    let commit = selfplay::git_output(&["rev-parse", "HEAD"])?;
    selfplay::validate_commit_hash(&commit)?;
    if let Some((_, false)) = generation
        && !selfplay::git_output(&["status", "--porcelain"])?.is_empty()
    {
        return Err(data_error(ImportError::DirtyTree));
    }
    // 作成したファイルだけを回収し、既存の成果物は上書きしない。
    let mut created = Vec::new();
    let result = (|| {
        let output = create(&common.output)?;
        created.push(common.output.clone());
        let (mut text_output, mut writer) = if let Some((seed, _)) = generation {
            let pst = minase::eval::weights().map_err(data_error)?;
            let header = Header::new(
                "engine-default".to_owned(),
                commit.clone(),
                *pst.checksum(),
                common.nodes.get(),
                seed.get(),
                0,
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
                let stats = &result.completed.stats;
                let players = job.input.players.as_ref();
                details.insert(
                    job.input.id.clone(),
                    GameDetails {
                        game: job.number,
                        sente_rating: players
                            .and_then(|p| p.sente.as_ref())
                            .and_then(|p| p.rating),
                        gote_rating: players.and_then(|p| p.gote.as_ref()).and_then(|p| p.rating),
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
                // バッチ内の統合番号だけを詰める。Recordの対局番号は受理順のまま。
                result.completed.game_number =
                    u32::try_from(completed.len() + 1).map_err(data_error)?;
                completed.push(Ok(result.completed));
            }
            if let Some(writer) = &mut writer {
                selfplay::merge_completed_games(
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
            let path = sidecar(&common.output, ".games.json");
            write_json(&path, &details)?;
            created.push(path);
            let path = sidecar(&common.output, ".provenance.json");
            selfplay::write_mapped_provenance(
                &selfplay::ProvenanceArguments {
                    input: common.output.clone(),
                    output: path.clone(),
                    result_origin: ResultOrigin::Human,
                    start_origin: StartOrigin::Random,
                    lambda: 0.0,
                    search_condition: SearchCondition::Standalone,
                },
                Some(
                    jobs.iter()
                        .map(|job| GameOrigin {
                            game: job.number,
                            id: job.input.id.clone(),
                            ply: None,
                        })
                        .collect(),
                ),
            )?;
            created.push(path);
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
fn main() {
    let result = match Arguments::parse().command {
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
