//! 本番Driverと一様評価器による探索時間、経路再生量、スナップショット費用の測定。
//!
//! `--adjudication before`では審判呼出し数と裁定理由別件数を0にする。
//! 回数は測定後の木から数え、測定区間へ計測処理を入れない。
//! Gumbel探索はQの尺度0.1、最終着手の乱数の倍率1.0で測定する。
//! 時間の項目（`elapsed_ms`、`summary_ms`、`snapshots`の計測値）以外の出力は、
//! 同じ入力とシードで決定的であり、審判層の変更前後の回帰の検査に用いる。

use std::collections::BTreeMap;
use std::error::Error;
use std::hint::black_box;
use std::num::{NonZeroU64, NonZeroUsize};
use std::time::Instant;

use minase::datagen::opening::generate_opening;
use minase::nn::{Evaluator, UniformEvaluator};
use minase::search::SearchSnapshot;
use minase::search::mcts::gumbel::Gumbel;
use minase::search::mcts::puct::PuctConfig;
use minase::search::mcts::tree::{Node, NodeState, SelectionRule};
use minase::search::mcts::{DrawValue, Driver, EvaluationRequest, Step};
use minase_core::notation::usi;
use minase_core::rng::{XorShift64, derive_seed};
use minase_core::{
    Color, Game, GameResult, GameStatus, Move, PieceCode, PieceKind, Position, PositionBuilder,
    Rules, Square,
};
use serde_json::{Value, json};

// minaseの実行ファイルと同じアロケータで対局履歴と探索木を確保する。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const CAPACITY: usize = 64 * 1024 * 1024;
const SNAPSHOT_SAMPLES: usize = 1000;

struct Case {
    category: String,
    start: Position,
    moves: Vec<Move>,
    seed: u64,
}

impl Case {
    fn game(&self) -> Result<Game> {
        let mut game = Game::from_position(Rules::ENGINE_DEFAULT, self.start.clone());
        for &mv in &self.moves {
            if game.play(mv)? != GameStatus::Ongoing {
                return Err("a measurement root must be ongoing".into());
            }
        }
        Ok(game)
    }

    fn input(&self) -> Result<Value> {
        let mut game = Game::from_position(Rules::ENGINE_DEFAULT, self.start.clone());
        let mut moves = Vec::new();
        for &mv in &self.moves {
            moves.push(usi::text_generated(game.position(), mv));
            game.play(mv)?;
        }
        Ok(
            json!({"category": self.category, "seed": self.seed, "start": minase_core::to_sfen(&self.start), "moves": moves, "root_key": game.position().zobrist(), "plies": game.ply_count()}),
        )
    }
}

fn cases(seed: NonZeroU64, games: usize) -> Result<Vec<Case>> {
    let mut cases = Vec::new();
    for index in 0..games {
        let game_seed = derive_seed(seed.get(), index as u64);
        let (mut game, mut moves) = generate_opening(Rules::ENGINE_DEFAULT, game_seed)?;
        let mut rng = XorShift64::new(derive_seed(game_seed.get(), 1));
        cases.push(Case {
            category: "opening".into(),
            start: Position::initial(),
            moves: moves.clone(),
            seed: game_seed.get(),
        });
        while moves.len() < 600 {
            let legal = game.legal_moves();
            let mv = legal[rng
                .index(NonZeroUsize::new(legal.len()).ok_or("ongoing game has no legal move")?)];
            match game.play(mv)? {
                GameStatus::Ongoing => {
                    moves.push(mv);
                    if [100, 300, 600].contains(&moves.len()) {
                        cases.push(Case {
                            category: format!("history_{}", moves.len()),
                            start: Position::initial(),
                            moves: moves.clone(),
                            seed: game_seed.get(),
                        });
                    }
                }
                GameStatus::Finished(result) => {
                    cases.push(Case {
                        category: match result {
                            GameResult::Win { reason, .. } => format!("near_{reason:?}"),
                            GameResult::Draw { reason } => format!("near_{reason:?}"),
                        },
                        start: Position::initial(),
                        moves,
                        seed: game_seed.get(),
                    });
                    break;
                }
            }
        }
    }
    // 監査の固定局面を加える。空の着手列を持つ明示的な開始局面であり、履歴は推定しない。
    for (name, pieces) in [
        (
            "fixed_exhaustion_win",
            vec![
                (0, 0, Color::Black, PieceKind::King),
                (11, 11, Color::White, PieceKind::King),
                (4, 4, Color::Black, PieceKind::Rook),
                (4, 5, Color::White, PieceKind::Pawn),
            ],
        ),
        (
            "fixed_exhaustion_draw",
            vec![
                (0, 0, Color::Black, PieceKind::King),
                (11, 11, Color::White, PieceKind::King),
                (0, 1, Color::White, PieceKind::Pawn),
            ],
        ),
        (
            "fixed_mate",
            vec![
                (2, 2, Color::Black, PieceKind::King),
                (0, 0, Color::White, PieceKind::King),
                (1, 2, Color::Black, PieceKind::FreeKing),
                (11, 5, Color::White, PieceKind::Pawn),
            ],
        ),
    ] {
        let mut builder = PositionBuilder::new(Color::Black);
        for (file, rank, color, kind) in pieces {
            builder.put(
                Square::new(file, rank).unwrap(),
                PieceCode::new(color, kind).unwrap(),
            )?;
        }
        cases.push(Case {
            category: name.into(),
            start: builder.finish()?,
            moves: Vec::new(),
            seed: seed.get(),
        });
    }
    Ok(cases)
}

fn evaluate(
    driver: &mut Driver,
    requests: &[EvaluationRequest],
    logits: &mut Vec<f32>,
    values: &mut Vec<f32>,
) -> Result<()> {
    let queries: Vec<_> = requests.iter().map(EvaluationRequest::query).collect();
    UniformEvaluator.evaluate(&queries, logits, values)?;
    let mut offset = 0;
    for (request, &value) in requests.iter().zip(values.iter()) {
        let count = request.query().moves.len();
        driver.evaluate(request, &logits[offset..offset + count], value)?;
        offset += count;
    }
    Ok(())
}

enum SearchOutput {
    Gumbel {
        selected_edge: usize,
        policy_target: Vec<f32>,
    },
    Puct {
        best_move: Option<Move>,
    },
}

fn search(game: &Game, seed: u64, gumbel: bool) -> Result<(Driver, f64, SearchOutput)> {
    let started = Instant::now();
    let snapshot = SearchSnapshot::from_game(game)?;
    let gumbel_config = if gumbel {
        let mut rng = XorShift64::new(derive_seed(seed, 2));
        let noise = (0..snapshot.root_moves().len())
            .map(|_| {
                // 自己対局と同じ53ビットの中点による変換。
                let unit = 1.0 / (1_u64 << 53) as f64;
                let u = ((rng.next() >> 11) as f64 + 0.5) * unit;
                let log_u = if u == 1.0 {
                    (-0.5 * unit).ln_1p()
                } else {
                    u.ln()
                };
                -(-log_u).ln() as f32
            })
            .collect();
        Some(Gumbel::new(noise, 32, 0.1)?)
    } else {
        None
    };
    let config = PuctConfig::default();
    let rule = match &gumbel_config {
        Some(config) => SelectionRule::Gumbel(config),
        None => SelectionRule::Puct(&config),
    };
    let mut driver = Driver::new(
        snapshot,
        CAPACITY,
        if gumbel {
            DrawValue::SelfPlay
        } else {
            DrawValue::Game {
                cp_per_logit: 1072.65,
            }
        },
    )?;
    let mut logits = Vec::new();
    let mut values = Vec::new();
    loop {
        let root = driver.tree().root();
        let done = if gumbel {
            root.edges
                .iter()
                .map(|edge| u64::from(edge.visits))
                .sum::<u64>()
                >= 32
        } else {
            driver.simulations() >= 800
        };
        if done || matches!(root.state, NodeState::Proven(_)) {
            break;
        }
        let mut requests = Vec::new();
        for _ in 0..if gumbel { 1 } else { 256 } {
            if matches!(driver.tree().root().state, NodeState::Proven(_))
                || (!gumbel && driver.simulations() + requests.len() as u64 >= 800)
            {
                break;
            }
            match driver.select_leaf(rule)? {
                Step::Evaluate(request) => requests.push(request),
                Step::BackedUp => {}
                Step::Collision => break,
                Step::Capacity => return Err("measurement tree capacity exhausted".into()),
            }
        }
        if !requests.is_empty() {
            evaluate(&mut driver, &requests, &mut logits, &mut values)?;
        }
    }
    let output = if let Some(gumbel_config) = gumbel_config {
        SearchOutput::Gumbel {
            selected_edge: black_box(gumbel_config.action(driver.tree().root(), 1.0)?),
            policy_target: black_box(gumbel_config.policy_target(driver.tree().root())?),
        }
    } else {
        SearchOutput::Puct {
            best_move: black_box(driver.tree().best_move()),
        }
    };
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((driver, elapsed_ms, output))
}

#[derive(Default)]
struct Counts {
    depths: BTreeMap<usize, u64>,
    terminals: BTreeMap<String, u64>,
    winners: BTreeMap<String, u64>,
    terminal_leaves: u64,
    classified: u64,
    replayed: u64,
}

// 成功した探索では子1個につき分類1回であり、根からの深さだけ再生する。
// 取り消し、容量不足、再利用のない測定なので、作成後の木から正確に数えられる。
fn count(node: &Node, depth: usize, game: Option<&Game>, counts: &mut Counts) -> Result<()> {
    for edge in &node.edges {
        if edge.virtual_loss != 0 {
            return Err("unfinished virtual loss in measurement".into());
        }
        let Some(child) = edge.child.as_deref() else {
            continue;
        };
        counts.classified += 1;
        counts.replayed += (depth + 1) as u64;
        *counts.depths.entry(depth + 1).or_default() += 1;
        if child.edges.is_empty()
            && !matches!(
                child.state,
                NodeState::Expanded | NodeState::Unexpanded | NodeState::Pending { .. }
            )
        {
            counts.terminal_leaves += 1;
        }
        let mut next = game.cloned();
        if let Some(next) = next.as_mut()
            && let GameStatus::Finished(result) = next.play(edge.mv)?
        {
            if !child.edges.is_empty() {
                return Err("exact terminal has children".into());
            }
            let (reason, winner) = match result {
                GameResult::Win { winner, reason } => {
                    (format!("win_{reason:?}"), format!("{winner:?}"))
                }
                GameResult::Draw { reason } => (format!("draw_{reason:?}"), "draw".into()),
            };
            *counts.terminals.entry(reason).or_default() += 1;
            *counts.winners.entry(winner).or_default() += 1;
        }
        count(child, depth + 1, next.as_ref(), counts)?;
    }
    Ok(())
}

fn quantiles(mut times: Vec<f64>) -> Value {
    times.sort_by(f64::total_cmp);
    // nearest-rank: ceil(p*n)番目。標本数と併記する。
    let percentile = |p: f64| times[(p * times.len() as f64).ceil() as usize - 1];
    json!({"n": times.len(), "median": percentile(0.5), "p90": percentile(0.9), "p99": percentile(0.99)})
}

fn snapshots(cases: &[Case]) -> Result<Value> {
    let mut rows = Vec::new();
    for plies in [0, 100, 300, 600] {
        let game = if plies == 0 {
            Game::with_default_rules()
        } else {
            cases
                .iter()
                .find(|case| case.moves.len() == plies)
                .ok_or_else(|| format!("no game with {plies} plies; increase --games"))?
                .game()?
        };
        let source = SearchSnapshot::from_game(&game)?;
        let mut construction = Vec::new();
        let mut cloning = Vec::new();
        for _ in 0..SNAPSHOT_SAMPLES {
            let started = Instant::now();
            drop(black_box(SearchSnapshot::from_game(black_box(&game))?));
            construction.push(started.elapsed().as_secs_f64() * 1e6);
            let started = Instant::now();
            drop(black_box(black_box(&source).clone()));
            cloning.push(started.elapsed().as_secs_f64() * 1e6);
        }
        rows.push(json!({"plies": plies, "construction_us": quantiles(construction), "clone_us": quantiles(cloning)}));
    }
    Ok(json!(rows))
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut options = BTreeMap::new();
    while let Some(key) = args.next() {
        if !["--seed", "--games", "--adjudication"].contains(&key.as_str()) {
            return Err(format!("unknown option: {key}").into());
        }
        let value = args.next().ok_or("missing option value")?;
        if options.insert(key, value).is_some() {
            return Err("duplicate option".into());
        }
    }
    let seed = options
        .remove("--seed")
        .ok_or("--seed required")?
        .parse::<NonZeroU64>()?;
    let games = options
        .remove("--games")
        .ok_or("--games required")?
        .parse::<NonZeroUsize>()?
        .get();
    let adjudication = match options
        .remove("--adjudication")
        .ok_or("--adjudication required")?
        .as_str()
    {
        "before" => false,
        "after" => true,
        _ => return Err("--adjudication must be before or after".into()),
    };
    let cases = cases(seed, games)?;
    let snapshot_times = snapshots(&cases)?;
    let mut rows = Vec::new();
    let mut times: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for case in &cases {
        for (mode, gumbel) in [("gumbel", true), ("puct", false)] {
            let game = case.game()?;
            let (driver, ms, output) = search(&game, case.seed, gumbel)?;
            let mut counts = Counts {
                winners: BTreeMap::from([
                    ("Black".into(), 0),
                    ("White".into(), 0),
                    ("draw".into(), 0),
                ]),
                ..Counts::default()
            };
            count(
                driver.tree().root(),
                0,
                adjudication.then_some(&game),
                &mut counts,
            )?;
            let mut row = json!({
                "input": case.input()?,
                "mode": mode,
                "elapsed_ms": ms,
                "simulations": driver.simulations(),
                "replayed_moves": counts.replayed,
                "game_play_calls": if adjudication { counts.replayed } else { 0 },
                "depth_counts": counts.depths,
                "classified_leaves": counts.classified,
                "fixed_leaves": counts.terminal_leaves,
                "terminal_leaves": counts.terminals.values().sum::<u64>(),
                "approximate_cutoffs": if adjudication { counts.terminal_leaves - counts.terminals.values().sum::<u64>() } else { 0 },
                "terminal_reasons": counts.terminals,
                "terminal_winners": counts.winners,
            });
            match output {
                SearchOutput::Gumbel {
                    selected_edge,
                    policy_target,
                } => {
                    let mv = driver.tree().root().edges[selected_edge].mv;
                    row["selected_move"] = json!(usi::text_generated(game.position(), mv));
                    row["policy_target"] = json!(policy_target);
                }
                SearchOutput::Puct { best_move } => {
                    row["best_move"] =
                        json!(best_move.map(|mv| usi::text_generated(game.position(), mv)));
                }
            }
            rows.push(row);
            times.entry(mode.into()).or_default().push(ms);
            times
                .entry(format!("{mode}/{}", case.category))
                .or_default()
                .push(ms);
        }
    }
    let summaries: BTreeMap<_, _> = times
        .into_iter()
        .map(|(name, values)| (name, quantiles(values)))
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"seed": seed.get(), "games": games, "adjudication": adjudication, "case_count": cases.len(), "capacity_bytes": CAPACITY, "snapshots": snapshot_times, "summary_ms": summaries, "searches": rows})
        )?
    );
    Ok(())
}
