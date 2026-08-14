//! ランダム対局の検証ハーネス。合法手適用の不変条件を長時間対局で検査する。

use std::process;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{CommandFactory, Parser, error::ErrorKind};
use minase::core::rules::parse_rule_set;
use minase::rng::XorShift64;
use minase::{
    Color, DrawReason, Game, GameError, GameResult, GameStatus, Move, RuleCode, Rules, Square,
    WinReason, to_sfen,
};

/// 1局を打ち切る手数上限の既定値。
const DEFAULT_MAX_PLY: u32 = 4096;
/// splitmix64の增分定数(黄金比の64ビット表現)。
const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
/// splitmix64の第1混合乗数。
const SPLITMIX_MIX1: u64 = 0xBF58_476D_1CE4_E5B9;
/// splitmix64の第2混合乗数。
const SPLITMIX_MIX2: u64 = 0x94D0_49BB_1331_11EB;
/// 集計に使う勝利理由の一覧。
const WIN_REASONS: [WinReason; 7] = [
    WinReason::RoyalCapture,
    WinReason::Repetition,
    WinReason::PieceExhaustion,
    WinReason::BareKing,
    WinReason::Stalemate,
    WinReason::Mate,
    WinReason::Resignation,
];
/// 集計に使う引き分け理由の一覧。
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
    /// 採用するローカルルールコード列または規則セット名。
    #[arg(long, required = true, value_parser = parse_rule_set_argument)]
    rules: RuleSetArgument,
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

/// 解析済みの`--rules`引数。
#[derive(Clone)]
struct RuleSetArgument(Vec<RuleCode>);

/// `--rules`の値を規則セット名またはコード列として解析する。
fn parse_rule_set_argument(input: &str) -> Result<RuleSetArgument, String> {
    parse_rule_set(input).map(RuleSetArgument)
}

/// 正常に終了した1局の結果。
struct CompletedGame {
    /// 終了時点の手数。
    plies: u32,
    /// 終局結果または打ち切り。
    outcome: Outcome,
    /// 実際に適用した全手順。
    moves: Vec<Move>,
}

/// 終局または手数上限による打ち切りを表す。
#[derive(Clone, Copy)]
enum Outcome {
    /// 裁定による終局。
    Finished(GameResult),
    /// 手数上限による打ち切り。
    Cutoff,
}

/// 異常時に再現に必要な情報を保持する。
struct Failure<'a> {
    /// 検出した不変条件違反の説明。
    issue: String,
    /// 採用ルールの表示文字列。
    rules: &'a str,
    /// 全局共通の基本シード。
    base_seed: u64,
    /// 1起算の局番号。
    game_number: u64,
    /// 違反を検出した手数。
    ply: u32,
    /// 違反に関与した着手。
    problem_move: Option<Move>,
    /// 違反直前の局面のSFEN。
    previous_sfen: String,
    /// 違反までに適用した全手順。
    moves: &'a [Move],
}

/// 全局の結果と手数を集計する。
#[derive(Default)]
struct Summary {
    /// 集計した局数。
    games: u64,
    /// 対局者ごとの勝利数。
    wins: [u64; 2],
    /// 引き分け数。
    draws: u64,
    /// 勝利理由ごとの件数。
    win_reasons: [u64; WIN_REASONS.len()],
    /// 引き分け理由ごとの件数。
    draw_reasons: [u64; DRAW_REASONS.len()],
    /// 手数上限による打ち切り数。
    cutoffs: u64,
    /// 全局の手数合計。
    total_plies: u64,
    /// 最短の手数。
    min_plies: Option<u32>,
    /// 最長の手数。
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

/// 引数を検証し、指定局数のランダム対局を実行して集計を出力する。
fn main() {
    let arguments = Arguments::parse();
    let rules = match Rules::from_codes(&arguments.rules.0) {
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
    let rules_text = rules_text(&arguments.rules.0);
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

#[cfg(test)]
mod tests {
    use super::*;

    // D8-HARN-16(random-play.mdコマンドライン引数節): `--games`既定1、
    // `--max-ply`既定4096、`--seed`省略可、`--verify-all`・`--verbose`受理。
    // `--rules`は明文で必須、`--game`と`--games`の同時指定はエラー、
    // 局番号は1起算(0は不正)。
    #[test]
    fn cli_defaults_flags_and_conflicts_follow_the_design_document() {
        let defaults = Arguments::try_parse_from(["random_play", "--rules", "engine-default"])
            .expect("the minimal documented invocation must be accepted");
        assert_eq!(defaults.games, 1);
        assert_eq!(defaults.max_ply, 4096);
        assert_eq!(defaults.seed, None);
        assert_eq!(defaults.game, None);
        assert!(!defaults.verify_all);
        assert!(!defaults.verbose);

        let full = Arguments::try_parse_from([
            "random_play",
            "--rules",
            "R1",
            "--games",
            "30",
            "--seed",
            "7",
            "--max-ply",
            "64",
            "--verify-all",
            "--verbose",
        ])
        .expect("all documented options must be accepted together");
        assert_eq!(full.games, 30);
        assert_eq!(full.seed, Some(7));
        assert_eq!(full.max_ply, 64);
        assert!(full.verify_all);
        assert!(full.verbose);

        // --rules省略はエラー(match_runnerと異なり明文で必須)
        assert!(Arguments::try_parse_from(["random_play"]).is_err());
        // --gameと--gamesの同時指定はエラー
        assert!(
            Arguments::try_parse_from([
                "random_play",
                "--rules",
                "R1",
                "--game",
                "2",
                "--games",
                "5",
            ])
            .is_err()
        );
        // 局番号は1起算のため0は受理しない
        assert!(
            Arguments::try_parse_from(["random_play", "--rules", "R1", "--game", "0"]).is_err()
        );
    }

    // D8-HARN-16/D8-HARN-12(RULES.md第33条): 値文法はエンジンバイナリと統一。
    // プリセット名は大文字小文字を区別せず解決され、規則コードとの併記と
    // R0の指定は拒否される。
    #[test]
    fn rules_argument_resolves_presets_case_insensitively_and_rejects_combinations() {
        let engine_default =
            Arguments::try_parse_from(["random_play", "--rules", "engine-default"])
                .expect("the engine-default preset must be accepted");
        assert_eq!(engine_default.rules.0, [RuleCode::R1]);

        let preset = Arguments::try_parse_from(["random_play", "--rules", "LISHOGI"])
            .expect("preset names must match case-insensitively");
        let codes = Arguments::try_parse_from(["random_play", "--rules", "L1,L2,P3,R1,E1,E3"])
            .expect("an explicit rule code list must be accepted");
        assert_eq!(
            preset.rules.0,
            [
                RuleCode::L1,
                RuleCode::L2,
                RuleCode::P3,
                RuleCode::R1,
                RuleCode::E1,
                RuleCode::E3,
            ]
        );
        assert_eq!(preset.rules.0, codes.rules.0);

        for invalid in ["lishogi,P1", "R0", "R0,R1"] {
            assert!(
                Arguments::try_parse_from(["random_play", "--rules", invalid]).is_err(),
                "rules {invalid:?} must be rejected"
            );
        }
    }

    // D8-HARN-17(random-play.mdシード派生節): 局シードは
    // splitmix64(base_seed.wrapping_add(n))であり、式は仕様として固定されて
    // いる。参照値は仕様式から独立に(実装を経由せずに)計算した。
    #[test]
    fn game_seed_follows_the_documented_splitmix64_derivation() {
        // splitmix64(1 + 1) = splitmix64(2)
        assert_eq!(game_seed(1, 1), 0x9758_35DE_1C97_56CE);
        assert_eq!(game_seed(0xDEAD_BEEF, 41), 0xA472_D968_8D97_8A28);
        // 加算はラッピング演算: u64::MAX + 3 ≡ 2 ≡ 1 + 1 (mod 2^64)
        assert_eq!(game_seed(u64::MAX, 3), game_seed(1, 1));
        // 派生値0は固定の非ゼロ定数0x9E37_79B9_7F4A_7C15へ置換される。
        // 仕様式の各段は全単射なので出力0の原像は一意で、逆算により
        // 0x61C8_8646_80B5_83EB(增分定数の2の補数)である。
        assert_eq!(
            game_seed(0x61C8_8646_80B5_83EB - 3, 3),
            0x9E37_79B9_7F4A_7C15
        );
    }

    // D8-HARN-17性質/D8-HARN-18(random-play.md): 同一の(--rules, --seed,
    // --game, --max-ply)で着手列が完全に再現される。手数上限への到達は
    // 異常ではなく打ち切りである。
    #[test]
    fn same_seed_and_game_number_reproduce_the_same_game() {
        let rules = Rules::from_codes(&[RuleCode::R1]).expect("R1 alone must be a valid rule set");
        let first = run_game(rules, "R1", 20_260_814, 1, 16, false);
        let second = run_game(rules, "R1", 20_260_814, 1, 16, false);
        assert_eq!(first.moves, second.moves);
        assert_eq!(first.plies, second.plies);
        assert_eq!(first.plies, 16);
        assert!(matches!(first.outcome, Outcome::Cutoff));
    }

    // D8-HARN-18(random-play.md対局ループと検証節): --verify-all指定時は毎手、
    // 全合法手を複製した対局へ適用して受理を全数検証する。検証の失敗は
    // プロセス異常終了として現れるため、完走自体が観測である。
    #[test]
    fn verify_all_verification_completes_on_a_short_game() {
        let rules = Rules::from_codes(&[RuleCode::R1]).expect("R1 alone must be a valid rule set");
        let completed = run_game(rules, "R1", 20_260_814, 2, 4, true);
        assert_eq!(completed.plies, 4);
        assert_eq!(completed.moves.len(), 4);
        assert!(matches!(completed.outcome, Outcome::Cutoff));
    }
}
