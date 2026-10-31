//! 対局用PUCTの推論バッチを比べ、推論回数、深さ、および根の訪問分布を測る。
//!
//! 深さには根の初回評価（0手）を含め、集計の平均は局面ごとの値を等しく重み付けする。
//! バッチ長の分位点は全局面の推論を合わせて求める。訪問回数が同じ手はUSI表記順とする。
//! 根の訪問合計が0なら全変動距離とその平均はnullとする。時間以外はCPUで決定的になる。

use std::collections::BTreeMap;
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::PathBuf;
use std::time::Instant;

use minase::nn::{Evaluator, ExecutionProvider, Network, OnnxEvaluator};
use minase::search::mcts::tree::{Node, NodeState, SelectionRule};
use minase::search::mcts::{Config, DrawValue, Driver, EvaluationRequest, Step};
use minase::search::{DRAW_SCORE, SearchSnapshot};
use minase_core::notation::{sfen::parse_extended_sfen, usi};
use minase_core::{Bitboard, Color, Game, GameResult, GameStatus, PieceKind, Rules};
use serde_json::{Value, json};

// minaseの実行ファイルと同じアロケータで対局履歴と探索木を確保する。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct Args {
    nn_model: PathBuf,
    provider: ExecutionProvider,
    fixture: PathBuf,
    positions: NonZeroUsize,
    simulations: NonZeroU64,
    batch_sizes: Vec<NonZeroUsize>,
}

impl Args {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut options = BTreeMap::new();
        while let Some(key) = args.next() {
            if ![
                "--nn-model",
                "--backend",
                "--cache-dir",
                "--fixture",
                "--positions",
                "--simulations",
                "--batch-sizes",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("unknown option: {key}").into());
            }
            let value = args
                .next()
                .filter(|value| !value.is_empty() && !value.starts_with("--"))
                .ok_or_else(|| format!("{key} requires a value"))?;
            if options.insert(key.clone(), value).is_some() {
                return Err(format!("duplicate option: {key}").into());
            }
        }
        let mut take = |key: &str| options.remove(key).ok_or_else(|| format!("{key} required"));
        let nn_model = take("--nn-model")?.into();
        let cache_root = take("--cache-dir")?.into();
        let provider = match take("--backend")?.as_str() {
            "tensorrt" => ExecutionProvider::TensorRt { cache_root },
            "cpu" => ExecutionProvider::Cpu,
            _ => return Err("--backend must be tensorrt or cpu".into()),
        };
        let fixture = take("--fixture")?.into();
        let positions = take("--positions")?.parse()?;
        let simulations = take("--simulations")?.parse()?;
        let mut batch_sizes = take("--batch-sizes")?
            .split(',')
            .map(str::parse::<NonZeroUsize>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        batch_sizes.sort_unstable();
        if batch_sizes.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("--batch-sizes contains a duplicate".into());
        }
        Ok(Self {
            nn_model,
            provider,
            fixture,
            positions,
            simulations,
            batch_sizes,
        })
    }
}

struct Case {
    sfen: String,
    game: Game,
}

fn read_cases(reader: impl BufRead, count: NonZeroUsize) -> Result<Vec<Case>> {
    let mut cases = Vec::new();
    for (index, line) in reader.lines().take(count.get()).enumerate() {
        let parse = || -> Result<Case> {
            let row: Value = serde_json::from_str(&line?)?;
            let sfen = row["sfen"].as_str().ok_or("sfen must be a string")?;
            let setup = parse_extended_sfen(sfen, Rules::ENGINE_DEFAULT.moves)?;
            let game = Game::from_position(Rules::ENGINE_DEFAULT, setup.restore_position()?);
            validate_root(&game)?;
            Ok(Case {
                sfen: sfen.to_owned(),
                game,
            })
        };
        cases.push(parse().map_err(|error| format!("fixture line {}: {error}", index + 1))?);
    }
    if cases.len() != count.get() {
        return Err(format!("fixture has {} positions, need {count}", cases.len()).into());
    }
    Ok(cases)
}

// Game::from_positionは開始時に裁定しない。履歴のないengine-defaultの根について、
// 第21条・第22条の終局条件を検査する。駒枯れの猶予中は審判と同じく詰みを判定しない。
fn validate_root(game: &Game) -> Result<()> {
    let position = game.position();
    let royals = Color::ALL.map(|color| position.royal_pieces(color));
    if game.status() != GameStatus::Ongoing || royals.iter().any(|royal| royal.is_empty()) {
        return Err("measurement root is terminal: missing royal or finished game".into());
    }
    let moves = game.legal_moves();
    if moves.is_empty() {
        return Err("measurement root is terminal: no legal moves".into());
    }
    let remaining = Color::ALL.map(|color| {
        let dead = Bitboard::from_squares(
            (position.pieces_of_kind(color, PieceKind::Pawn)
                | position.pieces_of_kind(color, PieceKind::Lance))
            .into_iter()
            .filter(|square| square.rank() == if color == Color::Black { 11 } else { 0 }),
        );
        position.pieces_of(color) & !dead
    });
    if remaining.map(|pieces| pieces.popcount()) == [1, 1]
        && Color::ALL.into_iter().all(|color| {
            royals[color.index()].into_iter().all(|square| {
                position
                    .attackers_to_by(color.opposite(), square, position.occupied())
                    .is_empty()
            })
        })
    {
        return Err("measurement root is terminal: piece exhaustion draw".into());
    }
    let extra = match remaining.map(|pieces| pieces.popcount()) {
        [2, 1] => Some(Color::Black),
        [1, 2] => Some(Color::White),
        _ => None,
    };
    if let Some(color) = extra {
        let non_royals = remaining[color.index()] & !royals[color.index()];
        let unpromoted_pawn = non_royals.into_iter().any(|square| {
            position.piece_at(square).is_some_and(|piece| {
                !piece.is_promoted()
                    && matches!(piece.kind(), Some(PieceKind::Pawn | PieceKind::GoBetween))
            })
        });
        if !unpromoted_pawn {
            let grace = position.side_to_move() != color
                && moves.iter().any(|&mv| {
                    position
                        .captured_squares(mv)
                        .into_iter()
                        .flatten()
                        .any(|square| remaining[color.index()].contains(square))
                });
            return if grace {
                Ok(())
            } else {
                Err("measurement root is terminal: piece exhaustion win".into())
            };
        }
    }
    for mv in moves {
        let mut next = game.clone();
        let escapes = match next.play(mv)? {
            GameStatus::Finished(GameResult::Win { winner, .. }) => {
                winner == position.side_to_move()
            }
            GameStatus::Finished(GameResult::Draw { .. }) => true,
            GameStatus::Ongoing => !next.legal_moves().into_iter().any(|reply| {
                let captured = next.position().captured_squares(reply);
                next.position()
                    .royal_pieces(position.side_to_move())
                    .into_iter()
                    .all(|square| captured.contains(&Some(square)))
            }),
        };
        if escapes {
            return Ok(());
        }
    }
    Err("measurement root is terminal: mate".into())
}

fn evaluate_batch(
    driver: &mut Driver,
    evaluator: &mut dyn Evaluator,
    requests: &[EvaluationRequest],
    logits: &mut Vec<f32>,
    values: &mut Vec<f32>,
) -> Result<()> {
    let queries: Vec<_> = requests.iter().map(EvaluationRequest::query).collect();
    evaluator.evaluate(&queries, logits, values)?;
    let total_logits = queries.iter().map(|query| query.moves.len()).sum::<usize>();
    if values.len() != requests.len() || logits.len() != total_logits {
        return Err("evaluator returned an incorrect output shape".into());
    }
    let mut offset = 0;
    for (request, &value) in requests.iter().zip(values.iter()) {
        let end = offset + request.query().moves.len();
        driver.evaluate(request, &logits[offset..end], value)?;
        offset = end;
    }
    Ok(())
}

#[derive(Default)]
struct Depths {
    count: u64,
    sum: u64,
    max: usize,
}

// 新しい木では各節点の評価は1回だけで、評価済みの印は証明後も残る。
// 終端の逆伝播を除き、根を含む評価依頼の深さを計測区間の外で数える。
fn count_depths(node: &Node, depth: usize, depths: &mut Depths) {
    if node.value.is_some() {
        depths.count += 1;
        depths.sum += depth as u64;
        depths.max = depths.max.max(depth);
    }
    for child in node.edges.iter().filter_map(|edge| edge.child.as_deref()) {
        count_depths(child, depth + 1, depths);
    }
}

struct Measurement {
    batch_size: usize,
    batch_lengths: Vec<usize>,
    collisions: u64,
    backed_up: u64,
    simulations: u64,
    leaf_depth_mean: f64,
    leaf_depth_max: usize,
    root_visits: BTreeMap<String, u32>,
    elapsed_seconds: f64,
}

impl Measurement {
    fn ranked_moves(&self) -> Vec<(&str, u32)> {
        let mut moves: Vec<_> = self
            .root_visits
            .iter()
            .map(|(mv, &visits)| (mv.as_str(), visits))
            .collect();
        moves.sort_by(|(a_move, a_visits), (b_move, b_visits)| {
            b_visits.cmp(a_visits).then(a_move.cmp(b_move))
        });
        moves
    }

    fn compare(&self, baseline: &Self) -> Result<(bool, Option<f64>)> {
        Ok((
            self.ranked_moves()[0].0 == baseline.ranked_moves()[0].0,
            total_variation(&self.root_visits, &baseline.root_visits)?,
        ))
    }

    fn detail(&self, baseline: &Self) -> Result<Value> {
        let ranking = self.ranked_moves();
        let (agrees, distance) = self.compare(baseline)?;
        let top: Vec<_> = ranking
            .iter()
            .take(10)
            .map(|(mv, visits)| json!({"move": mv, "visits": visits}))
            .collect();
        Ok(json!({
            "batch_size": self.batch_size,
            "inference_calls": self.batch_lengths.len(),
            "batch_lengths": self.batch_lengths,
            "collisions": self.collisions,
            "backed_up": self.backed_up,
            "simulations": self.simulations,
            "leaf_depth_mean": self.leaf_depth_mean,
            "leaf_depth_max": self.leaf_depth_max,
            "root_top_10": top,
            "most_visited_move": ranking[0].0,
            "elapsed_seconds": self.elapsed_seconds,
            "best_move_matches_baseline": agrees,
            "total_variation_distance": distance,
        }))
    }
}

fn search(
    game: &Game,
    evaluator: &mut dyn Evaluator,
    config: Config,
    simulations: NonZeroU64,
) -> Result<Measurement> {
    let snapshot = SearchSnapshot::from_game(game)?;
    let mut driver = Driver::new(
        snapshot,
        config.capacity_bytes,
        DrawValue::Game {
            cp_per_logit: config.cp_per_logit,
        },
    )?;
    let rule = SelectionRule::Puct(&config.puct);
    let mut logits = Vec::new();
    let mut values = Vec::new();
    let mut batch_lengths = Vec::new();
    let mut collisions = 0;
    let mut backed_up = 0;
    let mut capacity_exhausted = false;
    let started = Instant::now();
    let Step::Evaluate(root) = driver.select_leaf(rule)? else {
        return Err("new driver did not request root evaluation".into());
    };
    evaluate_batch(&mut driver, evaluator, &[root], &mut logits, &mut values)?;
    batch_lengths.push(1);
    // engine.rsのrunと同じ順序で集め、nodesだけで制限する。
    while !capacity_exhausted
        && driver.simulations() < simulations.get()
        && !matches!(driver.tree().root().state, NodeState::Proven(_))
    {
        let batch_size =
            (config.batch_size.get() as u64).min(simulations.get() - driver.simulations());
        let mut requests = Vec::new();
        for _ in 0..batch_size {
            if driver.simulations() + requests.len() as u64 >= simulations.get()
                || matches!(driver.tree().root().state, NodeState::Proven(_))
            {
                break;
            }
            match driver.select_leaf(rule)? {
                Step::Evaluate(request) => requests.push(request),
                Step::BackedUp => backed_up += 1,
                Step::Collision => {
                    collisions += 1;
                    break;
                }
                Step::Capacity => {
                    capacity_exhausted = true;
                    break;
                }
            }
        }
        if !requests.is_empty() {
            evaluate_batch(&mut driver, evaluator, &requests, &mut logits, &mut values)?;
            batch_lengths.push(requests.len());
        }
    }
    let elapsed_seconds = started.elapsed().as_secs_f64();
    let mut depths = Depths::default();
    count_depths(driver.tree().root(), 0, &mut depths);
    let evaluated = batch_lengths
        .iter()
        .map(|&length| length as u64)
        .sum::<u64>();
    if depths.count != evaluated || evaluated + backed_up != driver.simulations() {
        return Err("evaluation counts do not match the completed search".into());
    }
    Ok(Measurement {
        batch_size: config.batch_size.get(),
        batch_lengths,
        collisions,
        backed_up,
        simulations: driver.simulations(),
        leaf_depth_mean: depths.sum as f64 / depths.count as f64,
        leaf_depth_max: depths.max,
        root_visits: driver
            .tree()
            .root()
            .edges
            .iter()
            .map(|edge| (usi::text_generated(game.position(), edge.mv), edge.visits))
            .collect(),
        elapsed_seconds,
    })
}

fn total_variation(
    visits: &BTreeMap<String, u32>,
    baseline: &BTreeMap<String, u32>,
) -> Result<Option<f64>> {
    if visits.keys().ne(baseline.keys()) {
        return Err("root move sets differ between batch sizes".into());
    }
    let sum = visits.values().map(|&n| u64::from(n)).sum::<u64>();
    let baseline_sum = baseline.values().map(|&n| u64::from(n)).sum::<u64>();
    if sum == 0 || baseline_sum == 0 {
        return Ok(None);
    }
    Ok(Some(
        0.5 * visits
            .values()
            .zip(baseline.values())
            .map(|(&a, &b)| (f64::from(a) / sum as f64 - f64::from(b) / baseline_sum as f64).abs())
            .sum::<f64>(),
    ))
}

fn quantiles(mut lengths: Vec<usize>) -> Value {
    lengths.sort_unstable();
    // nearest-rank: ceil(p*n)番目。根の初回評価を含む標本数と併記する。
    let percentile = |p: f64| lengths[(p * lengths.len() as f64).ceil() as usize - 1];
    json!({"n": lengths.len(), "median": percentile(0.5), "p90": percentile(0.9), "p99": percentile(0.99)})
}

fn summary(searches: &[Vec<Measurement>], index: usize) -> Result<Value> {
    let runs: Vec<_> = searches.iter().map(|runs| &runs[index]).collect();
    let comparisons = searches
        .iter()
        .map(|runs| runs[index].compare(&runs[0]))
        .collect::<Result<Vec<_>>>()?;
    let count = runs.len() as f64;
    let mean_distance = comparisons
        .iter()
        .try_fold(0.0, |sum, (_, distance)| Some(sum + (*distance)?))
        .map(|sum| sum / count);
    Ok(json!({
        "batch_size": runs[0].batch_size,
        "inference_calls_mean": runs.iter().map(|run| run.batch_lengths.len() as f64).sum::<f64>() / count,
        "collisions_mean": runs.iter().map(|run| run.collisions as f64).sum::<f64>() / count,
        "leaf_depth_mean": runs.iter().map(|run| run.leaf_depth_mean).sum::<f64>() / count,
        "batch_length_quantiles": quantiles(runs.iter().flat_map(|run| run.batch_lengths.iter().copied()).collect()),
        "elapsed_seconds_total": runs.iter().map(|run| run.elapsed_seconds).sum::<f64>(),
        "best_move_agreement_rate": comparisons.iter().filter(|(agrees, _)| *agrees).count() as f64 / count,
        "total_variation_distance_mean": mean_distance,
    }))
}

fn main() -> Result<()> {
    let args = Args::parse(std::env::args().skip(1))?;
    let cases = read_cases(BufReader::new(File::open(&args.fixture)?), args.positions)?;
    // USIと同じく、埋め込みNNUEの勝率尺度とConfig::newの容量・PUCT係数を使う。
    let config = Config::new(minase::eval::weights()?.cp_per_logit());
    let network = Network::read(&args.nn_model)?;
    let max_batch = *args.batch_sizes.last().ok_or("--batch-sizes is empty")?;
    let mut evaluator = OnnxEvaluator::new(&network, &args.provider, max_batch)?;
    let mut searches = Vec::new();
    let mut details = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let mut runs = Vec::new();
        for &batch_size in &args.batch_sizes {
            runs.push(search(
                &case.game,
                &mut evaluator,
                Config {
                    batch_size,
                    ..config
                },
                args.simulations,
            )?);
        }
        let rows = runs
            .iter()
            .map(|run| run.detail(&runs[0]))
            .collect::<Result<Vec<_>>>()?;
        details.push(json!({"line": index + 1, "sfen": case.sfen, "searches": rows}));
        searches.push(runs);
    }
    let summaries = (0..args.batch_sizes.len())
        .map(|index| summary(&searches, index))
        .collect::<Result<Vec<_>>>()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "nn_model": args.nn_model,
            "backend": args.provider.name(),
            "fixture": args.fixture,
            "positions": args.positions.get(),
            "simulations": args.simulations.get(),
            "batch_sizes": args.batch_sizes.iter().map(|size| size.get()).collect::<Vec<_>>(),
            "baseline_batch_size": args.batch_sizes[0].get(),
            "capacity_bytes": config.capacity_bytes,
            "draw_value": {"mode": "game", "cp_per_logit": config.cp_per_logit, "root_cp": DRAW_SCORE},
            "summary": summaries,
            "positions_detail": details,
        }))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase::nn::{NnError, Query, UniformEvaluator};
    use minase_core::{Move, PieceCode, Position, PositionBuilder, Square};

    fn config(batch_size: usize) -> Config {
        Config {
            batch_size: NonZeroUsize::new(batch_size).unwrap(),
            ..Config::new(1072.65)
        }
    }

    fn pieces(side: Color, pieces: &[(u8, u8, Color, PieceKind)]) -> Game {
        let mut builder = PositionBuilder::new(side);
        for &(file, rank, color, kind) in pieces {
            builder
                .put(
                    Square::new(file, rank).unwrap(),
                    PieceCode::new(color, kind).unwrap(),
                )
                .unwrap();
        }
        Game::from_position(Rules::ENGINE_DEFAULT, builder.finish().unwrap())
    }

    fn row(game: &Game) -> String {
        json!({"sfen": format!("{} - 1 -", minase_core::to_sfen(game.position()))}).to_string()
    }

    // 指示書の根の初回評価、残り回数によるバッチ上限、および深さ0の算入を固定する。
    #[test]
    fn uniform_search_counts_root_and_partial_batch() {
        let game = Game::with_default_rules();
        let root = search(
            &game,
            &mut UniformEvaluator,
            config(8),
            NonZeroU64::new(1).unwrap(),
        )
        .unwrap();
        assert_eq!(root.batch_lengths, [1]);
        assert_eq!(root.simulations, 1);
        assert_eq!(root.leaf_depth_mean, 0.0);
        assert_eq!(root.leaf_depth_max, 0);
        assert_eq!(root.compare(&root).unwrap(), (true, None));
        let shallow = search(
            &game,
            &mut UniformEvaluator,
            config(8),
            NonZeroU64::new(4).unwrap(),
        )
        .unwrap();
        assert_eq!(shallow.batch_lengths, [1, 3]);
        assert_eq!(shallow.leaf_depth_mean, 0.75);
        assert_eq!(shallow.leaf_depth_max, 1);
        let run = search(
            &game,
            &mut UniformEvaluator,
            config(4),
            NonZeroU64::new(10).unwrap(),
        )
        .unwrap();
        assert_eq!(run.batch_lengths, [1, 4, 4, 1]);
        assert_eq!((run.simulations, run.backed_up, run.collisions), (10, 0, 0));
        assert_eq!(run.root_visits.values().sum::<u32>(), 9);
        assert_eq!(
            run.detail(&run).unwrap()["root_top_10"]
                .as_array()
                .unwrap()
                .len(),
            10
        );
    }

    // 衝突はシミュレーションに数えず、収集を切り上げて評価してから再開する。
    #[test]
    fn collisions_do_not_consume_the_budget_and_cpu_independent_counts_repeat() {
        let game = Game::with_default_rules();
        let run = search(
            &game,
            &mut UniformEvaluator,
            config(256),
            NonZeroU64::new(128).unwrap(),
        )
        .unwrap();
        let repeated = search(
            &game,
            &mut UniformEvaluator,
            config(256),
            NonZeroU64::new(128).unwrap(),
        )
        .unwrap();
        assert!(run.collisions > 0);
        assert!((run.collisions as usize) < run.batch_lengths.len());
        assert_eq!(run.simulations, 128);
        assert_eq!(
            run.batch_lengths.iter().sum::<usize>() as u64 + run.backed_up,
            128
        );
        assert_eq!(run.root_visits.values().sum::<u32>(), 127);
        assert!(run.leaf_depth_max > 1);
        let mut first = run.detail(&run).unwrap();
        let mut second = repeated.detail(&repeated).unwrap();
        first.as_object_mut().unwrap().remove("elapsed_seconds");
        second.as_object_mut().unwrap().remove("elapsed_seconds");
        assert_eq!(first, second);
    }

    struct FocusedEvaluator(Move);

    impl Evaluator for FocusedEvaluator {
        fn evaluate(
            &mut self,
            queries: &[Query<'_>],
            logits: &mut Vec<f32>,
            values: &mut Vec<f32>,
        ) -> std::result::Result<(), NnError> {
            logits.clear();
            values.clear();
            for query in queries {
                logits.extend(
                    query
                        .moves
                        .iter()
                        .map(|mv| if *mv == self.0 { 0.0 } else { -1000.0 }),
                );
                values.push(0.5);
            }
            Ok(())
        }
    }

    // 終局の葉は推論と深さに加えず、BackedUpだけを数える。証明された根では停止する。
    #[test]
    fn terminal_backups_and_proof_stop_do_not_add_inference() {
        let draw = pieces(
            Color::Black,
            &[
                (0, 0, Color::Black, PieceKind::King),
                (11, 11, Color::White, PieceKind::King),
                (0, 1, Color::White, PieceKind::Pawn),
            ],
        );
        let win = pieces(
            Color::Black,
            &[
                (0, 0, Color::Black, PieceKind::King),
                (5, 4, Color::Black, PieceKind::Rook),
                (5, 9, Color::White, PieceKind::King),
                (11, 5, Color::White, PieceKind::Pawn),
            ],
        );
        for (game, from, to, expected) in [(draw, (0, 0), (0, 1), 64), (win, (5, 4), (5, 9), 2)] {
            validate_root(&game).unwrap();
            let mv = Move {
                from: Square::new(from.0, from.1).unwrap(),
                mid: None,
                to: Square::new(to.0, to.1).unwrap(),
                promote: false,
            };
            let run = search(
                &game,
                &mut FocusedEvaluator(mv),
                config(8),
                NonZeroU64::new(64).unwrap(),
            )
            .unwrap();
            assert_eq!(run.batch_lengths, [1]);
            assert_eq!(run.simulations, expected);
            assert_eq!(run.backed_up, expected - 1);
            assert_eq!(run.leaf_depth_max, 0);
            assert_eq!(run.leaf_depth_mean, 0.0);
            assert_eq!(
                run.root_visits[&usi::text_generated(game.position(), mv)],
                (expected - 1) as u32
            );
        }
    }

    // 全変動距離は全合法手の正規化した分布のL1距離の半分である。
    #[test]
    fn total_variation_normalizes_each_search_and_quantiles_use_nearest_rank() {
        let distribution = |counts: [u32; 3]| {
            ["a", "b", "c"]
                .into_iter()
                .zip(counts)
                .map(|(mv, n)| (mv.to_owned(), n))
                .collect()
        };
        let a = distribution([2, 2, 0]);
        let b = distribution([0, 4, 4]);
        assert_eq!(total_variation(&a, &a).unwrap(), Some(0.0));
        assert_eq!(total_variation(&a, &b).unwrap(), Some(0.5));
        assert_eq!(total_variation(&b, &a).unwrap(), Some(0.5));
        assert_eq!(
            total_variation(&distribution([1, 0, 0]), &distribution([0, 0, 3])).unwrap(),
            Some(1.0)
        );
        assert_eq!(total_variation(&a, &distribution([0, 0, 0])).unwrap(), None);
        let mut tail_a: BTreeMap<_, _> = (0..12).map(|n| (format!("move{n:02}"), 100)).collect();
        tail_a.insert("move10".into(), 10);
        tail_a.insert("move11".into(), 0);
        let mut tail_b = tail_a.clone();
        tail_b.insert("move10".into(), 0);
        tail_b.insert("move11".into(), 10);
        assert!((total_variation(&tail_a, &tail_b).unwrap().unwrap() - 1.0 / 101.0).abs() < 1e-12);
        assert_eq!(
            quantiles((1..=10).rev().collect()),
            json!({"n": 10, "median": 5, "p90": 9, "p99": 10})
        );
    }

    // 指定行数までを順番に読み、不足、不正な行、終局局面を読み飛ばさず拒否する。
    #[test]
    fn fixtures_preserve_order_and_reject_terminal_roots() {
        let initial = Game::with_default_rules();
        let first = row(&initial);
        let input = format!("{first}\ninvalid json\n");
        let cases = read_cases(input.as_bytes(), NonZeroUsize::new(1).unwrap()).unwrap();
        assert_eq!(cases[0].game.position(), &Position::initial());
        let mut next = initial.clone();
        next.play(next.legal_moves()[0]).unwrap();
        let ordered = format!("{first}\n{}\n", row(&next));
        let cases = read_cases(ordered.as_bytes(), NonZeroUsize::new(2).unwrap()).unwrap();
        assert_eq!(cases[0].game.position(), initial.position());
        assert_eq!(cases[1].game.position(), next.position());
        let lion = json!({"sfen": format!("{} 6f 7 -", minase_core::to_sfen(initial.position()))})
            .to_string();
        let cases = read_cases(lion.as_bytes(), NonZeroUsize::new(1).unwrap()).unwrap();
        assert!(cases[0].game.position().lion_capture_square().is_some());
        assert!(read_cases(input.as_bytes(), NonZeroUsize::new(2).unwrap()).is_err());
        assert!(read_cases(first.as_bytes(), NonZeroUsize::new(2).unwrap()).is_err());
        for input in ["", "{}", "{\"sfen\":3}", "{\"sfen\":\"invalid\"}"] {
            assert!(read_cases(input.as_bytes(), NonZeroUsize::new(1).unwrap()).is_err());
        }
        for game in [
            pieces(Color::Black, &[(0, 0, Color::Black, PieceKind::King)]),
            pieces(
                Color::Black,
                &[
                    (0, 0, Color::Black, PieceKind::King),
                    (11, 11, Color::White, PieceKind::King),
                ],
            ),
            pieces(
                Color::Black,
                &[
                    (0, 0, Color::Black, PieceKind::King),
                    (11, 11, Color::White, PieceKind::King),
                    (5, 4, Color::Black, PieceKind::Rook),
                ],
            ),
            pieces(
                Color::White,
                &[
                    (2, 2, Color::Black, PieceKind::King),
                    (0, 0, Color::White, PieceKind::King),
                    (1, 1, Color::Black, PieceKind::FreeKing),
                    (11, 5, Color::White, PieceKind::Pawn),
                ],
            ),
        ] {
            assert!(read_cases(row(&game).as_bytes(), NonZeroUsize::new(1).unwrap()).is_err());
        }
        let grace = pieces(
            Color::Black,
            &[
                (0, 0, Color::Black, PieceKind::King),
                (11, 11, Color::White, PieceKind::King),
                (0, 1, Color::White, PieceKind::Rook),
            ],
        );
        validate_root(&grace).unwrap();
    }

    // 引数には既定値を設けず、比較の基準は入力順によらず最小のバッチとする。
    #[test]
    fn arguments_require_explicit_nonzero_values_and_unique_batches() {
        let valid = [
            "--nn-model",
            "net.onnx",
            "--backend",
            "cpu",
            "--cache-dir",
            "/tmp/cache",
            "--fixture",
            "positions.jsonl",
            "--positions",
            "3",
            "--simulations",
            "64",
            "--batch-sizes",
            "32,1,8",
        ];
        let parse = |args: &[&str]| Args::parse(args.iter().map(|arg| (*arg).to_owned()));
        let args = parse(&valid).unwrap();
        assert_eq!(
            args.batch_sizes.iter().map(|n| n.get()).collect::<Vec<_>>(),
            [1, 8, 32]
        );
        for (index, value) in [
            (3, "cuda"),
            (9, "0"),
            (11, "0"),
            (13, ""),
            (13, "1,0"),
            (13, "1,1"),
            (13, "1,"),
            (13, "8,no"),
            (9, "184467440737095516160"),
        ] {
            let mut args = valid;
            args[index] = value;
            assert!(parse(&args).is_err());
        }
        for pair in 0..7 {
            let mut args = valid.to_vec();
            args.drain(pair * 2..pair * 2 + 2);
            assert!(parse(&args).is_err());
        }
        assert!(parse(&valid[..13]).is_err());
        let mut duplicate = valid.to_vec();
        duplicate.extend(["--positions", "3"]);
        assert!(parse(&duplicate).is_err());
        let mut unknown = valid.to_vec();
        unknown.extend(["--unknown", "3"]);
        assert!(parse(&unknown).is_err());
    }
}
