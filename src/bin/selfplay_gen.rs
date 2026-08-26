//! 評価関数の学習に使う自己対局データの生成と検査。

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io;
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
struct Candidate {
    /// 記録時点の局面。
    position: Position,
    /// 手番側視点の探索値。
    score: i16,
    /// 記録時点の手数。
    ply: u16,
}

/// 1局分のレコードと統計。
struct CompletedGame {
    /// 1から始まる対局番号。
    game_number: u32,
    /// 終局対局から採用したレコード。
    records: Vec<Record>,
    /// 打ち切り対局を含む生成統計。
    stats: Statistics,
}

/// データ生成全体または1局分の統計。
#[derive(Default)]
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
        self.recorded_positions += other.recorded_positions;
        self.excluded_mate_band += other.excluded_mate_band;
        self.excluded_tactical += other.excluded_tactical;
        self.excluded_repetition += other.excluded_repetition;
        self.total_plies += other.total_plies;
        self.searched_plies += other.searched_plies;
        self.searched_nodes += other.searched_nodes;
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

    let total = thread::scope(|scope| -> io::Result<Statistics> {
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
                        play_game(
                            pst,
                            rules,
                            arguments.seed,
                            game_number,
                            arguments.nodes,
                            arguments.max_ply,
                            &mut table,
                        )
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

        let mut pending = BTreeMap::new();
        let mut next_to_write = 1_u64;
        let mut completed_count = 0_u64;
        let mut total = Statistics::default();
        for message in receiver {
            let completed = message?;
            pending.insert(completed.game_number, completed);
            while let Some(completed) = pending.remove(&(next_to_write as u32)) {
                for record in &completed.records {
                    writer.write_record(record).map_err(training_error)?;
                }
                total.merge(&completed.stats);
                completed_count += 1;
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
                next_to_write += 1;
            }
        }
        if next_to_write != u64::from(arguments.games) + 1 {
            return Err(invalid_data(format!(
                "worker channel closed after {} of {} games",
                next_to_write - 1,
                arguments.games
            )));
        }
        Ok(total)
    })?;

    let file = writer.finish().map_err(training_error)?;
    file.sync_all()?;
    print_generation_summary(
        arguments,
        rule_set,
        generation_commit,
        &total,
        start.elapsed().as_secs_f64(),
    );
    Ok(())
}

/// 1局をランダム序盤から終局まで進め、採用レコードを返す。
fn play_game(
    pst: &Pst,
    rules: Rules,
    base_seed: u64,
    game_number: u32,
    nodes: u32,
    max_ply: u16,
    table: &mut TranspositionTable,
) -> io::Result<CompletedGame> {
    let game_seed = derive_seed(base_seed, u64::from(game_number));
    let mut game = generate_opening(rules, game_seed)?;
    table.clear();
    let limits = SearchLimits::new(None, Some(u64::from(nodes)), None, None)
        .expect("the CLI parser accepts only non-zero node limits");
    let mut candidates = Vec::new();
    let mut stats = Statistics::default();
    let generator = MoveGenerator::new(rules.moves);

    while game.result().is_none() && game.ply_count() < u32::from(max_ply) {
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
                    Record::from_position(
                        &candidate.position,
                        candidate.score,
                        outcome,
                        game_number,
                        candidate.ply,
                    )
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

/// 生成統計を設計書へ転記できる1項目1行の形式で表示する。
fn print_generation_summary(
    arguments: &GenerateArguments,
    rule_set: &str,
    generation_commit: &str,
    stats: &Statistics,
    elapsed_seconds: f64,
) {
    let searched = stats.searched_positions as f64;
    let games = f64::from(arguments.games);
    let rate = |count: u64| {
        if searched == 0.0 {
            0.0
        } else {
            count as f64 * 100.0 / searched
        }
    };
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
    println!("searched_positions: {}", stats.searched_positions);
    println!("recorded_positions: {}", stats.recorded_positions);
    println!("excluded_mate_band: {}", stats.excluded_mate_band);
    println!(
        "excluded_mate_band_rate_percent: {:.6}",
        rate(stats.excluded_mate_band)
    );
    println!("excluded_tactical: {}", stats.excluded_tactical);
    println!(
        "excluded_tactical_rate_percent: {:.6}",
        rate(stats.excluded_tactical)
    );
    println!("excluded_repetition: {}", stats.excluded_repetition);
    println!(
        "excluded_repetition_rate_percent: {:.6}",
        rate(stats.excluded_repetition)
    );
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
