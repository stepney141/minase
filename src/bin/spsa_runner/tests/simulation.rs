//! spsa-gain-calibration.mdの模擬比較。入力は測定記録の定数に固定する。

use super::{
    model::{IterationRecord, Settings, rates, uniform},
    pair,
    params::Parameter,
    runner,
};
use minase::rng::{XorShift64, derive_seed, splitmix64};
use std::{collections::BTreeMap, env, num::NonZeroUsize, sync::Mutex, thread, time::Instant};

const BUDGETS: [u64; 3] = [375, 750, 1500];
const PAIRS: usize = 8;
const MARGIN: f64 = 0.5;
const MIN_SEEDS: usize = 200;
const PILOT_SEEDS: usize = 20;
// docs/measurements/spsa-gain-simulation.mdの事前検査3で確定した本数。
const DECAY_DIAGNOSTIC_SEEDS: usize = 1843;
const COUNTS: [f64; 5] = [2363.0, 391.0, 5961.0, 348.0, 2381.0];
const SIGNS: [[i8; 2]; 5] = [[-1, -1], [-1, 0], [1, -1], [1, 0], [1, 1]];
// model.rsの摂動系列とは異なる用途の定数。
const GAME_STREAM: u64 = 0x5350_5341_5f53_494d;

// docs/measurements/spsa-gain-simulation.mdの開始値と信号の合計。
const MEASURED: [(&str, f64, i64); 22] = [
    ("LmrDivisor", 200.0, -315),
    ("LmrHistoryThreshold", 128.0, -123),
    ("FutilityMargin1", 50.0, 1),
    ("FutilityMargin2", 150.0, 115),
    ("FutilityMargin3", 150.0, 361),
    ("SeeMargin1", 0.0, -223),
    ("SeeMargin2", 200.0, -129),
    ("SeeMargin3", 0.0, 127),
    ("AspirationDelta", 50.0, 11),
    ("AspirationGrowth", 200.0, -115),
    ("NullMoveBase", 2400.0, 861),
    ("NullMoveSlope", 200.0, 163),
    ("HistoryLimit", 16384.0, 317),
    ("CorrectionCap", 200.0, -17),
    ("CorrectionWeight", 32.0, 133),
    ("DeltaMargin", 200.0, 181),
    ("ExpectedPlies", 450.0, -209),
    ("MinMoves", 100.0, -213),
    ("IncrementShare", 70.0, 283),
    ("HardSoftRatio", 400.0, 7),
    ("HardRemainingShare", 25.0, 51),
    ("IterationRatio", 250.0, -175),
];

#[derive(Clone, Copy)]
struct Candidate {
    alpha: f64,
    gamma: f64,
    width_divisor: f64,
    r_end: f64,
}

const CANDIDATES: [Candidate; 5] = [
    Candidate {
        alpha: 0.602,
        gamma: 0.101,
        width_divisor: 20.0,
        r_end: 0.002,
    },
    Candidate {
        alpha: 0.0,
        gamma: 0.0,
        width_divisor: 10.0,
        r_end: 0.002,
    },
    Candidate {
        alpha: 0.0,
        gamma: 0.0,
        width_divisor: 6.0,
        r_end: 0.002,
    },
    Candidate {
        alpha: 0.602,
        gamma: 1.0 / 6.0,
        width_divisor: 20.0,
        r_end: 0.002,
    },
    Candidate {
        alpha: 0.0,
        gamma: 0.0,
        width_divisor: 6.0,
        r_end: 0.001,
    },
];

// 選定規則には含めず、C2と同じ終了時の摂動幅で減衰を比べる。
const DECAY_CANDIDATES: [(usize, Candidate); 2] = [
    (
        5,
        Candidate {
            alpha: 0.602,
            gamma: 0.101,
            width_divisor: 6.0,
            r_end: 0.002,
        },
    ),
    (
        6,
        Candidate {
            alpha: 1.0,
            gamma: 1.0 / 6.0,
            width_divisor: 6.0,
            r_end: 0.002,
        },
    ),
];

#[derive(Clone, Copy)]
enum Shape {
    Quadratic,
    Measured,
    Asymmetric,
    Boundary,
}

struct Term {
    optimum: f64,
    width: f64,
    clamped: bool,
}

struct Scenario {
    name: &'static str,
    shape: Shape,
    depth: f64,
    parameters: Vec<Parameter>,
    terms: Vec<Term>,
}

/// 既存の合成試験と同じ宣言表から範囲だけを読む。
fn ranges() -> Vec<Parameter> {
    let table = include_str!("../../../search/alphabeta/params.rs")
        .split_once("parameters! {")
        .unwrap()
        .1
        .split_once("\n}")
        .unwrap()
        .0;
    let parameters: Vec<_> = table
        .lines()
        .filter(|line| line.contains("): "))
        .map(|line| {
            let (name, rest) = line.trim().split_once('(').unwrap();
            let (_, values) = rest.split_once("): ").unwrap();
            let values: Vec<i32> = values
                .trim_end_matches(';')
                .split(',')
                .map(|value| value.trim().parse().unwrap())
                .collect();
            assert_eq!(values.len(), 3);
            let (min, max) = (values[1], values[2]);
            assert!(min < max);
            Parameter {
                name: name.to_owned(),
                start: f64::from(min),
                min,
                max,
                c_end: f64::from(max - min) / 20.0,
                r_end: 0.002,
            }
        })
        .collect();
    assert_eq!(parameters.len(), MEASURED.len());
    for (p, &(name, start, _)) in parameters.iter().zip(&MEASURED) {
        assert_eq!(p.name, name);
        assert!((f64::from(p.min)..=f64::from(p.max)).contains(&start));
    }
    parameters
}

impl Scenario {
    fn new(name: &'static str, shape: Shape, depth: f64, ranges: &[Parameter]) -> Self {
        let mut parameters = ranges.to_vec();
        let terms = parameters
            .iter_mut()
            .enumerate()
            .map(|(i, p)| {
                let min = f64::from(p.min);
                let max = f64::from(p.max);
                let width = max - min;
                let centre = (min + max) / 2.0;
                let optimum = match shape {
                    Shape::Quadratic => {
                        p.start = centre + if i % 2 == 0 { 0.2 } else { -0.2 } * width;
                        centre
                    }
                    Shape::Measured => {
                        p.start = MEASURED[i].1;
                        let signal = MEASURED[i].2 as f64 / 1500.0;
                        let mean_c = 1.111837 * width / 20.0;
                        p.start
                            + 800.0 * width.powi(2) * signal
                                / (256.0 * std::f64::consts::LN_10 * depth * mean_c)
                    }
                    Shape::Asymmetric => {
                        p.start = centre + 0.2 * width;
                        centre
                    }
                    Shape::Boundary => {
                        p.start = min;
                        min
                    }
                };
                Term {
                    optimum: optimum.clamp(min, max),
                    width,
                    clamped: optimum < min || optimum > max,
                }
            })
            .collect();
        Self {
            name,
            shape,
            depth,
            parameters,
            terms,
        }
    }

    fn loss(&self, values: impl IntoIterator<Item = f64>) -> f64 {
        values
            .into_iter()
            .zip(&self.terms)
            .map(|(x, term)| {
                let factor = if matches!(self.shape, Shape::Asymmetric) && x < term.optimum {
                    5.0
                } else {
                    1.0
                };
                factor * self.depth * ((x - term.optimum) / (term.width / 2.0)).powi(2)
            })
            .sum()
    }

    fn start_loss(&self) -> f64 {
        self.loss(self.parameters.iter().map(|p| p.start))
    }

    fn settings(&self, candidate: Candidate, iterations: u64, seed: u64) -> Settings {
        let mut parameters = self.parameters.clone();
        for p in &mut parameters {
            p.c_end = f64::from(p.max - p.min) / candidate.width_divisor;
            p.r_end = candidate.r_end;
        }
        Settings {
            parameters,
            alpha: candidate.alpha,
            gamma: candidate.gamma,
            a: 0.1 * iterations as f64,
            iterations,
            pairs_per_iteration: PAIRS,
            seed,
            concurrency: 1,
        }
    }
}

fn scenarios(ranges: &[Parameter]) -> Vec<Scenario> {
    [
        ("Q5", Shape::Quadratic, 5.0),
        ("Q20", Shape::Quadratic, 20.0),
        ("Q80", Shape::Quadratic, 80.0),
        ("M5", Shape::Measured, 5.0),
        ("M20", Shape::Measured, 20.0),
        ("M80", Shape::Measured, 80.0),
        ("A20", Shape::Asymmetric, 20.0),
        ("B20", Shape::Boundary, 20.0),
    ]
    .into_iter()
    .map(|(name, shape, depth)| Scenario::new(name, shape, depth, ranges))
    .collect()
}

/// 最大指数を引いて正規化し、極端なElo差でもオーバーフローを避ける。
fn tilted(lambda: f64) -> [f64; 5] {
    let weights: [f64; 5] =
        std::array::from_fn(|j| COUNTS[j] * (lambda * (j as f64 - 2.0) - 2.0 * lambda.abs()).exp());
    let sum: f64 = weights.iter().sum();
    weights.map(|weight| weight / sum)
}

/// 期待得点の誤差が設計書の許容値に入るまで二分する。
fn probabilities(delta: f64) -> [f64; 5] {
    assert!(delta.is_finite());
    let target = 1.0 / (1.0 + 10.0_f64.powf(-delta / 400.0));
    // この両端では期待得点が0と1からそれぞれ10^-12未満に収まる。
    let (mut low, mut high) = (-128.0, 128.0);
    for _ in 0..100 {
        let lambda = (low + high) / 2.0;
        let q = tilted(lambda);
        let score: f64 = q.iter().enumerate().map(|(j, p)| p * j as f64 / 4.0).sum();
        if (score - target).abs() <= 1e-12 {
            return q;
        }
        if score < target {
            low = lambda;
        } else {
            high = lambda;
        }
    }
    panic!("exponential tilt did not converge: delta={delta}");
}

fn sample_category(q: [f64; 5], seed: u64, number: u64) -> usize {
    let mut rng = XorShift64::new(derive_seed(splitmix64(seed ^ GAME_STREAM), number));
    let u = uniform(&mut rng);
    let mut cumulative = 0.0;
    for (j, probability) in q.iter().take(4).enumerate() {
        cumulative += probability;
        if u < cumulative {
            return j;
        }
    }
    4
}

#[derive(Default)]
struct DifferenceMoments {
    count: u64,
    sum: i64,
    squares: i64,
}

impl DifferenceMoments {
    fn variance(&self) -> f64 {
        assert!(self.count > 1);
        (self.squares as f64 - (self.sum as f64).powi(2) / self.count as f64)
            / (self.count - 1) as f64
    }
}

struct Observations {
    theta: Vec<f64>,
    second_half_sum: Vec<f64>,
    second_half_count: u64,
    signal_sums: Vec<i64>,
    clipped: u64,
    differences: DifferenceMoments,
}

impl Observations {
    fn persist(&mut self, settings: &Settings, record: &IterationRecord) {
        assert_eq!(record.application, self.differences.count + 1);
        assert_eq!(record.k, record.application);
        for (i, p) in settings.parameters.iter().enumerate() {
            let c = rates(settings, p, record.k).0;
            if self.theta[i] - c < f64::from(p.min) || self.theta[i] + c > f64::from(p.max) {
                self.clipped += 1;
            }
            self.signal_sums[i] += record.d * i64::from(record.flip[i]);
            if record.application > settings.iterations / 2 {
                self.second_half_sum[i] += record.theta[i];
            }
        }
        if record.application > settings.iterations / 2 {
            self.second_half_count += 1;
        }
        self.theta.clone_from(&record.theta);
        self.differences.count += 1;
        self.differences.sum += record.d;
        self.differences.squares += record.d * record.d;
    }
}

struct Run {
    theta: Vec<f64>,
    losses: [f64; 2],
    clip_fraction: f64,
    signal_sums: Vec<i64>,
    differences: DifferenceMoments,
}

fn simulate(scenario: &Scenario, candidate: Candidate, iterations: u64, seed: u64) -> Run {
    let settings = scenario.settings(candidate, iterations, seed);
    let count = settings.parameters.len();
    let observations = Mutex::new(Observations {
        theta: settings.initial_theta(),
        second_half_sum: vec![0.0; count],
        second_half_count: 0,
        signal_sums: vec![0; count],
        clipped: 0,
        differences: DifferenceMoments::default(),
    });
    let summary = runner::run(
        &settings,
        &BTreeMap::new(),
        |values, _| {
            let delta = scenario.loss(values.minus.iter().copied().map(f64::from))
                - scenario.loss(values.plus.iter().copied().map(f64::from));
            let category = sample_category(probabilities(delta), seed, values.number);
            Ok(Some(pair(values.number, SIGNS[category])))
        },
        |record| {
            observations.lock().unwrap().persist(&settings, record);
            Ok(())
        },
    )
    .unwrap();
    let observations = observations.into_inner().unwrap();
    assert_eq!(summary.applied, iterations);
    assert_eq!(summary.valid_pairs, iterations * PAIRS as u64);
    assert_eq!(summary.theta, observations.theta);
    assert_eq!(observations.second_half_count, iterations - iterations / 2);
    let average = observations
        .second_half_sum
        .iter()
        .map(|sum| sum / observations.second_half_count as f64);
    Run {
        losses: [
            scenario.loss(observations.theta.iter().copied()),
            scenario.loss(average),
        ],
        theta: observations.theta,
        clip_fraction: observations.clipped as f64 / (iterations as f64 * count as f64),
        signal_sums: observations.signal_sums,
        differences: observations.differences,
    }
}

/// シードの連続区間を分担し、完了順によらずシード順に連結する。
fn run_seeds(scenario: &Scenario, candidate: Candidate, iterations: u64, seeds: usize) -> Vec<Run> {
    let workers = match env::var("SPSA_SIM_THREADS") {
        Ok(value) => value
            .parse::<NonZeroUsize>()
            .expect("SPSA_SIM_THREADS must be a positive integer")
            .get(),
        Err(env::VarError::NotPresent) => thread::available_parallelism().unwrap().get(),
        Err(env::VarError::NotUnicode(_)) => {
            panic!("SPSA_SIM_THREADS must be a positive integer (invalid Unicode)")
        }
    }
    .min(seeds);
    let chunk = seeds.div_ceil(workers);
    thread::scope(|scope| {
        let handles: Vec<_> = (0..seeds)
            .step_by(chunk)
            .map(|start| {
                scope.spawn(move || {
                    (start..(start + chunk).min(seeds))
                        .map(|i| simulate(scenario, candidate, iterations, i as u64 + 1))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    })
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, count) = values.fold((0.0, 0_usize), |(sum, count), x| (sum + x, count + 1));
    assert!(count > 0);
    sum / count as f64
}

fn mean_and_sd(values: &[f64]) -> (f64, f64) {
    assert!(values.len() > 1);
    let mean = mean(values.iter().copied());
    let variance =
        values.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64;
    (mean, variance.sqrt())
}

/// 分散、収縮率、必要なシード数を本実験の前に検査する。
fn prechecks(ranges: &[Parameter], scenarios: &[Scenario]) -> usize {
    let zero = Scenario::new("zero", Shape::Quadratic, 0.0, ranges);
    let runs = run_seeds(&zero, CANDIDATES[0], 1500, MIN_SEEDS);
    let single = &runs[0].differences;
    let pooled = runs
        .iter()
        .fold(DifferenceMoments::default(), |mut total, run| {
            total.count += run.differences.count;
            total.sum += run.differences.sum;
            total.squares += run.differences.squares;
            total
        });
    let reference = 13.7819;
    let tolerance = 2.0 * reference * (2.0 / (single.count - 1) as f64).sqrt();
    println!(
        "precheck1\tseed=1\tvariance={:.12}\ttolerance={tolerance:.12}\tpooled_n={}\tpooled_variance={:.12}\tempirical_variance=13.2802",
        single.variance(),
        pooled.count,
        pooled.variance()
    );
    assert!((single.variance() - reference).abs() <= tolerance);

    let q20 = scenarios.iter().find(|s| s.name == "Q20").unwrap();
    let runs = run_seeds(q20, CANDIDATES[0], 1500, MIN_SEEDS);
    let movement = mean(runs.iter().flat_map(|run| {
        run.theta
            .iter()
            .zip(&q20.parameters)
            .zip(&q20.terms)
            .map(|((&x, p), term)| {
                (x - p.start) * (term.optimum - p.start).signum() / (p.start - term.optimum).abs()
            })
    }));
    println!("precheck2\tmean_signed_movement={movement:.12}\texpected=0.171");
    assert!((0.0855..=0.342).contains(&movement));

    let mut max_sd: f64 = 0.0;
    for scenario in scenarios {
        let current = run_seeds(scenario, CANDIDATES[0], 1500, PILOT_SEEDS);
        let candidate = run_seeds(scenario, CANDIDATES[2], 1500, PILOT_SEEDS);
        let differences: Vec<_> = candidate
            .iter()
            .zip(&current)
            .map(|(x, y)| x.losses[0] - y.losses[0])
            .collect();
        let sd = mean_and_sd(&differences).1;
        println!("precheck3\tscenario={}\tpaired_sd={sd:.12}", scenario.name);
        max_sd = max_sd.max(sd);
    }
    let seeds = MIN_SEEDS.max((4.0 * max_sd / MARGIN).powi(2).ceil() as usize);
    println!("precheck3\ts_max={max_sd:.12}\tS={seeds}");
    if seeds > 1000 {
        // 設計書は1,000本を超えるSを、所要時間を利用者へ報告してから使うと定める。
        println!("precheck3\tS={seeds} exceeds 1000; runtime scales linearly with S");
    }
    seeds
}

type Results = BTreeMap<(usize, usize, u64), Vec<Run>>;
const ESTIMATORS: [&str; 2] = ["final", "average"];

struct Comparison {
    mean_loss: f64,
    mean_diff: f64,
    se_diff: f64,
    ratio: f64,
    clip_fraction: f64,
}

fn compare(
    runs: &[Run],
    current: &[Run],
    estimator: usize,
    reference_estimator: usize,
) -> Comparison {
    assert_eq!(runs.len(), current.len());
    let differences: Vec<_> = runs
        .iter()
        .zip(current)
        .map(|(x, y)| x.losses[estimator] - y.losses[reference_estimator])
        .collect();
    let (mean_diff, sd) = mean_and_sd(&differences);
    let mean_loss = mean(runs.iter().map(|run| run.losses[estimator]));
    let current_loss = mean(current.iter().map(|run| run.losses[reference_estimator]));
    // 設計書のゼロ損失の規約は、どちらかの平均が0なら比を1とする。
    let ratio = if mean_loss == 0.0 || current_loss == 0.0 {
        1.0
    } else {
        mean_loss / current_loss
    };
    Comparison {
        mean_loss,
        mean_diff,
        se_diff: sd / (runs.len() as f64).sqrt(),
        ratio,
        clip_fraction: mean(runs.iter().map(|run| run.clip_fraction)),
    }
}

/// 比較先の推定量を区別し、同じシードの差から標準誤差を求める。
#[test]
fn comparison_uses_paired_seeds_and_reference_estimator() {
    let run = |losses| Run {
        theta: Vec::new(),
        losses,
        clip_fraction: 0.25,
        signal_sums: Vec::new(),
        differences: DifferenceMoments::default(),
    };
    let runs = [run([2.0, 6.0]), run([4.0, 10.0]), run([6.0, 14.0])];
    let reference = [run([1.0, 5.0]), run([1.0, 7.0]), run([1.0, 9.0])];
    // 後半平均どうしの差は[1, 3, 5]、現行の最終値との差は[5, 9, 13]。
    let same_estimator = compare(&runs, &reference, 1, 1);
    assert_eq!(same_estimator.mean_loss, 10.0);
    assert_eq!(same_estimator.mean_diff, 3.0);
    assert!((same_estimator.se_diff - (4.0_f64 / 3.0).sqrt()).abs() < 1e-12);
    assert!((same_estimator.ratio - 10.0 / 7.0).abs() < 1e-12);
    assert_eq!(same_estimator.clip_fraction, 0.25);
    let current = compare(&runs, &reference, 1, 0);
    assert_eq!(current.mean_diff, 9.0);
    assert!((current.se_diff - (16.0_f64 / 3.0).sqrt()).abs() < 1e-12);
    assert_eq!(current.ratio, 10.0);
}

fn print_losses(results: &Results, scenarios: &[Scenario]) {
    println!(
        "candidate\tscenario\tbudget_pairs\testimator\tmean_loss\tstart_loss\tmean_diff_vs_current\tse_diff\tclip_fraction"
    );
    for candidate in 0..CANDIDATES.len() {
        for (i, scenario) in scenarios.iter().enumerate() {
            for budget in BUDGETS {
                for (estimator, name) in ESTIMATORS.iter().enumerate() {
                    let c = compare(
                        &results[&(candidate, i, budget)],
                        &results[&(0, i, 1500)],
                        estimator,
                        0,
                    );
                    println!(
                        "C{candidate}\t{}\t{}\t{name}\t{:.12}\t{:.12}\t{:.12}\t{:.12}\t{:.12}",
                        scenario.name,
                        budget * PAIRS as u64,
                        c.mean_loss,
                        scenario.start_loss(),
                        c.mean_diff,
                        c.se_diff,
                        c.clip_fraction
                    );
                }
            }
        }
    }
}

fn print_calibration(results: &Results, scenarios: &[Scenario]) {
    println!(
        "calibration_scenario\tcoefficient\trequested_signal\tachieved_signal\toptimum_clamped"
    );
    for (i, scenario) in scenarios
        .iter()
        .enumerate()
        .filter(|(_, s)| matches!(s.shape, Shape::Measured))
    {
        let runs = &results[&(0, i, 1500)];
        for (j, &(name, _, signal_sum)) in MEASURED.iter().enumerate() {
            let achieved = runs.iter().map(|run| run.signal_sums[j]).sum::<i64>() as f64
                / (runs.len() as f64 * 1500.0);
            println!(
                "{}\t{name}\t{:.12}\t{achieved:.12}\t{}",
                scenario.name,
                signal_sum as f64 / 1500.0,
                scenario.terms[j].clamped
            );
        }
    }
}

/// 全場面の非劣性と、場面を等しく重み付けした損失比を返す。
fn assess(
    results: &Results,
    scenario_count: usize,
    candidate: usize,
    budget: u64,
    estimator: usize,
) -> (bool, f64) {
    let mut eligible = true;
    let mut log_sum = 0.0;
    for i in 0..scenario_count {
        let c = compare(
            &results[&(candidate, i, budget)],
            &results[&(0, i, 1500)],
            estimator,
            0,
        );
        eligible &= c.mean_diff + 2.0 * c.se_diff <= MARGIN;
        log_sum += c.ratio.ln();
    }
    (eligible, (log_sum / scenario_count as f64).exp())
}

/// 同値なら候補番号順、最終θの順を保って3段階の選定を行う。
fn print_selection(results: &Results, scenario_count: usize) {
    let mut selected = 0;
    let (current_eligible, mut best_ratio) = assess(results, scenario_count, 0, 1500, 0);
    assert!(current_eligible);
    println!("stage1_candidate\tgeometric_mean\teligible");
    for candidate in 0..CANDIDATES.len() {
        let (eligible, ratio) = assess(results, scenario_count, candidate, 1500, 0);
        println!("C{candidate}\t{ratio:.12}\t{eligible}");
        if eligible && ratio < best_ratio {
            selected = candidate;
            best_ratio = ratio;
        }
    }
    for budget in BUDGETS {
        let final_value = assess(results, scenario_count, selected, budget, 0);
        let average = assess(results, scenario_count, selected, budget, 1);
        if !final_value.0 && !average.0 {
            continue;
        }
        let estimator = if average.0 && (!final_value.0 || average.1 < final_value.1) {
            1
        } else {
            0
        };
        println!("selected_candidate\tbudget_pairs\testimator");
        println!(
            "C{selected}\t{}\t{}",
            budget * PAIRS as u64,
            ESTIMATORS[estimator]
        );
        return;
    }
    panic!("stage 1 must provide an eligible final estimator at 12000 pairs");
}

/// 20シードの実測から、事前検査を含む本実験と再現確認の時間を外挿する。
#[test]
#[ignore]
fn gain_simulation_timing() {
    let ranges = ranges();
    let scenario = Scenario::new("Q20", Shape::Quadratic, 20.0, &ranges);
    let mut elapsed = Vec::new();
    println!("timing_candidate\tscenario\tbudget_pairs\tseeds\twall_seconds");
    for budget in BUDGETS {
        let start = Instant::now();
        let runs = run_seeds(&scenario, CANDIDATES[0], budget, PILOT_SEEDS);
        let seconds = start.elapsed().as_secs_f64();
        assert_eq!(runs.len(), PILOT_SEEDS);
        println!(
            "C0\tQ20\t{}\t{PILOT_SEEDS}\t{seconds:.6}",
            budget * PAIRS as u64
        );
        elapsed.push(seconds);
    }
    let main_seconds =
        elapsed.iter().sum::<f64>() * CANDIDATES.len() as f64 * 8.0 * MIN_SEEDS as f64
            / PILOT_SEEDS as f64;
    // 検査1・2は各200本、検査3は2候補×8場面×20本。すべてN=1500。
    let precheck_runs = 2 * MIN_SEEDS + 2 * 8 * PILOT_SEEDS;
    let precheck_seconds = elapsed[2] * precheck_runs as f64 / PILOT_SEEDS as f64;
    let total = main_seconds + precheck_seconds;
    println!(
        "extrapolation\tS={MIN_SEEDS}\tmain_seconds={main_seconds:.6}\tprechecks_seconds={precheck_seconds:.6}\ttotal_seconds={total:.6}\ttwo_runs_seconds={:.6}",
        2.0 * total
    );
    println!(
        "extrapolation assumes the measured Q20/C0 throughput for every scenario and candidate; available_parallelism={}",
        thread::available_parallelism().unwrap()
    );
}

/// 設計書の事前検査、本実験、信号の較正、および構成の選定を順に出力する。
#[test]
#[ignore]
fn gain_simulation() {
    let ranges = ranges();
    let scenarios = scenarios(&ranges);
    let seeds = prechecks(&ranges, &scenarios);
    let mut results = Results::new();
    for (candidate, &settings) in CANDIDATES.iter().enumerate() {
        for (i, scenario) in scenarios.iter().enumerate() {
            for budget in BUDGETS {
                results.insert(
                    (candidate, i, budget),
                    run_seeds(scenario, settings, budget, seeds),
                );
            }
        }
    }
    print_losses(&results, &scenarios);
    print_calibration(&results, &scenarios);
    print_selection(&results, scenarios.len());
}

/// 選定済みのC2と減衰候補を、固定したシードで診断する。
#[test]
#[ignore]
fn gain_decay_diagnostic() {
    let start = Instant::now();
    let scenarios = scenarios(&ranges());
    let candidates = [
        (0, CANDIDATES[0]),
        (2, CANDIDATES[2]),
        DECAY_CANDIDATES[0],
        DECAY_CANDIDATES[1],
    ];
    let mut results = Results::new();
    for (candidate, settings) in candidates {
        for (i, scenario) in scenarios.iter().enumerate() {
            for budget in BUDGETS {
                results.insert(
                    (candidate, i, budget),
                    run_seeds(scenario, settings, budget, DECAY_DIAGNOSTIC_SEEDS),
                );
            }
        }
    }
    println!(
        "candidate\tscenario\tbudget_pairs\testimator\tmean_loss\tmean_diff_vs_C2_same_budget_same_estimator\tse_diff\tmean_diff_vs_current\tse_diff_current\tclip_fraction"
    );
    for (candidate, _) in candidates {
        for (i, scenario) in scenarios.iter().enumerate() {
            for budget in BUDGETS {
                for (estimator, name) in ESTIMATORS.iter().enumerate() {
                    let runs = &results[&(candidate, i, budget)];
                    let c2 = compare(runs, &results[&(2, i, budget)], estimator, estimator);
                    let current = compare(runs, &results[&(0, i, 1500)], estimator, 0);
                    println!(
                        "C{candidate}\t{}\t{}\t{name}\t{:.12}\t{:.12}\t{:.12}\t{:.12}\t{:.12}\t{:.12}",
                        scenario.name,
                        budget * PAIRS as u64,
                        c2.mean_loss,
                        c2.mean_diff,
                        c2.se_diff,
                        current.mean_diff,
                        current.se_diff,
                        c2.clip_fraction
                    );
                }
            }
        }
    }
    println!(
        "diagnostic_candidate\tbudget_pairs\testimator\tnon_inferior_vs_current\tgeometric_mean_vs_current\tgeometric_mean_vs_C2_same_budget_same_estimator"
    );
    for (candidate, _) in DECAY_CANDIDATES {
        for budget in BUDGETS {
            for (estimator, name) in ESTIMATORS.iter().enumerate() {
                let (eligible, current_ratio) =
                    assess(&results, scenarios.len(), candidate, budget, estimator);
                let c2_ratio = mean((0..scenarios.len()).map(|i| {
                    compare(
                        &results[&(candidate, i, budget)],
                        &results[&(2, i, budget)],
                        estimator,
                        estimator,
                    )
                    .ratio
                    .ln()
                }))
                .exp();
                println!(
                    "C{candidate}\t{}\t{name}\t{eligible}\t{current_ratio:.12}\t{c2_ratio:.12}",
                    budget * PAIRS as u64
                );
            }
        }
    }
    println!("elapsed_wall_seconds\t{:.6}", start.elapsed().as_secs_f64());
}
