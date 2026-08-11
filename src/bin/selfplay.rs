use std::process;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{CommandFactory, Parser, Subcommand, error::ErrorKind};
use minase::core::rules::parse_rule_set;
use minase::rng::{XorShift64, derive_seed};
use minase::search::{MAX_PLY, SearchConfig, search};
use minase::stats::{GsprtDecision, estimate_elo, gsprt_decision, gsprt_llr};
use minase::{Color, Game, GameResult, GameStatus, Move, RuleCode, Rules, Square};

const DEFAULT_MAX_PLY: u32 = 4096;
const DEFAULT_MAX_PAIRS: u64 = 1000;

/// 自己対局ハーネスのコマンドライン引数。
#[derive(Parser)]
#[command(name = "selfplay")]
struct Arguments {
    /// 全ペアの乱数列を派生させる基本シード。
    #[arg(long)]
    seed: Option<u64>,
    /// 採用するローカルルールコード列または規則セット名。
    #[arg(
        long,
        default_value = "engine-default",
        value_parser = parse_rule_set_argument
    )]
    rules: RuleSetArgument,
    /// 1局を打ち切る手数上限。
    #[arg(long, default_value_t = DEFAULT_MAX_PLY, value_parser = parse_positive_u32)]
    max_ply: u32,
    /// 候補側の設定。`random`または`depth=N[,nodes=M]`を指定する。
    #[arg(long, default_value = "random", value_parser = parse_player_spec)]
    candidate: PlayerSpec,
    /// 基準側の設定。`random`または`depth=N[,nodes=M]`を指定する。
    #[arg(long, default_value = "random", value_parser = parse_player_spec)]
    baseline: PlayerSpec,
    /// 実行する統計モード。
    #[command(subcommand)]
    mode: Mode,
}

/// 自己対局結果の集計方法。
#[derive(Subcommand)]
enum Mode {
    /// ペンタノミアルGSPRTでH0またはH1を逐次判定する。
    Gsprt {
        /// 判定を保留して停止する実行ペア数の上限。
        #[arg(long, default_value_t = DEFAULT_MAX_PAIRS, value_parser = parse_positive_u64)]
        max_pairs: u64,
    },
    /// 固定ペア数からEloと95%信頼区間を推定する。
    Elo {
        /// 実行するペア数。
        #[arg(long, value_parser = parse_positive_u64)]
        pairs: u64,
    },
}

#[derive(Clone)]
struct RuleSetArgument(Vec<RuleCode>);

fn parse_rule_set_argument(input: &str) -> Result<RuleSetArgument, String> {
    parse_rule_set(input).map(RuleSetArgument)
}

/// 自己対局プレイヤーの設定。
#[derive(Clone, PartialEq, Eq, Debug)]
struct PlayerSpec {
    text: String,
    kind: PlayerKind,
}

/// ランダム着手または固定制限の探索を表す。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PlayerKind {
    Random,
    Search { depth: u32, nodes: Option<u64> },
}

/// プレイヤー設定を解析する。
fn parse_player_spec(input: &str) -> Result<PlayerSpec, String> {
    if input == "random" {
        return Ok(PlayerSpec {
            text: input.to_owned(),
            kind: PlayerKind::Random,
        });
    }

    let mut fields = input.split(',');
    let depth_field = fields
        .next()
        .expect("split always returns at least one field");
    let depth = depth_field
        .strip_prefix("depth=")
        .ok_or_else(|| "player spec must be 'random' or 'depth=N[,nodes=M]'".to_owned())
        .and_then(parse_search_depth)?;
    let nodes = fields
        .next()
        .map(|field| {
            field
                .strip_prefix("nodes=")
                .ok_or_else(|| "the second player-spec field must be 'nodes=M'".to_owned())
                .and_then(parse_positive_u64)
        })
        .transpose()?;
    if fields.next().is_some() {
        return Err("player spec has too many comma-separated fields".to_owned());
    }

    Ok(PlayerSpec {
        text: input.to_owned(),
        kind: PlayerKind::Search { depth, nodes },
    })
}

/// 自己対局で着手を選ぶプレイヤー。
trait Player {
    /// 設定名を返す。
    fn name(&self) -> &str;

    /// 合法手から着手を1つ選ぶ。
    fn choose_move(&mut self, game: &Game, legal_moves: &[Move]) -> Move;
}

/// 合法手を一様ランダムに選ぶプレイヤー。
struct RandomPlayer {
    name: String,
    rng: XorShift64,
}

impl RandomPlayer {
    /// 設定名とシードからプレイヤーを作る。
    fn new(name: String, seed: u64) -> Self {
        Self {
            name,
            rng: XorShift64::new(seed),
        }
    }
}

/// 固定深さと任意のノード上限で探索するプレイヤー。
struct EnginePlayer {
    name: String,
    config: SearchConfig,
}

impl Player for EnginePlayer {
    fn name(&self) -> &str {
        &self.name
    }

    fn choose_move(&mut self, game: &Game, legal_moves: &[Move]) -> Move {
        search(
            game.position(),
            game.rules(),
            legal_moves,
            game.search_key_history(),
            &self.config,
        )
        .best_move
    }
}

/// 設定と当該局のシードからプレイヤーを作る。
fn make_player(spec: &PlayerSpec, seed: u64) -> Box<dyn Player> {
    match spec.kind {
        PlayerKind::Random => Box::new(RandomPlayer::new(spec.text.clone(), seed)),
        PlayerKind::Search { depth, nodes } => Box::new(EnginePlayer {
            name: spec.text.clone(),
            config: SearchConfig { depth, nodes },
        }),
    }
}

impl Player for RandomPlayer {
    fn name(&self) -> &str {
        &self.name
    }

    fn choose_move(&mut self, _game: &Game, legal_moves: &[Move]) -> Move {
        legal_moves[self.rng.index(legal_moves.len())]
    }
}

/// ペア対局で共有する開始状態。
struct Opening {
    game: Game,
    seed: u64,
    moves: Vec<Move>,
}

/// 1局の完走または手数上限による打ち切り。
#[derive(Clone, Copy)]
enum PlayedGame {
    Finished { plies: u32, result: GameResult },
    Cutoff { plies: u32 },
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

/// 探索が扱える範囲の深さを解析する。
fn parse_search_depth(text: &str) -> Result<u32, String> {
    let depth = parse_positive_u32(text)?;
    if depth > MAX_PLY {
        return Err(format!("search depth must not exceed {MAX_PLY}"));
    }
    Ok(depth)
}

/// 現在時刻から基本シードを生成する。
fn time_seed() -> Result<u64, std::time::SystemTimeError> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(nanos as u64 ^ (nanos >> 64) as u64)
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

/// 規則コード列をカンマ区切りで返す。
fn rules_text(codes: &[RuleCode]) -> String {
    codes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// 終局しない8手から12手の開始手順を生成する。
fn generate_opening(rules: Rules, pair_seed: u64) -> Opening {
    let mut opening_seed = derive_seed(pair_seed, 0);
    loop {
        let mut game = Game::new(rules).expect("validated rules must contain a repetition rule");
        let mut rng = XorShift64::new(opening_seed);
        let opening_plies = 8 + rng.index(5);
        let mut moves = Vec::with_capacity(opening_plies);
        let mut finished = false;

        for _ in 0..opening_plies {
            let legal_moves = game.legal_moves();
            assert!(
                !legal_moves.is_empty(),
                "ongoing game must have legal moves"
            );
            let selected = legal_moves[rng.index(legal_moves.len())];
            moves.push(selected);
            let status = game
                .play(selected)
                .expect("a move returned by legal_moves must be accepted");
            if matches!(status, GameStatus::Finished(_)) {
                finished = true;
                break;
            }
        }

        if !finished {
            return Opening {
                game,
                seed: opening_seed,
                moves,
            };
        }
        opening_seed = derive_seed(opening_seed, 0);
    }
}

/// 1局を既存の対局管理層で進行する。
fn play_game(
    mut game: Game,
    max_ply: u32,
    player_a_color: Color,
    player_a: &mut dyn Player,
    player_b: &mut dyn Player,
) -> PlayedGame {
    loop {
        if game.ply_count() >= max_ply {
            return PlayedGame::Cutoff {
                plies: game.ply_count(),
            };
        }

        let legal_moves = game.legal_moves();
        assert!(
            !legal_moves.is_empty(),
            "ongoing game must have legal moves"
        );
        let selected = if game.position().side_to_move() == player_a_color {
            player_a.choose_move(&game, &legal_moves)
        } else {
            player_b.choose_move(&game, &legal_moves)
        };
        let status = game
            .play(selected)
            .expect("a move returned by a player must be legal");
        if let GameStatus::Finished(result) = status {
            return PlayedGame::Finished {
                plies: game.ply_count(),
                result,
            };
        }
    }
}

/// 1局の結果を候補Aから見た半点単位の得点へ変換する。
fn half_points(result: GameResult, player_a_color: Color) -> u8 {
    match result {
        GameResult::Win { winner, .. } if winner == player_a_color => 2,
        GameResult::Win { .. } => 0,
        GameResult::Draw { .. } => 1,
    }
}

/// 1局の結果を表示文字列へ変換する。
fn played_game_text(game: PlayedGame) -> String {
    match game {
        PlayedGame::Finished {
            plies,
            result: GameResult::Win { winner, reason },
        } => format!("plies={plies} result=win winner={winner:?} reason={reason:?}"),
        PlayedGame::Finished {
            plies,
            result: GameResult::Draw { reason },
        } => format!("plies={plies} result=draw reason={reason:?}"),
        PlayedGame::Cutoff { plies } => format!("plies={plies} result=cutoff"),
    }
}

/// 1ペアを実行し、完走時はペンタノミアル分類の添字を返す。
fn run_pair(
    rules: Rules,
    rules_text: &str,
    base_seed: u64,
    pair_number: u64,
    max_ply: u32,
    candidate: &PlayerSpec,
    baseline: &PlayerSpec,
) -> Option<usize> {
    let pair_seed = derive_seed(base_seed, pair_number);
    let opening = generate_opening(rules, pair_seed);
    let game1_a_seed = derive_seed(pair_seed, 1);
    let game1_b_seed = derive_seed(pair_seed, 2);
    let game2_a_seed = derive_seed(pair_seed, 3);
    let game2_b_seed = derive_seed(pair_seed, 4);
    let mut game1_a = make_player(candidate, game1_a_seed);
    let mut game1_b = make_player(baseline, game1_b_seed);
    let mut game2_a = make_player(candidate, game2_a_seed);
    let mut game2_b = make_player(baseline, game2_b_seed);

    println!(
        "pair {pair_number}: pair_seed={pair_seed} opening_seed={} player_a={} player_b={} rules={rules_text} max_ply={max_ply}",
        opening.seed,
        game1_a.name(),
        game1_b.name()
    );
    println!("pair {pair_number} opening: plies={}", opening.moves.len());
    for (index, &mv) in opening.moves.iter().enumerate() {
        println!("  {}: {}", index + 1, move_text(mv));
    }

    println!(
        "pair {pair_number} game 1 settings: A=Black seed_a={game1_a_seed} B=White seed_b={game1_b_seed}"
    );
    let game1 = play_game(
        opening.game.clone(),
        max_ply,
        Color::Black,
        game1_a.as_mut(),
        game1_b.as_mut(),
    );
    println!("pair {pair_number} game 1: {}", played_game_text(game1));

    println!(
        "pair {pair_number} game 2 settings: B=Black seed_b={game2_b_seed} A=White seed_a={game2_a_seed}"
    );
    let game2 = play_game(
        opening.game,
        max_ply,
        Color::White,
        game2_a.as_mut(),
        game2_b.as_mut(),
    );
    println!("pair {pair_number} game 2: {}", played_game_text(game2));

    let (
        PlayedGame::Finished {
            result: game1_result,
            ..
        },
        PlayedGame::Finished {
            result: game2_result,
            ..
        },
    ) = (game1, game2)
    else {
        println!("pair {pair_number} result: discarded");
        return None;
    };
    let category = usize::from(
        half_points(game1_result, Color::Black) + half_points(game2_result, Color::White),
    );
    println!(
        "pair {pair_number} result: score_a={:.1} category={category}",
        category as f64 / 2.0
    );
    Some(category)
}

/// GSPRTの判定を表示文字列へ変換する。
const fn decision_text(decision: GsprtDecision) -> &'static str {
    match decision {
        GsprtDecision::AcceptH0 => "H0",
        GsprtDecision::Continue => "pending",
        GsprtDecision::AcceptH1 => "H1",
    }
}

/// 無限大を含むEloを表示する。
fn elo_text(elo: f64) -> String {
    if elo == f64::INFINITY {
        "+inf".to_owned()
    } else if elo == f64::NEG_INFINITY {
        "-inf".to_owned()
    } else {
        format!("{elo:.6}")
    }
}

/// GSPRTの最終集計を表示する。
fn print_gsprt_summary(
    results: &[u64; 5],
    discarded_pairs: u64,
    decision: GsprtDecision,
    elapsed: Duration,
) {
    println!(
        "summary: mode=gsprt pairs={} valid_pairs={} discarded_pairs={discarded_pairs}",
        results.iter().sum::<u64>() + discarded_pairs,
        results.iter().sum::<u64>()
    );
    println!("pentanomial: {results:?}");
    println!("llr: {:.10}", gsprt_llr(results));
    println!("decision: {}", decision_text(decision));
    println!("elapsed: {:.6} s", elapsed.as_secs_f64());
}

/// 固定局数Eloの最終集計を表示する。
fn print_elo_summary(results: &[u64; 5], discarded_pairs: u64, elapsed: Duration) {
    println!(
        "summary: mode=elo pairs={} valid_pairs={} discarded_pairs={discarded_pairs}",
        results.iter().sum::<u64>() + discarded_pairs,
        results.iter().sum::<u64>()
    );
    println!("pentanomial: {results:?}");
    if results.iter().sum::<u64>() == 0 {
        println!("elo: unavailable ci95=unavailable");
    } else {
        let estimate = estimate_elo(results);
        println!(
            "elo: estimate={} ci95=[{}, {}]",
            elo_text(estimate.elo),
            elo_text(estimate.lower),
            elo_text(estimate.upper)
        );
    }
    println!("elapsed: {:.6} s", elapsed.as_secs_f64());
}

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
    println!("max_ply: {}", arguments.max_ply);
    println!("candidate: {}", arguments.candidate.text);
    println!("baseline: {}", arguments.baseline.text);

    let (target_pairs, use_gsprt) = match arguments.mode {
        Mode::Gsprt { max_pairs } => (max_pairs, true),
        Mode::Elo { pairs } => (pairs, false),
    };
    let start = Instant::now();
    let mut results = [0; 5];
    let mut valid_pairs = 0;
    let mut discarded_pairs = 0;
    let mut pair_number = 0_u64;
    let mut decision = GsprtDecision::Continue;

    while pair_number < target_pairs && (!use_gsprt || decision == GsprtDecision::Continue) {
        pair_number = pair_number.checked_add(1).expect("pair number overflow");
        match run_pair(
            rules,
            &rules_text,
            base_seed,
            pair_number,
            arguments.max_ply,
            &arguments.candidate,
            &arguments.baseline,
        ) {
            Some(category) => {
                results[category] += 1;
                valid_pairs += 1;
                if use_gsprt {
                    let llr = gsprt_llr(&results);
                    decision = gsprt_decision(llr);
                    println!(
                        "statistics: valid_pairs={valid_pairs} pentanomial={results:?} llr={llr:.10} decision={}",
                        decision_text(decision)
                    );
                }
            }
            None => discarded_pairs += 1,
        }
    }

    if use_gsprt {
        print_gsprt_summary(&results, discarded_pairs, decision, start.elapsed());
    } else {
        print_elo_summary(&results, discarded_pairs, start.elapsed());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase::{DrawReason, WinReason};

    #[test]
    fn arguments_accept_both_modes_and_default_rules() {
        let gsprt = Arguments::try_parse_from(["selfplay", "gsprt"])
            .expect("the GSPRT mode must be accepted");
        assert_eq!(gsprt.rules.0, [RuleCode::R1]);
        assert_eq!(gsprt.candidate.kind, PlayerKind::Random);
        assert_eq!(gsprt.baseline.kind, PlayerKind::Random);
        assert!(matches!(
            gsprt.mode,
            Mode::Gsprt {
                max_pairs: DEFAULT_MAX_PAIRS
            }
        ));

        let elo = Arguments::try_parse_from(["selfplay", "elo", "--pairs", "20"])
            .expect("the Elo mode must be accepted");
        assert!(matches!(elo.mode, Mode::Elo { pairs: 20 }));
    }

    #[test]
    fn arguments_accept_search_player_specs() {
        let arguments = Arguments::try_parse_from([
            "selfplay",
            "--candidate",
            "depth=2,nodes=1000",
            "--baseline",
            "depth=1",
            "elo",
            "--pairs",
            "1",
        ])
        .expect("search player specs must be accepted");
        assert_eq!(
            arguments.candidate.kind,
            PlayerKind::Search {
                depth: 2,
                nodes: Some(1000)
            }
        );
        assert_eq!(
            arguments.baseline.kind,
            PlayerKind::Search {
                depth: 1,
                nodes: None
            }
        );
    }

    #[test]
    fn arguments_reject_invalid_player_specs() {
        for spec in [
            "depth=0",
            "depth=257",
            "nodes=1",
            "depth=1,foo=2",
            "depth=1,nodes=2,x=3",
        ] {
            assert!(
                Arguments::try_parse_from(["selfplay", "--candidate", spec, "elo", "--pairs", "1"])
                    .is_err(),
                "invalid spec {spec:?} must be rejected"
            );
        }
    }

    #[test]
    fn rules_argument_rejects_preset_combined_with_code() {
        let error = match Arguments::try_parse_from(["selfplay", "--rules", "lishogi,P1", "gsprt"])
        {
            Ok(_) => panic!("a preset combined with a rule code must be rejected"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("preset 'lishogi' must be specified alone")
        );
    }

    #[test]
    fn game_results_convert_to_candidate_half_points() {
        let win = GameResult::Win {
            winner: Color::Black,
            reason: WinReason::RoyalCapture,
        };
        let loss = GameResult::Win {
            winner: Color::White,
            reason: WinReason::RoyalCapture,
        };
        let draw = GameResult::Draw {
            reason: DrawReason::Repetition,
        };
        assert_eq!(half_points(win, Color::Black), 2);
        assert_eq!(half_points(loss, Color::Black), 0);
        assert_eq!(half_points(draw, Color::Black), 1);
    }
}
