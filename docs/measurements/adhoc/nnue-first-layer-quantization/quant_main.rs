//! Throwaway offline measurement of the NNUE nonlinear-path quantization error.
//! `quant extract` reproduces the trainer's validation sample and caches features and totals.
//! `quant analyze` loads the cache and evaluates every variant. Neither touches the repository.

use std::{
    error::Error,
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

use minase::eval::{
    features::active_features_for,
    nnue::{
        self, Nnue,
        head::{DirectScrelu, Head},
    },
};
use minase_trainer::{
    data::{Dataset, epoch_permutation},
    nn::{Sha256Hasher, hex},
    nnue::{
        data::build_batches,
        model::{Checkpoint, Parameter, quantize},
        reference,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const RUN: &str = "/home/stepney141/board-games/minase/data/nnue-early-best-epoch/no-lookahead";
const OUT: &str = "/tmp/claude-1000/-home-stepney141-board-games-minase/c260f619-012e-495e-bcda-ad11e5d6e714/scratchpad/quant";
const SAMPLE: usize = 10_000;
const REFERENCE_BATCH: usize = 256;
const D: usize = 13_680;
const EVALUATION_LIMIT: i64 = 28_999;
const OUTPUT_DENOMINATOR: f64 = 127.0 * 4096.0;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Serialize, Deserialize)]
struct Record {
    index: u64,
    pieces: u32,
    /// Active base features for the side to move.
    us: Vec<u16>,
    /// Active base features for the opponent.
    them: Vec<u16>,
    /// `reference::evaluate_f64` total, centipawns.
    reference_total: f64,
    /// `nnue::evaluate_for_diagnostics` on the decoded nnue.bin, centipawns.
    minase_total: i32,
}

#[derive(Serialize, Deserialize)]
struct Cache {
    indices_sha256: String,
    records: Vec<Record>,
}

fn read_json(path: impl AsRef<Path>) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn extract() -> Result<()> {
    let config = read_json(format!("{RUN}/config.json"))?;
    let paths: Vec<PathBuf> = config["arguments"]["data"]
        .as_array()
        .ok_or("config.json arguments.data")?
        .iter()
        .map(|p| PathBuf::from(p.as_str().expect("path string")))
        .collect();
    let checkpoint = Checkpoint::read(format!("{RUN}/best.npz"))?;
    let bytes = fs::read(format!("{RUN}/nnue.bin"))?;
    let net = Nnue::decode(&bytes)?;
    eprintln!("opening {} MNSD files with 4 threads", paths.len());
    let started = std::time::Instant::now();
    let dataset = Dataset::open(&paths, None, NonZeroUsize::new(4).unwrap())?
        .with_lambda(checkpoint.lambda)?;
    eprintln!(
        "opened in {:.1}s: {} validation positions of {} records",
        started.elapsed().as_secs_f64(),
        dataset.validation_indices().len(),
        dataset.record_count()
    );
    let mut indices = epoch_permutation(dataset.validation_indices(), checkpoint.progress.seed, 0);
    indices.truncate(SAMPLE);
    let mut checksum = Sha256Hasher::default();
    let mut records = Vec::with_capacity(indices.len());
    for chunk in indices.chunks(REFERENCE_BATCH) {
        let batch = build_batches(&dataset, checkpoint.model.shape().feature_set(), &[chunk])?
            .pop()
            .expect("one batch was requested");
        let floating = reference::evaluate_f64(&checkpoint.model, &batch)?;
        for (&index, &reference_total) in chunk.iter().zip(&floating) {
            let position = dataset.record(index)?.to_position()?;
            let side = position.side_to_move();
            let mut us = Vec::new();
            active_features_for(side, &position, |f| us.push(f as u16));
            let mut them = Vec::new();
            active_features_for(side.opposite(), &position, |f| them.push(f as u16));
            records.push(Record {
                index,
                pieces: position.occupied().popcount(),
                us,
                them,
                reference_total,
                minase_total: nnue::evaluate_for_diagnostics(&net, &position, side),
            });
            checksum.update(&index.to_le_bytes());
        }
    }
    let cache = Cache {
        indices_sha256: hex(&checksum.finalize()),
        records,
    };
    fs::write(format!("{OUT}/sample-cache.json"), serde_json::to_vec(&cache)?)?;
    eprintln!("cached {} positions, sha256 {}", cache.records.len(), cache.indices_sha256);
    Ok(())
}

/// nnue.bin body, parsed independently of the inference code.
struct Bin {
    width: usize,
    cp_per_logit: f32,
    linear: Vec<[i16; 2]>,
    weights: Vec<i16>,
    biases: Vec<i32>,
    head: DirectScrelu,
    output_weights: Vec<i16>,
    output_bias: i32,
}

fn parse_bin(bytes: &[u8]) -> Result<Bin> {
    let width = u32::from_le_bytes(bytes[104..108].try_into()?) as usize;
    let cp_per_logit = f32::from_le_bytes(bytes[108..112].try_into()?);
    let i16_at = |o: usize| i16::from_le_bytes([bytes[o], bytes[o + 1]]);
    let i32_at = |o: usize| i32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let mut offset = 192;
    let linear = (0..D).map(|i| [i16_at(offset + 4 * i), i16_at(offset + 4 * i + 2)]).collect();
    offset += D * 4 + 47 * 4;
    let weights = (0..D * width).map(|i| i16_at(offset + 2 * i)).collect();
    offset += D * width * 2;
    let biases = (0..width).map(|i| i32_at(offset + 4 * i)).collect();
    offset += width * 4;
    let head = DirectScrelu::decode(&bytes[offset..], width, true)?;
    let output_weights = (0..2 * width).map(|i| i16_at(offset + 2 * i)).collect();
    let output_bias = i32_at(offset + 4 * width);
    assert_eq!(offset + 4 * width + 4, bytes.len(), "nnue.bin length");
    Ok(Bin { width, cp_per_logit, linear, weights, biases, head, output_weights, output_bias })
}

fn rounded(value: f32, scale: f64) -> i64 {
    (f64::from(value) * scale).round_ties_even() as i64
}

#[derive(Clone, Copy)]
enum Activation {
    /// `a = (c*c) >> k`.
    Shift(u32),
    /// `a = floor(c*c*127 / S^2)`.
    Floor,
    /// `a = c*c*127 / S^2` as a rational.
    Exact,
    /// `a = c*c / 128` as a rational, at S=127 only.
    Exact128,
}

struct Variant {
    id: &'static str,
    description: &'static str,
    scale: i64,
    activation: Activation,
}

const VARIANTS: &[Variant] = &[
    Variant { id: "1", description: "S=127, a=(c*c)>>7 (shipped)", scale: 127, activation: Activation::Shift(7) },
    Variant { id: "2", description: "S=127, a=c*c/127 exact", scale: 127, activation: Activation::Exact },
    Variant { id: "3", description: "S=127, a=floor(c*c/127)", scale: 127, activation: Activation::Floor },
    Variant { id: "extra-a", description: "S=127, a=c*c/128 exact (mismatch kept, floor removed)", scale: 127, activation: Activation::Exact128 },
    Variant { id: "4a-254", description: "S=254, a=floor(c*c*127/S^2)", scale: 254, activation: Activation::Floor },
    Variant { id: "4a-508", description: "S=508, a=floor(c*c*127/S^2)", scale: 508, activation: Activation::Floor },
    Variant { id: "4a-1016", description: "S=1016, a=floor(c*c*127/S^2)", scale: 1016, activation: Activation::Floor },
    Variant { id: "4b-254", description: "S=254, a=c*c*127/S^2 exact", scale: 254, activation: Activation::Exact },
    Variant { id: "4b-508", description: "S=508, a=c*c*127/S^2 exact", scale: 508, activation: Activation::Exact },
    Variant { id: "4b-1016", description: "S=1016, a=c*c*127/S^2 exact", scale: 1016, activation: Activation::Exact },
    Variant { id: "extra-b-254", description: "S=254, a=(c*c)>>9 (literal shift, mismatch kept)", scale: 254, activation: Activation::Shift(9) },
    Variant { id: "extra-b-508", description: "S=508, a=(c*c)>>11 (literal shift, mismatch kept)", scale: 508, activation: Activation::Shift(11) },
    Variant { id: "extra-b-1016", description: "S=1016, a=(c*c)>>13 (literal shift, mismatch kept)", scale: 1016, activation: Activation::Shift(13) },
];

const SCALES: [i64; 4] = [127, 254, 508, 1016];

#[derive(Serialize, Clone)]
struct Stats {
    mean_absolute: f64,
    se_mean_absolute: f64,
    p99_absolute: f64,
    max_absolute: f64,
    mean_signed: f64,
    sd_signed: f64,
}

fn stats(errors: &[f64]) -> Stats {
    let n = errors.len() as f64;
    let mut absolute: Vec<f64> = errors.iter().map(|e| e.abs()).collect();
    assert!(absolute.iter().all(|e| e.is_finite()));
    absolute.sort_by(f64::total_cmp);
    let mean_signed = errors.iter().sum::<f64>() / n;
    let variance = errors.iter().map(|e| (e - mean_signed).powi(2)).sum::<f64>() / (n - 1.0);
    let mean_absolute = absolute.iter().sum::<f64>() / n;
    let variance_absolute = absolute.iter().map(|e| (e - mean_absolute).powi(2)).sum::<f64>() / (n - 1.0);
    Stats {
        mean_absolute,
        se_mean_absolute: (variance_absolute / n).sqrt(),
        p99_absolute: absolute[(0.99 * n).ceil() as usize - 1],
        max_absolute: *absolute.last().unwrap(),
        mean_signed,
        sd_signed: variance.sqrt(),
    }
}

fn sd(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
}

/// Nearest-rank percentile of a sorted slice.
fn percentile<T: Copy>(sorted: &[T], p: f64) -> T {
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn top_sum(values: &mut [i64], count: usize) -> i64 {
    values.sort_unstable_by(|a, b| b.cmp(a));
    values.iter().take(count).sum()
}

fn analyze() -> Result<()> {
    let mut text = String::new();
    macro_rules! say {
        ($($arg:tt)*) => {{
            let line = format!($($arg)*);
            println!("{line}");
            text.push_str(&line);
            text.push('\n');
        }};
    }

    let summary = read_json(format!("{RUN}/summary.json"))?;
    let cache: Cache = serde_json::from_slice(&fs::read(format!("{OUT}/sample-cache.json"))?)?;
    let checkpoint = Checkpoint::read(format!("{RUN}/best.npz"))?;
    let model = &checkpoint.model;
    let bytes = fs::read(format!("{RUN}/nnue.bin"))?;
    let bin = parse_bin(&bytes)?;
    let h = bin.width;
    assert_eq!(h, model.shape().width());
    let n = cache.records.len();

    // Ladder 0: the sample is the trainer's.
    let summary_sha = summary["quantization"]["indices_sha256"].as_str().unwrap();
    assert_eq!(cache.indices_sha256, summary_sha, "sample indices differ from summary.json");
    say!("sample: {n} positions, indices_sha256 equals summary.json ({summary_sha})");

    // Ladder 1: nnue.bin is exactly `quantize(best.npz)`, and my rounding is exactly the trainer's.
    let quantized = quantize(model)?;
    assert_eq!(quantized.linear, bin.linear, "linear");
    assert_eq!(quantized.weights, bin.weights, "weights");
    assert_eq!(quantized.biases, bin.biases, "biases");
    assert_eq!(quantized.output_weights, bin.output_weights, "output weights");
    assert_eq!(quantized.output_bias, bin.output_bias, "output bias");
    let cp_per_logit_f32 = model.shape().cp_per_logit();
    assert_eq!(cp_per_logit_f32, bin.cp_per_logit, "cp_per_logit");
    let cpl = f64::from(cp_per_logit_f32);
    let shipped_scale = (cpl * 65536.0 / OUTPUT_DENOMINATOR).round_ties_even() as i64;
    say!(
        "cp_per_logit={cpl}, shipped conversion factor scale={shipped_scale} (unrounded {:.6}, relative {:+.4}%)",
        cpl * 65536.0 / OUTPUT_DENOMINATOR,
        (shipped_scale as f64 / (cpl * 65536.0 / OUTPUT_DENOMINATOR) - 1.0) * 100.0
    );

    let w_f = model.expanded_first_layer();
    assert_eq!(w_f.len(), (D + 1) * h);
    let b_f = model.expanded_biases();
    let o_f = model.expanded_output_weights();
    let ob_f = model.parameters()[Parameter::OutputBias][0];
    let l_f = model.expanded_linear();
    let o: Vec<i64> = o_f.iter().map(|&v| rounded(v, 4096.0)).collect();
    let ob = rounded(ob_f, OUTPUT_DENOMINATOR);
    assert!(o.iter().zip(&bin.output_weights).all(|(&a, &b)| a == i64::from(b)));
    assert_eq!(ob, i64::from(bin.output_bias));
    let quantized_w: Vec<(i64, Vec<i64>, Vec<i64>)> = SCALES
        .iter()
        .map(|&s| {
            (
                s,
                w_f[..D * h].iter().map(|&v| rounded(v, s as f64)).collect(),
                b_f.iter().map(|&v| rounded(v, s as f64)).collect(),
            )
        })
        .collect();
    assert!(quantized_w[0].1.iter().zip(&bin.weights).all(|(&a, &b)| a == i64::from(b)));
    assert!(quantized_w[0].2.iter().zip(&bin.biases).all(|(&a, &b)| a == i64::from(b)));
    say!("ladder 1 passed: nnue.bin arrays equal quantize(best.npz) and my rounding at S=127");

    // Accumulators per scale, f64 accumulators, and the f64 reference of the nonlinear path.
    let mut accumulators: Vec<Vec<i32>> = Vec::new();
    for (_, w, b) in &quantized_w {
        let mut acc = vec![0_i32; n * 2 * h];
        for (r, record) in cache.records.iter().enumerate() {
            for (v, features) in [&record.us, &record.them].into_iter().enumerate() {
                let out = &mut acc[(r * 2 + v) * h..(r * 2 + v + 1) * h];
                for (o, &bias) in out.iter_mut().zip(b) {
                    *o = i32::try_from(bias)?;
                }
                for &f in features {
                    let row = &w[f as usize * h..(f as usize + 1) * h];
                    for (o, &weight) in out.iter_mut().zip(row) {
                        *o += i32::try_from(weight)?;
                    }
                }
            }
        }
        accumulators.push(acc);
    }
    let mut acc_f = vec![0.0_f64; n * 2 * h];
    let mut reference_nonlinear = vec![0.0_f64; n];
    let mut linear_int = vec![0_i64; n];
    let mut limit_out = vec![0.0_f64; n];
    for (r, record) in cache.records.iter().enumerate() {
        let mut logit = f64::from(ob_f);
        let mut limit = ob as f64;
        for (v, features) in [&record.us, &record.them].into_iter().enumerate() {
            let out = &mut acc_f[(r * 2 + v) * h..(r * 2 + v + 1) * h];
            for (j, a) in out.iter_mut().enumerate() {
                let mut hidden = 0.0_f64;
                for &f in features {
                    hidden += f64::from(w_f[f as usize * h + j]);
                }
                hidden += f64::from(b_f[j]);
                *a = hidden;
                let s = hidden.clamp(0.0, 1.0).powi(2);
                logit += s * f64::from(o_f[v * h + j]);
                limit += s * 127.0 * o[v * h + j] as f64;
            }
        }
        reference_nonlinear[r] = cpl * logit;
        limit_out[r] = limit;

        // Ladder 2: my f64 decomposition reproduces evaluate_f64.
        let mut sum = [0.0_f64; 2];
        let (mut mg, mut eg) = (0_i64, 0_i64);
        for &f in &record.us {
            sum[0] += f64::from(l_f[f as usize][0]);
            sum[1] += f64::from(l_f[f as usize][1]);
            mg += i64::from(bin.linear[f as usize][0]);
            eg += i64::from(bin.linear[f as usize][1]);
        }
        let q = i64::from(record.pieces.saturating_sub(2)).min(90);
        let ratio = q as f64 / 90.0;
        let linear_f64 = ratio * sum[0] + (1.0 - ratio) * sum[1];
        let difference = linear_f64 + reference_nonlinear[r] - record.reference_total;
        assert!(difference.abs() < 1e-6, "f64 decomposition differs by {difference} at {}", record.index);
        linear_int[r] = (q * mg + (90 - q) * eg) / 720;
    }
    say!("ladder 2 passed: lin_f64 + cp_per_logit*head equals evaluate_f64 within 1e-6 cp on all positions");

    let reference_sd = sd(&reference_nonlinear);
    let reference_total: Vec<f64> = cache.records.iter().map(|r| r.reference_total).collect();
    let total_sd = sd(&reference_total);
    let linear_f64: Vec<f64> = reference_total.iter().zip(&reference_nonlinear).map(|(t, nl)| t - nl).collect();
    let reference_abs_mean = reference_nonlinear.iter().map(|x| x.abs()).sum::<f64>() / n as f64;
    say!(
        "reference nonlinear path: sd={reference_sd:.3} cp, mean abs={reference_abs_mean:.3} cp, min={:.1}, max={:.1}",
        reference_nonlinear.iter().cloned().fold(f64::INFINITY, f64::min),
        reference_nonlinear.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
    );
    say!("reference total: sd={total_sd:.3} cp; reference linear path: sd={:.3} cp", sd(&linear_f64));

    // Variants.
    let mut rows = Vec::new();
    say!("");
    say!(
        "{:<13} {:>8} {:>8} {:>8} {:>8} {:>8} | {:>8} {:>8} {:>8} {:>8} | {:>8} {:>8} {:>8} | {:>7}  description",
        "id", "MAE", "p99", "max", "mean", "sd", "shipMAE", "shipP99", "shipMax", "shipMean", "totMAE", "totP99", "totMax", "MAE/SD%"
    );
    let report_row = |id: &str, description: &str, exact: &[f64], shipped: &[f64], total: &[f64], text: &mut String, rows: &mut Vec<Value>| {
        let (a, b, c) = (stats(exact), stats(shipped), stats(total));
        let share = a.mean_absolute / reference_sd * 100.0;
        let line = format!(
            "{:<13} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3} | {:>8.3} {:>8.3} {:>8.3} {:>8.3} | {:>8.3} {:>8.3} {:>8.3} | {:>7.3}  {}",
            id, a.mean_absolute, a.p99_absolute, a.max_absolute, a.mean_signed, a.sd_signed,
            b.mean_absolute, b.p99_absolute, b.max_absolute, b.mean_signed,
            c.mean_absolute, c.p99_absolute, c.max_absolute, share, description
        );
        println!("{line}");
        text.push_str(&line);
        text.push('\n');
        rows.push(json!({"id": id, "description": description, "nonlinear_exact_conversion": a,
            "nonlinear_shipped_conversion": b, "total_with_shipped_linear": c,
            "mean_absolute_over_reference_sd_percent": share}));
    };

    let mut baseline_total_stats = None;
    let mut exact_by_id: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
    let mut shipped_by_id: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
    for variant in VARIANTS {
        let s_index = SCALES.iter().position(|&s| s == variant.scale).unwrap();
        let acc = &accumulators[s_index];
        let s = variant.scale;
        let denominator: i128 = match variant.activation {
            Activation::Exact => i128::from(s * s),
            Activation::Exact128 => 128,
            Activation::Shift(_) | Activation::Floor => 1,
        };
        let mut exact = Vec::with_capacity(n);
        let mut shipped = Vec::with_capacity(n);
        let mut total = Vec::with_capacity(n);
        let mut clamped_positions = 0_usize;
        for (r, record) in cache.records.iter().enumerate() {
            let mut numerator = i128::from(ob) * denominator;
            for j in 0..2 * h {
                let c = i64::from(acc[r * 2 * h + j]).clamp(0, s);
                let weight = i128::from(o[j]);
                numerator += match variant.activation {
                    Activation::Shift(k) => weight * i128::from((c * c) >> k),
                    Activation::Floor => weight * i128::from(c * c * 127 / (s * s)),
                    Activation::Exact => weight * i128::from(c * c * 127),
                    Activation::Exact128 => weight * i128::from(c * c),
                };
            }
            let out = numerator as f64 / denominator as f64;
            exact.push(out * cpl / OUTPUT_DENOMINATOR - reference_nonlinear[r]);
            let shipped_cp = (numerator * i128::from(shipped_scale) + 32_768 * denominator)
                .div_euclid(65_536 * denominator) as i64;
            shipped.push(shipped_cp as f64 - reference_nonlinear[r]);
            let total_cp = (linear_int[r] + shipped_cp).clamp(-EVALUATION_LIMIT, EVALUATION_LIMIT);
            total.push(total_cp as f64 - record.reference_total);

            if variant.id == "1" {
                // Ladder 3: per-position equality with minase's Head::output and the decoded total.
                let q = i32::try_from(record.pieces.saturating_sub(2).min(90))?;
                let us = &acc[r * 2 * h..r * 2 * h + h];
                let them = &acc[r * 2 * h + h..(r + 1) * 2 * h];
                let minase_out = bin.head.output(us, them, q);
                assert_eq!(numerator, i128::from(minase_out), "Head::output differs at {}", record.index);
                assert_eq!(total_cp, i64::from(record.minase_total), "total differs at {}", record.index);
                if (linear_int[r] + shipped_cp).abs() >= EVALUATION_LIMIT {
                    clamped_positions += 1;
                }
            }
        }
        report_row(variant.id, variant.description, &exact, &shipped, &total, &mut text, &mut rows);
        exact_by_id.insert(variant.id.to_owned(), exact);
        shipped_by_id.insert(variant.id.to_owned(), shipped);
        if variant.id == "1" {
            baseline_total_stats = Some((stats(&total), clamped_positions));
        }
    }
    let (mut exact, mut shipped, mut total) = (Vec::new(), Vec::new(), Vec::new());
    for (r, record) in cache.records.iter().enumerate() {
        exact.push(limit_out[r] * cpl / OUTPUT_DENOMINATOR - reference_nonlinear[r]);
        let shipped_cp = (limit_out[r] * shipped_scale as f64 / 65536.0 + 0.5).floor();
        shipped.push(shipped_cp - reference_nonlinear[r]);
        total.push((linear_int[r] as f64 + shipped_cp).clamp(-(EVALUATION_LIMIT as f64), EVALUATION_LIMIT as f64) - record.reference_total);
    }
    report_row("extra-c", "S=inf (f64 accumulators), exact activation, output weights at 4096 only", &exact, &shipped, &total, &mut text, &mut rows);
    exact_by_id.insert("extra-c".to_owned(), exact);
    shipped_by_id.insert("extra-c".to_owned(), shipped);

    // Gain error: OLS slope of the error on the f64 reference, per variant.
    say!("");
    let reference_mean = reference_nonlinear.iter().sum::<f64>() / n as f64;
    let reference_variance = reference_nonlinear.iter().map(|x| (x - reference_mean).powi(2)).sum::<f64>();
    let gain = |errors: &[f64]| {
        let mean = errors.iter().sum::<f64>() / n as f64;
        errors.iter().zip(&reference_nonlinear).map(|(e, x)| (e - mean) * (x - reference_mean)).sum::<f64>() / reference_variance
    };
    let mut gains = Vec::new();
    for (id, errors) in &exact_by_id {
        let slope = gain(errors);
        let residual: Vec<f64> = errors.iter().zip(&reference_nonlinear).map(|(e, x)| e - slope * x).collect();
        say!("gain {id:<13} slope={:+.4}% of reference, error sd after removing the slope={:.3} cp", slope * 100.0, sd(&residual));
        gains.push(json!({"id": id, "slope": slope, "residual_sd": sd(&residual)}));
    }

    // Stepwise decomposition per position, in the exact conversion except for the conversion step itself.
    let difference = |a: &[f64], b: &[f64]| -> Vec<f64> { a.iter().zip(b).map(|(x, y)| x - y).collect() };
    let e = |id: &str| exact_by_id[id].as_slice();
    let total_error = shipped_by_id["1"].clone();
    let total_square = total_error.iter().map(|x| x * x).sum::<f64>();
    let component = |label: &str, values: &[f64], text: &mut String, rows: &mut Vec<Value>| {
        let m = values.iter().map(|x| x.abs()).sum::<f64>() / n as f64;
        let rms = (values.iter().map(|x| x * x).sum::<f64>() / n as f64).sqrt();
        let mean = values.iter().sum::<f64>() / n as f64;
        let share = values.iter().zip(&total_error).map(|(x, t)| x * t).sum::<f64>() / total_square * 100.0;
        let line = format!("step {label:<58} MAE={m:>7.3} RMS={rms:>7.3} mean={mean:>+7.3} share of baseline MSE={share:>+7.2}%");
        println!("{line}");
        text.push_str(&line);
        text.push('\n');
        rows.push(json!({"label": label, "mae": m, "rms": rms, "mean": mean, "share_of_baseline_mse_percent": share}));
    };
    let mut steps = Vec::new();
    say!("");
    let total_rms = (total_square / n as f64).sqrt();
    say!("baseline nonlinear error (shipped conversion): RMS={total_rms:.3} cp. Order A: reference -> output weights -> first layer -> floor -> 127/128 -> conversion");
    component("A output weights at 4096 [extra-c - reference]", e("extra-c"), &mut text, &mut steps);
    component("A first layer S=127 [2 - extra-c]", &difference(e("2"), e("extra-c")), &mut text, &mut steps);
    component("A floor [3 - 2]", &difference(e("3"), e("2")), &mut text, &mut steps);
    component("A 127/128 [1 - 3]", &difference(e("1"), e("3")), &mut text, &mut steps);
    component("A conversion scale=135 and rounding [shipped 1 - exact 1]", &difference(&shipped_by_id["1"], e("1")), &mut text, &mut steps);
    say!("Order B swaps the last two activation steps: 127/128 first, floor second");
    component("B 127/128 [extra-a - 2]", &difference(e("extra-a"), e("2")), &mut text, &mut steps);
    component("B floor [1 - extra-a]", &difference(e("1"), e("extra-a")), &mut text, &mut steps);

    say!("");
    say!("per-scale components (per-position differences, exact conversion):");
    let mut per_scale = Vec::new();
    for (scale, exact_id, floor_id, shift_id) in [
        (127, "2", "3", "1"),
        (254, "4b-254", "4a-254", "extra-b-254"),
        (508, "4b-508", "4a-508", "extra-b-508"),
        (1016, "4b-1016", "4a-1016", "extra-b-1016"),
    ] {
        let rms = |values: &[f64]| (values.iter().map(|x| x * x).sum::<f64>() / n as f64).sqrt();
        let mae = |values: &[f64]| values.iter().map(|x| x.abs()).sum::<f64>() / n as f64;
        let first = difference(e(exact_id), e("extra-c"));
        let floor = difference(e(floor_id), e(exact_id));
        let mismatch = difference(e(shift_id), e(floor_id));
        say!(
            "S={scale:<5} first layer MAE={:>7.3} RMS={:>7.3} | floor MAE={:>6.3} RMS={:>6.3} mean={:>+7.3} | 127/128 MAE={:>6.3} RMS={:>6.3} mean={:>+7.3}",
            mae(&first), rms(&first), mae(&floor), rms(&floor), floor.iter().sum::<f64>() / n as f64,
            mae(&mismatch), rms(&mismatch), mismatch.iter().sum::<f64>() / n as f64
        );
        per_scale.push(json!({"s": scale,
            "first_layer": {"mae": mae(&first), "rms": rms(&first)},
            "floor": {"mae": mae(&floor), "rms": rms(&floor), "mean": floor.iter().sum::<f64>() / n as f64},
            "mismatch": {"mae": mae(&mismatch), "rms": rms(&mismatch), "mean": mismatch.iter().sum::<f64>() / n as f64}}));
    }

    // Ladder 4: the trainer's own number.
    let (baseline_total, clamped_positions) = baseline_total_stats.unwrap();
    let summary_mean = summary["quantization"]["mean_absolute_error_cp"].as_f64().unwrap();
    let summary_max = summary["quantization"]["max_absolute_error_cp"].as_f64().unwrap();
    say!("");
    say!("ladder 3 passed: Head::output and the decoded total are equal per position on all {n} positions; positions at the +-{EVALUATION_LIMIT} clamp: {clamped_positions}");
    say!(
        "ladder 4: baseline total MAE={:.15} vs summary {:.15} (diff {:.3e}); max={:.14} vs summary {:.14} (diff {:.3e})",
        baseline_total.mean_absolute, summary_mean, baseline_total.mean_absolute - summary_mean,
        baseline_total.max_absolute, summary_max, baseline_total.max_absolute - summary_max
    );
    assert!((baseline_total.mean_absolute - summary_mean).abs() < 1e-9, "baseline total MAE differs from summary.json");
    assert!((baseline_total.max_absolute - summary_max).abs() < 1e-9, "baseline total max differs from summary.json");

    // Linear path alone.
    let linear_errors: Vec<f64> = linear_int.iter().zip(&linear_f64).map(|(&i, &f)| i as f64 - f).collect();
    let linear_stats = stats(&linear_errors);
    say!(
        "linear path alone (integer minus f64): MAE={:.3}, p99={:.3}, max={:.3}, mean={:.3}, sd={:.3}",
        linear_stats.mean_absolute, linear_stats.p99_absolute, linear_stats.max_absolute, linear_stats.mean_signed, linear_stats.sd_signed
    );

    // Active features.
    let mut counts: Vec<usize> = cache.records.iter().map(|r| r.us.len()).collect();
    assert!(cache.records.iter().all(|r| r.us.len() == r.them.len()));
    counts.sort_unstable();
    let lion_positions = cache.records.iter().filter(|r| r.us.iter().any(|&f| usize::from(f) >= 13_536)).count();
    let count_mean = counts.iter().sum::<usize>() as f64 / n as f64;
    let count_max = *counts.last().unwrap();
    say!("");
    say!(
        "active features per perspective: mean={count_mean:.3}, min={}, p1={}, p50={}, p99={}, max={count_max}; lion-trigger positions={lion_positions}",
        counts[0], percentile(&counts, 1.0), percentile(&counts, 50.0), percentile(&counts, 99.0)
    );

    // Accumulator distribution at S=127.
    let percentile_points = [0.1, 1.0, 5.0, 25.0, 50.0, 75.0, 95.0, 99.0, 99.9];
    let acc127 = &accumulators[0];
    let mut pooled = acc127.clone();
    pooled.sort_unstable();
    let fraction = |values: &[i32], f: &dyn Fn(i32) -> bool| values.iter().filter(|&&v| f(v)).count() as f64 / values.len() as f64;
    let pooled_le0 = fraction(&pooled, &|v| v <= 0);
    let pooled_ge127 = fraction(&pooled, &|v| v >= 127);
    say!(
        "S=127 pooled accumulators ({} values): min={}, max={}, {}; fraction<=0={:.5}, fraction>=127={:.5}, fraction strictly inside={:.5}",
        pooled.len(), pooled[0], pooled[pooled.len() - 1],
        percentile_points.iter().map(|&p| format!("p{p}={}", percentile(&pooled, p))).collect::<Vec<_>>().join(", "),
        pooled_le0, pooled_ge127, 1.0 - pooled_le0 - pooled_ge127
    );
    let mut csv = String::from("unit,min,p0.1,p1,p5,p25,p50,p75,p95,p99,p99.9,max,fraction_le0,fraction_ge127\n");
    let mut per_unit: Vec<[f64; 6]> = Vec::with_capacity(h);
    for j in 0..h {
        let mut values: Vec<i32> = (0..n * 2).map(|i| acc127[i * h + j]).collect();
        values.sort_unstable();
        let (le0, ge127) = (fraction(&values, &|v| v <= 0), fraction(&values, &|v| v >= 127));
        csv.push_str(&format!(
            "{j},{},{},{},{le0:.6},{ge127:.6}\n",
            values[0],
            percentile_points.iter().map(|&p| percentile(&values, p).to_string()).collect::<Vec<_>>().join(","),
            values[values.len() - 1],
        ));
        per_unit.push([values[0] as f64, values[values.len() - 1] as f64, percentile(&values, 50.0) as f64, le0, ge127, 1.0 - le0 - ge127]);
    }
    fs::write(format!("{OUT}/per_unit_s127.csv"), csv)?;
    for (label, i) in [("min", 0), ("max", 1), ("p50", 2), ("fraction<=0", 3), ("fraction>=127", 4), ("fraction inside", 5)] {
        let mut column: Vec<f64> = per_unit.iter().map(|row| row[i]).collect();
        column.sort_by(f64::total_cmp);
        say!(
            "S=127 per-unit {label}: across {h} units min={:.4}, p25={:.4}, median={:.4}, p75={:.4}, max={:.4}",
            column[0], percentile(&column, 25.0), percentile(&column, 50.0), percentile(&column, 75.0), column[h - 1]
        );
    }

    // Range per scale.
    say!("");
    let mut range_rows = Vec::new();
    let distinct_counts: Vec<usize> = {
        let mut values = vec![count_max, 92, 145];
        values.sort_unstable();
        values.dedup();
        values
    };
    for (i, (s, w, b)) in quantized_w.iter().enumerate() {
        let acc = &accumulators[i];
        let observed = acc.iter().map(|&v| i64::from(v).abs()).max().unwrap();
        let (low, high) = (*acc.iter().min().unwrap(), *acc.iter().max().unwrap());
        let max_weight = w.iter().map(|v| v.abs()).max().unwrap();
        let max_bias = b.iter().map(|v| v.abs()).max().unwrap();
        let inside = acc.iter().filter(|&&v| v > 0 && i64::from(v) < *s).count() as f64 / acc.len() as f64;
        say!(
            "S={s}: observed |acc|max={observed} (min={low}, max={high}), fits i16 observed={}, fraction strictly inside (0,S)={inside:.5}, max|w|={max_weight}, max|b|={max_bias}",
            observed <= 32_767
        );
        let row_max: Vec<i64> = (0..D).map(|f| w[f * h..(f + 1) * h].iter().map(|v| v.abs()).max().unwrap()).collect();
        let mut bounds = Vec::new();
        for &count in &distinct_counts {
            // L1: the task's bound, per-row maxima over units.
            let mut rows_copy = row_max.clone();
            let l1 = max_bias + top_sum(&mut rows_copy, count);
            // L2: per-unit bound over all rows.
            let mut l2 = 0;
            // L3: per-unit bound with at most one active row per square, and the lion trigger as a further slot.
            let mut l3 = 0;
            for j in 0..h {
                let mut column: Vec<i64> = (0..D).map(|f| w[f * h + j].abs()).collect();
                let mut slots = vec![0_i64; 145];
                for (f, &value) in column.iter().enumerate() {
                    let slot = if f >= 13_536 { 144 } else { f % 144 };
                    slots[slot] = slots[slot].max(value);
                }
                l2 = l2.max(b[j].abs() + top_sum(&mut column, count));
                l3 = l3.max(b[j].abs() + top_sum(&mut slots, count));
            }
            say!(
                "S={s} static worst case N={count}: L1(per-row max)={l1} fits i16={}, L2(per-unit)={l2} fits i16={}, L3(per-unit, one row per square)={l3} fits i16={}",
                l1 <= 32_767, l2 <= 32_767, l3 <= 32_767
            );
            bounds.push(json!({"n": count, "l1_per_row_max": l1, "l2_per_unit": l2, "l3_per_unit_per_square": l3}));
        }
        range_rows.push(json!({"s": s, "observed_abs_max": observed, "observed_min": low, "observed_max": high,
            "fraction_strictly_inside": inside, "max_abs_weight": max_weight, "max_abs_bias": max_bias, "static_worst_case": bounds}));
    }

    let results = json!({
        "sample": {"positions": n, "indices_sha256": cache.indices_sha256, "split": "validation"},
        "cp_per_logit": cpl,
        "shipped_scale": shipped_scale,
        "reference": {"nonlinear_sd_cp": reference_sd, "nonlinear_mean_abs_cp": reference_abs_mean, "total_sd_cp": total_sd, "linear_sd_cp": sd(&linear_f64)},
        "variants": rows,
        "gain": gains,
        "decomposition_steps": steps,
        "per_scale_components": per_scale,
        "linear_path_alone": linear_stats,
        "baseline_total": baseline_total,
        "positions_at_clamp": clamped_positions,
        "active_features": {"mean": count_mean, "max": count_max, "min": counts[0], "lion_trigger_positions": lion_positions},
        "accumulators_s127": {
            "min": pooled[0], "max": pooled[pooled.len() - 1],
            "percentiles": percentile_points.iter().map(|&p| json!({"p": p, "value": percentile(&pooled, p)})).collect::<Vec<_>>(),
            "fraction_le0": pooled_le0, "fraction_ge127": pooled_ge127,
        },
        "range": range_rows,
    });
    fs::write(format!("{OUT}/results.json"), serde_json::to_vec_pretty(&results)?)?;
    fs::write(format!("{OUT}/results.txt"), &text)?;
    Ok(())
}

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("extract") => extract(),
        Some("analyze") => analyze(),
        _ => Err("usage: quant extract|analyze".into()),
    }
}
