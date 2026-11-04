//! best.npzを使い、整数化の各段が量子化誤差へ与える寄与を切り分ける使い捨ての集計。
use minase_trainer::data::Dataset;
use minase_trainer::nnue::data::{InputFeatureSet, SENTINEL, build_batches};
use minase_trainer::nnue::model::{Checkpoint, Parameter};
use std::num::NonZeroUsize;
use std::path::PathBuf;

#[derive(Clone, Copy, Default)]
struct Cfg {
    q_acc: bool,
    floor_s: bool,
    round_s: bool,
    q_w1: bool,
    q_b1: bool,
    floor_u: bool,
    round_u: bool,
    floor_c: bool,
    round_c: bool,
    q_w2: bool,
    q_b2: bool,
    floor_y: bool,
    round_y: bool,
    q_o: bool,
    q_v: bool,
    q_b: bool,
    q_scale: bool,
    /// 隠れ層の重みの尺度。0なら64。
    w_scale: u32,
    /// yの細かさ（127×2^nの段階）。
    y_bits: u32,
    /// uの細かさ。
    u_bits: u32,
}

const H: usize = 256;
const K: f64 = 1072.6529541015625;

struct P {
    o: Vec<f64>,
    w1: Vec<f64>,
    b1: Vec<f64>,
    w2: Vec<f64>,
    b2: Vec<f64>,
    v: Vec<f64>,
    b: f64,
}

fn rnd(x: f64, s: f64) -> f64 {
    (x * s).round_ties_even() / s
}

fn eval(c: &Cfg, p: &P, xf: &[f64], xi: &[i64]) -> f64 {
    let q = H / 4;
    let mut s = vec![0.0_f64; H];
    for view in 0..2 {
        let val = |j: usize| -> f64 {
            let i = view * H + j;
            if c.q_acc {
                (xi[i] as f64 / 255.0).clamp(0.0, 1.0)
            } else {
                xf[i].clamp(0.0, 1.0)
            }
        };
        for j in 0..q {
            let pairs = [(j, j + q, j), (j + 2 * q, j + 3 * q, j + q)];
            for (a, b, out) in pairs {
                let (ca, cb) = (val(a) * 255.0, val(b) * 255.0);
                let value = if c.round_s {
                    ((ca * cb + 256.0) / 512.0).floor().min(127.0) / 127.0
                } else if c.floor_s {
                    (ca * cb / 512.0).floor() / 127.0
                } else {
                    ca * cb / (255.0 * 255.0)
                };
                s[view * H / 2 + out] = value;
            }
        }
    }
    let w_scale = if c.w_scale == 0 { 64.0 } else { c.w_scale as f64 };
    let mut u = [0.0_f64; 30];
    for k in 0..15 {
        let mut z = 0.0;
        for i in 0..H {
            let w = p.w1[k * H + i];
            z += (if c.q_w1 { rnd(w, w_scale) } else { w }) * s[i];
        }
        z += if c.q_b1 { rnd(p.b1[k], 8128.0) } else { p.b1[k] };
        let zi = z * 8128.0;
        let ubits = (1u32 << c.u_bits) as f64;
        u[k] = if c.round_u {
            ((zi * zi + 262_144.0) / 524_288.0).floor().min(127.0) / 127.0
        } else if c.floor_u {
            ((zi * zi / 524_288.0 * ubits).floor() / ubits).min(127.0) / 127.0
        } else {
            (127.0 / 128.0 * z * z).min(1.0)
        };
        u[15 + k] = if c.round_c {
            ((zi + 32.0) / 64.0).floor().clamp(0.0, 127.0) / 127.0
        } else if c.floor_c {
            ((zi / 64.0 * ubits).floor() / ubits).clamp(0.0, 127.0) / 127.0
        } else {
            z.clamp(0.0, 1.0)
        };
    }
    let mut y = [0.0_f64; 32];
    for k in 0..32 {
        let mut a = 0.0;
        for i in 0..30 {
            let w = p.w2[k * 30 + i];
            a += (if c.q_w2 { rnd(w, w_scale) } else { w }) * u[i];
        }
        a += if c.q_b2 { rnd(p.b2[k], 8128.0) } else { p.b2[k] };
        let ai = a * 8128.0;
        let ybits = (1u32 << c.y_bits) as f64;
        y[k] = if c.round_y {
            ((ai + 32.0) / 64.0).floor().clamp(0.0, 127.0) / 127.0
        } else if c.floor_y {
            ((ai / 64.0 * ybits).floor() / ybits).clamp(0.0, 127.0) / 127.0
        } else {
            a.clamp(0.0, 1.0)
        };
    }
    let mut out = if c.q_b { rnd(p.b, 127.0 * 4096.0) } else { p.b };
    for i in 0..H {
        out += (if c.q_o { rnd(p.o[i], 4096.0) } else { p.o[i] }) * s[i];
    }
    for k in 0..32 {
        out += (if c.q_v { rnd(p.v[k], 4096.0) } else { p.v[k] }) * y[k];
    }
    let cp = if c.q_scale {
        let scale = (K * 65_536.0 / (127.0 * 4096.0)).round_ties_even();
        out * 127.0 * 4096.0 * scale / 65_536.0
    } else {
        out * K
    };
    cp
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let checkpoint = Checkpoint::read(&args[1]).unwrap();
    let files: Vec<PathBuf> = args[2..].iter().map(PathBuf::from).collect();
    let dataset = Dataset::open(&files, None, NonZeroUsize::new(8).unwrap()).unwrap();
    let vi = dataset.validation_indices();
    let stride = (vi.len() / 10_000).max(1);
    let indices: Vec<u64> = vi.iter().step_by(stride).copied().take(10_000).collect();
    println!("validation {} sample {}", vi.len(), indices.len());
    let model = &checkpoint.model;
    let params = model.parameters();
    assert_eq!(model.shape().width(), H);
    let p = P {
        o: model.expanded_output_weights().iter().map(|&x| x as f64).collect(),
        w1: model.expanded_hidden1_weights().iter().map(|&x| x as f64).collect(),
        b1: params[Parameter::Hidden1Biases].iter().map(|&x| x as f64).collect(),
        w2: params[Parameter::Hidden2Weights].iter().map(|&x| x as f64).collect(),
        b2: params[Parameter::Hidden2Biases].iter().map(|&x| x as f64).collect(),
        v: params[Parameter::Hidden2Output].iter().map(|&x| x as f64).collect(),
        b: params[Parameter::OutputBias][0] as f64,
    };
    let first = &params[Parameter::FirstLayer];
    let biases = model.expanded_biases();
    let mut xs: Vec<(Vec<f64>, Vec<i64>)> = Vec::new();
    for chunk in indices.chunks(512) {
        let batch = build_batches(&dataset, InputFeatureSet::P, &[chunk]).unwrap().pop().unwrap();
        for b in 0..batch.len() {
            let mut xf = vec![0.0_f64; 2 * H];
            let mut xi = vec![0_i64; 2 * H];
            for v in 0..2 {
                for j in 0..H {
                    xf[v * H + j] = biases[j] as f64;
                    xi[v * H + j] = (biases[j] as f64 * 255.0).round_ties_even() as i64;
                }
                let cols = &batch.features()[(b * 2 + v) * 145..(b * 2 + v + 1) * 145];
                for &word in cols {
                    if word == SENTINEL {
                        continue;
                    }
                    let row = word as usize / 2;
                    let rot = word & 1 != 0;
                    for j in 0..H {
                        let unit = (j + if rot { H / 2 } else { 0 }) % H;
                        let w = first[row * H + unit] as f64;
                        xf[v * H + j] += w;
                        xi[v * H + j] += (w * 255.0).round_ties_even() as i64;
                    }
                }
            }
            xs.push((xf, xi));
        }
    }
    let base = Cfg::default();
    let reference: Vec<f64> = xs.iter().map(|(f, i)| eval(&base, &p, f, i)).collect();
    let std = {
        let m = reference.iter().sum::<f64>() / reference.len() as f64;
        (reference.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / reference.len() as f64).sqrt()
    };
    println!("nonlinear std {:.1} cp", std);
    let measure = |name: &str, c: Cfg| {
        let mut abs = 0.0;
        let mut signed = 0.0;
        let mut sq = 0.0;
        let mut max = 0.0_f64;
        for ((f, i), r) in xs.iter().zip(&reference) {
            let e = eval(&c, &p, f, i) - r;
            abs += e.abs();
            signed += e;
            sq += e * e;
            max = max.max(e.abs());
        }
        let n = xs.len() as f64;
        let mean = signed / n;
        println!(
            "{name:34} mean_abs {:6.2}  mean {:7.2}  sd {:6.2}  max {:6.1}",
            abs / n,
            mean,
            (sq / n - mean * mean).sqrt(),
            max
        );
    };
    let full = Cfg {
        q_acc: true,
        floor_s: true,
        q_w1: true,
        q_b1: true,
        floor_u: true,
        floor_c: true,
        q_w2: true,
        q_b2: true,
        floor_y: true,
        q_o: true,
        q_v: true,
        q_b: true,
        q_scale: true,
        ..Cfg::default()
    };
    println!("--- each stage alone");
    measure("first layer rounding", Cfg { q_acc: true, ..base });
    measure("s floor", Cfg { floor_s: true, ..base });
    measure("W1 rounding", Cfg { q_w1: true, ..base });
    measure("b1 rounding", Cfg { q_b1: true, ..base });
    measure("u square floor", Cfg { floor_u: true, ..base });
    measure("u clip floor", Cfg { floor_c: true, ..base });
    measure("W2 rounding", Cfg { q_w2: true, ..base });
    measure("b2 rounding", Cfg { q_b2: true, ..base });
    measure("y floor", Cfg { floor_y: true, ..base });
    measure("o rounding", Cfg { q_o: true, ..base });
    measure("v rounding", Cfg { q_v: true, ..base });
    measure("b rounding", Cfg { q_b: true, ..base });
    measure("cp scale", Cfg { q_scale: true, ..base });
    println!("--- full integer and leave one out");
    measure("all stages", full);
    measure("  without first layer rounding", Cfg { q_acc: false, ..full });
    measure("  without s floor", Cfg { floor_s: false, ..full });
    measure("  without W1 rounding", Cfg { q_w1: false, ..full });
    measure("  without u floors", Cfg { floor_u: false, floor_c: false, ..full });
    measure("  without W2 rounding", Cfg { q_w2: false, ..full });
    measure("  without y floor", Cfg { floor_y: false, ..full });
    println!("--- variants of the integer formula");
    measure("round nearest in s", Cfg { round_s: true, ..full });
    measure("round nearest in u", Cfg { round_u: true, round_c: true, ..full });
    measure("round nearest in y", Cfg { round_y: true, ..full });
    measure("round nearest in s,u,y", Cfg { round_s: true, round_u: true, round_c: true, round_y: true, ..full });
    measure("round nearest in u,y", Cfg { round_u: true, round_c: true, round_y: true, ..full });
    measure("W1,W2 scale 256", Cfg { w_scale: 256, ..full });
    measure("W1,W2 scale 1024", Cfg { w_scale: 1024, ..full });
    measure("y finer x16", Cfg { y_bits: 4, ..full });
    measure("u finer x16", Cfg { u_bits: 4, ..full });
    measure("u,y finer x16", Cfg { u_bits: 4, y_bits: 4, ..full });
    measure("W scale 256, u,y finer x16", Cfg { w_scale: 256, u_bits: 4, y_bits: 4, ..full });
}
