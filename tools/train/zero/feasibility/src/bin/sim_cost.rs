use minase::training::records::Reader;
use minase_core::rng::splitmix64;
use minase_core::{Move, MoveGenerator, MoveRules, Position};
use serde_json::json;
use std::collections::BTreeMap;
use std::error::Error;
use std::fs::File;
use std::hint::black_box;
use std::io::{BufReader, Write};
use std::time::{Duration, Instant};

#[path = "../encoding.rs"]
mod encoding;

use encoding::{INPUT_SIZE, POLICY_SIZE, encode, policy_label};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct Config {
    data: Vec<String>,
    positions: usize,
    stride: usize,
    simulations: usize,
    considered: usize,
    seed: u64,
    breakdown: bool,
}

impl Config {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut data = Vec::new();
        let mut options = BTreeMap::new();
        while let Some(key) = args.next() {
            if !matches!(
                key.as_str(),
                "--data"
                    | "--positions"
                    | "--stride"
                    | "--simulations"
                    | "--considered"
                    | "--seed"
                    | "--breakdown"
            ) {
                return Err(format!("未知の引数: {key}").into());
            }
            let value = args.next().ok_or_else(|| format!("{key} の値が必要"))?;
            if key == "--data" {
                for path in value.split(',') {
                    if path.is_empty() {
                        return Err("--data に空のパスがある".into());
                    }
                    data.push(path.to_owned());
                }
            } else if options.insert(key.clone(), value).is_some() {
                return Err(format!("引数が重複している: {key}").into());
            }
        }
        if data.is_empty() {
            return Err("--data が必要".into());
        }
        let mut take = |key: &str| options.remove(key).ok_or_else(|| format!("{key} が必要"));
        let positions = take("--positions")?.parse()?;
        let stride = take("--stride")?.parse()?;
        let simulations = take("--simulations")?.parse()?;
        let considered = take("--considered")?.parse()?;
        let seed = take("--seed")?.parse()?;
        let breakdown = match take("--breakdown")?.as_str() {
            "on" => true,
            "off" => false,
            _ => return Err("--breakdown は on または off".into()),
        };
        if positions == 0 || stride == 0 || simulations == 0 || considered == 0 {
            return Err("positions、stride、simulations、considered は1以上が必要".into());
        }
        Ok(Self {
            data,
            positions,
            stride,
            simulations,
            considered,
            seed,
            breakdown,
        })
    }
}

struct Sample {
    position: Position,
    rules: MoveRules,
    legal_moves: usize,
}

fn read_samples(config: &Config) -> Result<Vec<Sample>> {
    let mut samples = Vec::new();
    let mut ordinal = 0_u64;
    let mut moves = Vec::new();
    for path in &config.data {
        let file = File::open(path).map_err(|e| format!("{path}: {e}"))?;
        let mut reader = Reader::new(BufReader::new(file)).map_err(|e| format!("{path}: {e}"))?;
        let rules = reader.header().rules().moves;
        let generator = MoveGenerator::new(rules);
        while let Some(game) = reader.read_game().map_err(|e| format!("{path}: {e}"))? {
            for position in game.positions() {
                let selected = ordinal.is_multiple_of(config.stride as u64);
                ordinal += 1;
                if !selected {
                    continue;
                }
                moves.clear();
                generator.generate_moves(position, &mut moves);
                if moves.is_empty() {
                    continue;
                }
                samples.push(Sample {
                    position: position.clone(),
                    rules,
                    legal_moves: moves.len(),
                });
                if samples.len() == config.positions {
                    return Ok(samples);
                }
            }
        }
    }
    Err(format!(
        "抽出できた局面は{}個で、指定の{}個に足りない",
        samples.len(),
        config.positions
    )
    .into())
}

struct Rng(u64);

impl Rng {
    fn uniform_open(&mut self) -> f64 {
        let bits = splitmix64(self.0);
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        ((bits >> 12) as f64 + 0.5) / 4_503_599_627_370_496.0
    }

    fn gumbel(&mut self) -> f32 {
        (-(-self.uniform_open().ln()).ln()) as f32
    }
}

struct Evaluator {
    logits: Vec<f32>,
    input: Vec<f32>,
}

impl Evaluator {
    fn new(rng: &mut Rng) -> Self {
        Self {
            logits: (0..POLICY_SIZE)
                .map(|_| (rng.uniform_open() * 4.0 - 2.0) as f32)
                .collect(),
            input: vec![0.0; INPUT_SIZE],
        }
    }
}

const SELECTION: usize = 0;
const MAKE_UNMAKE: usize = 1;
const MOVE_GENERATION: usize = 2;
const ENCODING: usize = 3;
const POLICY: usize = 4;
const BACKUP: usize = 5;
const SECTION_NAMES: [&str; 6] = [
    "selection",
    "make_unmake",
    "move_generation",
    "encoding",
    "policy",
    "backup",
];

#[derive(Default)]
struct Timings([Duration; 6]);

fn tick<const BREAKDOWN: bool>() -> Option<Instant> {
    if BREAKDOWN {
        Some(Instant::now())
    } else {
        None
    }
}

impl Timings {
    fn finish(&mut self, section: usize, start: Option<Instant>) {
        if let Some(start) = start {
            self.0[section] += start.elapsed();
        }
    }
}

struct Edge {
    mv: Move,
    logit: f32,
    prior: f32,
    visits: usize,
    value_sum: f32,
    child: Option<usize>,
}

struct Node {
    value: f32,
    children: Vec<Edge>,
}

impl Node {
    fn terminal(value: f32) -> Self {
        Self {
            value,
            children: Vec::new(),
        }
    }
}

fn expand<const BREAKDOWN: bool>(
    position: &Position,
    generator: &MoveGenerator,
    evaluator: &mut Evaluator,
    timings: &mut Timings,
) -> Node {
    let side = position.side_to_move();
    if position.royal_pieces(side).is_empty() {
        return Node::terminal(0.0);
    }
    if position.royal_pieces(side.opposite()).is_empty() {
        return Node::terminal(1.0);
    }

    let start = tick::<BREAKDOWN>();
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    timings.finish(MOVE_GENERATION, start);
    if moves.is_empty() {
        return Node::terminal(0.0);
    }

    let start = tick::<BREAKDOWN>();
    encode(position, &mut evaluator.input);
    // 推論を呼ばなくても、バッファへの書き込みを最適化で消させない。
    black_box(evaluator.input.as_slice());
    timings.finish(ENCODING, start);
    let hash = splitmix64(position.zobrist() ^ position.rights_zobrist());
    let value = (hash >> 40) as f32 / 16_777_215.0;

    let start = tick::<BREAKDOWN>();
    let mut children: Vec<_> = moves
        .into_iter()
        .map(|mv| Edge {
            mv,
            logit: evaluator.logits[policy_label(mv, side)],
            prior: 0.0,
            visits: 0,
            value_sum: 0.0,
            child: None,
        })
        .collect();
    let max_logit = children
        .iter()
        .map(|edge| edge.logit)
        .fold(f32::NEG_INFINITY, f32::max);
    let mut sum = 0.0;
    for edge in &mut children {
        edge.logit -= max_logit;
        edge.prior = edge.logit.exp();
        sum += edge.prior;
    }
    for edge in &mut children {
        edge.prior = (edge.prior / sum).max(f32::MIN_POSITIVE);
    }
    timings.finish(POLICY, start);
    Node { value, children }
}

fn considered_visits(m: usize, n: usize) -> Vec<usize> {
    if m <= 1 {
        return (0..n).collect();
    }
    let log2max = (usize::BITS - (m - 1).leading_zeros()) as usize;
    let mut sequence = Vec::with_capacity(n);
    let mut visits = vec![0; m];
    let mut considered = m;
    while sequence.len() < n {
        let extra = (n / log2max / considered).max(1);
        for _ in 0..extra {
            for &visit in &visits[..considered] {
                sequence.push(visit);
                if sequence.len() == n {
                    return sequence;
                }
            }
            for visit in &mut visits[..considered] {
                *visit += 1;
            }
        }
        considered = (considered / 2).max(2);
    }
    sequence
}

fn completed_q(node: &Node, output: &mut Vec<f32>) -> usize {
    let mut visits = 0;
    let mut max_visits = 0;
    let mut visited_prior = 0.0;
    let mut weighted_q = 0.0;
    for edge in &node.children {
        visits += edge.visits;
        max_visits = max_visits.max(edge.visits);
        if edge.visits != 0 {
            visited_prior += edge.prior;
            weighted_q += edge.prior * edge.value_sum / edge.visits as f32;
        }
    }
    let mixed = if visits == 0 {
        node.value
    } else {
        (node.value + visits as f32 * weighted_q / visited_prior) / (visits as f32 + 1.0)
    };
    output.clear();
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for edge in &node.children {
        let q = if edge.visits == 0 {
            mixed
        } else {
            edge.value_sum / edge.visits as f32
        };
        min = min.min(q);
        max = max.max(q);
        output.push(q);
    }
    let range = (max - min).max(1e-8);
    let scale = (50.0 + max_visits as f32) * 0.1;
    for q in output {
        *q = scale * ((*q - min) / range);
    }
    visits
}

fn root_action(node: &Node, gumbels: &[f32], transformed_q: &[f32], visit: usize) -> usize {
    let mut best = None;
    let mut best_score = f32::NEG_INFINITY;
    for (index, edge) in node.children.iter().enumerate() {
        if edge.visits == visit {
            let score = gumbels[index] + edge.logit + transformed_q[index];
            if score > best_score {
                best = Some(index);
                best_score = score;
            }
        }
    }
    best.expect("訪問回数表に対応する根の候補がある")
}

fn improved_policy(node: &Node, transformed_q: &mut [f32]) {
    let mut max = f32::NEG_INFINITY;
    for (value, edge) in transformed_q.iter_mut().zip(&node.children) {
        *value += edge.logit;
        max = max.max(*value);
    }
    let mut sum = 0.0;
    for value in transformed_q.iter_mut() {
        *value = (*value - max).exp();
        sum += *value;
    }
    for value in transformed_q {
        *value /= sum;
    }
}

fn interior_action(node: &Node, visits: usize, transformed_q: &mut [f32]) -> usize {
    improved_policy(node, transformed_q);
    let mut best = 0;
    let mut best_score = f32::NEG_INFINITY;
    for (index, edge) in node.children.iter().enumerate() {
        let score = transformed_q[index] - edge.visits as f32 / (visits as f32 + 1.0);
        if score > best_score {
            best = index;
            best_score = score;
        }
    }
    best
}

fn backup(tree: &mut [Node], path: &[(usize, usize)], mut value: f32) {
    for &(node, action) in path.iter().rev() {
        value = 1.0 - value;
        let edge = &mut tree[node].children[action];
        edge.visits += 1;
        edge.value_sum += value;
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
struct SearchStats {
    expanded_nodes: usize,
    evaluated_leaves: usize,
    leaf_legal_moves: usize,
    max_depth: usize,
    terminal_simulations: usize,
}

fn search<const BREAKDOWN: bool>(
    sample: &mut Sample,
    simulations: usize,
    schedules: &[Vec<usize>],
    evaluator: &mut Evaluator,
    rng: &mut Rng,
    timings: &mut Timings,
) -> SearchStats {
    let generator = MoveGenerator::new(sample.rules);
    let root = expand::<BREAKDOWN>(&sample.position, &generator, evaluator, timings);
    let mut stats = SearchStats {
        expanded_nodes: usize::from(!root.children.is_empty()),
        ..SearchStats::default()
    };
    let start = tick::<BREAKDOWN>();
    let gumbels: Vec<_> = root.children.iter().map(|_| rng.gumbel()).collect();
    let m = root.children.len().min(schedules.len());
    let schedule = (m != 0).then(|| &schedules[m - 1]);
    timings.finish(SELECTION, start);
    let mut tree = vec![root];
    let mut path = Vec::new();
    let mut undos = Vec::new();
    let mut scratch = Vec::new();
    for _ in 0..simulations {
        path.clear();
        undos.clear();
        let mut current = 0;
        loop {
            if tree[current].children.is_empty() {
                break;
            }
            let start = tick::<BREAKDOWN>();
            let node = &tree[current];
            let visits = completed_q(node, &mut scratch);
            let action = if current == 0 {
                root_action(
                    node,
                    &gumbels,
                    &scratch,
                    schedule.expect("根に候補がある")[visits],
                )
            } else {
                interior_action(node, visits, &mut scratch)
            };
            timings.finish(SELECTION, start);

            let edge = &tree[current].children[action];
            let start = tick::<BREAKDOWN>();
            let undo = sample.position.make_move_unchecked(edge.mv, sample.rules);
            timings.finish(MAKE_UNMAKE, start);
            undos.push(undo);
            path.push((current, action));
            if let Some(child) = edge.child {
                current = child;
            } else {
                let child = expand::<BREAKDOWN>(&sample.position, &generator, evaluator, timings);
                if !child.children.is_empty() {
                    stats.expanded_nodes += 1;
                    stats.evaluated_leaves += 1;
                    stats.leaf_legal_moves += child.children.len();
                }
                let next = tree.len();
                tree[current].children[action].child = Some(next);
                tree.push(child);
                current = next;
                break;
            }
        }
        stats.max_depth = stats.max_depth.max(path.len());
        stats.terminal_simulations += usize::from(tree[current].children.is_empty());
        let value = tree[current].value;
        let start = tick::<BREAKDOWN>();
        backup(&mut tree, &path, value);
        timings.finish(BACKUP, start);
        let start = tick::<BREAKDOWN>();
        while let Some(undo) = undos.pop() {
            sample.position.unmake_move(undo);
        }
        timings.finish(MAKE_UNMAKE, start);
    }

    if !tree[0].children.is_empty() {
        let start = tick::<BREAKDOWN>();
        completed_q(&tree[0], &mut scratch);
        let max_visits = tree[0]
            .children
            .iter()
            .map(|edge| edge.visits)
            .max()
            .expect("根に子がある");
        let selected = root_action(&tree[0], &gumbels, &scratch, max_visits);
        improved_policy(&tree[0], &mut scratch);
        black_box((tree[0].children[selected].mv, scratch.as_slice()));
        timings.finish(SELECTION, start);
    }
    stats
}

fn percentile(sorted: &[f64], percent: usize) -> f64 {
    let rank = (sorted.len() as u128 * percent as u128).div_ceil(100) as usize;
    sorted[rank - 1]
}

fn measure<const BREAKDOWN: bool>(config: &Config, mut samples: Vec<Sample>) -> Result<()> {
    let total_simulations = samples
        .len()
        .checked_mul(config.simulations)
        .ok_or("シミュレーション総数がusizeを超える")?;
    let mut rng = Rng(config.seed);
    let mut evaluator = Evaluator::new(&mut rng);
    let schedules: Vec<_> = (1..=config.considered.min(config.simulations))
        .map(|m| considered_visits(m, config.simulations))
        .collect();
    let mut timings = Timings::default();
    let mut total = Duration::ZERO;
    let mut times = Vec::new();
    let mut expanded_nodes = 0;
    let mut evaluated_leaves = 0;
    let mut leaf_legal_moves = 0;
    let mut max_depths = 0;
    let mut terminal_simulations = 0;
    for sample in &mut samples {
        let start = Instant::now();
        let stats = search::<BREAKDOWN>(
            sample,
            config.simulations,
            &schedules,
            &mut evaluator,
            &mut rng,
            &mut timings,
        );
        let elapsed = start.elapsed();
        total += elapsed;
        times.push(elapsed.as_secs_f64() * 1e6);
        expanded_nodes += stats.expanded_nodes;
        evaluated_leaves += stats.evaluated_leaves;
        leaf_legal_moves += stats.leaf_legal_moves;
        max_depths += stats.max_depth;
        terminal_simulations += stats.terminal_simulations;
    }
    times.sort_unstable_by(f64::total_cmp);
    let count = samples.len() as f64;
    let simulations = total_simulations as f64;
    let root_moves: usize = samples.iter().map(|sample| sample.legal_moves).sum();
    let mean_leaf_moves =
        (evaluated_leaves != 0).then(|| leaf_legal_moves as f64 / evaluated_leaves as f64);
    let mut output = json!({
        "positions": samples.len(),
        "simulations": total_simulations,
        "mean_us_per_simulation": total.as_secs_f64() * 1e6 / simulations,
        "simulations_per_second": simulations / total.as_secs_f64(),
        "search_time_us": { "p50": percentile(&times, 50), "p90": percentile(&times, 90), "p99": percentile(&times, 99) },
        "mean_root_legal_moves": root_moves as f64 / count,
        "mean_leaf_legal_moves": mean_leaf_moves,
        "mean_expanded_nodes": expanded_nodes as f64 / count,
        "mean_max_depth": max_depths as f64 / count,
        "terminal_leaf_fraction": terminal_simulations as f64 / simulations,
        "simulations_per_position": config.simulations,
        "considered": config.considered,
        "stride": config.stride,
        "seed": config.seed,
        "percentile_method": "nearest_rank",
        "expanded_nodes_include_root": true,
        "leaf_legal_moves_scope": "newly_expanded_nonterminal_leaves",
    });
    if BREAKDOWN {
        let mut sections = serde_json::Map::new();
        for (name, elapsed) in SECTION_NAMES.into_iter().zip(timings.0) {
            sections.insert(
                name.to_owned(),
                json!(elapsed.as_secs_f64() * 1e6 / simulations),
            );
        }
        let accounted: Duration = timings.0.into_iter().sum();
        let other = total
            .checked_sub(accounted)
            .ok_or("区分の合計が全体の計測時間を超えた")?;
        sections.insert(
            "other".to_owned(),
            json!(other.as_secs_f64() * 1e6 / simulations),
        );
        output["breakdown_us_per_simulation"] = sections.into();
    }
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    serde_json::to_writer_pretty(&mut out, &output)?;
    writeln!(out)?;
    Ok(())
}

fn run() -> Result<()> {
    let config = Config::parse(std::env::args().skip(1))?;
    let samples = read_samples(&config)?;
    if config.breakdown {
        measure::<true>(&config, samples)
    } else {
        measure::<false>(&config, samples)
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("sim_cost: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase_core::{Color, PieceCode, PieceKind, PositionBuilder, Rules, Square};

    fn node(value: f32, priors_visits_q: &[(f32, usize, f32)]) -> Node {
        Node {
            value,
            children: priors_visits_q
                .iter()
                .enumerate()
                .map(|(i, &(prior, visits, q))| Edge {
                    mv: Move {
                        from: Square::new(0, 0).unwrap(),
                        mid: None,
                        to: Square::new(i as u8 + 1, 0).unwrap(),
                        promote: false,
                    },
                    logit: prior.ln(),
                    prior,
                    visits,
                    value_sum: q * visits as f32,
                    child: None,
                })
                .collect(),
        }
    }

    #[test]
    fn visit_schedule_matches_specified_halving_rounds() {
        let mut expected = vec![0; 16];
        expected.extend([1; 8]);
        expected.extend([2; 4]);
        expected.extend([3; 4]);
        assert_eq!(considered_visits(16, 32), expected);
        assert_eq!(considered_visits(1, 5), vec![0, 1, 2, 3, 4]);
        assert_eq!(considered_visits(3, 10), vec![0, 0, 0, 1, 1, 2, 2, 3, 3, 4]);
    }

    #[test]
    fn completed_q_uses_prior_weighted_mix_and_max_visits_scale() {
        let node = node(0.25, &[(0.6, 2, 0.75), (0.3, 1, 0.25), (0.1, 0, 0.0)]);
        let mut values = Vec::new();
        assert_eq!(completed_q(&node, &mut values), 3);
        // 訪問済みQの方策加重平均は7/12、未訪問の混合価値は1/2。
        for (actual, expected) in values.iter().zip([5.2, 0.0, 2.6]) {
            assert!((actual - expected).abs() < 1e-6);
        }
        assert_eq!(root_action(&node, &[0.0, 1000.0, 100.0], &values, 2), 0);
        assert_eq!(root_action(&node, &[0.0, 1000.0, 100.0], &values, 1), 1);
    }

    #[test]
    fn equal_completed_values_remain_finite_and_internal_visits_reduce_priority() {
        let unvisited = node(0.75, &[(0.6, 0, 0.0), (0.4, 0, 0.0)]);
        let mut values = Vec::new();
        assert_eq!(completed_q(&unvisited, &mut values), 0);
        assert_eq!(values, vec![0.0, 0.0]);
        let visited = node(0.75, &[(0.6, 1, 0.75), (0.4, 0, 0.0)]);
        let total = completed_q(&visited, &mut values);
        assert_eq!(interior_action(&visited, total, &mut values), 1);
        assert!((values.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn backup_complements_the_value_at_each_ply() {
        let mut tree = vec![node(0.5, &[(1.0, 0, 0.0)]), node(0.5, &[(1.0, 0, 0.0)])];
        backup(&mut tree, &[(0, 0), (1, 0)], 0.2);
        assert_eq!(tree[0].children[0].visits, 1);
        assert_eq!(tree[1].children[0].visits, 1);
        assert!((tree[0].children[0].value_sum - 0.2).abs() < 1e-6);
        assert!((tree[1].children[0].value_sum - 0.8).abs() < 1e-6);
    }

    #[test]
    fn royal_capture_bypasses_encoding_and_policy_evaluation() {
        let mut builder = PositionBuilder::new(Color::Black);
        for (file, rank, color, kind) in [
            (11, 0, Color::Black, PieceKind::King),
            (0, 0, Color::Black, PieceKind::Rook),
            (0, 1, Color::White, PieceKind::King),
        ] {
            builder
                .put(
                    Square::new(file, rank).unwrap(),
                    PieceCode::new(color, kind).unwrap(),
                )
                .unwrap();
        }
        let mut position = builder.finish().unwrap();
        let original = position.clone();
        let rules = Rules::ENGINE_DEFAULT.moves;
        let undo = position.make_move_unchecked(
            Move {
                from: Square::new(0, 0).unwrap(),
                mid: None,
                to: Square::new(0, 1).unwrap(),
                promote: false,
            },
            rules,
        );
        let mut evaluator = Evaluator {
            input: Vec::new(),
            logits: Vec::new(),
        };
        let terminal = expand::<false>(
            &position,
            &MoveGenerator::new(rules),
            &mut evaluator,
            &mut Timings::default(),
        );
        assert_eq!(terminal.value, 0.0);
        assert!(terminal.children.is_empty());
        position.unmake_move(undo);
        assert_eq!(position, original);
    }

    #[test]
    fn search_restores_position_and_breakdown_does_not_change_work() {
        let position = Position::initial();
        let rules = Rules::ENGINE_DEFAULT.moves;
        let mut moves = Vec::new();
        MoveGenerator::new(rules).generate_moves(&position, &mut moves);
        let mut sample = Sample {
            position: position.clone(),
            rules,
            legal_moves: moves.len(),
        };
        let schedules: Vec<_> = (1..=16).map(|m| considered_visits(m, 32)).collect();
        let mut rng = Rng(1);
        let mut evaluator = Evaluator::new(&mut rng);
        let mut timings = Timings::default();
        let off = search::<false>(
            &mut sample,
            32,
            &schedules,
            &mut evaluator,
            &mut rng,
            &mut timings,
        );
        assert_eq!(sample.position, position);
        assert_eq!(timings.0, [Duration::ZERO; 6]);
        let mut rng = Rng(1);
        let mut evaluator = Evaluator::new(&mut rng);
        let on = search::<true>(
            &mut sample,
            32,
            &schedules,
            &mut evaluator,
            &mut rng,
            &mut timings,
        );
        assert_eq!(sample.position, position);
        assert_eq!(on, off);
        assert_eq!(off.evaluated_leaves + off.terminal_simulations, 32);
        assert_eq!(off.expanded_nodes, off.evaluated_leaves + 1);
        assert!(off.max_depth > 1);
    }

    #[test]
    fn arguments_are_required_and_repeated_data_paths_are_accepted() {
        let args = "--data first.bin,second.bin --data third.bin --positions 50 --stride 997 --simulations 32 --considered 16 --seed 0 --breakdown off";
        let parse = |text: &str| Config::parse(text.split_whitespace().map(str::to_owned));
        assert_eq!(
            parse(args).unwrap().data,
            ["first.bin", "second.bin", "third.bin"]
        );
        for key_and_value in [
            "--positions 50",
            "--stride 997",
            "--simulations 32",
            "--considered 16",
            "--seed 0",
            "--breakdown off",
        ] {
            assert!(parse(&args.replace(key_and_value, "")).is_err());
        }
        for invalid in [
            args.replace("--positions 50", "--positions 0"),
            args.replace("--stride 997", "--stride 0"),
            args.replace("--breakdown off", "--breakdown invalid"),
            format!("{args} --seed 2"),
        ] {
            assert!(parse(&invalid).is_err());
        }
    }
}
