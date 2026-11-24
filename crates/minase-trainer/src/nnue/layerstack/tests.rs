use super::model::{ALPHA, HALF, Uniform, canonical, unit};
use super::reference::{crelu_grad, forward_validated, square_grad};
use super::*;
use Parameter::*;
use minase::eval::features::{FEATURE_COUNT, mirror_feature};
use minase::eval::nnue::{LayerStackNetwork, LayerStackParameters, Network};
use minase_core::test_util::{bench_positions, position_from_codes, sampled_random_positions};
use minase_core::{
    Color, Position, Square,
    rules::{MoveRules, Rules},
};

fn scale() -> Scale {
    Scale::new(1072.65).unwrap()
}
fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "actual={actual:.15e}, expected={expected:.15e}, tolerance={tolerance}"
    );
}
fn settings() -> Settings {
    Settings {
        scale: scale(),
        rules: Rules::JCSA,
        lambda: 0.7,
        seed: 314,
        optimizer: OptimizerConfig::default(),
    }
}
fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

#[test]
fn scales_validate_teacher_and_rounded_divisor() {
    for kt in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(Scale::new(kt), Err(Error::TeacherScale(_))));
    }
    for kt in [
        f64::MIN_POSITIVE,
        8128.0 / (f64::from(i32::MAX) + 1.0),
        16257.0,
        f64::MAX,
    ] {
        assert!(matches!(Scale::new(kt), Err(Error::OutputScale(_))));
    }
    assert_eq!(scale().divisor(), 8);
    assert_eq!(scale().output(), 1016.0);
    assert_eq!(Scale::new(679.03).unwrap().divisor(), 12);
    assert_eq!(Scale::new(16256.0).unwrap().divisor(), 1);
    assert_eq!(
        Scale::new(8128.0 / f64::from(i32::MAX)).unwrap().divisor(),
        i32::MAX
    );
}

#[test]
fn canonical_mapping_and_rotation_match_every_a_feature() {
    use crate::nnue::layerstack::input::decode_a_feature;
    for f in 0..FEATURE_COUNT {
        assert_eq!(
            canonical(f),
            f / 144 * 72 + f % 144 / 12 * 6 + (f % 12).min(11 - f % 12)
        );
        let packed = 2 * canonical(f) + usize::from(f % 12 >= 6);
        assert_eq!(decode_a_feature(packed as u32).unwrap() as usize, f);
        assert_eq!(canonical(f), canonical(mirror_feature(f)));
        assert_ne!(f % 12 >= 6, mirror_feature(f) % 12 >= 6);
        for j in 0..H {
            assert_eq!(unit(f, j), if f % 12 >= 6 { (j + H / 2) % H } else { j });
        }
    }
}

#[test]
fn initialization_uses_input_fan_in_fixed_seed_and_signed_material() {
    let a = Parameters::initialize(31, scale());
    let b = Parameters::initialize(31, scale());
    assert_eq!(a, b);
    assert_ne!(a, Parameters::initialize(32, scale()));
    for p in Parameter::ALL {
        for &v in &a[p][..p.active_len()] {
            match p {
                FtWeights => assert!(v.abs() <= (1.0_f32 / 13680.0).sqrt()),
                L1Weights | L1SharedWeights | L2Weights | L3Weights => assert!(v.abs() <= 0.01),
                Psqt => (),
                _ => assert_eq!(v, 0.0),
            }
        }
    }
    // 王将、太子、先獅子は0。歩兵の値100は初期化の仕様の例として独立に固定する。
    for side in 0..2 {
        for state in [11, 21] {
            assert!(
                a[Psqt][(side * 47 + state) * 72..(side * 47 + state + 1) * 72]
                    .iter()
                    .all(|v| *v == 0.0)
            );
        }
        let sign = if side == 0 { 1.0 } else { -1.0 };
        assert!(
            a[Psqt][(side * 47 + 29) * 72..(side * 47 + 30) * 72]
                .iter()
                .all(|&v| v == (sign * 100.0 / 1016.0) as f32)
        );
    }
    assert!(a[Psqt][94 * 72..].iter().all(|v| *v == 0.0));
    a.validate().unwrap();
    // バイアス、PSQT、埋め草で乱数を消費しない宣言順も固定する。
    let mut rng = Uniform(31);
    for p in [FtWeights, L1Weights, L1SharedWeights, L2Weights, L3Weights] {
        let half = if p == FtWeights {
            (1.0_f64 / 13680.0).sqrt()
        } else {
            0.01
        };
        for &v in &a[p][..p.active_len()] {
            assert_eq!(v, (rng.next() * half) as f32);
        }
    }
}

#[test]
fn hand_calculated_forward_covers_negative_square_saturation_skip_and_psqt() {
    // p[0]=p[128]=127/512、y0=p0/2+1/8、y1=-1/2、y2=2、y15=p128/4+1/16。
    // z=(alpha*y0², alpha/4, 1, ..., y0, 0, 1, ...)。u0=z0/2+z1/4+z2/8+z15/4+1/8。
    let mut w = Parameters::zeros();
    w[FtBiases][0] = 0.5;
    w[L1Weights][0] = 0.25;
    w[L1SharedWeights][0] = 0.25;
    w[L1Biases][0] = 0.0625;
    w[L1SharedBiases][0] = 0.0625;
    w[L1Biases][1] = -0.5;
    w[L1Biases][2] = 2.0;
    w[L1Weights][15 * H + 128] = 0.25;
    w[L1Biases][15] = 0.0625;
    w[L2Weights][0] = 0.5;
    w[L2Weights][1] = 0.25;
    w[L2Weights][2] = 0.125;
    w[L2Weights][15] = 0.25;
    w[L2Biases][0] = 0.125;
    w[L2Biases][1] = 1.5;
    w[L2Biases][2] = -0.25;
    w[L3Weights][0] = 0.5;
    w[L3Weights][1] = 0.25;
    w[L3Weights][2] = 0.75;
    w[L3Bias][0] = -0.125;
    w[Psqt][0] = 0.25;
    w[Psqt][72] = -0.5;
    let f = Features::new([vec![0], vec![144]]).unwrap();
    let out = forward(&w, &f).unwrap();
    let p = 127.0 / 512.0;
    let y = p / 2.0 + 0.125;
    let z = 127.0 / 128.0 * y * y;
    // 各層の出力を単精度へ丸める。比較は引き続き完全一致を要求する。
    let u = f64::from((z / 2.0 + (127.0 / 512.0) / 4.0 + 0.125 + y / 4.0 + 0.125) as f32);
    let n = f64::from((u / 2.0 + 0.25 - 0.125 + p / 4.0 + 0.0625) as f32);
    let output = f64::from((n + 0.375) as f32);
    for (j, &value) in out.p.iter().enumerate() {
        assert_eq!(value, if j == 0 || j == 128 { p } else { 0.0 });
    }
    assert_eq!(out.y[0], y);
    assert_eq!(out.y[1], -0.5);
    assert_eq!(out.y[2], 2.0);
    assert_eq!(out.y[15], p / 4.0 + 0.0625);
    assert_eq!(out.z[0], z);
    assert_eq!(out.z[1], 127.0 / 512.0);
    assert_eq!(out.z[2], 1.0);
    assert_eq!(out.z[15], y);
    assert_eq!(out.z[16], 0.0);
    assert_eq!(out.z[17], 1.0);
    assert_eq!(out.u[0], u);
    assert_eq!(out.n, n);
    assert_eq!(out.psqt, [0.25, -0.5]);
    assert_eq!(out.output, output);
    let probability = sigmoid(1016.0 * output / 1072.65);
    close(
        loss(&w, scale(), std::slice::from_ref(&f), &[0.25]).unwrap(),
        (probability - 0.25).powi(2),
        1e-15,
    );
    let (_, g) = backward(&w, scale(), &[f], &[0.25]).unwrap();
    let gy = 2.0 * (probability - 0.25) * probability * (1.0 - probability) * 1016.0 / 1072.65;
    close(f64::from(g[Psqt][0]), gy / 2.0, 1e-8);
    close(f64::from(g[Psqt][72]), -gy / 2.0, 1e-8);
    close(f64::from(g[L1Biases][15]), gy, 1e-8);
    assert_eq!(g[L1Weights], g[L1SharedWeights]);
    assert_eq!(g[L1Biases], g[L1SharedBiases]);
}

fn smooth_weights() -> Parameters {
    let mut w = Parameters::initialize(19, scale());
    w[Psqt].fill(0.0);
    w[FtBiases].fill(0.375);
    for p in [L1Weights, L1SharedWeights] {
        for v in &mut w[p] {
            *v = 0.0005 + v.abs() * 0.1;
        }
    }
    for (k, bias) in w[L1Biases][..15].iter_mut().enumerate() {
        *bias = if k % 3 == 0 { -0.375 } else { 0.25 };
    }
    w[L1Biases][15] = 0.125;
    w[L2Weights].fill(0.015625);
    w[L2Biases].fill(0.25);
    w[L3Weights].fill(0.03125);
    w
}

#[test]
fn finite_differences_cover_all_parameter_arrays_and_all_active_components() {
    let mut w = smooth_weights();
    // 同じ正準行を左右両側、両視点、複数局面から参照し、回転した勾配の和を検査する。
    let features = [
        Features::new([vec![0, 11, 144], vec![12, 155]]).unwrap(),
        Features::new([vec![11, 12], vec![144, 0]]).unwrap(),
    ];
    let targets = [0.125, 0.25];
    let (_, g) = backward(&w, scale(), &features, &targets).unwrap();
    let active = [0, 6, 72];
    for p in Parameter::ALL {
        for i in 0..p.count() {
            let used = match p {
                FtWeights => active.contains(&(i / H)),
                Psqt => active.contains(&i),
                _ => true,
            };
            if !used {
                assert_eq!(g[p][i], 0.0, "{p:?}[{i}]");
                continue;
            }
            let old = w[p][i];
            let lo = old - 0.001;
            let hi = old + 0.001;
            let objective = |weights: &Parameters| -> f64 {
                features
                    .iter()
                    .zip(targets)
                    .map(|(f, t)| {
                        (sigmoid(1016.0 / 1072.65 * finite_difference_output(weights, f))
                            - f64::from(t))
                        .powi(2)
                    })
                    .sum::<f64>()
                    / 2.0
            };
            w[p][i] = hi;
            let plus = objective(&w);
            w[p][i] = lo;
            let minus = objective(&w);
            w[p][i] = old;
            let numerical = (plus - minus) / (f64::from(hi) - f64::from(lo));
            let analytic = f64::from(g[p][i]);
            let tolerance = 1e-3 * analytic.abs().max(numerical.abs()) + 1e-10;
            assert!(
                (numerical - analytic).abs() <= tolerance,
                "{p:?}[{i}] numerical={numerical}, analytic={analytic}"
            );
        }
    }
    assert_eq!(g[L1Weights], g[L1SharedWeights]);
    assert_eq!(g[L1Biases], g[L1SharedBiases]);
}

#[test]
fn activation_derivatives_are_zero_at_both_boundaries() {
    for x in [-1.0, 0.0, 1.0, 2.0] {
        assert_eq!(crelu_grad(x), 0.0);
    }
    assert_eq!(crelu_grad(0.5), 1.0);
    assert_eq!(square_grad(0.0), 0.0);
    let inside = (128.0_f32 / 127.0).sqrt();
    let edge = inside.next_up();
    // 単精度の平方根は非飽和側に丸まる。隣接する表現可能な値で両側を検査する。
    assert!(ALPHA as f32 * inside * inside < 1.0);
    assert_ne!(square_grad(inside), 0.0);
    assert_ne!(square_grad(-inside), 0.0);
    assert!(ALPHA as f32 * edge * edge >= 1.0);
    for x in [-edge, edge, -2.0, 2.0] {
        assert_eq!(square_grad(x), 0.0);
    }
    close(f64::from(square_grad(-0.5)), -127.0 / 128.0, 0.0);
    close(f64::from(square_grad(0.5)), 127.0 / 128.0, 0.0);
}

#[test]
fn adam_matches_two_hand_calculated_updates_and_both_groups() {
    use crate::nn::AdamWGroup;
    let config = OptimizerConfig {
        ft: AdamWGroup {
            lr: 0.002,
            weight_decay: 0.1,
        },
        head: AdamWGroup {
            lr: 0.004,
            weight_decay: 0.2,
        },
    };
    let mut opt = AdamW::new(config).unwrap();
    let mut w = Parameters::zeros();
    let mut g = Parameters::zeros();
    for p in Parameter::ALL {
        w[p][0] = 0.5;
        g[p][0] = 0.25;
    }
    let mut expected = [0.5_f32; 11];
    let mut em = 0.0_f32;
    let mut ev = 0.0_f32;
    for (step, grad) in [(1, 0.25_f32), (2, -0.125)] {
        for p in Parameter::ALL {
            g[p][0] = grad;
        }
        // β1=.9、β2=.999の定義式。f32モーメントをf64で補正する。
        em += (grad - em) * 0.1;
        ev = 0.999 * ev + 0.001 * (grad * grad);
        for p in Parameter::ALL {
            let group = if matches!(p, FtWeights | Psqt) {
                config.ft
            } else {
                config.head
            };
            let lr = group.lr * 0.5;
            expected[p as usize] *= (1.0 - lr * group.weight_decay) as f32;
            expected[p as usize] -= (lr * (f64::from(em) / (1.0 - 0.9_f64.powi(step)))
                / ((f64::from(ev) / (1.0 - 0.999_f64.powi(step))).sqrt() + 1e-8))
                as f32;
        }
        opt.update(&mut w, &g, 0.5).unwrap();
        for p in Parameter::ALL {
            assert_eq!(w[p][0].to_bits(), expected[p as usize].to_bits());
            assert_eq!(opt.m[p][0].to_bits(), em.to_bits());
            assert_eq!(opt.v[p][0].to_bits(), ev.to_bits());
        }
        assert_eq!(opt.step, step as u64);
    }
    for p in Parameter::ALL {
        assert!(w[p][p.active_len()..].iter().all(|v| v.to_bits() == 0));
        assert!(opt.m[p][p.active_len()..].iter().all(|v| v.to_bits() == 0));
        assert!(opt.v[p][p.active_len()..].iter().all(|v| v.to_bits() == 0));
    }
}

#[test]
fn adam_clips_only_the_required_arrays_and_rejects_bad_inputs_atomically() {
    let mut w = Parameters::zeros();
    let mut g = Parameters::zeros();
    let mut opt = AdamW::new(OptimizerConfig::default()).unwrap();
    for p in Parameter::ALL {
        let boundary = match p {
            FtWeights | FtBiases => 224.0 / 127.0,
            L1Weights | L1SharedWeights => 127.0 / 128.0,
            Psqt | L3Bias => 10.0,
            _ => 127.0 / 64.0,
        };
        w[p][0] = boundary;
        w[p][1.min(p.active_len() - 1)] = -boundary;
        if !matches!(p, Psqt | L3Bias) {
            // 境界からさらに外へ動かす勾配で、更新後の制限を検査する。
            g[p][0] = -1.0;
            g[p][1] = 1.0;
        }
    }
    opt.update(&mut w, &g, 1.0).unwrap();
    for p in Parameter::ALL {
        let specified_limit = match p {
            FtWeights | FtBiases => Some(224.0 / 127.0),
            L1Weights | L1SharedWeights => Some(127.0 / 128.0),
            Psqt | L3Bias => None,
            _ => Some(127.0 / 64.0),
        };
        if let Some(limit) = specified_limit {
            assert_eq!(w[p][0], limit);
            assert_eq!(w[p][1], -limit);
        } else {
            assert_eq!(w[p][p.active_len().min(2) - 1], -10.0);
        }
    }
    let original = w.clone();
    let state = opt.clone();
    for multiplier in [-1.0, 0.0, f64::NAN, f64::INFINITY] {
        assert!(opt.update(&mut w, &g, multiplier).is_err());
        assert_eq!(w, original);
        assert_eq!(opt, state);
    }
    g[L3Bias][0] = f32::MAX;
    assert!(opt.update(&mut w, &g, 1.0).is_err());
    assert_eq!(w, original);
    assert_eq!(opt, state);
    g[L3Bias][0] = 0.0;
    g[FtWeights][(ROWS - 1) * H] = 1.0;
    assert!(opt.update(&mut w, &g, 1.0).is_err());
    assert_eq!(w, original);
    assert_eq!(opt, state);
}

#[test]
fn adam_l1_weight_boundaries_allow_export_of_shared_sums() {
    let mut w = Parameters::zeros();
    let mut g = Parameters::zeros();
    for p in [L1Weights, L1SharedWeights] {
        for (i, gradient) in g[p].iter_mut().enumerate() {
            *gradient = if i % 2 == 0 { -1.0 } else { 1.0 };
        }
    }
    let mut opt = AdamW::new(OptimizerConfig {
        head: crate::nn::AdamWGroup {
            lr: 2.0,
            weight_decay: 0.0,
        },
        ..OptimizerConfig::default()
    })
    .unwrap();
    // 同符号の更新で両配列を正負の境界まで動かし、和の書き出しを検査する。
    opt.update(&mut w, &g, 1.0).unwrap();
    let bytes = export(&w, scale()).unwrap();
    Network::decode(&bytes).unwrap();
    let q = quantize(&w, scale()).unwrap();
    for (i, &value) in q.l1_weights.iter().enumerate() {
        let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
        for p in [L1Weights, L1SharedWeights] {
            assert_eq!(w[p][i], sign * (127.0 / 128.0));
        }
        assert_eq!(value, if i % 2 == 0 { 127 } else { -127 });
        assert!(i16::from(value).abs() <= 127);
    }
}

fn positions() -> Vec<Position> {
    bench_positions()
        .into_iter()
        .chain(sampled_random_positions(MoveRules::standard()))
        .collect()
}
fn mirror(position: &Position) -> Position {
    let square = |s: Square| Square::new(11 - s.file(), s.rank()).unwrap();
    let pieces = position
        .occupied()
        .into_iter()
        .map(|s| (square(s), position.piece_at(s).unwrap()))
        .collect::<Vec<_>>();
    let mut reflected = position_from_codes(position.side_to_move(), &pieces);
    reflected
        .set_lion_capture(position.lion_taken_by_non_lion().map(|t| square(t.square)))
        .unwrap();
    reflected
}
fn perturbed_weights() -> Parameters {
    let mut w = Parameters::initialize(2718, scale());
    let mut rng = Uniform(123);
    for p in Parameter::ALL {
        for v in &mut w[p][..p.active_len()] {
            *v += (rng.next() * if p == FtWeights { 0.005 } else { 0.1 }) as f32;
        }
    }
    for v in &mut w[FtBiases] {
        *v += 0.4;
    }
    w[L1Biases][0] = -0.5;
    w[L1Biases][1] = 1.5;
    w[L2Biases][0] = 1.5;
    w[L2Biases][1] = -0.5;
    w
}

#[test]
fn mirror_invariance_holds_for_bench_and_random_game_positions() {
    let w = perturbed_weights();
    for position in positions() {
        for side in Color::ALL {
            let a = forward_validated(&w, &Features::from_position(&position, side));
            let b = forward_validated(&w, &Features::from_position(&mirror(&position), side));
            close(a.output, b.output, 1e-6);
        }
    }
    LayerStackNetwork::new(quantize(&w, scale()).unwrap()).unwrap();
}

// tatara形式のLayerStackの整数式に基づく参照。エンジンの活性化、累算器、評価関数を呼ばない。
fn integer_reference(q: &LayerStackParameters, features: &Features) -> i32 {
    let mut input = [0_i64; 256];
    let mut sums = [0_i64; 2];
    for (v, rows) in features.views().iter().enumerate() {
        let mut a: [i64; 256] = std::array::from_fn(|j| i64::from(q.ft_biases[j]));
        for &f in rows {
            for (j, x) in a.iter_mut().enumerate() {
                *x += i64::from(q.ft_weights[f * 256 + j]);
            }
            sums[v] += i64::from(q.psqt_weights[f]);
        }
        for j in 0..128 {
            input[v * 128 + j] = (a[j].clamp(0, 127) * a[j + 128].clamp(0, 127)) >> 7;
        }
    }
    let y: [i64; 16] = std::array::from_fn(|k| {
        input
            .iter()
            .enumerate()
            .fold(i64::from(q.l1_biases[k]), |s, (j, x)| {
                s + i64::from(q.l1_weights[k * 256 + j]) * x
            })
    });
    let z: [i64; 30] = std::array::from_fn(|j| {
        if j < 15 {
            ((y[j] * y[j]) >> 19).clamp(0, 127)
        } else {
            (y[j - 15] >> 6).clamp(0, 127)
        }
    });
    let u: [i64; 32] = std::array::from_fn(|k| {
        let pre = z
            .iter()
            .enumerate()
            .fold(i64::from(q.l2_biases[k]), |s, (j, x)| {
                s + i64::from(q.l2_weights[k * 30 + j]) * x
            });
        (pre >> 6).clamp(0, 127)
    });
    let n = u
        .iter()
        .enumerate()
        .fold(i64::from(q.output_bias) + y[15], |s, (j, x)| {
            s + i64::from(q.output_weights[j]) * x
        });
    ((n + (sums[0] - sums[1]) / 2) / i64::from(q.fv_scale)).clamp(-28999, 28999) as i32
}

#[test]
fn exported_engine_matches_independent_integer_stages_on_both_perspectives() {
    let w = perturbed_weights();
    let q = quantize(&w, scale()).unwrap();
    let network = Network::decode(&export(&w, scale()).unwrap()).unwrap();
    for position in positions() {
        for side in Color::ALL {
            assert_eq!(
                network.evaluate_for(&position, side).unwrap(),
                integer_reference(&q, &Features::from_position(&position, side))
            );
        }
    }
}

#[test]
fn export_adds_shared_terms_before_rounding_and_checks_ranges() {
    let mut w = Parameters::zeros();
    w[L1Weights][0] = 0.375 / 64.0;
    w[L1SharedWeights][0] = 0.375 / 64.0;
    w[L1Biases][0] = 0.375 / 8128.0;
    w[L1SharedBiases][0] = 0.375 / 8128.0;
    w[FtWeights][0] = 0.75 / 127.0;
    w[FtWeights][128] = -0.75 / 127.0;
    w[FtBiases][0] = 0.5;
    w[FtBiases][1] = -0.5;
    let q = quantize(&w, scale()).unwrap();
    assert_eq!(q.ft_biases[0], 64);
    assert_eq!(q.ft_biases[1], -64);
    assert_eq!(q.ft_biases[128], 64);
    assert_eq!(q.ft_biases[129], -64);
    assert_eq!(q.l1_weights[0], 1);
    assert_eq!(q.l1_biases[0], 1);
    // 半幅回転とバイアス共有を展開後の整数で検査する。
    assert_eq!(q.ft_weights[0], 1);
    assert_eq!(q.ft_weights[128], -1);
    assert_eq!(q.ft_weights[11 * 256], -1);
    assert_eq!(q.ft_weights[11 * 256 + 128], 1);
    for (p, value, name) in [
        (FtWeights, 300.0, "ft_weights"),
        (L1Weights, 2.0, "l1_weights+l1_shared_weights"),
        (Psqt, 300000.0, "psqt"),
    ] {
        let mut invalid = w.clone();
        invalid[p][0] = value;
        match export(&invalid, scale()).unwrap_err() {
            Error::Quantization { name: n, index, .. } => {
                assert_eq!(n, name);
                assert_eq!(index, 0);
            }
            e => panic!("unexpected {e}"),
        }
    }
    // 型に収まっても、エンジンの累算値・PSQT・バイアス制約に反すれば拒否する。
    let mut invalid = Parameters::zeros();
    invalid[FtWeights][..(ROWS - 1) * H].fill(2.0);
    assert!(matches!(export(&invalid, scale()), Err(Error::Network(_))));
    let mut invalid = Parameters::zeros();
    invalid[Psqt][0] = 600.0;
    assert!(matches!(export(&invalid, scale()), Err(Error::Network(_))));
    let mut invalid = Parameters::zeros();
    invalid[L3Bias][0] = 10000.0;
    assert!(matches!(export(&invalid, scale()), Err(Error::Network(_))));
    // 所定の第1層の制限は、全升が最大値でもエンジンの検査を通る。
    let mut bounded = Parameters::zeros();
    bounded[FtWeights][..(ROWS - 1) * H].fill(224.0 / 127.0);
    bounded[FtBiases].fill(224.0 / 127.0);
    LayerStackNetwork::new(quantize(&bounded, scale()).unwrap()).unwrap();
}

#[test]
fn quantization_report_has_signed_absolute_maximum_errors_and_saturation() {
    // 後段のバイアス1/64のみ。8128/64=127、整数評価127/8=15、実数評価1016/64=15.875。
    let mut w = Parameters::zeros();
    w[L3Bias][0] = 1.0 / 64.0;
    let ps = bench_positions();
    let samples = ps
        .iter()
        .flat_map(|p| Color::ALL.map(|s| (p, s)))
        .collect::<Vec<_>>();
    let r = quantization_report(&w, scale(), &samples).unwrap();
    assert_eq!(r.samples, samples.len());
    assert_eq!(r.mean_signed_cp, -0.875);
    assert_eq!(r.mean_absolute_cp, 0.875);
    assert_eq!(r.max_absolute_cp, 0.875);
    assert_eq!(r.ft_saturation, 1.0);
    assert_eq!(r.l1_square_saturation, 1.0);
    assert_eq!(r.l1_linear_saturation, 1.0);
    assert_eq!(r.l2_saturation, 1.0);
    // それぞれの分母を独立に確かめる。1つの共有FTバイアスが両視点で4成分を非飽和にする。
    w[FtBiases][0] = 0.5;
    w[L1Biases][0] = 0.5;
    w[L1Biases][1] = -0.5;
    w[L2Biases][0] = 0.5;
    let r = quantization_report(&w, scale(), &samples).unwrap();
    assert_eq!(r.ft_saturation, 127.0 / 128.0);
    assert_eq!(r.l1_square_saturation, 13.0 / 15.0);
    assert_eq!(r.l1_linear_saturation, 14.0 / 15.0);
    assert_eq!(r.l2_saturation, 31.0 / 32.0);
    assert!(quantization_report(&w, scale(), &[]).is_err());
}

#[test]
fn checkpoint_resume_preserves_next_update_bits() {
    let settings = settings();
    let mut checkpoint = Checkpoint {
        progress: progress(),
        parameters: Parameters::initialize(settings.seed, settings.scale),
        optimizer: AdamW::new(settings.optimizer).unwrap(),
        settings,
    };
    let mut gradients = Parameters::zeros();
    for p in Parameter::ALL {
        for (i, g) in gradients[p][..p.active_len()].iter_mut().enumerate() {
            *g = ((i % 7) as f32 - 3.0) * 0.001;
        }
    }
    checkpoint
        .optimizer
        .update(&mut checkpoint.parameters, &gradients, 0.7)
        .unwrap();
    checkpoint.progress.completed_steps = checkpoint.optimizer.step;
    let mut bytes = Vec::new();
    checkpoint.encode(&mut bytes).unwrap();
    let mut resumed = Checkpoint::decode(&bytes).unwrap();
    assert_eq!(checkpoint, resumed);
    resumed.check_settings(&checkpoint.settings).unwrap();
    for c in [&mut checkpoint, &mut resumed] {
        c.optimizer
            .update(&mut c.parameters, &gradients, 0.4)
            .unwrap();
    }
    assert_eq!(checkpoint.optimizer.step, 2);
    for (a, b) in [
        (&checkpoint.parameters, &resumed.parameters),
        (&checkpoint.optimizer.m, &resumed.optimizer.m),
        (&checkpoint.optimizer.v, &resumed.optimizer.v),
    ] {
        for p in Parameter::ALL {
            for (x, y) in a[p].iter().zip(&b[p]) {
                assert_eq!(x.to_bits(), y.to_bits());
            }
        }
    }
    // 再開条件に予算や並列度を保存していないことを、配列とメタデータの双方で確かめる。
    checkpoint.progress.completed_steps = checkpoint.optimizer.step;
    bytes.clear();
    checkpoint.encode(&mut bytes).unwrap();
    let arrays = crate::nn::npz::decode(&bytes).unwrap();
    let meta: serde_json::Value = serde_json::from_slice(&arrays["meta"].data).unwrap();
    for key in ["total_superbatches", "time_limit", "threads"] {
        assert!(!arrays.contains_key(key));
        assert!(meta.get(key).is_none());
    }
    for change in 0..5 {
        let mut s = checkpoint.settings.clone();
        match change {
            0 => s.scale = Scale::new(679.03).unwrap(),
            1 => s.rules = Rules::LISHOGI,
            2 => s.lambda = 0.5,
            3 => s.seed += 1,
            _ => s.optimizer.head.lr *= 2.0,
        }
        assert!(matches!(
            resumed.check_settings(&s),
            Err(Error::SettingsMismatch)
        ));
    }
}

#[test]
fn checkpoint_rejects_missing_extra_wrong_shape_type_and_inconsistent_settings() {
    use crate::nn::npz;
    let settings = settings();
    let c = Checkpoint {
        progress: progress(),
        parameters: Parameters::zeros(),
        optimizer: AdamW::new(settings.optimizer).unwrap(),
        settings,
    };
    let mut bytes = Vec::new();
    c.encode(&mut bytes).unwrap();
    let arrays = npz::decode(&bytes).unwrap();
    for mutation in 0..12 {
        let mut changed = arrays.clone();
        match mutation {
            0 => {
                changed.remove("weights.l1_shared_weights");
            }
            1 => {
                changed.insert("extra".into(), changed["step"].clone());
            }
            2 => changed.get_mut("weights.ft_biases").unwrap().shape = vec![1, 128],
            3 => changed.get_mut("weights.ft_biases").unwrap().dtype = "<i4".into(),
            4 => changed.get_mut("width").unwrap().data = 128_u64.to_le_bytes().to_vec(),
            5 => changed.get_mut("fv_scale").unwrap().data = 7_u64.to_le_bytes().to_vec(),
            6 => changed.get_mut("lambda").unwrap().data = 1.5_f64.to_le_bytes().to_vec(),
            7 => changed.get_mut("teacher_scale").unwrap().data = f64::NAN.to_le_bytes().to_vec(),
            8 => changed.get_mut("v.l3_bias").unwrap().data = (-1.0_f32).to_le_bytes().to_vec(),
            9 => {
                let a = changed.get_mut("m.psqt").unwrap();
                let n = a.data.len();
                a.data[n - 4..].copy_from_slice(&1.0_f32.to_le_bytes());
            }
            10 | 11 => {
                let a = changed.get_mut("meta").unwrap();
                let mut json: serde_json::Value = serde_json::from_slice(&a.data).unwrap();
                if mutation == 10 {
                    json["architecture"] = "Simple".into();
                } else {
                    json["feature_set"] = "HalfKPVm".into();
                }
                a.data = serde_json::to_vec(&json).unwrap();
                a.shape = vec![a.data.len()];
            }
            _ => unreachable!(),
        }
        let values = changed.into_iter().collect::<Vec<_>>();
        let encoded = npz::encode(
            &values
                .iter()
                .map(|(k, v)| (k.as_str(), v.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(Checkpoint::decode(&encoded).is_err(), "mutation {mutation}");
    }
}

#[test]
fn bad_external_arguments_return_errors() {
    assert!(Features::new([vec![FEATURE_COUNT], vec![]]).is_err());
    assert!(Features::new([vec![0; 146], vec![]]).is_err());
    let f = Features::new([vec![], vec![]]).unwrap();
    let mut w = Parameters::zeros();
    for target in [-1.0, 1.1, f32::NAN, f32::INFINITY] {
        assert!(backward(&w, scale(), std::slice::from_ref(&f), &[target]).is_err());
    }
    assert!(loss(&w, scale(), &[], &[]).is_err());
    assert!(backward(&w, scale(), std::slice::from_ref(&f), &[]).is_err());
    w[L3Bias][0] = f32::NAN;
    assert!(forward(&w, &f).is_err());
    let mut s = settings();
    s.lambda = f64::NAN;
    assert!(s.validate().is_err());
    assert!(Checkpoint::decode(b"not a checkpoint").is_err());
}

#[test]
fn asymmetric_first_layer_rotates_and_sums_gradients_without_averaging() {
    // θ[0,0]=1/8、θ[0,128]=1/4、β0=1/4。
    // us={f0,Mf0}では両成分5/8、them={f0}では(3/8,1/2)。
    // y15=p_us/2+p_them/4だけを出力し、両視点の取り違えも検出する。
    let mut w = Parameters::zeros();
    w[FtWeights][0] = 0.125;
    w[FtWeights][128] = 0.25;
    w[FtBiases][0] = 0.25;
    w[L1Weights][15 * H] = 0.5;
    w[L1Weights][15 * H + 128] = 0.25;
    let features = Features::new([vec![0, 11], vec![0]]).unwrap();
    let f = forward(&w, &features).unwrap();
    assert_eq!(f.a[0][0], 0.625);
    assert_eq!(f.a[0][128], 0.625);
    assert_eq!(f.a[1][0], 0.375);
    assert_eq!(f.a[1][128], 0.5);
    let pus = 127.0 / 128.0 * 25.0 / 64.0;
    let pthem = 127.0 / 128.0 * 3.0 / 16.0;
    assert_eq!(f.p[0], pus);
    assert_eq!(f.p[128], pthem);
    assert_eq!(f.output, pus / 2.0 + pthem / 4.0);
    let sigma = sigmoid(1016.0 / 1072.65 * (pus / 2.0 + pthem / 4.0));
    let gy = 2.0 * (sigma - 0.25) * sigma * (1.0 - sigma) * 1016.0 / 1072.65;
    let (_, g) = backward(&w, scale(), &[features], &[0.25]).unwrap();
    // θ0はusの両回転とthemの第1成分から、θ128はthemの第2成分からも加算を受ける。
    close(
        f64::from(g[FtWeights][0]),
        gy * 127.0 / 128.0 * (0.625 + 0.125),
        1e-8,
    );
    close(
        f64::from(g[FtWeights][128]),
        gy * 127.0 / 128.0 * (0.625 + 0.09375),
        1e-8,
    );
    close(
        f64::from(g[FtBiases][0]),
        gy * 127.0 / 128.0 * (0.625 + 0.125 + 0.09375),
        1e-8,
    );
    close(f64::from(g[Psqt][0]), gy / 2.0, 1e-8);
}

#[test]
fn integer_psqt_halves_and_final_scores_truncate_negative_odd_values_toward_zero() {
    use minase_core::{PieceCode, PieceKind};
    let pawn = PieceCode::new(Color::Black, PieceKind::Pawn).unwrap();
    let position = position_from_codes(Color::Black, &[(Square::new(0, 0).unwrap(), pawn)]);
    for sign in [1.0_f32, -1.0] {
        let mut w = Parameters::zeros();
        // 自駒の歩兵、相対段0、筋0。相手駒側の行は0のままとする。
        w[Psqt][29 * 72] = sign * 17.0 / 8128.0;
        w[L3Bias][0] = 1.0 / 8128.0;
        let q = quantize(&w, scale()).unwrap();
        let net = Network::decode(&export(&w, scale()).unwrap()).unwrap();
        let expected = if sign > 0.0 { [1, 0] } else { [0, 1] };
        // (+17/2+1)/8=1、(-17/2+1)/8=0。各除算を0方向に丸める。
        for (side, expected) in Color::ALL.into_iter().zip(expected) {
            assert_eq!(
                integer_reference(&q, &Features::from_position(&position, side)),
                expected
            );
            assert_eq!(net.evaluate_for(&position, side).unwrap(), expected);
        }
    }
}

// 差分の差し引きで単精度の丸めが増幅しないよう、数式の目的関数だけを倍精度で評価する。
fn finite_difference_output(w: &Parameters, features: &Features) -> f64 {
    let c = |x: f64| x.clamp(0.0, 1.0);
    let mut a = [[0.0; H]; 2];
    let mut psqt = [0.0; 2];
    for (v, rows) in features.views().iter().enumerate() {
        for (j, value) in a[v].iter_mut().enumerate() {
            *value = f64::from(w[FtBiases][j % HALF]);
        }
        for &f in rows {
            let row = canonical(f);
            psqt[v] += f64::from(w[Psqt][row]);
            for (j, value) in a[v].iter_mut().enumerate() {
                *value += f64::from(w[FtWeights][row * H + unit(f, j)]);
            }
        }
    }
    let p: [f64; H] =
        std::array::from_fn(|j| ALPHA * c(a[j / HALF][j % HALF]) * c(a[j / HALF][j % HALF + HALF]));
    let y: [f64; 16] = std::array::from_fn(|k| {
        let bias = f64::from(w[L1Biases][k]) + f64::from(w[L1SharedBiases][k]);
        p.iter().enumerate().fold(bias, |s, (i, x)| {
            s + (f64::from(w[L1Weights][k * H + i]) + f64::from(w[L1SharedWeights][k * H + i])) * x
        })
    });
    let z: [f64; 30] = std::array::from_fn(|i| {
        if i < 15 {
            c(ALPHA * y[i] * y[i])
        } else {
            c(y[i - 15])
        }
    });
    let u: [f64; 32] = std::array::from_fn(|k| {
        z.iter()
            .enumerate()
            .fold(f64::from(w[L2Biases][k]), |s, (i, x)| {
                s + f64::from(w[L2Weights][k * 30 + i]) * x
            })
    });
    let n = u
        .iter()
        .enumerate()
        .fold(f64::from(w[L3Bias][0]) + y[15], |s, (i, x)| {
            s + f64::from(w[L3Weights][i]) * c(*x)
        });
    n + (psqt[0] - psqt[1]) / 2.0
}

#[test]
fn forward_uses_single_precision_and_the_gpu_reduction_tree() {
    let mut w = Parameters::zeros();
    w[FtBiases][0] = 0.5;
    w[FtWeights][0] = 16_777_216.0;
    w[FtWeights][72 * H] = -16_777_216.0;
    let f = Features::new([vec![0, 144], vec![]]).unwrap();
    // (0.5 + 2^24) - 2^24 is zero in f32 and 0.5 in f64.
    assert_eq!(forward(&w, &f).unwrap().a[0][0], 0.0);

    let mut w = Parameters::zeros();
    w[FtBiases].fill(0.5);
    w[L1Weights][15 * H] = 67_108_864.0;
    w[L1Weights][15 * H + 1] = -67_108_864.0;
    w[L1Weights][15 * H + 16] = 1.0;
    let empty = Features::new([vec![], vec![]]).unwrap();
    // p=127/512. The 16,8,4,2,1 tree first adds p to 127*2^17,
    // where f32 rounds it away, then cancels the two large terms.
    // An ascending sum or an f64 tree instead retains p.
    let result = forward(&w, &empty).unwrap();
    assert_eq!(result.y[15], 0.0);
    assert_eq!(result.output, 0.0);
}

#[test]
fn psqt_preserves_small_residuals_with_single_precision_compensation() {
    let mut w = Parameters::zeros();
    w[Psqt][0] = 67_108_864.0;
    w[Psqt][72] = 1.0;
    w[Psqt][144] = -67_108_864.0;
    let f = Features::new([vec![0, 144, 288], vec![]]).unwrap();
    let result = forward(&w, &f).unwrap();
    assert_eq!(result.psqt, [1.0, 0.0]);
    assert_eq!(result.output, 0.5);
}

fn progress() -> crate::nnue::train::Progress {
    crate::nnue::train::Progress {
        completed_superbatches: 0,
        completed_steps: 0,
        elapsed_seconds: 0.0,
        config: crate::nn::checkpoint::Configuration {
            settings: serde_json::json!({}),
            inputs: serde_json::json!({}),
            preprocessing: serde_json::json!({}),
        },
    }
}
