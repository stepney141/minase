use std::process;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{CommandFactory, Parser, error::ErrorKind};
use minase::rng::XorShift64;
use minase::{
    Color, DrawReason, Game, GameError, GameResult, GameStatus, Move, RuleCode, Rules, Square,
    WinReason, to_sfen,
};

const DEFAULT_MAX_PLY: u32 = 4096;
const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
const SPLITMIX_MIX1: u64 = 0xBF58_476D_1CE4_E5B9;
const SPLITMIX_MIX2: u64 = 0x94D0_49BB_1331_11EB;
const WIN_REASONS: [WinReason; 7] = [
    WinReason::RoyalCapture,
    WinReason::Repetition,
    WinReason::PieceExhaustion,
    WinReason::BareKing,
    WinReason::Stalemate,
    WinReason::Mate,
    WinReason::Resignation,
];
const DRAW_REASONS: [DrawReason; 4] = [
    DrawReason::Repetition,
    DrawReason::PieceExhaustion,
    DrawReason::BareKing,
    DrawReason::Agreement,
];

/// ランダム対局検証ハーネスのコマンドライン引数。
#[derive(Parser)]
#[command(name = "random_play")]
struct Arguments {
    /// 実行する対局数。
    #[arg(
        long,
        default_value_t = 1,
        conflicts_with = "game",
        value_parser = parse_positive_u64
    )]
    games: u64,
    /// 全局の乱数列を派生させる基本シード。
    #[arg(long)]
    seed: Option<u64>,
    /// 単独で再現する1起算の局番号。
    #[arg(long, value_parser = parse_positive_u64)]
    game: Option<u64>,
    /// 採用するローカルルールコード列。
    #[arg(long, required = true, value_delimiter = ',', value_parser = parse_rule_code)]
    rules: Vec<RuleCode>,
    /// 1局を打ち切る手数上限。
    #[arg(long, default_value_t = DEFAULT_MAX_PLY, value_parser = parse_positive_u32)]
    max_ply: u32,
    /// 毎手の全合法手を複製した対局へ適用して検証する。
    #[arg(long)]
    verify_all: bool,
    /// 各局の全手順を表示する。
    #[arg(long)]
    verbose: bool,
}

/// 正常に終了した1局の結果。
struct CompletedGame {
    plies: u32,
    outcome: Outcome,
    moves: Vec<Move>,
}

/// 終局または手数上限による打ち切りを表す。
#[derive(Clone, Copy)]
enum Outcome {
    Finished(GameResult),
    Cutoff,
}

/// 異常時に再現に必要な情報を保持する。
struct Failure<'a> {
    issue: String,
    rules: &'a str,
    base_seed: u64,
    game_number: u64,
    ply: u32,
    problem_move: Option<Move>,
    previous_sfen: String,
    moves: &'a [Move],
}

/// 全局の結果と手数を集計する。
#[derive(Default)]
struct Summary {
    games: u64,
    wins: [u64; 2],
    draws: u64,
    win_reasons: [u64; WIN_REASONS.len()],
    draw_reasons: [u64; DRAW_REASONS.len()],
    cutoffs: u64,
    total_plies: u64,
    min_plies: Option<u32>,
    max_plies: u32,
}

impl Summary {
    /// 1局の結果を集計へ加える。
    fn record(&mut self, completed: &CompletedGame) {
        self.games += 1;
        self.total_plies += u64::from(completed.plies);
        self.min_plies = Some(
            self.min_plies
                .map_or(completed.plies, |minimum| minimum.min(completed.plies)),
        );
        self.max_plies = self.max_plies.max(completed.plies);

        match completed.outcome {
            Outcome::Finished(GameResult::Win { winner, reason }) => {
                self.wins[winner.index()] += 1;
                self.win_reasons[win_reason_index(reason)] += 1;
            }
            Outcome::Finished(GameResult::Draw { reason }) => {
                self.draws += 1;
                self.draw_reasons[draw_reason_index(reason)] += 1;
            }
            Outcome::Cutoff => self.cutoffs += 1,
        }
    }

    /// 全局の集計を表示する。
    fn print(&self, elapsed: Duration) {
        let average = self.total_plies as f64 / self.games as f64;
        println!("summary: games={}", self.games);
        println!(
            "results: BlackWins={} WhiteWins={} Draws={} Cutoffs={}",
            self.wins[Color::Black.index()],
            self.wins[Color::White.index()],
            self.draws,
            self.cutoffs
        );
        println!("win reasons:");
        for reason in WIN_REASONS {
            println!(
                "  {reason:?}={}",
                self.win_reasons[win_reason_index(reason)]
            );
        }
        println!("draw reasons:");
        for reason in DRAW_REASONS {
            println!(
                "  {reason:?}={}",
                self.draw_reasons[draw_reason_index(reason)]
            );
        }
        println!(
            "plies: min={} max={} average={average:.3}",
            self.min_plies.expect("at least one game must be recorded"),
            self.max_plies
        );
        println!("elapsed: {:.6} s", elapsed.as_secs_f64());
    }
}

/// 0より大きい`u64`を解析する。
fn parse_positive_u64(text: &str) -> Result<u64, String> {
    let value = text
        .parse::<u64>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if value == 0 {
        return Err("value must be at least 1".to_owned());
    }
    Ok(value)
}

/// 0より大きい`u32`を解析する。
fn parse_positive_u32(text: &str) -> Result<u32, String> {
    let value = text
        .parse::<u32>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if value == 0 {
        return Err("value must be at least 1".to_owned());
    }
    Ok(value)
}

/// ローカルルールコードを解析する。
fn parse_rule_code(text: &str) -> Result<RuleCode, String> {
    match text {
        "L0" => Ok(RuleCode::L0),
        "L1" => Ok(RuleCode::L1),
        "L2" => Ok(RuleCode::L2),
        "L3" => Ok(RuleCode::L3),
        "P1" => Ok(RuleCode::P1),
        "P2" => Ok(RuleCode::P2),
        "P3" => Ok(RuleCode::P3),
        "P4" => Ok(RuleCode::P4),
        "R1" => Ok(RuleCode::R1),
        "R2" => Ok(RuleCode::R2),
        "R3" => Ok(RuleCode::R3),
        "E1" => Ok(RuleCode::E1),
        "E2" => Ok(RuleCode::E2),
        "E3" => Ok(RuleCode::E3),
        _ => Err(format!("unknown rule code '{text}'")),
    }
}

/// ルールコードの表示文字列を返す。
const fn rule_code_text(code: RuleCode) -> &'static str {
    match code {
        RuleCode::L0 => "L0",
        RuleCode::L1 => "L1",
        RuleCode::L2 => "L2",
        RuleCode::L3 => "L3",
        RuleCode::P1 => "P1",
        RuleCode::P2 => "P2",
        RuleCode::P3 => "P3",
        RuleCode::P4 => "P4",
        RuleCode::R1 => "R1",
        RuleCode::R2 => "R2",
        RuleCode::R3 => "R3",
        RuleCode::E1 => "E1",
        RuleCode::E2 => "E2",
        RuleCode::E3 => "E3",
    }
}

/// 採用するルールコード列をカンマ区切りで返す。
fn rules_text(codes: &[RuleCode]) -> String {
    codes
        .iter()
        .map(|&code| rule_code_text(code))
        .collect::<Vec<_>>()
        .join(",")
}

/// 現在時刻から基本シードを生成する。
fn time_seed() -> Result<u64, std::time::SystemTimeError> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(nanos as u64 ^ (nanos >> 64) as u64)
}

/// 指定値をsplitmix64の有限混合で変換する。
fn splitmix64(value: u64) -> u64 {
    let mut mixed = value.wrapping_add(SPLITMIX_GAMMA);
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(SPLITMIX_MIX1);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(SPLITMIX_MIX2);
    mixed ^ (mixed >> 31)
}

/// 基本シードと1起算の局番号から局シードを派生する。
fn game_seed(base_seed: u64, game_number: u64) -> u64 {
    match splitmix64(base_seed.wrapping_add(game_number)) {
        0 => SPLITMIX_GAMMA,
        seed => seed,
    }
}

/// 升をperftと同じ0起算座標で表記する。
fn square_text(square: Square) -> String {
    format!("({},{})", square.file(), square.rank())
}

/// 着手をperftの`move_text`と同じ形式で表記する。
fn move_text(mv: Move) -> String {
    if let Some(mid) = mv.mid {
        format!(
            "double {}->{}->{}{}",
            square_text(mv.from),
            square_text(mid),
            square_text(mv.to),
            if mv.promote { "+" } else { "" }
        )
    } else {
        format!(
            "move {}->{}{}",
            square_text(mv.from),
            square_text(mv.to),
            if mv.promote { "+" } else { "" }
        )
    }
}

/// 異常情報と全手順を標準エラー出力へ表示して終了する。
fn fail(failure: Failure<'_>) -> ! {
    eprintln!("random-play invariant failure: {}", failure.issue);
    eprintln!("rules: {}", failure.rules);
    eprintln!("seed: {}", failure.base_seed);
    eprintln!("game: {}", failure.game_number);
    eprintln!("ply: {}", failure.ply);
    match failure.problem_move {
        Some(mv) => eprintln!("problem move: {}", move_text(mv)),
        None => eprintln!("problem move: none"),
    }
    eprintln!("previous SFEN: {}", failure.previous_sfen);
    eprintln!("moves:");
    if failure.moves.is_empty() {
        eprintln!("  (none)");
    } else {
        for (index, &mv) in failure.moves.iter().enumerate() {
            eprintln!("  {}: {}", index + 1, move_text(mv));
        }
    }
    process::exit(1);
}

/// 勝利理由に対応する集計添字を返す。
const fn win_reason_index(reason: WinReason) -> usize {
    match reason {
        WinReason::RoyalCapture => 0,
        WinReason::Repetition => 1,
        WinReason::PieceExhaustion => 2,
        WinReason::BareKing => 3,
        WinReason::Stalemate => 4,
        WinReason::Mate => 5,
        WinReason::Resignation => 6,
    }
}

/// 引き分け理由に対応する集計添字を返す。
const fn draw_reason_index(reason: DrawReason) -> usize {
    match reason {
        DrawReason::Repetition => 0,
        DrawReason::PieceExhaustion => 1,
        DrawReason::BareKing => 2,
        DrawReason::Agreement => 3,
    }
}

/// 結果を1局1行の表示形式へ変換する。
fn outcome_text(outcome: Outcome) -> String {
    match outcome {
        Outcome::Finished(GameResult::Win { winner, reason }) => {
            format!("win winner={winner:?} reason={reason:?}")
        }
        Outcome::Finished(GameResult::Draw { reason }) => {
            format!("draw reason={reason:?}")
        }
        Outcome::Cutoff => "cutoff".to_owned(),
    }
}

/// 指定局番号のランダム対局を実行して不変条件を検査する。
fn run_game(
    rules: Rules,
    rules_text: &str,
    base_seed: u64,
    game_number: u64,
    max_ply: u32,
    verify_all: bool,
) -> CompletedGame {
    let mut game = Game::new(rules).expect("validated rules must contain a repetition rule");
    let mut rng = XorShift64::new(game_seed(base_seed, game_number));
    let mut history = Vec::new();

    loop {
        if game.ply_count() >= max_ply {
            return CompletedGame {
                plies: game.ply_count(),
                outcome: Outcome::Cutoff,
                moves: history,
            };
        }

        let legal_moves = game.legal_moves();
        let previous_sfen = to_sfen(game.position());
        if legal_moves.is_empty() {
            fail(Failure {
                issue: "ongoing game has no legal moves".to_owned(),
                rules: rules_text,
                base_seed,
                game_number,
                ply: game.ply_count(),
                problem_move: None,
                previous_sfen,
                moves: &history,
            });
        }

        if verify_all {
            for &candidate in &legal_moves {
                let mut probe = game.clone();
                if let Err(error) = probe.play(candidate) {
                    fail(Failure {
                        issue: format!("legal move rejected during full verification: {error}"),
                        rules: rules_text,
                        base_seed,
                        game_number,
                        ply: game.ply_count() + 1,
                        problem_move: Some(candidate),
                        previous_sfen: previous_sfen.clone(),
                        moves: &history,
                    });
                }
            }
        }

        let selected = legal_moves[rng.index(legal_moves.len())];
        let status = match game.play(selected) {
            Ok(status) => status,
            Err(error) => fail(Failure {
                issue: format!("legal move rejected: {error}"),
                rules: rules_text,
                base_seed,
                game_number,
                ply: game.ply_count() + 1,
                problem_move: Some(selected),
                previous_sfen,
                moves: &history,
            }),
        };
        history.push(selected);

        if let Err(error) = game.position().validate() {
            fail(Failure {
                issue: format!("position validation failed: {error}"),
                rules: rules_text,
                base_seed,
                game_number,
                ply: game.ply_count(),
                problem_move: Some(selected),
                previous_sfen,
                moves: &history,
            });
        }

        let GameStatus::Finished(result) = status else {
            continue;
        };

        let terminal_sfen = to_sfen(game.position());
        let terminal_moves = game.legal_moves();
        if let Some(&candidate) = terminal_moves.first() {
            fail(Failure {
                issue: format!(
                    "finished game returned {} legal moves",
                    terminal_moves.len()
                ),
                rules: rules_text,
                base_seed,
                game_number,
                ply: game.ply_count() + 1,
                problem_move: Some(candidate),
                previous_sfen: terminal_sfen,
                moves: &history,
            });
        }

        match game.play(selected) {
            Err(GameError::GameAlreadyOver) => {}
            unexpected => fail(Failure {
                issue: format!("additional move after game end returned {unexpected:?}"),
                rules: rules_text,
                base_seed,
                game_number,
                ply: game.ply_count() + 1,
                problem_move: Some(selected),
                previous_sfen: terminal_sfen,
                moves: &history,
            }),
        }

        return CompletedGame {
            plies: game.ply_count(),
            outcome: Outcome::Finished(result),
            moves: history,
        };
    }
}

/// 1局の結果と、指定時は全手順を表示して集計へ加える。
fn report_game(game_number: u64, completed: CompletedGame, verbose: bool, summary: &mut Summary) {
    println!(
        "game {game_number}: plies={} result={}",
        completed.plies,
        outcome_text(completed.outcome)
    );
    if verbose {
        println!("game {game_number} moves:");
        for (index, &mv) in completed.moves.iter().enumerate() {
            println!("  {}: {}", index + 1, move_text(mv));
        }
    }
    summary.record(&completed);
}

fn main() {
    let arguments = Arguments::parse();
    let rules = match Rules::from_codes(&arguments.rules) {
        Ok(rules) => rules,
        Err(error) => Arguments::command()
            .error(ErrorKind::ValueValidation, error.to_string())
            .exit(),
    };
    if let Err(error) = Game::new(rules) {
        Arguments::command()
            .error(ErrorKind::ValueValidation, error.to_string())
            .exit();
    }

    let base_seed = match arguments.seed {
        Some(seed) => seed,
        None => match time_seed() {
            Ok(seed) => seed,
            Err(error) => {
                eprintln!("failed to generate a seed from the current time: {error}");
                process::exit(1);
            }
        },
    };
    let rules_text = rules_text(&arguments.rules);
    println!("rules: {rules_text}");
    println!("seed: {base_seed}");

    let start = Instant::now();
    let mut summary = Summary::default();
    let mut execute = |game_number| {
        let completed = run_game(
            rules,
            &rules_text,
            base_seed,
            game_number,
            arguments.max_ply,
            arguments.verify_all,
        );
        report_game(game_number, completed, arguments.verbose, &mut summary);
    };

    match arguments.game {
        Some(game_number) => execute(game_number),
        None => {
            for game_number in 1..=arguments.games {
                execute(game_number);
            }
        }
    }
    summary.print(start.elapsed());
}
