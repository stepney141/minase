//! 評価関数の学習に使う自己対局データの生成と検査。

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Seek, SeekFrom, Write};
use std::num::{NonZeroU64, NonZeroUsize};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use clap::{Parser, Subcommand};
use minase::core::rules::parse_rule_set;
use minase::eval::Pst;
use minase::eval::provenance::{
    GameOrigin, Provenance, ResultOrigin, SearchCondition, StartOrigin, Teacher,
};
use minase::eval::rescore::{self, RescoreEntry, RescoreHeader, RescoreReader, RescoreStatus};
use minase::eval::training_data::{
    Error as TrainingDataError, Header, Outcome, Reader, Record, Writer, best_move_is_tactical,
};
use minase::rng::{XorShift64, derive_seed};
use minase::search::{DEFAULT_THREADS, SearchLimits, SearchSnapshot, TranspositionTable, search};
use minase::{
    Color, DrawReason, Game, GameResult, GameStatus, MoveGenerator, Position, Rules, WinReason,
    to_sfen,
};

/// グローバルアロケータ。探索を行う既存バイナリと同じくmimallocを使う。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// 教師探索の既定ノード上限。
const DEFAULT_NODES: u32 = 100_000;
/// 1局の既定手数上限。
const DEFAULT_MAX_PLY: u16 = 4_000;
/// ワーカーごとの既定置換表容量。
const DEFAULT_HASH_MB: NonZeroUsize = NonZeroUsize::new(16).unwrap();
/// 詰み帯として除外する探索値の絶対値下限。
pub(crate) const MATE_BAND_START: u32 = 29_000;
/// ランダム着手を配置する序盤終了後の手数幅。
const INJECTION_WINDOW: usize = 80;
/// 注入オフセットのヒストグラム区間数。
const INJECTION_HISTOGRAM_BINS: usize = 8;
/// 探索値の取り得る値の数。
const SCORE_VALUE_COUNT: usize = 65_536;

/// 自己対局データ生成器のコマンドライン引数。
#[derive(Parser)]
#[command(name = "selfplay_gen")]
struct Arguments {
    /// 実行する操作。
    #[command(subcommand)]
    command: Operation,
}

/// 自己対局データに対する操作。
#[derive(Subcommand)]
enum Operation {
    /// 自己対局から学習データを生成する。
    Generate(GenerateArguments),
    /// 学習データの形式と全局面を検査する。
    Inspect(InspectArguments),
    /// 保存済みの局面を探索し、MNRSへ付け直す。
    Rescore(RescoreArguments),
    /// 既存MNSDに明示的な来歴を付ける。
    Provenance(ProvenanceArguments),
}

/// 付け直しの引数。探索の並列化は局面単位で行う。
#[derive(clap::Args)]
struct RescoreArguments {
    /// 読み取り専用で開く元のMNSD。
    #[arg(long)]
    input: PathBuf,
    /// 昇順で重複のない対象番号の一覧。
    #[arg(long)]
    targets: PathBuf,
    /// 新規作成または再開するMNRS。
    #[arg(long)]
    output: PathBuf,
    /// 教師のMNPT。省略時は埋め込み重み。
    #[arg(long)]
    pst: Option<PathBuf>,
    /// 各局面のノード上限。
    #[arg(long, value_parser = parse_positive_u32)]
    nodes: u32,
    /// 各ワーカーの置換表容量(MB)。
    #[arg(long, default_value_t = DEFAULT_HASH_MB, value_parser = parse_positive_usize)]
    hash_mb: NonZeroUsize,
    /// 並行して探索する局面数。
    #[arg(long, default_value = "1", value_parser = parse_positive_usize)]
    concurrency: NonZeroUsize,
    /// 変更のある作業ツリーでの実行を許可する。
    #[arg(long)]
    allow_dirty: bool,
}

/// 既存MNSDの来歴作成の引数。
#[derive(clap::Args)]
pub(crate) struct ProvenanceArguments {
    /// 元のMNSD。
    #[arg(long)]
    pub(crate) input: PathBuf,
    /// 新規作成する来歴JSON。
    #[arg(long)]
    pub(crate) output: PathBuf,
    /// 対局結果の由来。本操作ではselfplayだけを扱う。
    #[arg(long, value_enum)]
    pub(crate) result_origin: ResultOrigin,
    /// 開始局面の由来。本操作ではrandomだけを扱う。
    #[arg(long, value_enum)]
    pub(crate) start_origin: StartOrigin,
    /// 教師値に占める探索値の割合。
    #[arg(long)]
    pub(crate) lambda: f64,
    /// 教師の探索条件。
    #[arg(long, value_enum, default_value = "in-game")]
    pub(crate) search_condition: SearchCondition,
}

/// `generate`サブコマンドの引数。
#[derive(clap::Args)]
struct GenerateArguments {
    /// 新規作成する出力ファイル。
    #[arg(long, required = true)]
    output: PathBuf,
    /// 生成する対局数。
    #[arg(long, required_unless_present = "openings", conflicts_with = "openings", value_parser = parse_positive_u32)]
    games: Option<u32>,
    /// 実戦開始の局面一覧。1行につき1局を生成する。
    #[arg(long)]
    openings: Option<PathBuf>,
    /// 対局シードの派生元。
    #[arg(long, required = true)]
    seed: u64,
    /// 1探索のノード上限。
    #[arg(long, default_value_t = DEFAULT_NODES, value_parser = parse_positive_u32)]
    nodes: u32,
    /// 1局に注入するランダム着手の上限回数。
    #[arg(long, required = true, value_parser = clap::value_parser!(u8).range(0..=80))]
    random_moves: u8,
    /// 同時に走らせる自己対局数。
    #[arg(long, default_value = "1", value_parser = parse_positive_usize)]
    concurrency: NonZeroUsize,
    /// 1局の手数上限。
    #[arg(long, default_value_t = DEFAULT_MAX_PLY, value_parser = parse_positive_u16)]
    max_ply: u16,
    /// ワーカーごとの置換表容量(MB)。
    #[arg(long, default_value_t = DEFAULT_HASH_MB, value_parser = parse_positive_usize)]
    hash_mb: NonZeroUsize,
    /// 変更のある作業ツリーからの生成を許可する。
    #[arg(long)]
    allow_dirty: bool,
}

/// 1局の生成に共通する探索と注入の設定。
#[derive(Clone, Copy)]
struct PlaySettings {
    /// 対局シードの派生元。
    base_seed: u64,
    /// 1探索のノード上限。
    nodes: u32,
    /// 1局に注入するランダム着手の上限回数。
    random_moves: u8,
    /// 1局の手数上限。
    max_ply: u16,
}

/// 実戦開始の局面と、元棋譜での位置。
struct Opening {
    origin: GameOrigin,
    position: Position,
    ply: u32,
}

/// 開始局面一覧または生成引数の不整合。
#[derive(Debug)]
enum OpeningError {
    RandomMoves,
    MissingGames,
    Empty,
    Line {
        line: usize,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    InvalidFields,
    InvalidPosition,
}

impl std::fmt::Display for OpeningError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RandomMoves => f.write_str("--openings requires --random-moves 0"),
            Self::MissingGames => f.write_str("--games is required without --openings"),
            Self::Empty => f.write_str("opening list is empty"),
            Self::Line { line, source } => write!(f, "opening line {line}: {source}"),
            Self::InvalidFields => {
                f.write_str("expected <id> <ply> <extended SFEN>, with matching move number")
            }
            Self::InvalidPosition => {
                f.write_str("opening must have both royals, legal moves, and no deferred promotion")
            }
        }
    }
}

impl std::error::Error for OpeningError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Line { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

/// 一覧の全行を検査し、先獅子の状態を含めて復元する。
fn read_openings(path: &Path) -> io::Result<Vec<Opening>> {
    use minase::notation::sfen::parse_extended_sfen;
    let mut openings = Vec::new();
    for (index, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let parse = || -> Result<Opening, Box<dyn std::error::Error + Send + Sync>> {
            let line = line?;
            let mut fields = line.split_whitespace();
            let id = fields.next().ok_or(OpeningError::InvalidFields)?;
            let ply: u32 = fields.next().ok_or(OpeningError::InvalidFields)?.parse()?;
            let sfen = fields.collect::<Vec<_>>().join(" ");
            let setup = parse_extended_sfen(&sfen, Rules::ENGINE_DEFAULT.moves)?;
            if ply >= u32::from(u16::MAX) || setup.next_move_number() != ply + 1 {
                return Err(OpeningError::InvalidFields.into());
            }
            let (mut position, lion, _) = setup.into_parts();
            position.set_lion_capture(lion)?;
            let game = Game::from_position(Rules::ENGINE_DEFAULT, position.clone());
            if !position.promotion_deferred().is_empty()
                || game.legal_moves().is_empty()
                || position.royal_pieces(Color::Black).is_empty()
                || position.royal_pieces(Color::White).is_empty()
            {
                return Err(OpeningError::InvalidPosition.into());
            }
            Ok(Opening {
                origin: GameOrigin {
                    game: u32::try_from(index + 1)?,
                    id: id.to_owned(),
                    ply: Some(ply),
                },
                position,
                ply,
            })
        };
        openings.push(parse().map_err(|source| {
            data_error(OpeningError::Line {
                line: index + 1,
                source,
            })
        })?);
    }
    if openings.is_empty() {
        return Err(data_error(OpeningError::Empty));
    }
    Ok(openings)
}

/// 生成の最初の探索に渡す局面を返す。
fn starting_game(rules: Rules, seed: NonZeroU64, opening: Option<&Opening>) -> io::Result<Game> {
    match opening {
        Some(opening) => Ok(Game::from_position(rules, opening.position.clone())),
        None => generate_opening(rules, seed),
    }
}

/// `inspect`サブコマンドの引数。
#[derive(clap::Args)]
struct InspectArguments {
    /// 検査する学習データファイル。
    path: PathBuf,
    /// 内容を表示する先頭レコード数。
    #[arg(long, default_value_t = 0)]
    dump: u64,
}

/// 終局後に結果を付ける記録候補。
#[derive(Clone, PartialEq, Eq, Debug)]
struct Candidate {
    /// 記録時点の局面。
    position: Position,
    /// 手番側視点の探索値。
    score: i16,
    /// 記録時点の手数。
    ply: u16,
    /// 記録時点の探索キー。
    search_key: u64,
}

/// 探索キーを伴う書き出し対象レコード。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct CompletedRecord {
    /// 固定長形式へ書き出すレコード。
    pub(crate) record: Record,
    /// 対局間の局面重複を判定する探索キー。
    pub(crate) search_key: u64,
}

/// 1局分のレコードと統計。
#[derive(PartialEq, Eq, Debug)]
pub(crate) struct CompletedGame {
    /// 1から始まる対局番号。
    pub(crate) game_number: u32,
    /// 終局対局から採用したレコード。
    pub(crate) records: Vec<CompletedRecord>,
    /// 打ち切り対局を含む生成統計。
    pub(crate) stats: Statistics,
}

/// データ生成全体または1局分の統計。
#[derive(Default, PartialEq, Eq, Debug)]
pub(crate) struct Statistics {
    /// 手数上限で破棄した対局数。
    discarded_games: u64,
    /// 先手勝ちの対局数。
    black_wins: u64,
    /// 後手勝ちの対局数。
    white_wins: u64,
    /// 引き分けの対局数。
    draws: u64,
    /// 王駒捕獲による勝利数。
    royal_capture_wins: u64,
    /// 反復裁定による勝利数。
    repetition_wins: u64,
    /// 駒枯れによる勝利数。
    piece_exhaustion_wins: u64,
    /// 裸玉による勝利数。
    bare_king_wins: u64,
    /// 合法手なしによる勝利数。
    stalemate_wins: u64,
    /// 詰みによる勝利数。
    mate_wins: u64,
    /// 投了による勝利数。
    resignation_wins: u64,
    /// 反復裁定による引き分け数。
    repetition_draws: u64,
    /// 駒枯れによる引き分け数。
    piece_exhaustion_draws: u64,
    /// 裸玉による引き分け数。
    bare_king_draws: u64,
    /// 合意による引き分け数。
    agreement_draws: u64,
    /// 探索した局面数。
    searched_positions: u64,
    /// 記録境界以後に探索した局面数。
    recordable_positions: u64,
    /// ファイルへ記録した局面数。
    pub(crate) recorded_positions: u64,
    /// 詰み帯の探索値による除外数。
    pub(crate) excluded_mate_band: u64,
    /// 捕獲または成りの最善手による除外数。
    pub(crate) excluded_tactical: u64,
    /// 現局面の再出現による除外数。
    pub(crate) excluded_repetition: u64,
    /// ランダム手を含む全対局の総手数。
    total_plies: u64,
    /// 探索で指した総手数。
    searched_plies: u64,
    /// 探索が訪問したノード合計。
    searched_nodes: u64,
    /// 予定したランダム着手の合計。
    planned_injections: u64,
    /// 実施したランダム着手の合計。
    performed_injections: u64,
    /// 実施した注入オフセットを10手幅で数えた度数。
    injection_offset_histogram: [u64; INJECTION_HISTOGRAM_BINS],
}

impl Statistics {
    /// 1局分の統計を合計へ加える。
    fn merge(&mut self, other: &Self) {
        self.discarded_games += other.discarded_games;
        self.black_wins += other.black_wins;
        self.white_wins += other.white_wins;
        self.draws += other.draws;
        self.royal_capture_wins += other.royal_capture_wins;
        self.repetition_wins += other.repetition_wins;
        self.piece_exhaustion_wins += other.piece_exhaustion_wins;
        self.bare_king_wins += other.bare_king_wins;
        self.stalemate_wins += other.stalemate_wins;
        self.mate_wins += other.mate_wins;
        self.resignation_wins += other.resignation_wins;
        self.repetition_draws += other.repetition_draws;
        self.piece_exhaustion_draws += other.piece_exhaustion_draws;
        self.bare_king_draws += other.bare_king_draws;
        self.agreement_draws += other.agreement_draws;
        self.searched_positions += other.searched_positions;
        self.recordable_positions += other.recordable_positions;
        self.recorded_positions += other.recorded_positions;
        self.excluded_mate_band += other.excluded_mate_band;
        self.excluded_tactical += other.excluded_tactical;
        self.excluded_repetition += other.excluded_repetition;
        self.total_plies += other.total_plies;
        self.searched_plies += other.searched_plies;
        self.searched_nodes += other.searched_nodes;
        self.planned_injections += other.planned_injections;
        self.performed_injections += other.performed_injections;
        for (total, count) in self
            .injection_offset_histogram
            .iter_mut()
            .zip(other.injection_offset_histogram)
        {
            *total += count;
        }
    }

    /// 終局理由と勝敗を集計する。
    fn record_result(&mut self, result: GameResult) {
        match result {
            GameResult::Win { winner, reason } => {
                match winner {
                    Color::Black => self.black_wins += 1,
                    Color::White => self.white_wins += 1,
                }
                match reason {
                    WinReason::RoyalCapture => self.royal_capture_wins += 1,
                    WinReason::Repetition => self.repetition_wins += 1,
                    WinReason::PieceExhaustion => self.piece_exhaustion_wins += 1,
                    WinReason::BareKing => self.bare_king_wins += 1,
                    WinReason::Stalemate => self.stalemate_wins += 1,
                    WinReason::Mate => self.mate_wins += 1,
                    WinReason::Resignation => self.resignation_wins += 1,
                }
            }
            GameResult::Draw { reason } => {
                self.draws += 1;
                match reason {
                    DrawReason::Repetition => self.repetition_draws += 1,
                    DrawReason::PieceExhaustion => self.piece_exhaustion_draws += 1,
                    DrawReason::BareKing => self.bare_king_draws += 1,
                    DrawReason::Agreement => self.agreement_draws += 1,
                }
            }
        }
    }
}

/// 1局のランダム着手予定と記録開始手数。
#[derive(Clone, PartialEq, Eq, Debug)]
struct InjectionPlan {
    /// 注入する対局開始時からの手数。昇順で重複しない。
    plies: Vec<u32>,
    /// 記録対象とする最初の手数。
    record_from: u32,
}

/// 書き出したレコードから対局横断で求める統計。
pub(crate) struct RecordedStatistics {
    /// i16の全探索値に対応する度数表。
    score_frequencies: Vec<u64>,
    /// 既出局面の探索キー。
    search_keys: HashSet<u64>,
    /// 以前の対局にも現れた局面数。
    duplicate_positions: u64,
}

impl Default for RecordedStatistics {
    fn default() -> Self {
        Self {
            score_frequencies: vec![0; SCORE_VALUE_COUNT],
            search_keys: HashSet::new(),
            duplicate_positions: 0,
        }
    }
}

impl RecordedStatistics {
    /// 対局番号順に書き出す1レコードを集計する。
    fn record(&mut self, completed: &CompletedRecord) {
        self.score_frequencies[score_index(completed.record.score())] += 1;
        if !self.search_keys.insert(completed.search_key) {
            self.duplicate_positions += 1;
        }
    }
}

/// i16の探索値を昇順の度数表添字へ変換する。
fn score_index(score: i16) -> usize {
    usize::try_from(i32::from(score) - i32::from(i16::MIN))
        .expect("an i16 score index must be non-negative")
}

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

/// コマンドを実行し、失敗時は説明を標準エラーへ出して非0終了する。
fn main() {
    if let Err(error) = minase::eval::weights() {
        eprintln!("error: embedded evaluation weights are invalid: {error}");
        process::exit(1);
    }
    if let Err(error) = run() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

/// 解析したサブコマンドを実行する。
fn run() -> io::Result<()> {
    match Arguments::parse().command {
        Operation::Generate(arguments) => generate(&arguments),
        Operation::Inspect(arguments) => inspect(&arguments),
        Operation::Rescore(arguments) => rescore(&arguments),
        Operation::Provenance(arguments) => write_provenance(&arguments),
    }
}

/// 来歴を確定して出力ファイルを作り、生成が失敗すれば出力を削除する。
fn generate(arguments: &GenerateArguments) -> io::Result<()> {
    let openings = arguments
        .openings
        .as_ref()
        .map(|path| read_openings(path))
        .transpose()?;
    if openings.is_some() && arguments.random_moves != 0 {
        return Err(data_error(OpeningError::RandomMoves));
    }
    let games = match &openings {
        Some(openings) => u32::try_from(openings.len()).map_err(data_error)?,
        None => arguments
            .games
            .ok_or_else(|| data_error(OpeningError::MissingGames))?,
    };
    let rules = engine_default_rules()?;
    let rule_set = rules.to_string();
    let generation_commit = git_output(&["rev-parse", "HEAD"])?;
    validate_commit_hash(&generation_commit)?;
    let status = git_output(&["status", "--porcelain"])?;
    if !status.is_empty() && !arguments.allow_dirty {
        return Err(invalid_data(
            "the working tree is dirty; commit changes or pass --allow-dirty",
        ));
    }

    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&arguments.output)?;
    let result = write_training_data(
        file,
        arguments,
        rules,
        &rule_set,
        &generation_commit,
        openings.as_deref(),
        games,
    );
    // 失敗した出力を残すと、Readerが受理する空ファイルや破損ファイルが最終パスに
    // 残り、同じコマンドの再実行もcreate_newで拒否されるため、失敗時は削除する。
    if result.is_err()
        && let Err(remove_error) = fs::remove_file(&arguments.output)
    {
        eprintln!(
            "error: cannot remove incomplete output {}: {remove_error}",
            arguments.output.display()
        );
    }
    result?;
    let mut provenance_path = arguments.output.as_os_str().to_owned();
    provenance_path.push(".provenance.json");
    write_mapped_provenance(
        &ProvenanceArguments {
            input: arguments.output.clone(),
            output: provenance_path.into(),
            result_origin: ResultOrigin::Selfplay,
            start_origin: if openings.is_some() {
                StartOrigin::HumanGame
            } else {
                StartOrigin::Random
            },
            lambda: 0.75,
            search_condition: SearchCondition::InGame,
        },
        openings.map(|openings| openings.into_iter().map(|opening| opening.origin).collect()),
    )
}

/// 自己対局を並列実行し、対局番号順にレコードを書き出して要約を表示する。
fn write_training_data(
    file: File,
    arguments: &GenerateArguments,
    rules: Rules,
    rule_set: &str,
    generation_commit: &str,
    openings: Option<&[Opening]>,
    games: u32,
) -> io::Result<()> {
    // 探索は埋め込み学習PSTで着手するので、その重み本体の検査和を生成元として残す。
    let pst = minase::eval::weights().map_err(|error| invalid_data(error.to_string()))?;
    let network_checksum = *pst.checksum();
    let header = Header::new(
        rule_set.to_owned(),
        generation_commit.to_owned(),
        network_checksum,
        arguments.nodes,
        arguments.seed,
        0,
    )
    .map_err(training_error)?;
    let mut writer = Writer::new(file, header).map_err(training_error)?;
    let next_game = AtomicU64::new(1);
    let (sender, receiver) = mpsc::channel::<io::Result<CompletedGame>>();
    let start = Instant::now();
    let progress_interval = u64::from(games).div_ceil(20).max(1);
    let play_settings = PlaySettings {
        base_seed: arguments.seed,
        nodes: arguments.nodes,
        random_moves: arguments.random_moves,
        max_ply: arguments.max_ply,
    };

    let total = thread::scope(|scope| -> io::Result<(Statistics, RecordedStatistics)> {
        for _ in 0..arguments.concurrency.get() {
            let sender = sender.clone();
            let next_game = &next_game;
            let pst = pst.as_ref();
            scope.spawn(move || {
                let mut table = match TranspositionTable::new(arguments.hash_mb.get()) {
                    Ok(table) => table,
                    Err(error) => {
                        let _ = sender.send(Err(invalid_data(error.to_string())));
                        return;
                    }
                };
                loop {
                    let game_number = next_game.fetch_add(1, Ordering::Relaxed);
                    if game_number > u64::from(games) {
                        break;
                    }
                    let game_number = match u32::try_from(game_number) {
                        Ok(number) => number,
                        Err(error) => {
                            let _ = sender.send(Err(invalid_data(error.to_string())));
                            break;
                        }
                    };
                    let completed = match catch_unwind(AssertUnwindSafe(|| match openings {
                        Some(items) => play_game_from_opening(
                            pst,
                            rules,
                            game_number,
                            play_settings,
                            &mut table,
                            Some(&items[game_number as usize - 1]),
                        ),
                        None => play_game(pst, rules, game_number, play_settings, &mut table),
                    })) {
                        Ok(completed) => completed,
                        Err(_) => Err(invalid_data(format!(
                            "self-play worker panicked in game {game_number}"
                        ))),
                    };
                    let failed = completed.is_err();
                    if sender.send(completed).is_err() || failed {
                        break;
                    }
                }
            });
        }
        drop(sender);

        merge_completed_games(receiver, &mut writer, games, |completed_count, total| {
            if completed_count.is_multiple_of(progress_interval)
                || completed_count == u64::from(games)
            {
                eprintln!(
                    "progress: games={completed_count}/{} records={} elapsed_seconds={:.3}",
                    games,
                    total.recorded_positions,
                    start.elapsed().as_secs_f64()
                );
            }
        })
    })?;

    let (total, recorded) = total;

    let file = writer.finish().map_err(training_error)?;
    file.sync_all()?;
    print_generation_summary(
        arguments,
        rule_set,
        generation_commit,
        &total,
        &recorded,
        start.elapsed().as_secs_f64(),
    );
    Ok(())
}

/// 任意の到着順の対局を対局番号順に書き出して集計する。
pub(crate) fn merge_completed_games<I, W, F>(
    messages: I,
    writer: &mut Writer<W>,
    games: u32,
    mut on_completed: F,
) -> io::Result<(Statistics, RecordedStatistics)>
where
    I: IntoIterator<Item = io::Result<CompletedGame>>,
    W: Write + Seek,
    F: FnMut(u64, &Statistics),
{
    let mut pending = BTreeMap::new();
    let mut next_to_write = 1_u64;
    let mut completed_count = 0_u64;
    let mut total = Statistics::default();
    let mut recorded = RecordedStatistics::default();
    for message in messages {
        let completed = message?;
        pending.insert(completed.game_number, completed);
        while let Some(completed) = pending.remove(&(next_to_write as u32)) {
            for record in &completed.records {
                writer
                    .write_record(&record.record)
                    .map_err(training_error)?;
                recorded.record(record);
            }
            total.merge(&completed.stats);
            completed_count += 1;
            on_completed(completed_count, &total);
            next_to_write += 1;
        }
    }
    if next_to_write != u64::from(games) + 1 {
        return Err(invalid_data(format!(
            "worker channel closed after {} of {} games",
            next_to_write - 1,
            games
        )));
    }
    Ok((total, recorded))
}

/// 1局をランダム序盤から終局まで進め、採用レコードを返す。
fn play_game(
    pst: &Pst,
    rules: Rules,
    game_number: u32,
    settings: PlaySettings,
    table: &mut TranspositionTable,
) -> io::Result<CompletedGame> {
    play_game_from_opening(pst, rules, game_number, settings, table, None)
}

/// 実戦の開始局面では乱数手を入れず、次の局面から記録する。
fn play_game_from_opening(
    pst: &Pst,
    rules: Rules,
    game_number: u32,
    settings: PlaySettings,
    table: &mut TranspositionTable,
    opening: Option<&Opening>,
) -> io::Result<CompletedGame> {
    let game_seed = derive_seed(settings.base_seed, u64::from(game_number));
    let mut game = starting_game(rules, game_seed, opening)?;
    let ply_offset = opening.map_or(0, |opening| opening.ply);
    let opening_ply = game.ply_count();
    let (mut injection_plan, mut injection_rng) =
        plan_injections(game_seed, opening_ply, settings.random_moves);
    if opening.is_some() {
        injection_plan.record_from = 1;
    }
    table.clear();
    let limits = SearchLimits::new(None, Some(u64::from(settings.nodes)), None, None)
        .expect("the CLI parser accepts only non-zero node limits");
    let mut candidates = Vec::new();
    let mut stats = Statistics {
        planned_injections: u64::try_from(injection_plan.plies.len())
            .expect("the injection count is at most 80"),
        ..Statistics::default()
    };
    let generator = MoveGenerator::new(rules.moves);

    while game.result().is_none() && game.ply_count() + ply_offset < u32::from(settings.max_ply) {
        if injection_plan
            .plies
            .binary_search(&game.ply_count())
            .is_ok()
        {
            let legal_moves = game.legal_moves();
            let move_count = NonZeroUsize::new(legal_moves.len()).ok_or_else(|| {
                invalid_data(format!(
                    "game {game_number} is ongoing but has no legal moves"
                ))
            })?;
            let selected = legal_moves[injection_rng.index(move_count)];
            game.play(selected).map_err(|error| {
                invalid_data(format!(
                    "game {game_number} rejected random injection move: {error}"
                ))
            })?;
            stats.performed_injections += 1;
            let offset = usize::try_from(game.ply_count() - 1 - opening_ply)
                .expect("an injection offset below 80 must fit in usize");
            stats.injection_offset_histogram[offset / 10] += 1;
            continue;
        }

        let snapshot = SearchSnapshot::from_game(&game).map_err(|_| {
            invalid_data(format!(
                "game {game_number} is ongoing but has no legal moves"
            ))
        })?;
        let search_result = search(pst, &snapshot, &limits, DEFAULT_THREADS, table)
            .map_err(|error| invalid_data(error.to_string()))?;
        stats.searched_positions += 1;
        stats.searched_nodes = stats
            .searched_nodes
            .checked_add(search_result.nodes)
            .ok_or_else(|| invalid_data("searched node count overflow"))?;

        if game.ply_count() < injection_plan.record_from {
            game.play(search_result.best_move).map_err(|error| {
                invalid_data(format!("game {game_number} rejected best move: {error}"))
            })?;
            stats.searched_plies += 1;
            continue;
        }
        stats.recordable_positions += 1;

        if search_result.score.unsigned_abs() >= MATE_BAND_START {
            stats.excluded_mate_band += 1;
        } else if best_move_is_tactical(game.position(), &generator, search_result.best_move)
            .map_err(|error| {
                invalid_data(format!(
                    "game {game_number} search returned an illegal best move: {error}"
                ))
            })?
        {
            stats.excluded_tactical += 1;
        } else if current_position_is_repeated(&game) {
            stats.excluded_repetition += 1;
        } else {
            let score = i16::try_from(search_result.score).map_err(|_| {
                invalid_data(format!(
                    "game {game_number} score {} does not fit in i16",
                    search_result.score
                ))
            })?;
            let ply = u16::try_from(game.ply_count() + ply_offset).map_err(|_| {
                invalid_data(format!(
                    "game {game_number} ply {} does not fit in u16",
                    game.ply_count()
                ))
            })?;
            candidates.push(Candidate {
                position: game.position().clone(),
                score,
                ply,
                search_key: *game
                    .search_key_history()
                    .last()
                    .expect("every game has an initial search key"),
            });
        }

        game.play(search_result.best_move).map_err(|error| {
            invalid_data(format!("game {game_number} rejected best move: {error}"))
        })?;
        stats.searched_plies += 1;
    }

    stats.total_plies = u64::from(game.ply_count() + ply_offset);
    let records = match game.result() {
        Some(result) => {
            stats.record_result(result);
            let records = candidates
                .into_iter()
                .map(|candidate| {
                    let outcome =
                        Outcome::from_game_result(result, candidate.position.side_to_move());
                    CompletedRecord {
                        record: Record::from_position(
                            &candidate.position,
                            candidate.score,
                            outcome,
                            game_number,
                            candidate.ply,
                        ),
                        search_key: candidate.search_key,
                    }
                })
                .collect::<Vec<_>>();
            stats.recorded_positions =
                u64::try_from(records.len()).map_err(|error| invalid_data(error.to_string()))?;
            records
        }
        None => {
            stats.discarded_games = 1;
            Vec::new()
        }
    };

    Ok(CompletedGame {
        game_number,
        records,
        stats,
    })
}

/// 対局シードからランダム着手の予定と記録開始手数を決める。
fn plan_injections(
    game_seed: NonZeroU64,
    opening_ply: u32,
    maximum: u8,
) -> (InjectionPlan, XorShift64) {
    let mut rng = XorShift64::new(derive_seed(game_seed.get(), 1));
    let plan = plan_injections_with_rng(&mut rng, opening_ply, maximum);
    (plan, rng)
}

/// 指定乱数列を進め、ランダム着手の予定と記録開始手数を決める。
fn plan_injections_with_rng(rng: &mut XorShift64, opening_ply: u32, maximum: u8) -> InjectionPlan {
    let planned = rng.index(
        NonZeroUsize::new(usize::from(maximum) + 1)
            .expect("the injection count range always contains zero"),
    );
    let mut offsets = std::array::from_fn::<_, INJECTION_WINDOW, _>(|index| index);
    for index in 0..planned {
        let remaining = NonZeroUsize::new(INJECTION_WINDOW - index)
            .expect("partial Fisher-Yates stops before the window is empty");
        let selected = index + rng.index(remaining);
        offsets.swap(index, selected);
    }
    let mut plies = offsets[..planned]
        .iter()
        .map(|&offset| {
            opening_ply
                .checked_add(u32::try_from(offset).expect("an offset below 80 fits in u32"))
                .expect("a game ply count cannot overflow within 80 plies")
        })
        .collect::<Vec<_>>();
    plies.sort_unstable();
    let record_from = plies.last().map_or(opening_ply, |&last| {
        last.checked_add(1)
            .expect("a game ply count cannot overflow after an injection")
    });
    InjectionPlan { plies, record_from }
}

/// 決定的な8手から16手のランダム序盤を作る。
fn generate_opening(rules: Rules, game_seed: NonZeroU64) -> io::Result<Game> {
    let mut opening_seed = derive_seed(game_seed.get(), 0);
    loop {
        let mut game = Game::new(rules);
        let mut rng = XorShift64::new(opening_seed);
        let opening_plies = 8 + rng.index(NonZeroUsize::new(9).unwrap());
        let mut finished = false;
        for _ in 0..opening_plies {
            let legal_moves = game.legal_moves();
            if legal_moves.is_empty() {
                return Err(invalid_data(
                    "an opening game is ongoing but has no legal moves",
                ));
            }
            let move_count = NonZeroUsize::new(legal_moves.len())
                .expect("the empty move list was rejected above");
            let selected = legal_moves[rng.index(move_count)];
            let status = game.play(selected).map_err(|error| {
                invalid_data(format!("random opening move was rejected: {error}"))
            })?;
            if matches!(status, GameStatus::Finished(_)) {
                finished = true;
                break;
            }
        }
        if !finished {
            return Ok(game);
        }
        opening_seed = derive_seed(opening_seed.get(), 0);
    }
}

/// 現局面の探索キーが同じ対局の過去に現れているかを返す。
pub(crate) fn current_position_is_repeated(game: &Game) -> bool {
    let history = game.search_key_history();
    let Some((current, previous)) = history.split_last() else {
        return false;
    };
    previous.contains(current)
}

/// 探索値の度数表から平均と母標準偏差を返す。
fn score_mean_and_std(frequencies: &[u64]) -> Option<(f64, f64)> {
    let mut count = 0_u64;
    let mut sum = 0.0;
    let mut squared_sum = 0.0;
    for (index, &frequency) in frequencies.iter().enumerate() {
        if frequency == 0 {
            continue;
        }
        let score = index as f64 + f64::from(i16::MIN);
        count += frequency;
        let frequency_as_f64 = frequency as f64;
        sum += score * frequency_as_f64;
        squared_sum += score * score * frequency_as_f64;
    }
    if count == 0 {
        return None;
    }
    let count = count as f64;
    let mean = sum / count;
    let variance = (squared_sum / count - mean * mean).max(0.0);
    Some((mean, variance.sqrt()))
}

/// 累積度数が指定百分率以上となる最初の探索値を返す。
fn score_percentile(frequencies: &[u64], percentile: u8) -> Option<i16> {
    let count = frequencies
        .iter()
        .map(|&frequency| u128::from(frequency))
        .sum::<u128>();
    if count == 0 {
        return None;
    }
    let target = (count * u128::from(percentile)).div_ceil(100);
    let mut cumulative = 0_u128;
    for (index, &frequency) in frequencies.iter().enumerate() {
        cumulative += u128::from(frequency);
        if cumulative >= target {
            let score =
                i32::try_from(index).expect("a score index fits in i32") + i32::from(i16::MIN);
            return Some(i16::try_from(score).expect("a score-table index maps to i16"));
        }
    }
    unreachable!("the cumulative frequency reaches the total count")
}

/// 件数を百分率へ変換し、分母が0なら`n/a`を返す。
fn format_rate_percent(count: u64, total: u64) -> String {
    if total == 0 {
        "n/a".to_owned()
    } else {
        format!("{:.6}", count as f64 * 100.0 / total as f64)
    }
}

/// 生成統計を設計書へ転記できる1項目1行の形式で表示する。
fn print_generation_summary(
    arguments: &GenerateArguments,
    rule_set: &str,
    generation_commit: &str,
    stats: &Statistics,
    recorded: &RecordedStatistics,
    elapsed_seconds: f64,
) {
    let game_count = stats.black_wins + stats.white_wins + stats.draws + stats.discarded_games;
    let games = game_count as f64;
    let per_second = |count: u64| {
        if elapsed_seconds == 0.0 {
            0.0
        } else {
            count as f64 / elapsed_seconds
        }
    };

    println!("summary:");
    println!("seed: {}", arguments.seed);
    println!("games: {game_count}");
    println!("nodes: {}", arguments.nodes);
    println!("concurrency: {}", arguments.concurrency);
    println!("max_ply: {}", arguments.max_ply);
    println!("hash_mb: {}", arguments.hash_mb);
    println!("random_moves_max: {}", arguments.random_moves);
    println!("rules: {rule_set}");
    println!("commit: {generation_commit}");
    println!("games_completed: {game_count}");
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
        let start = index * 10;
        println!("injection_offset_histogram_{start}_{}: {count}", start + 9);
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
    match score_mean_and_std(&recorded.score_frequencies) {
        Some((mean, standard_deviation)) => {
            println!("score_mean: {mean:.6}");
            println!("score_std: {standard_deviation:.6}");
            for percentile in [1, 5, 25, 50, 75, 95, 99] {
                let score = score_percentile(&recorded.score_frequencies, percentile)
                    .expect("a non-empty score table has every requested percentile");
                println!("score_p{percentile:02}: {score}");
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
        per_second(stats.searched_positions)
    );
    println!(
        "recorded_positions_per_second: {:.6}",
        per_second(stats.recorded_positions)
    );
    println!("searched_nodes_total: {}", stats.searched_nodes);
}

/// 学習データを全件復号し、ヘッダと内容の要約を表示する。
fn inspect(arguments: &InspectArguments) -> io::Result<()> {
    let file = File::open(&arguments.path)?;
    let mut reader = Reader::new(file).map_err(training_error)?;
    let header = reader.header();
    println!("header:");
    println!("magic: MNSD");
    println!(
        "format_version: {}",
        minase::eval::training_data::FORMAT_VERSION
    );
    println!("record_length: {}", minase::eval::training_data::RECORD_LEN);
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

/// 下位エラーの型を保持してコマンドの不正データエラーへ変換する。
pub(crate) fn data_error(error: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

/// 付け直しコマンドの進行に関する失敗。
#[derive(Debug)]
enum RescoreCommandError {
    /// ワーカーが停止した。
    WorkerStopped,
    /// 探索ワーカーがパニックした。
    WorkerPanicked,
    /// 総ノード数の桁あふれ。
    NodeCountOverflow,
    /// 来歴コマンドが扱わない棋譜由来の指定。
    UnsupportedOrigin,
}

impl std::fmt::Display for RescoreCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::WorkerStopped => "rescore worker stopped before returning a result",
            Self::WorkerPanicked => "rescore worker panicked",
            Self::NodeCountOverflow => "rescore node count overflow",
            Self::UnsupportedOrigin => "provenance requires --result-origin selfplay and --start-origin random; game mappings belong to the game converter",
        })
    }
}

impl std::error::Error for RescoreCommandError {}

/// 元MNSDから教師情報を取り、明示された由来とともに来歴を書き出す。
fn write_provenance(arguments: &ProvenanceArguments) -> io::Result<()> {
    if arguments.result_origin != ResultOrigin::Selfplay
        || arguments.start_origin != StartOrigin::Random
    {
        return Err(data_error(RescoreCommandError::UnsupportedOrigin));
    }
    write_mapped_provenance(arguments, None)
}

/// 対局番号と実戦棋譜の対応を含めて来歴を書く。
pub(crate) fn write_mapped_provenance(
    arguments: &ProvenanceArguments,
    games: Option<Vec<GameOrigin>>,
) -> io::Result<()> {
    let mut input = File::open(&arguments.input)?;
    let checksum = rescore::sha256(&mut input)?;
    let reader = Reader::new(BufReader::new(input)).map_err(data_error)?;
    let header = reader.header();
    let provenance = Provenance {
        format: "minase-provenance".to_owned(),
        version: 1,
        mnsd_sha256: hex(&checksum),
        teacher: Teacher {
            generation_commit: header.generation_commit().to_owned(),
            network_checksum: hex(header.network_checksum()),
            nodes: header.teacher_nodes(),
            rule_set: header.rule_set().to_owned(),
            search_condition: arguments.search_condition,
        },
        result_origin: arguments.result_origin,
        start_origin: arguments.start_origin,
        lambda: arguments.lambda,
        games,
    };
    provenance.validate().map_err(data_error)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&arguments.output)?;
    provenance.write(&mut output).map_err(data_error)?;
    output.flush()
}

/// 付け直し全体の集計。再開前の記録も合計に含める。
#[derive(Default)]
struct RescoreSummary {
    rescored: u64,
    incomplete: u64,
    nodes: u64,
}

impl RescoreSummary {
    fn add(&mut self, entry: RescoreEntry) -> io::Result<()> {
        match entry.status() {
            RescoreStatus::NotTarget => (),
            RescoreStatus::Rescored => self.rescored += 1,
            RescoreStatus::Incomplete => self.incomplete += 1,
        }
        self.nodes = self
            .nodes
            .checked_add(entry.nodes())
            .ok_or_else(|| data_error(RescoreCommandError::NodeCountOverflow))?;
        Ok(())
    }
}

/// 既存MNRSは全条件と書き込み済み記録を検査し、新規ならヘッダを書く。
fn open_rescore_output(
    path: &Path,
    header: &RescoreHeader,
    targets: &[u64],
    summary: &mut RescoreSummary,
) -> io::Result<(File, u64)> {
    let encoded = header.encode().map_err(data_error)?;
    match OpenOptions::new().read(true).append(true).open(path) {
        Ok(file) => {
            let mut reader = RescoreReader::new(file).map_err(data_error)?;
            reader
                .header()
                .validate_resume(header)
                .map_err(data_error)?;
            let written = reader.written_count();
            for index in 0..written {
                let entry = reader
                    .read_entry()
                    .map_err(data_error)?
                    .ok_or_else(|| data_error(rescore::Error::InvalidLength))?;
                if (entry.status() != RescoreStatus::NotTarget)
                    != targets.binary_search(&index).is_ok()
                {
                    return Err(data_error(rescore::Error::TargetStatusMismatch(index)));
                }
                summary.add(entry)?;
            }
            Ok((reader.into_inner(), written))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&encoded)?;
            file.flush()?;
            Ok((file, 0))
        }
        Err(error) => Err(error),
    }
}

/// 履歴を復元せず、置換表を消去して1局面を探索する。
fn rescore_record(
    record: &Record,
    pst: &Pst,
    rules: Rules,
    limits: &SearchLimits,
    table: &mut TranspositionTable,
) -> io::Result<RescoreEntry> {
    let position = record.to_position().map_err(data_error)?;
    let game = Game::from_position(rules, position);
    let snapshot = SearchSnapshot::from_game(&game).map_err(data_error)?;
    table.clear();
    let result = search(pst, &snapshot, limits, DEFAULT_THREADS, table).map_err(data_error)?;
    if result.depth == 0 {
        return Ok(RescoreEntry::incomplete(result.nodes));
    }
    let tactical = best_move_is_tactical(
        game.position(),
        &MoveGenerator::new(rules.moves),
        result.best_move,
    )
    .map_err(data_error)?;
    let score = i16::try_from(result.score).map_err(data_error)?;
    RescoreEntry::rescored(tactical, score, result.depth, result.nodes).map_err(data_error)
}

/// 局面単位で並列化し、最大ワーカー数件の記録を番号順に書く。
///
/// 段階9「付け直しの探索の条件」。ワーカーは各自の表を使い回す。
/// 入力、未出力の結果ともに全件をメモリへ載せない。
fn write_rescore_records(
    reader: &mut Reader<BufReader<File>>,
    output: &mut File,
    arguments: &RescoreArguments,
    targets: &[u64],
    written: u64,
    pst: &Pst,
    summary: &mut RescoreSummary,
) -> io::Result<()> {
    let rules = engine_default_rules()?;
    let limits = SearchLimits::new(None, Some(u64::from(arguments.nodes)), None, None)
        .map_err(data_error)?;
    for _ in 0..written {
        reader.read_record().map_err(data_error)?;
    }
    let remaining = reader.header().record_count() - written;
    let workers =
        usize::try_from(remaining.min(arguments.concurrency.get() as u64)).map_err(data_error)?;
    thread::scope(|scope| -> io::Result<()> {
        let (results_tx, results_rx) = mpsc::channel();
        let mut jobs = Vec::new();
        for _ in 0..workers {
            let mut table = TranspositionTable::new(arguments.hash_mb.get()).map_err(data_error)?;
            let (job_tx, job_rx) = mpsc::sync_channel::<(usize, Record)>(1);
            jobs.push(job_tx);
            let results_tx = results_tx.clone();
            let limits = &limits;
            thread::Builder::new().spawn_scoped(scope, move || {
                while let Ok((slot, record)) = job_rx.recv() {
                    let result = match catch_unwind(AssertUnwindSafe(|| {
                        rescore_record(&record, pst, rules, limits, &mut table)
                    })) {
                        Ok(result) => result,
                        Err(_) => Err(data_error(RescoreCommandError::WorkerPanicked)),
                    };
                    let failed = result.is_err();
                    if results_tx.send((slot, result)).is_err() || failed {
                        break;
                    }
                }
            })?;
        }
        drop(results_tx);
        let mut index = written;
        let progress_interval = reader.header().record_count().div_ceil(20).max(1);
        while index < reader.header().record_count() {
            let mut entries = Vec::with_capacity(workers);
            let mut searching = 0;
            for job in &jobs {
                let Some(record) = reader.read_record().map_err(data_error)? else {
                    break;
                };
                let slot = entries.len();
                entries.push(RescoreEntry::not_target());
                if targets.binary_search(&(index + slot as u64)).is_ok() {
                    job.send((slot, record))
                        .map_err(|_| data_error(RescoreCommandError::WorkerStopped))?;
                    searching += 1;
                }
            }
            for _ in 0..searching {
                let (slot, entry) = results_rx
                    .recv()
                    .map_err(|_| data_error(RescoreCommandError::WorkerStopped))?;
                entries[slot] = entry?;
            }
            for entry in entries {
                // 1記録を1回のwrite_allで追記し、その記録を直ちにflushする。
                output.write_all(&entry.encode())?;
                output.flush()?;
                summary.add(entry)?;
                index += 1;
                if index.is_multiple_of(progress_interval)
                    || index == reader.header().record_count()
                {
                    eprintln!(
                        "progress: records={index}/{}",
                        reader.header().record_count()
                    );
                }
            }
        }
        Ok(())
    })
}

/// 実行条件を固定し、保存済みMNSDを読み取り専用で付け直す。
fn rescore(arguments: &RescoreArguments) -> io::Result<()> {
    let start = Instant::now();
    let generation_commit = git_output(&["rev-parse", "HEAD"])?;
    validate_commit_hash(&generation_commit)?;
    if !git_output(&["status", "--porcelain"])?.is_empty() && !arguments.allow_dirty {
        return Err(invalid_data(
            "the working tree is dirty; commit changes or pass --allow-dirty",
        ));
    }
    let pst = match &arguments.pst {
        Some(path) => std::sync::Arc::new(Pst::decode(&fs::read(path)?).map_err(data_error)?),
        None => minase::eval::weights().map_err(data_error)?,
    };
    let mut input = File::open(&arguments.input)?;
    let mnsd_sha256 = rescore::sha256(&mut input)?;
    input.seek(SeekFrom::Start(0))?;
    let mut reader = Reader::new(BufReader::new(input)).map_err(data_error)?;
    let targets = rescore::read_targets(
        BufReader::new(File::open(&arguments.targets)?),
        reader.header().record_count(),
    )
    .map_err(data_error)?;
    let header = RescoreHeader {
        mnsd_sha256,
        record_count: reader.header().record_count(),
        targets_sha256: rescore::targets_checksum(&targets),
        target_count: targets.len() as u64,
        network_checksum: *pst.checksum(),
        nodes: arguments.nodes,
        // MNSDのヘッダと同じ表記（規則コードの列挙）で書く。
        rule_set: engine_default_rules()?.to_string(),
        hash_mb: u32::try_from(arguments.hash_mb.get()).map_err(data_error)?,
        generation_commit,
        binary_sha256: rescore::sha256(File::open(std::env::current_exe()?)?)?,
    };
    let mut summary = RescoreSummary::default();
    let (mut output, written) =
        open_rescore_output(&arguments.output, &header, &targets, &mut summary)?;
    let previous = summary.rescored + summary.incomplete;
    if written == header.record_count {
        println!("already_complete: true");
    } else {
        write_rescore_records(
            &mut reader,
            &mut output,
            arguments,
            &targets,
            written,
            &pst,
            &mut summary,
        )?;
    }
    let elapsed = start.elapsed().as_secs_f64();
    let searched = summary.rescored + summary.incomplete - previous;
    println!("targets: {}", header.target_count);
    println!("rescored: {}", summary.rescored);
    println!("incomplete_depth_one: {}", summary.incomplete);
    println!("searched_nodes: {}", summary.nodes);
    println!("elapsed_seconds: {elapsed:.3}");
    println!(
        "mean_seconds_per_position: {:.6}",
        if searched == 0 {
            0.0
        } else {
            elapsed / searched as f64
        }
    );
    Ok(())
}

/// `engine-default`を公開規則解析APIから構築する。
fn engine_default_rules() -> io::Result<Rules> {
    let codes =
        parse_rule_set("engine-default").map_err(|error| invalid_data(error.to_string()))?;
    Rules::from_codes(&codes).map_err(|error| invalid_data(error.to_string()))
}

/// gitサブコマンドを実行し、末尾の改行を除いた標準出力を返す。
pub(crate) fn git_output(arguments: &[&str]) -> io::Result<String> {
    let output = Command::new("git").args(arguments).output()?;
    if !output.status.success() {
        return Err(invalid_data(format!(
            "git {} failed with status {}: {}",
            arguments.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim_end_matches(['\r', '\n']).to_owned())
        .map_err(|error| invalid_data(format!("git output is not UTF-8: {error}")))
}

/// 生成コミットが40桁の16進ASCIIであることを検査する。
pub(crate) fn validate_commit_hash(commit: &str) -> io::Result<()> {
    if commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(invalid_data(format!(
            "git rev-parse HEAD returned an invalid full hash: {commit:?}"
        )))
    }
}

/// バイト列を小文字16進文字列へ変換する。
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        text.push(DIGITS[usize::from(byte >> 4)] as char);
        text.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    text
}

/// 0より大きい`u32`を解析する。
fn parse_positive_u32(text: &str) -> Result<u32, String> {
    let value = text
        .parse::<u32>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if value == 0 {
        Err("value must be greater than zero".to_owned())
    } else {
        Ok(value)
    }
}

/// 0より大きい`u16`を解析する。
fn parse_positive_u16(text: &str) -> Result<u16, String> {
    let value = text
        .parse::<u16>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if value == 0 {
        Err("value must be greater than zero".to_owned())
    } else {
        Ok(value)
    }
}

/// 0より大きい`usize`を解析する。
fn parse_positive_usize(text: &str) -> Result<NonZeroUsize, String> {
    let value = text
        .parse::<usize>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    NonZeroUsize::new(value).ok_or_else(|| "value must be greater than zero".to_owned())
}

/// 学習データエラーをコマンドの不正データエラーへ変換する。
fn training_error(error: TrainingDataError) -> io::Error {
    invalid_data(error.to_string())
}

/// 説明を`InvalidData`の入出力エラーへ変換する。
fn invalid_data(error: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// 小さなMNSDを含む一時ディレクトリ。各テストの入出力を分離する。
    struct RescoreFixture {
        directory: PathBuf,
        arguments: RescoreArguments,
    }

    impl RescoreFixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let directory = std::env::temp_dir().join(format!(
                "minase-rescore-{}-{}",
                process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&directory).unwrap();
            let input = directory.join("input.mnsd");
            let targets = directory.join("targets.txt");
            fs::write(&targets, b"0\n2\n4\n").unwrap();
            let header = Header::new(
                "engine-default".into(),
                "a".repeat(40),
                [7; 32],
                100_000,
                42,
                0,
            )
            .unwrap();
            let mut writer = Writer::new(File::create(&input).unwrap(), header).unwrap();
            // 同じ局面を別の記録番号にも置き、以前の探索結果からの独立性を検証する。
            for index in 0..5 {
                writer
                    .write_record(&Record::from_position(
                        &Position::initial(),
                        17,
                        Outcome::Draw,
                        1,
                        index,
                    ))
                    .unwrap();
            }
            writer.finish().unwrap();
            let arguments = RescoreArguments {
                input,
                targets,
                output: directory.join("output.mnrs"),
                pst: None,
                nodes: 200,
                hash_mb: NonZeroUsize::new(1).unwrap(),
                concurrency: NonZeroUsize::new(1).unwrap(),
                allow_dirty: true,
            };
            Self {
                directory,
                arguments,
            }
        }
    }

    impl Drop for RescoreFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    // 指示書D: 同じ条件、局面ごとの独立性、並列数によらない記録順、入力の不変性。
    #[test]
    fn rescore_is_reproducible_parallel_and_preserves_source() {
        let mut fixture = RescoreFixture::new();
        // 置換表の再利用が結果へ影響し得る深さまで探索する。
        fixture.arguments.nodes = 3_000;
        let before = rescore::sha256(File::open(&fixture.arguments.input).unwrap()).unwrap();
        rescore(&fixture.arguments).unwrap();
        let first = fs::read(&fixture.arguments.output).unwrap();
        fixture.arguments.output = fixture.directory.join("parallel.mnrs");
        fixture.arguments.concurrency = NonZeroUsize::new(3).unwrap();
        rescore(&fixture.arguments).unwrap();
        let second = fs::read(&fixture.arguments.output).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            rescore::sha256(File::open(&fixture.arguments.input).unwrap()).unwrap(),
            before
        );
        let mut reader = RescoreReader::new(Cursor::new(first)).unwrap();
        assert!(reader.is_complete());
        assert_eq!(reader.header().mnsd_sha256, before);
        assert_eq!(reader.header().target_count, 3);
        assert_eq!(
            reader.header().targets_sha256,
            rescore::targets_checksum(&[0, 2, 4])
        );
        assert_eq!(
            reader.header().network_checksum,
            *minase::eval::weights().unwrap().checksum()
        );
        assert_eq!(reader.header().rule_set, "L0,P0,R1,E0");
        let entries: Vec<_> = (0..5)
            .map(|_| reader.read_entry().unwrap().unwrap())
            .collect();
        assert_eq!(entries[0], entries[2]);
        assert_eq!(entries[2], entries[4]);
        assert_eq!(entries[0].status(), RescoreStatus::Rescored);
        assert!(entries[0].depth() >= 1);
        assert!(entries[0].nodes() > 0);
        assert_eq!(entries[1].encode(), [0; 16]);
        assert_eq!(entries[3].encode(), [0; 16]);
    }

    // 指示書D: 記録境界で中断した場合だけ再開し、無中断の出力に一致する。
    #[test]
    fn rescore_resumes_exactly_and_rejects_condition_changes_and_torn_entries() {
        let mut fixture = RescoreFixture::new();
        rescore(&fixture.arguments).unwrap();
        let complete = fs::read(&fixture.arguments.output).unwrap();
        for count in [0, 1, 3, 5] {
            fs::write(&fixture.arguments.output, &complete[..240 + count * 16]).unwrap();
            rescore(&fixture.arguments).unwrap();
            assert_eq!(fs::read(&fixture.arguments.output).unwrap(), complete);
        }
        let interrupted = &complete[..240 + 16];
        fs::write(&fixture.arguments.output, interrupted).unwrap();
        fixture.arguments.nodes = 201;
        let error = rescore(&fixture.arguments).unwrap_err();
        assert!(matches!(
            error.get_ref().unwrap().downcast_ref::<rescore::Error>(),
            Some(rescore::Error::HeaderMismatch("nodes"))
        ));
        assert_eq!(fs::read(&fixture.arguments.output).unwrap(), interrupted);
        fixture.arguments.nodes = 200;
        for (offset, expected) in [
            (8, "mnsd_sha256"),
            (40, "record_count"),
            (204, "binary_sha256"),
        ] {
            let mut changed = interrupted.to_vec();
            changed[offset] ^= 1;
            fs::write(&fixture.arguments.output, &changed).unwrap();
            let error = rescore(&fixture.arguments).unwrap_err();
            assert!(
                matches!(error.get_ref().unwrap().downcast_ref::<rescore::Error>(), Some(rescore::Error::HeaderMismatch(field)) if *field == expected)
            );
            assert_eq!(fs::read(&fixture.arguments.output).unwrap(), changed);
        }
        fs::write(&fixture.arguments.output, &complete[..240 + 17]).unwrap();
        let error = rescore(&fixture.arguments).unwrap_err();
        assert!(matches!(
            error.get_ref().unwrap().downcast_ref::<rescore::Error>(),
            Some(rescore::Error::InvalidLength)
        ));
    }

    #[test]
    fn rescore_marks_unfinished_depth_and_rejects_positions_without_legal_moves() {
        let mut fixture = RescoreFixture::new();
        fixture.arguments.nodes = 1;
        rescore(&fixture.arguments).unwrap();
        let mut reader =
            RescoreReader::new(File::open(&fixture.arguments.output).unwrap()).unwrap();
        let entry = reader.read_entry().unwrap().unwrap();
        assert_eq!(entry.status(), RescoreStatus::Incomplete);
        assert_eq!(entry.score(), 0);
        assert_eq!(entry.depth(), 0);
        assert!(!entry.tactical());
        assert!(entry.nodes() > 0);
        let empty = Record::from_position(&Position::empty(Color::Black), 0, Outcome::Draw, 1, 0);
        let limits = SearchLimits::new(None, Some(200), None, None).unwrap();
        let error = rescore_record(
            &empty,
            &minase::eval::weights().unwrap(),
            engine_default_rules().unwrap(),
            &limits,
            &mut test_table(),
        )
        .unwrap_err();
        assert!(matches!(
            error
                .get_ref()
                .unwrap()
                .downcast_ref::<minase::search::SearchError>(),
            Some(minase::search::SearchError::NoLegalMoves)
        ));
    }

    #[test]
    fn provenance_command_uses_header_and_refuses_overwrite_or_implicit_origins() {
        let fixture = RescoreFixture::new();
        let mut arguments = ProvenanceArguments {
            input: fixture.arguments.input.clone(),
            output: fixture.directory.join("provenance.json"),
            result_origin: ResultOrigin::Selfplay,
            start_origin: StartOrigin::Random,
            lambda: 0.75,
            search_condition: SearchCondition::InGame,
        };
        write_provenance(&arguments).unwrap();
        let provenance = Provenance::read(File::open(&arguments.output).unwrap()).unwrap();
        assert_eq!(
            provenance.mnsd_sha256,
            hex(&rescore::sha256(File::open(&arguments.input).unwrap()).unwrap())
        );
        assert_eq!(provenance.teacher.generation_commit, "a".repeat(40));
        assert_eq!(provenance.teacher.network_checksum, hex(&[7; 32]));
        assert_eq!(provenance.teacher.nodes, 100_000);
        assert_eq!(provenance.lambda, 0.75);
        assert!(provenance.games.is_none());
        assert_eq!(
            write_provenance(&arguments).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        arguments.output = fixture.directory.join("invalid.json");
        arguments.start_origin = StartOrigin::HumanGame;
        assert!(write_provenance(&arguments).is_err());
        assert!(!arguments.output.exists());
        for required in ["--result-origin", "--start-origin", "--lambda"] {
            let mut args = vec![
                "selfplay_gen",
                "provenance",
                "--input",
                "in",
                "--output",
                "out",
                "--result-origin",
                "selfplay",
                "--start-origin",
                "random",
                "--lambda",
                "0.75",
            ];
            let index = args.iter().position(|&value| value == required).unwrap();
            args.drain(index..index + 2);
            assert!(Arguments::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn generate_emits_provenance_after_completing_mnsd() {
        let fixture = RescoreFixture::new();
        let arguments = GenerateArguments {
            output: fixture.directory.join("generated.mnsd"),
            games: Some(1),
            openings: None,
            seed: 42,
            nodes: 1,
            random_moves: 0,
            concurrency: NonZeroUsize::new(1).unwrap(),
            max_ply: 1,
            hash_mb: NonZeroUsize::new(1).unwrap(),
            allow_dirty: true,
        };
        generate(&arguments).unwrap();
        let path = fixture.directory.join("generated.mnsd.provenance.json");
        let provenance = Provenance::read(File::open(path).unwrap()).unwrap();
        let reader = Reader::new(File::open(&arguments.output).unwrap()).unwrap();
        assert_eq!(
            provenance.teacher.generation_commit,
            reader.header().generation_commit()
        );
        assert_eq!(
            provenance.teacher.network_checksum,
            hex(reader.header().network_checksum())
        );
        assert_eq!(
            provenance.mnsd_sha256,
            hex(&rescore::sha256(File::open(&arguments.output).unwrap()).unwrap())
        );
        assert_eq!(provenance.teacher.search_condition, SearchCondition::InGame);
        assert_eq!(provenance.result_origin, ResultOrigin::Selfplay);
        assert_eq!(provenance.start_origin, StartOrigin::Random);
    }

    // フェーズ5: 一覧を最初の探索へそのまま渡し、開始局面だけは記録しない。
    #[test]
    fn human_opening_is_exact_and_records_start_after_its_original_ply() {
        use minase::notation::{
            sfen::{SetupPosition, to_extended_sfen},
            usi,
        };
        let fixture = RescoreFixture::new();
        let input: serde_json::Value =
            include_str!("../../tests/fixtures/lishogi_import_cases.ndjson")
                .lines()
                .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                .find(|game| game["id"] == "opening-prefix-2u7dwJf9")
                .unwrap();
        let mut original = Game::new(Rules::ENGINE_DEFAULT);
        for text in input["moves"].as_str().unwrap().split_whitespace().take(60) {
            original
                .play(usi::parse(original.position(), text).unwrap())
                .unwrap();
        }
        let setup = SetupPosition::new(
            original.position().clone(),
            original.position().lion_capture_square(),
            61,
        )
        .unwrap();
        let path = fixture.directory.join("openings.txt");
        fs::write(&path, format!("source 60 {}\n", to_extended_sfen(&setup))).unwrap();
        let openings = read_openings(&path).unwrap();
        let start = starting_game(
            Rules::ENGINE_DEFAULT,
            derive_seed(42, 1),
            Some(&openings[0]),
        )
        .unwrap();
        assert_eq!(start.position(), original.position());
        assert_eq!(start.ply_count(), 0);
        let completed = play_game_from_opening(
            &minase::eval::weights().unwrap(),
            Rules::ENGINE_DEFAULT,
            1,
            PlaySettings {
                base_seed: 42,
                nodes: 200,
                random_moves: 0,
                max_ply: 4000,
            },
            &mut test_table(),
            Some(&openings[0]),
        )
        .unwrap();
        assert_eq!(completed.stats.discarded_games, 0);
        assert_eq!(completed.stats.planned_injections, 0);
        assert!(!completed.records.is_empty());
        assert!(completed.records.iter().all(|r| r.record.ply() > 60));
    }

    /// テスト用の小さい置換表を作る。
    fn test_table() -> TranspositionTable {
        TranspositionTable::new(1).expect("one MiB is a valid table size")
    }

    /// テスト用の対局設定を返す。
    const fn test_settings(base_seed: u64, random_moves: u8, max_ply: u16) -> PlaySettings {
        PlaySettings {
            base_seed,
            nodes: 100,
            random_moves,
            max_ply,
        }
    }

    /// 指定条件の対局について序盤と注入計画を再現する。
    fn opening_and_plan(
        rules: Rules,
        base_seed: u64,
        game_number: u32,
        maximum: u8,
    ) -> (u32, InjectionPlan) {
        let game_seed = derive_seed(base_seed, u64::from(game_number));
        let game = generate_opening(rules, game_seed).expect("the fixed opening is valid");
        let opening_ply = game.ply_count();
        let (plan, _) = plan_injections(game_seed, opening_ply, maximum);
        (opening_ply, plan)
    }

    /// 同じ対局条件はレコードと全統計を再現する。
    #[test]
    fn play_game_is_deterministic_for_same_seed_and_arguments() {
        let rules = engine_default_rules().expect("engine-default rules are valid");
        let pst = minase::eval::weights().expect("embedded weights are valid");
        let mut first_table = test_table();
        let mut second_table = test_table();
        let settings = test_settings(7, 4, 600);
        let first = play_game(pst.as_ref(), rules, 1, settings, &mut first_table)
            .expect("the fixed game is valid");
        let second = play_game(pst.as_ref(), rules, 1, settings, &mut second_table)
            .expect("the fixed game is valid");

        assert!(!first.records.is_empty());
        assert_eq!(first, second);
        let (_, plan) = opening_and_plan(rules, 7, 1, 4);
        assert!(
            first
                .records
                .iter()
                .all(|completed| u32::from(completed.record.ply()) >= plan.record_from)
        );
        assert_eq!(
            first.stats.recordable_positions,
            first
                .stats
                .total_plies
                .saturating_sub(u64::from(plan.record_from))
        );
        assert_eq!(
            first.stats.recordable_positions,
            first.stats.recorded_positions
                + first.stats.excluded_mate_band
                + first.stats.excluded_tactical
                + first.stats.excluded_repetition
        );
    }

    /// 注入計画の途中で打ち切った対局でも実施回数と探索回数を正しく集計する。
    #[test]
    fn injection_counts_and_offsets_stay_within_configured_bounds() {
        let rules = engine_default_rules().expect("engine-default rules are valid");
        let pst = minase::eval::weights().expect("embedded weights are valid");
        let maximum = 80;

        let game_number = 1;
        let (opening_ply, plan) = opening_and_plan(rules, 19, game_number, maximum);
        let mut table = test_table();
        let completed = play_game(
            pst.as_ref(),
            rules,
            game_number,
            test_settings(19, maximum, 32),
            &mut table,
        )
        .expect("the fixed game is valid");

        assert!(plan.plies.len() <= usize::from(maximum));
        assert_eq!(
            completed.stats.planned_injections,
            u64::try_from(plan.plies.len()).unwrap()
        );
        assert!(completed.stats.performed_injections > 0);
        assert!(completed.stats.performed_injections < completed.stats.planned_injections);
        assert_eq!(completed.stats.total_plies, 32);
        assert!(u64::from(plan.record_from) > completed.stats.total_plies);
        assert!(completed.records.is_empty());
        assert_eq!(
            completed
                .stats
                .injection_offset_histogram
                .iter()
                .sum::<u64>(),
            completed.stats.performed_injections
        );

        let mut expected_histogram = [0_u64; INJECTION_HISTOGRAM_BINS];
        for &ply in plan
            .plies
            .iter()
            .filter(|&&ply| u64::from(ply) < completed.stats.total_plies)
        {
            let offset = usize::try_from(ply - opening_ply).unwrap();
            assert!(offset < INJECTION_WINDOW);
            expected_histogram[offset / 10] += 1;
        }
        assert_eq!(
            completed.stats.injection_offset_histogram,
            expected_histogram
        );
        assert_eq!(
            completed.stats.searched_plies + completed.stats.performed_injections,
            completed.stats.total_plies - u64::from(opening_ply)
        );
        assert_eq!(
            completed.stats.searched_positions,
            completed.stats.searched_plies
        );
    }

    /// 注入計画は0以上80未満の異なるオフセットと予定由来の境界を返す。
    #[test]
    fn injection_plan_has_unique_offsets_and_planned_boundary() {
        let opening_ply = 12;
        for number in 1..=32 {
            let game_seed = derive_seed(31, number);
            let (plan, _) = plan_injections(game_seed, opening_ply, 80);
            let unique = plan.plies.iter().copied().collect::<BTreeSet<_>>();

            assert_eq!(unique.len(), plan.plies.len());
            assert!(plan.plies.len() <= 80);
            assert!(
                plan.plies
                    .iter()
                    .all(|&ply| (opening_ply..opening_ply + 80).contains(&ply))
            );
            assert_eq!(
                plan.record_from,
                plan.plies.last().map_or(opening_ply, |&last| last + 1)
            );
        }
    }

    /// 上限0では注入を予定せず序盤終了局面から記録する。
    #[test]
    fn zero_random_moves_produces_empty_plan_at_opening_boundary() {
        let opening_ply = 12;
        let (plan, _) = plan_injections(derive_seed(41, 12), opening_ply, 0);
        assert!(plan.plies.is_empty());
        assert_eq!(plan.record_from, opening_ply);
    }

    /// 段階7の試行生成で選んだ4,000手を、省略時の手数上限とする。
    #[test]
    fn omitted_max_ply_uses_measured_generation_cap() {
        let arguments = Arguments::try_parse_from([
            "selfplay_gen",
            "generate",
            "--output",
            "unused.bin",
            "--games",
            "1",
            "--seed",
            "1",
            "--random-moves",
            "0",
        ])
        .expect("the required generation arguments are valid");
        let Operation::Generate(arguments) = arguments.command else {
            panic!("generate must select generation arguments");
        };

        assert_eq!(arguments.max_ply, 4_000);
    }

    /// CLIはランダム着手上限の0と80だけを境界値として受理する。
    #[test]
    fn random_moves_accepts_only_zero_through_eighty() {
        let arguments = |value: Option<&str>| {
            let mut input = vec![
                "selfplay_gen",
                "generate",
                "--output",
                "unused.bin",
                "--games",
                "1",
                "--seed",
                "1",
            ];
            if let Some(value) = value {
                input.extend(["--random-moves", value]);
            }
            Arguments::try_parse_from(input)
        };

        assert!(arguments(Some("0")).is_ok());
        assert!(arguments(Some("80")).is_ok());
        assert!(arguments(Some("81")).is_err());
        assert!(arguments(Some("-1")).is_err());
        assert!(arguments(None).is_err());
    }

    /// 探索値度数表から平均、母標準偏差、分位点を求める。
    #[test]
    fn score_distribution_uses_signed_order_and_nearest_rank() {
        let mut frequencies = vec![0_u64; SCORE_VALUE_COUNT];
        for score in [-2, 0, 2] {
            frequencies[score_index(score)] += 1;
        }

        let (mean, standard_deviation) =
            score_mean_and_std(&frequencies).expect("the table is non-empty");
        assert_eq!(mean, 0.0);
        assert!((standard_deviation - (8.0_f64 / 3.0).sqrt()).abs() < f64::EPSILON);
        assert_eq!(score_percentile(&frequencies, 1), Some(-2));
        assert_eq!(score_percentile(&frequencies, 50), Some(0));
        assert_eq!(score_percentile(&frequencies, 99), Some(2));
        assert_eq!(score_index(i16::MIN), 0);
        assert_eq!(score_index(i16::MAX), SCORE_VALUE_COUNT - 1);

        frequencies.fill(0);
        assert_eq!(score_mean_and_std(&frequencies), None);
        assert_eq!(score_percentile(&frequencies, 50), None);
    }

    /// テスト用の完了対局を1レコード付きで作る。
    fn completed_game(game_number: u32) -> CompletedGame {
        let game = Game::new(engine_default_rules().expect("engine-default rules are valid"));
        let score = i16::try_from(game_number).unwrap() - 2;
        let search_key = if game_number >= 3 {
            10
        } else {
            u64::from(game_number) * 10
        };
        CompletedGame {
            game_number,
            records: vec![CompletedRecord {
                record: Record::from_position(
                    game.position(),
                    score,
                    Outcome::Draw,
                    game_number,
                    game.ply_count().try_into().unwrap(),
                ),
                search_key,
            }],
            stats: Statistics {
                searched_positions: u64::from(game_number),
                recordable_positions: 1,
                recorded_positions: 1,
                searched_nodes: u64::from(game_number) * 100,
                ..Statistics::default()
            },
        }
    }

    /// 到着順を変えて統合し、完成バイト列と集計を返す。
    fn merge_in_order(order: &[u32]) -> (Vec<u8>, Statistics, Vec<u64>, u64) {
        let header = Header::new("L0,P0,R1,E0".to_owned(), "0".repeat(40), [0; 32], 100, 1, 0)
            .expect("the test header is valid");
        let mut writer =
            Writer::new(Cursor::new(Vec::new()), header).expect("the in-memory writer is valid");
        let messages = order.iter().map(|&number| Ok(completed_game(number)));
        let (statistics, recorded) = merge_completed_games(messages, &mut writer, 4, |_, _| {})
            .expect("all four games are present");
        let bytes = writer
            .finish()
            .expect("the in-memory writer finishes")
            .into_inner();
        (
            bytes,
            statistics,
            recorded.score_frequencies,
            recorded.duplicate_positions,
        )
    }

    /// 同じ対局集合は到着順によらず対局番号順のデータと集計になる。
    #[test]
    fn completed_games_are_merged_independently_of_arrival_order() {
        let sequential = merge_in_order(&[1, 2, 3, 4]);
        let shuffled = merge_in_order(&[4, 2, 1, 3]);

        assert_eq!(sequential, shuffled);
        assert_eq!(sequential.1.recorded_positions, 4);
        assert_eq!(sequential.2.iter().sum::<u64>(), 4);
        assert_eq!(sequential.3, 2);
    }

    /// 対局番号が欠けたまま入力が終われば統合を拒否する。
    #[test]
    fn completed_game_merge_rejects_a_missing_game_number() {
        let header = Header::new("L0,P0,R1,E0".to_owned(), "0".repeat(40), [0; 32], 100, 1, 0)
            .expect("the test header is valid");
        let mut writer =
            Writer::new(Cursor::new(Vec::new()), header).expect("the in-memory writer is valid");
        let messages = [1, 3, 4].map(|number| Ok(completed_game(number)));

        let error = match merge_completed_games(messages, &mut writer, 4, |_, _| {}) {
            Ok(_) => panic!("game 2 is missing"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    /// 除外率は分母0なら数値ではなく`n/a`になる。
    #[test]
    fn rate_percent_is_not_available_for_zero_denominator() {
        assert_eq!(format_rate_percent(0, 0), "n/a");
        assert_eq!(format_rate_percent(1, 4), "25.000000");
    }
}
