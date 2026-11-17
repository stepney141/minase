use super::*;
use Parameter as P;
use minase::eval::nnue::{Network, SimpleParameters};
use minase_core::Color;

const WIDTH: usize = 16;
fn scale() -> Scale {
    Scale::new(1072.65).unwrap()
}
fn close(a: f64, b: f64, tolerance: f64) {
    assert!(
        (a - b).abs() <= tolerance,
        "{a} != {b}, tolerance {tolerance}"
    );
}
fn active_model() -> Parameters {
    let mut w = Parameters::zeros(WIDTH).unwrap();
    w[P::FtBiases].fill(0.25);
    w[P::L1Weights].fill(0.015625);
    w[P::L1Biases].fill(0.125);
    w[P::L2Weights].fill(0.03125);
    w[P::L2Biases].fill(0.125);
    w[P::L3Weights].fill(0.0625);
    w[P::L3Bias][0] = -0.125;
    w
}
#[test]
fn hand_computed_forward_and_teacher_scale() {
    // a=1/4, x=1/4, y=32*(1/64)*(1/4)+1/8=1/4,
    // u=32*(1/32)*(1/4)+1/8=3/8, Y=32*(1/16)*(3/8)-1/8=5/8.
    let w = active_model();
    let f = Features::new([vec![], vec![]]).unwrap();
    let actual = forward(&w, &f).unwrap();
    assert!(actual.a.iter().flatten().all(|&v| v == 0.25));
    assert!(actual.x.iter().all(|&v| v == 0.25));
    assert_eq!(actual.y, [0.25; 32]);
    assert_eq!(actual.h1, [0.25; 32]);
    assert_eq!(actual.u, [0.375; 32]);
    assert_eq!(actual.h2, [0.375; 32]);
    assert_eq!(actual.output, 0.625);
    let probability = 1.0 / (1.0 + (-1016.0_f64 * 0.625 / 1072.65).exp());
    close(
        loss(&w, scale(), &[f], &[0.75]).unwrap(),
        (probability - 0.75).powi(2),
        1e-15,
    );
}

// Independent f64 version of the brief's equations. The perturbation is applied
// to one scalar without cloning the 964,224-row matrix or rounding it to f32.
fn precise_loss(
    w: &Parameters,
    features: &[Features],
    targets: &[f32],
    change: (P, usize, f64),
) -> f64 {
    let get = |p: P, i: usize| {
        f64::from(w[p][i])
            + if (p, i) == (change.0, change.1) {
                change.2
            } else {
                0.0
            }
    };
    let c = |x: f64| x.clamp(0.0, 1.0);
    features
        .iter()
        .zip(targets)
        .map(|(f, &target)| {
            let mut x = Vec::new();
            for view in f.views() {
                for j in 0..WIDTH {
                    x.push(c(get(P::FtBiases, j)
                        + view
                            .iter()
                            .map(|&r| {
                                get(P::FtWeights, r * WIDTH + j)
                                    + get(P::FtVirtual, (r % 13392) * WIDTH + j)
                            })
                            .sum::<f64>()));
                }
            }
            let h1: Vec<_> = (0..32)
                .map(|k| {
                    c(get(P::L1Biases, k)
                        + (0..2 * WIDTH)
                            .map(|j| get(P::L1Weights, k * 2 * WIDTH + j) * x[j])
                            .sum::<f64>())
                })
                .collect();
            let h2: Vec<_> = (0..32)
                .map(|k| {
                    c(get(P::L2Biases, k)
                        + (0..32)
                            .map(|j| get(P::L2Weights, k * 32 + j) * h1[j])
                            .sum::<f64>())
                })
                .collect();
            let y = get(P::L3Bias, 0) + (0..32).map(|k| get(P::L3Weights, k) * h2[k]).sum::<f64>();
            (1.0 / (1.0 + (-1016.0 * y / 1072.65).exp()) - f64::from(target)).powi(2)
        })
        .sum::<f64>()
        / targets.len() as f64
}
#[test]
fn all_parameter_arrays_have_correct_gradients_and_virtual_rows_sum_buckets() {
    let mut w = active_model();
    let features = [
        Features::new([vec![7, 13392 + 7], vec![7]]).unwrap(),
        Features::new([vec![2 * 13392 + 7], vec![13392 + 7]]).unwrap(),
    ];
    // All dense connections participate; FT rows in multiple buckets share virtual row 7.
    for p in [P::FtWeights, P::FtVirtual] {
        w[p][7 * WIDTH] = 0.015625;
    }
    let targets = [0.8, 0.1];
    let (_, gradient) = backward(&w, scale(), &features, &targets).unwrap();
    for p in P::ALL {
        let indices: Vec<_> = match p {
            P::FtWeights => [7, 13392 + 7, 2 * 13392 + 7, 42]
                .into_iter()
                .flat_map(|r| r * WIDTH..(r + 1) * WIDTH)
                .collect(),
            P::FtVirtual => [7, 42]
                .into_iter()
                .flat_map(|r| r * WIDTH..(r + 1) * WIDTH)
                .collect(),
            _ => (0..w[p].len()).collect(),
        };
        for i in indices {
            let epsilon = 1e-5;
            let numerical = (precise_loss(&w, &features, &targets, (p, i, epsilon))
                - precise_loss(&w, &features, &targets, (p, i, -epsilon)))
                / (2.0 * epsilon);
            close(
                f64::from(gradient[p][i]),
                numerical,
                2e-8 + 1e-5 * numerical.abs(),
            );
        }
    }
    for j in 0..WIDTH {
        let sum = (0..72)
            .map(|b| f64::from(gradient[P::FtWeights][(b * 13392 + 7) * WIDTH + j]))
            .sum::<f64>();
        close(f64::from(gradient[P::FtVirtual][7 * WIDTH + j]), sum, 1e-8);
    }
    assert!(
        gradient[P::FtWeights]
            .as_chunks::<WIDTH>()
            .0
            .iter()
            .enumerate()
            .filter(|(r, _)| ![7, 13392 + 7, 2 * 13392 + 7].contains(r))
            .all(|(_, row)| row.iter().all(|&v| v == 0.0))
    );
}
#[test]
fn activation_boundary_derivatives_are_zero_in_every_layer() {
    let mut w = active_model();
    let f = Features::new([vec![7], vec![]]).unwrap();
    for boundary in [0.0, 1.0] {
        w[P::FtBiases].fill(boundary);
        let (_, g) = backward(&w, scale(), std::slice::from_ref(&f), &[0.1]).unwrap();
        assert!(g[P::FtBiases].iter().all(|&v| v == 0.0));
        assert_eq!(g[P::FtWeights][7 * WIDTH], 0.0);
        assert_eq!(g[P::FtVirtual][7 * WIDTH], 0.0);
    }
    w[P::FtBiases].fill(0.25);
    w[P::L1Weights].fill(0.0);
    for boundary in [0.0, 1.0] {
        w[P::L1Biases].fill(boundary);
        let (_, g) = backward(&w, scale(), std::slice::from_ref(&f), &[0.1]).unwrap();
        assert!(g[P::L1Biases].iter().all(|&v| v == 0.0));
    }
    w[P::L2Weights].fill(0.0);
    for boundary in [0.0, 1.0] {
        w[P::L2Biases].fill(boundary);
        let (_, g) = backward(&w, scale(), std::slice::from_ref(&f), &[0.1]).unwrap();
        assert!(g[P::L2Biases].iter().all(|&v| v == 0.0));
    }
}
#[test]
fn adam_two_steps_groups_and_clipping() {
    let mut w = Parameters::zeros(WIDTH).unwrap();
    let mut g = Parameters::zeros(WIDTH).unwrap();
    for p in P::ALL {
        w[p][0] = 0.25;
        g[p][0] = 0.5;
    }
    let config = OptimizerConfig {
        ft: crate::nn::AdamWGroup {
            lr: 0.01,
            weight_decay: 0.1,
        },
        head: crate::nn::AdamWGroup {
            lr: 0.02,
            weight_decay: 0.2,
        },
    };
    let mut adam = AdamW::new(config, WIDTH).unwrap();
    for step in 1..=2 {
        adam.update(&mut w, &g, 1.0).unwrap();
        for p in P::ALL {
            let (lr, decay) = if matches!(p, P::FtWeights | P::FtVirtual) {
                (0.01, 0.1)
            } else {
                (0.02, 0.2)
            };
            let mut expected = 0.25;
            for _ in 0..step {
                expected = expected * (1.0 - lr * decay) - lr * 0.5 / (0.5 + 1e-8);
            }
            close(f64::from(w[p][0]), expected, 2e-7);
            close(
                f64::from(adam.m[p][0]),
                0.5 * (1.0 - 0.9_f64.powi(step)),
                1e-7,
            );
            close(
                f64::from(adam.v[p][0]),
                0.25 * (1.0 - 0.999_f64.powi(step)),
                1e-7,
            );
        }
    }
    for p in P::ALL {
        w[p][0] = 10.0;
        w[p][1.min(g[p].len() - 1)] = -10.0;
        g[p].fill(0.0);
    }
    let mut adam = AdamW::new(OptimizerConfig::default(), WIDTH).unwrap();
    adam.update(&mut w, &g, 1.0).unwrap();
    for (p, limit) in [
        (P::FtWeights, 112.0 / 127.0),
        (P::FtVirtual, 112.0 / 127.0),
        (P::FtBiases, 224.0 / 127.0),
        (P::L1Weights, 127.0 / 64.0),
        (P::L1Biases, 127.0 / 64.0),
        (P::L2Weights, 127.0 / 64.0),
        (P::L2Biases, 127.0 / 64.0),
        (P::L3Weights, 127.0 / 64.0),
    ] {
        assert_eq!(w[p][0], limit);
        assert_eq!(w[p][1], -limit);
    }
    assert_eq!(w[P::L3Bias][0], -10.0);
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
#[test]
fn checkpoint_preserves_next_update_bits_and_checks_settings_and_arrays() {
    let mut cp = Checkpoint {
        parameters: active_model(),
        optimizer: AdamW::new(OptimizerConfig::default(), WIDTH).unwrap(),
        settings: Settings {
            width: WIDTH,
            scale: scale(),
            rules: minase_core::Rules::JCSA,
            lambda: 0.7,
            seed: 71,
            optimizer: OptimizerConfig::default(),
        },
        progress: progress(),
    };
    let f = [Features::new([vec![7], vec![13392 + 7]]).unwrap()];
    let (_, g) = backward(&cp.parameters, scale(), &f, &[0.2]).unwrap();
    cp.optimizer.update(&mut cp.parameters, &g, 1.0).unwrap();
    cp.progress.completed_steps = 1;
    let bytes = cp.encode().unwrap();
    let mut restored = Checkpoint::decode(&bytes).unwrap();
    assert_eq!(cp, restored);
    for change in 0..7 {
        let mut settings = cp.settings.clone();
        match change {
            0 => settings.width = 32,
            1 => settings.scale = Scale::new(679.03).unwrap(),
            2 => settings.lambda = 0.6,
            3 => settings.seed += 1,
            4 => settings.rules = minase_core::Rules::LISHOGI,
            5 => settings.optimizer.ft.lr = 0.01,
            _ => settings.optimizer.head.weight_decay = 0.1,
        };
        assert!(matches!(
            restored.check_settings(&settings),
            Err(Error::SettingsMismatch)
        ));
    }
    restored.check_settings(&cp.settings).unwrap(); // Execution budget is absent from Settings.
    let (_, g) = backward(&cp.parameters, scale(), &f, &[0.9]).unwrap();
    cp.optimizer.update(&mut cp.parameters, &g, 0.5).unwrap();
    restored
        .optimizer
        .update(&mut restored.parameters, &g, 0.5)
        .unwrap();
    for p in P::ALL {
        for (a, b) in [
            (&cp.parameters, &restored.parameters),
            (&cp.optimizer.m, &restored.optimizer.m),
            (&cp.optimizer.v, &restored.optimizer.v),
        ] {
            assert!(
                a[p].iter()
                    .zip(&b[p])
                    .all(|(x, y)| x.to_bits() == y.to_bits())
            );
        }
    }
    // Rebuild NPZ entries through the public container codec to preserve ZIP checksums.
    let entries = crate::nn::npz::decode(&bytes).unwrap();
    for kind in 0..4 {
        let mut changed = entries.clone();
        match kind {
            0 => {
                changed.remove("weights.ft_virtual");
            }
            1 => {
                changed.insert("unexpected".into(), changed["width"].clone());
            }
            2 => {
                changed.insert(
                    "weights.l3_bias".into(),
                    crate::nn::npz::Array {
                        dtype: "<f8".into(),
                        shape: vec![1],
                        data: vec![0; 8],
                    },
                );
            }
            _ => {
                changed.insert(
                    "weights.l3_bias".into(),
                    crate::nn::npz::Array {
                        dtype: "<f4".into(),
                        shape: vec![1, 1],
                        data: vec![0; 4],
                    },
                );
            }
        }
        assert!(
            Checkpoint::decode(
                &crate::nn::npz::encode(
                    &changed
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.clone()))
                        .collect::<Vec<_>>()
                )
                .unwrap()
            )
            .is_err()
        );
    }
}

/// Independent integer equations from the brief; does not call the engine's affine helpers.
fn integer_reference(p: &SimpleParameters, features: &Features) -> i32 {
    let h = p.width;
    let mut x = Vec::new();
    for rows in features.views() {
        for j in 0..h {
            let sum = i64::from(p.ft_biases[j])
                + rows
                    .iter()
                    .map(|r| i64::from(p.ft_weights[r * h + j]))
                    .sum::<i64>();
            x.push(sum.clamp(0, 127));
        }
    }
    let layer = |weights: &[i8], biases: &[i32], x: &[i64]| -> Vec<i64> {
        biases
            .iter()
            .enumerate()
            .map(|(k, &b)| {
                ((i64::from(b)
                    + x.iter()
                        .enumerate()
                        .map(|(j, &value)| value * i64::from(weights[k * x.len() + j]))
                        .sum::<i64>())
                    >> 6)
                    .clamp(0, 127)
            })
            .collect()
    };
    let h1 = layer(&p.l1_weights, &p.l1_biases, &x);
    let h2 = layer(&p.l2_weights, &p.l2_biases, &h1);
    ((i64::from(p.output_bias)
        + h2.iter()
            .zip(&p.output_weights)
            .map(|(&x, &w)| x * i64::from(w))
            .sum::<i64>())
        / i64::from(p.fv_scale))
    .clamp(-28999, 28999) as i32
}
#[test]
fn exported_network_matches_integer_equations_and_folds_unseen_buckets() {
    let mut w = Parameters::initialize(17, WIDTH).unwrap();
    w[P::FtBiases].fill(0.25);
    w[P::L1Biases].fill(0.25);
    w[P::L2Biases].fill(0.25);
    w[P::L3Weights].fill(0.2);
    w[P::FtVirtual].fill(0.003);
    w[P::FtWeights][(71 * 13392 + 7) * WIDTH] = 0.004;
    w[P::FtVirtual][7 * WIDTH] = 0.004;
    let q = quantize(&w, scale()).unwrap();
    assert_eq!(q.ft_weights[(71 * 13392 + 7) * WIDTH], 1); // 127*(.004+.004), rounded once.
    let net = Network::decode(&export(&w, scale()).unwrap()).unwrap();
    let mut positions = minase_core::test_util::bench_positions();
    positions.extend(minase_core::test_util::sampled_random_positions(
        minase_core::MoveRules::standard(),
    ));
    for position in &positions {
        for side in Color::ALL {
            match Features::from_position(position, side) {
                Ok(f) => assert_eq!(
                    net.evaluate_for(position, side).unwrap(),
                    integer_reference(&q, &f)
                ),
                Err(Error::Anchor(error)) => {
                    assert_eq!(net.evaluate_for(position, side).unwrap_err(), error)
                }
                Err(error) => panic!("{error}"),
            }
        }
    }
    let samples: Vec<_> = positions
        .iter()
        .filter(|p| Features::from_position(p, p.side_to_move()).is_ok())
        .map(|p| (p, p.side_to_move()))
        .collect();
    let report = quantization_report(&w, scale(), &samples).unwrap();
    assert!(report.mean_absolute_cp.is_finite());
    w[P::FtWeights][0] = 200.0;
    w[P::FtVirtual][0] = 200.0;
    assert!(matches!(
        quantize(&w, scale()),
        Err(Error::Quantization { .. })
    ));
    w[P::FtWeights][0] = 1.0;
    w[P::FtVirtual][0] = 1.0; // individually legal i16, but an unsafe accumulator must be rejected by export.
    w[P::FtWeights].fill(10.0);
    assert!(export(&w, scale()).is_err());
}
#[test]
fn initialization_consumes_no_randomness_for_virtual_rows() {
    let w = Parameters::initialize(123, WIDTH).unwrap();
    assert!(w[P::FtVirtual].iter().all(|v| v.to_bits() == 0));
    let mut state = 123_u64;
    let mut off = Parameters::zeros(WIDTH).unwrap();
    for p in [
        P::FtWeights,
        P::FtBiases,
        P::L1Weights,
        P::L1Biases,
        P::L2Weights,
        P::L2Biases,
        P::L3Weights,
        P::L3Bias,
    ] {
        for v in &mut off[p] {
            let bits = minase_core::rng::splitmix64(state);
            state = state.wrapping_add(0x9e3779b97f4a7c15);
            *v = (((bits >> 11) as f64 / 9007199254740992.0 * 2.0 - 1.0) * 0.01) as f32;
        }
        assert_eq!(w[p], off[p]);
        assert!(w[p].iter().all(|v| v.abs() <= 0.01));
    }
    assert_eq!(export(&w, scale()).unwrap(), export(&off, scale()).unwrap());
}

#[test]
fn cpu_loop_records_three_files_exports_and_resumes_with_larger_budget() {
    use clap::Parser;
    use minase::training::records::{
        Entry, Exclusion, GameRecord, Header, Origin, Outcome, Policy, TeacherKind, Value, Writer,
    };
    #[derive(Parser)]
    struct Parsed {
        #[command(flatten)]
        args: crate::nnue::train::Args,
    }
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let directory =
        Directory(std::env::temp_dir().join(format!("simple-cpu-loop-{}", std::process::id())));
    std::fs::create_dir(&directory.0).unwrap();
    let header = Header::new(
        "JCSA".into(),
        "1".repeat(40),
        [2; 32],
        TeacherKind::AlphaBeta,
        200,
        11,
    )
    .unwrap();
    let path = directory.0.join("data.mnsd");
    let mut writer = Writer::new(std::fs::File::create(&path).unwrap(), header).unwrap();
    for game in 0..240 {
        let record = GameRecord::new(
            game,
            Origin::RandomSelfplay,
            Outcome::Game(minase_core::GameResult::Win {
                winner: Color::Black,
                reason: minase_core::WinReason::Resignation,
            }),
            0,
            minase_core::Position::initial(),
            vec![
                Entry::new(
                    None,
                    Value::AlphaBeta(120),
                    Exclusion::None,
                    Policy::None,
                    None,
                )
                .unwrap(),
            ],
        )
        .unwrap();
        writer.write_game(&record).unwrap();
    }
    writer.finish().unwrap();
    let mut args = Parsed::parse_from([
        "train",
        "--data",
        "unused",
        "--output-dir",
        "unused",
        "--rules",
        "JCSA",
        "--teacher-cp-per-logit",
        "1072.65",
        "--lambda",
        "0.75",
        "--superbatches",
        "2",
        "--batches-per-superbatch",
        "2",
        "--validation-superbatches",
        "1",
        "--lr-schedule",
        "cosine:3:0.027",
        "--device",
        "cpu",
        "--batch",
        "17",
        "--threads",
        "1",
        "--validation-sample",
        "3",
    ])
    .args;
    args.data = vec![path];
    args.output_dir = directory.0.join("run");
    let summary = trainer::run_width(args.clone(), WIDTH).unwrap();
    assert_eq!(summary.completed_steps, 4);
    args.resume = Some(args.output_dir.clone());
    args.superbatches = 3.try_into().unwrap();
    let summary = trainer::run_width(args.clone(), WIDTH).unwrap();
    assert_eq!(summary.completed_steps, 6);
    for name in ["training.jsonl", "summary.json", "config.json", "nnue.bin"] {
        assert!(args.output_dir.join(name).is_file());
    }
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(args.output_dir.join("config.json")).unwrap())
            .unwrap();
    assert_eq!(config["architecture"]["kind"], "Simple");
    assert_eq!(config["architecture"]["feature_set"], "HalfKPVm");
    assert_eq!(config["architecture"]["H"], 16);
    let records = std::fs::read_to_string(args.output_dir.join("training.jsonl")).unwrap();
    assert_eq!(records.lines().count(), 4);
    let network =
        Network::decode(&std::fs::read(args.output_dir.join("nnue.bin")).unwrap()).unwrap();
    assert!(
        network
            .evaluate_for(&minase_core::Position::initial(), Color::Black)
            .is_ok()
    );
}
