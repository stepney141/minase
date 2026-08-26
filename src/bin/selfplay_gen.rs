//! 評価関数の学習に使う自己対局データの生成と検査。

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, Write};
use std::num::{NonZeroU64, NonZeroUsize};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::process::{self, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use clap::{Parser, Subcommand};
use minase::core::rules::parse_rule_set;
use minase::eval::Pst;
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
const DEFAULT_MAX_PLY: u16 = 600;
/// ワーカーごとの既定置換表容量。
const DEFAULT_HASH_MB: NonZeroUsize = NonZeroUsize::new(16).unwrap();
/// 詰み帯として除外する探索値の絶対値下限。
const MATE_BAND_START: u32 = 29_000;
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
}

/// `generate`サブコマンドの引数。
#[derive(clap::Args)]
struct GenerateArguments {
    /// 新規作成する出力ファイル。
    #[arg(long, required = true)]
    output: PathBuf,
    /// 生成する対局数。
    #[arg(long, required = true, value_parser = parse_positive_u32)]
    games: u32,
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
struct CompletedRecord {
    /// 固定長形式へ書き出すレコード。
    record: Record,
    /// 対局間の局面重複を判定する探索キー。
    search_key: u64,
}

/// 1局分のレコードと統計。
#[derive(PartialEq, Eq, Debug)]
struct CompletedGame {
    /// 1から始まる対局番号。
    game_number: u32,
    /// 終局対局から採用したレコード。
    records: Vec<CompletedRecord>,
    /// 打ち切り対局を含む生成統計。
    stats: Statistics,
}

/// データ生成全体または1局分の統計。
#[derive(Default, PartialEq, Eq, Debug)]
struct Statistics {
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
    recorded_positions: u64,
    /// 詰み帯の探索値による除外数。
    excluded_mate_band: u64,
    /// 捕獲または成りの最善手による除外数。
    excluded_tactical: u64,
    /// 現局面の再出現による除外数。
    excluded_repetition: u64,
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
struct RecordedStatistics {
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
    }
}

/// 来歴を確定して出力ファイルを作り、生成が失敗すれば出力を削除する。
fn generate(arguments: &GenerateArguments) -> io::Result<()> {
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
    let result = write_training_data(file, arguments, rules, &rule_set, &generation_commit);
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
    result
}

/// 自己対局を並列実行し、対局番号順にレコードを書き出して要約を表示する。
fn write_training_data(
    file: File,
    arguments: &GenerateArguments,
    rules: Rules,
    rule_set: &str,
    generation_commit: &str,
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
    let progress_interval = u64::from(arguments.games).div_ceil(20).max(1);
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
                    if game_number > u64::from(arguments.games) {
                        break;
                    }
                    let game_number = match u32::try_from(game_number) {
                        Ok(number) => number,
                        Err(error) => {
                            let _ = sender.send(Err(invalid_data(error.to_string())));
                            break;
                        }
                    };
                    let completed = match catch_unwind(AssertUnwindSafe(|| {
                        play_game(pst, rules, game_number, play_settings, &mut table)
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

        merge_completed_games(
            receiver,
            &mut writer,
            arguments.games,
            |completed_count, total| {
                if completed_count.is_multiple_of(progress_interval)
                    || completed_count == u64::from(arguments.games)
                {
                    eprintln!(
                        "progress: games={completed_count}/{} records={} elapsed_seconds={:.3}",
                        arguments.games,
                        total.recorded_positions,
                        start.elapsed().as_secs_f64()
                    );
                }
            },
        )
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
fn merge_completed_games<I, W, F>(
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
    let game_seed = derive_seed(settings.base_seed, u64::from(game_number));
    let mut game = generate_opening(rules, game_seed)?;
    let opening_ply = game.ply_count();
    let (injection_plan, mut injection_rng) =
        plan_injections(game_seed, opening_ply, settings.random_moves);
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

    while game.result().is_none() && game.ply_count() < u32::from(settings.max_ply) {
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
            let ply = u16::try_from(game.ply_count()).map_err(|_| {
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

    stats.total_plies = u64::from(game.ply_count());
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
fn current_position_is_repeated(game: &Game) -> bool {
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
    let games = f64::from(arguments.games);
    let per_second = |count: u64| {
        if elapsed_seconds == 0.0 {
            0.0
        } else {
            count as f64 / elapsed_seconds
        }
    };

    println!("summary:");
    println!("seed: {}", arguments.seed);
    println!("games: {}", arguments.games);
    println!("nodes: {}", arguments.nodes);
    println!("concurrency: {}", arguments.concurrency);
    println!("max_ply: {}", arguments.max_ply);
    println!("hash_mb: {}", arguments.hash_mb);
    println!("random_moves_max: {}", arguments.random_moves);
    println!("rules: {rule_set}");
    println!("commit: {generation_commit}");
    println!("games_completed: {}", arguments.games);
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

/// `engine-default`を公開規則解析APIから構築する。
fn engine_default_rules() -> io::Result<Rules> {
    let codes =
        parse_rule_set("engine-default").map_err(|error| invalid_data(error.to_string()))?;
    Rules::from_codes(&codes).map_err(|error| invalid_data(error.to_string()))
}

/// gitサブコマンドを実行し、末尾の改行を除いた標準出力を返す。
fn git_output(arguments: &[&str]) -> io::Result<String> {
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
fn validate_commit_hash(commit: &str) -> io::Result<()> {
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
    }

    /// 複数局で注入回数と実施オフセットが指定範囲に収まる。
    #[test]
    fn injection_counts_and_offsets_stay_within_configured_bounds() {
        let rules = engine_default_rules().expect("engine-default rules are valid");
        let pst = minase::eval::weights().expect("embedded weights are valid");
        let maximum = 80;

        for game_number in 1..=8 {
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
            assert!(completed.stats.performed_injections <= completed.stats.planned_injections);
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
    }

    /// 記録は最後に予定した注入より後の局面だけを含む。
    #[test]
    fn records_start_at_or_after_planned_injection_boundary() {
        let rules = engine_default_rules().expect("engine-default rules are valid");
        let pst = minase::eval::weights().expect("embedded weights are valid");
        let (_, plan) = opening_and_plan(rules, 7, 1, 4);
        let mut table = test_table();
        let completed = play_game(pst.as_ref(), rules, 1, test_settings(7, 4, 600), &mut table)
            .expect("the fixed game is valid");

        assert!(!completed.records.is_empty());
        assert!(
            completed
                .records
                .iter()
                .all(|completed| u32::from(completed.record.ply()) >= plan.record_from)
        );
        assert_eq!(
            completed.stats.recordable_positions,
            completed
                .stats
                .total_plies
                .saturating_sub(u64::from(plan.record_from))
        );
        assert_eq!(
            completed.stats.recordable_positions,
            completed.stats.recorded_positions
                + completed.stats.excluded_mate_band
                + completed.stats.excluded_tactical
                + completed.stats.excluded_repetition
        );
    }

    /// 注入計画は0以上80未満の異なるオフセットと予定由来の境界を返す。
    #[test]
    fn injection_plan_has_unique_offsets_and_planned_boundary() {
        let opening_ply = 12;
        let (fixed_plan, _) = plan_injections(derive_seed(7, 1), opening_ply, 4);
        assert_eq!(fixed_plan.plies, [68, 72, 82]);
        assert_eq!(fixed_plan.record_from, 83);

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
        for opening_ply in [8, 12, 16] {
            let (plan, _) =
                plan_injections(derive_seed(41, u64::from(opening_ply)), opening_ply, 0);
            assert!(plan.plies.is_empty());
            assert_eq!(plan.record_from, opening_ply);
        }
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

    /// 既出の探索キーは2回目以降を重複局面として数える。
    #[test]
    fn recorded_statistics_counts_every_repeated_search_key() {
        let rules = engine_default_rules().expect("engine-default rules are valid");
        let game = Game::new(rules);
        let completed = |game_number, search_key| CompletedRecord {
            record: Record::from_position(
                game.position(),
                0,
                Outcome::Draw,
                game_number,
                game.ply_count().try_into().unwrap(),
            ),
            search_key,
        };
        let mut statistics = RecordedStatistics::default();

        for record in [
            completed(1, 1),
            completed(2, 2),
            completed(3, 1),
            completed(4, 1),
        ] {
            statistics.record(&record);
        }

        assert_eq!(statistics.duplicate_positions, 2);
        assert_eq!(statistics.score_frequencies[score_index(0)], 4);
    }

    /// テスト用の完了対局を1レコード付きで作る。
    fn completed_game(game_number: u32) -> CompletedGame {
        let game = Game::new(engine_default_rules().expect("engine-default rules are valid"));
        let score = i16::try_from(game_number).unwrap() - 2;
        let search_key = if game_number == 3 {
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
        assert_eq!(sequential.3, 1);
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
        assert_eq!(
            error.to_string(),
            "worker channel closed after 1 of 4 games"
        );
    }

    /// 除外率は分母0なら数値ではなく`n/a`になる。
    #[test]
    fn rate_percent_is_not_available_for_zero_denominator() {
        assert_eq!(format_rate_percent(0, 0), "n/a");
        assert_eq!(format_rate_percent(1, 4), "25.000000");
    }
}
