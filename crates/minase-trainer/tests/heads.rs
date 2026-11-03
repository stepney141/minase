//! 設計書nnue-architecture-revision.mdの6段階の整数式と学習の境界規約を検査する。
mod common;
use common::*;
use minase::eval::nnue::{
    self, Network,
    feature_set::{FeatureSet, HalfKpHmB8, P},
    head::PairwiseL16L32,
};
use minase_core::{Color, Position};
use minase_trainer::nnue::{
    data::{BatchSlot, InputFeatureSet, SENTINEL, decode_p_feature},
    model::{HeadKind, Model, Parameter, Shape, quantize},
    reference::{self, Forward},
};

// MNUE版5のバイト配置を直接読む。推論側の復号・累算器・活性化・出力関数を呼ばない。
fn integer_reference(bytes: &[u8], set: InputFeatureSet, batch: &BatchSlot, b: usize) -> i32 {
    let i16_at = |offset| i16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap()) as i64;
    let i32_at = |offset| i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as i64;
    assert_eq!(&bytes[..8], b"MNUE\x05\0\0\0");
    let h = u32::from_le_bytes(bytes[104..108].try_into().unwrap()) as usize;
    let cp_per_logit = f32::from_le_bytes(bytes[108..112].try_into().unwrap());
    let first = 192 + 13680 * 4 + 47 * 4;
    let bias = first + set.dimensions() * h * 2;
    let head = bias + h * 4;
    let mut linear = [0_i64; 2];
    for &f in &batch.linear_features()[b * 145..(b + 1) * 145] {
        if f != SENTINEL {
            let f = f as usize;
            let base = f / 72 * 144 + f % 72 / 6 * 12 + f % 6;
            for (e, value) in linear.iter_mut().enumerate() {
                *value += i16_at(192 + base * 4 + e * 2);
            }
        }
    }
    let mut s = Vec::with_capacity(h);
    for view in 0..2 {
        let mut c: Vec<_> = (0..h).map(|j| i32_at(bias + j * 4)).collect();
        for &f in &batch.features()[(b * 2 + view) * 145..(b * 2 + view + 1) * 145] {
            if f == SENTINEL {
                continue;
            }
            let row = match set {
                InputFeatureSet::P => decode_p_feature(f).unwrap() as usize,
                InputFeatureSet::HalfKpHmB8 => f as usize,
            };
            for (j, value) in c.iter_mut().enumerate() {
                *value += i16_at(first + (row * h + j) * 2);
            }
        }
        // 1. 各視点の4区画A,B,A^m,B^mから2組の要素積を作る。
        for half in [0, h / 2] {
            for j in 0..h / 4 {
                s.push((c[half + j].clamp(0, 255) * c[half + h / 4 + j].clamp(0, 255)) >> 9);
            }
        }
    }
    let w1 = head + 2 * h + 4;
    let b1 = w1 + 15 * h;
    let w2 = b1 + 15 * 4;
    let b2 = w2 + 32 * 30;
    let v = b2 + 32 * 4;
    assert_eq!(bytes.len(), v + 32 * 2);
    let mut u = [0_i64; 30];
    for k in 0..15 {
        // 2. W1の積和とb1。
        let z = s
            .iter()
            .enumerate()
            .fold(i32_at(b1 + k * 4), |sum, (i, &x)| {
                sum + (bytes[w1 + k * h + i] as i8 as i64) * x
            });
        // 3. 負のzも2乗し、2枝を連結する。
        u[k] = ((z * z) >> 19).min(127);
        u[15 + k] = (z >> 6).clamp(0, 127);
    }
    let y: [i64; 32] = std::array::from_fn(|k| {
        // 4. W2の積和、バイアス、算術シフト、切り詰め。
        let a = u
            .iter()
            .enumerate()
            .fold(i32_at(b2 + k * 4), |sum, (i, &x)| {
                sum + (bytes[w2 + k * 30 + i] as i8 as i64) * x
            });
        (a >> 6).clamp(0, 127)
    });
    // 5. 直接内積、幅32の内積、b。
    let out = s
        .iter()
        .enumerate()
        .map(|(i, &x)| i16_at(head + i * 2) * x)
        .sum::<i64>()
        + y.iter()
            .enumerate()
            .map(|(k, &x)| i16_at(v + k * 2) * x)
            .sum::<i64>()
        + i32_at(head + h * 2);
    // 6. cpへ換算し、線形経路を加える。
    let scale = (f64::from(cp_per_logit) * 65536.0 / (127.0 * 4096.0)).round_ties_even() as i64;
    let phase = (i64::from(batch.piece_counts()[b]) - 2).clamp(0, 90);
    let cp = (out * scale + 32768) >> 16;
    (cp + (phase * linear[0] + (90 - phase) * linear[1]) / 720).clamp(-28999, 28999) as i32
}

fn assert_network<F: FeatureSet>(bytes: &[u8], set: InputFeatureSet, positions: &[Position]) {
    let network = Network::<F, PairwiseL16L32>::decode(bytes).unwrap();
    for side in Color::ALL {
        let batch = BatchSlot::from_evaluation_positions(set, positions, side).unwrap();
        for (b, position) in positions.iter().enumerate() {
            let expected = integer_reference(bytes, set, &batch, b);
            assert_eq!(
                expected,
                nnue::evaluate_for_diagnostics(&network, position, side),
                "{set:?} {side:?} {b}"
            );
            if side == position.side_to_move() {
                assert_eq!(expected, nnue::evaluate(&network, position));
            }
        }
    }
}

#[test]
fn cpu_training_exports_a_network_matching_the_independent_integer_reference() {
    use clap::Parser;
    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        args: minase_trainer::nnue::train::Args,
    }
    let fixture = Fixture::new(&positions());
    let input = fixture.root.join("training.bin");
    std::fs::write(&input, common::training::records(false, true)).unwrap();
    let base = fixture.root.join("pst.bin");
    std::fs::write(&base, include_bytes!("../../minase/nets/pst.bin")).unwrap();
    let dataset = minase_trainer::data::Dataset::open(
        std::slice::from_ref(&input),
        None,
        std::num::NonZeroUsize::new(1).unwrap(),
    )
    .unwrap();
    let positions: Vec<_> = (0..dataset.record_count())
        .map(|i| dataset.record(i).unwrap().to_position().unwrap())
        .collect();
    for (set, name) in [
        (InputFeatureSet::P, "P"),
        (InputFeatureSet::HalfKpHmB8, "HalfKP-hm-b8"),
    ] {
        let out = fixture.root.join(name);
        let args = Cli::parse_from([
            "test",
            "--device",
            "cpu",
            "--feature-set",
            name,
            "--width",
            "8",
            "--head",
            "pairwise-l16-l32",
            "--data",
            input.to_str().unwrap(),
            "--base",
            base.to_str().unwrap(),
            "--output-dir",
            out.to_str().unwrap(),
            "--teacher-cp-per-logit",
            "calibrate",
            "--lambda",
            "1",
            "--epochs",
            "2",
            "--batch",
            "32",
            "--threads",
            "1",
            "--validation-sample",
            "9",
        ])
        .args;
        minase_trainer::nnue::train::run(args).unwrap();
        let bytes = std::fs::read(out.join("nnue.bin")).unwrap();
        match set {
            InputFeatureSet::P => assert_network::<P>(&bytes, set, &positions),
            InputFeatureSet::HalfKpHmB8 => assert_network::<HalfKpHmB8>(&bytes, set, &positions),
        }
        for row in common::training::logs(&out) {
            for field in [
                "saturation",
                "hidden1_saturation",
                "hidden2_saturation",
                "pairwise_both_positive",
                "first_layer_at_limit",
            ] {
                assert!(!row[field].is_null(), "{field}");
            }
        }
    }
}

#[test]
fn integer_reference_covers_nonzero_weights_negative_z_and_saturation() {
    let mut positions = positions();
    positions.push(Position::initial());
    positions.extend(positions.clone().iter().map(mirror));
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let mut model = model(set, 8);
        for p in Parameter::ALL {
            for (i, value) in model.parameter_mut(p).iter_mut().enumerate() {
                *value = ((i * 37 % 101) as f32 - 50.0)
                    / if p == Parameter::Linear { 1.0 } else { 100.0 };
            }
        }
        model.parameter_mut(Parameter::Hidden1Biases)[..5]
            .copy_from_slice(&[-1.5, -1.0, 0.0, 1.0, 1.5]);
        model.constrain().unwrap();
        let bytes = quantize(&model)
            .unwrap()
            .encode(&reference::v0_piece_values(), "L0,P0,R1,E0")
            .unwrap();
        match set {
            InputFeatureSet::P => assert_network::<P>(&bytes, set, &positions),
            InputFeatureSet::HalfKpHmB8 => assert_network::<HalfKpHmB8>(&bytes, set, &positions),
        }
    }
}

fn zero_model(set: InputFeatureSet, width: usize) -> Model {
    let mut model = model(set, width);
    for p in Parameter::ALL {
        model.parameter_mut(p).fill(0.0);
    }
    model
}

#[test]
fn real_evaluation_uses_pairwise_inputs_and_both_hidden_layers_without_integer_rounding() {
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let batch = BatchSlot::from_evaluation_positions(set, &positions(), Color::Black).unwrap();
        let mut model = zero_model(set, 4);
        model.parameter_mut(Parameter::Biases).fill(0.5);
        model.parameter_mut(Parameter::OutputWeights).fill(0.25);
        model.parameter_mut(Parameter::OutputBias)[0] = 0.0625;
        let inputs = model.shape().len(Parameter::OutputWeights);
        model.parameter_mut(Parameter::Hidden1Weights)[..inputs].fill(0.25);
        model.parameter_mut(Parameter::Hidden1Biases)[0] = -0.75;
        model.parameter_mut(Parameter::Hidden2Weights)[0] = 0.5;
        model.parameter_mut(Parameter::Hidden2Weights)[15] = 0.25;
        model.parameter_mut(Parameter::Hidden2Biases)[0] = 0.125;
        model.parameter_mut(Parameter::Hidden2Output)[0] = 0.5;
        // 各s=1/4、Σs=1よりz0=−1/2。u0=127/512、u15=0、y0=255/1024。
        // out=1/4+(1/2)(255/1024)+1/16=895/2048。整数推論の丸めは挟まない。
        for cp in reference::evaluate_f64(&model, &batch).unwrap() {
            assert_eq!(cp, 1000.0 * 895.0 / 2048.0);
        }
    }
}

#[test]
fn pairwise_backward_includes_both_factors_and_clipping_endpoints() {
    // H=8なので隣の因子は2列先。各因子と鏡映の対の値を区別する。
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let batch =
            BatchSlot::from_evaluation_positions(set, &positions()[..1], Color::Black).unwrap();
        let mut model = zero_model(set, 8);
        model.parameter_mut(Parameter::OutputWeights).fill(1.0);
        let acc = vec![0.0, 1.0, 0.25, 0.5, -0.25, 1.25, 0.75, 0.125];
        let f = Forward {
            acc: [acc.clone(), acc].concat(),
            lin: vec![[0.0; 2]],
        };
        let output = reference::output_backward(&model, &batch, &f, &[0.0]).unwrap();
        for view in 0..2 {
            // C'(x)×C(相手の因子)。境界0、1は微分1、範囲外は0。
            for (j, derivative) in [0.25, 0.5, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
                .into_iter()
                .enumerate()
            {
                assert_eq!(
                    output.d_acc[view * 8 + j],
                    (f64::from(output.g[0]) * derivative) as f32,
                    "{set:?} {j}"
                );
            }
        }
    }
}

#[test]
fn dual_activation_and_second_layer_backward_obey_boundary_convention() {
    let batch =
        BatchSlot::from_evaluation_positions(InputFeatureSet::P, &positions()[..1], Color::Black)
            .unwrap();
    let mut model = zero_model(InputFeatureSet::P, 4);
    model.parameter_mut(Parameter::Hidden2Output)[0] = 1.0;
    // W2の平方枝とclipped ReLU枝を別々に試し、aは境界から離す。
    for (z, square_d, clipped_d) in [
        (-1.01, 0.0, 0.0),
        (-1.0, -1.984375, 0.0),
        (-0.5, -0.9921875, 0.0),
        (0.0, 0.0, 1.0),
        (0.5, 0.9921875, 1.0),
        (1.0, 1.984375, 1.0),
        (1.01, 0.0, 0.0),
    ] {
        model.parameter_mut(Parameter::Hidden1Biases)[0] = z;
        for (branch, derivative) in [(0, square_d), (15, clipped_d)] {
            model.parameter_mut(Parameter::Hidden2Weights).fill(0.0);
            model.parameter_mut(Parameter::Hidden2Weights)[branch] = 0.25;
            model.parameter_mut(Parameter::Hidden2Biases)[0] = 0.25;
            let f = Forward {
                acc: vec![0.5; 8],
                lin: vec![[0.0; 2]],
            };
            let out = reference::output_backward(&model, &batch, &f, &[0.0]).unwrap();
            assert_eq!(
                out.d_hidden1_biases[0],
                (f64::from(out.g[0]) * 0.25 * derivative) as f32,
                "z={z}, branch={branch}"
            );
        }
    }
    // sqrt(128/127)自体はf32で表せない。前進のf32判定が1に丸まる最近傍とその両隣を検査する。
    let edge = (128.0_f32 / 127.0).sqrt();
    let below = f32::from_bits(edge.to_bits() - 1);
    let above = f32::from_bits(edge.to_bits() + 1);
    for sign in [-1.0, 1.0] {
        assert_eq!(
            reference::dual_square_derivative(sign * below),
            1.984375 * f64::from(sign * below)
        );
        assert_eq!(
            reference::dual_square_derivative(sign * edge),
            1.984375 * f64::from(sign * edge)
        );
        assert_eq!(reference::dual_square_derivative(sign * above), 0.0);
    }
    model.parameter_mut(Parameter::Hidden2Weights).fill(0.0);
    for (a, derivative) in [(-0.01, 0.0), (0.0, 1.0), (1.0, 1.0), (1.01, 0.0)] {
        model.parameter_mut(Parameter::Hidden2Biases)[0] = a;
        let f = Forward {
            acc: vec![0.5; 8],
            lin: vec![[0.0; 2]],
        };
        let out = reference::output_backward(&model, &batch, &f, &[0.0]).unwrap();
        assert_eq!(out.d_hidden2_biases[0], out.g[0] * derivative);
    }
}

#[test]
fn validation_counts_both_layers_and_positive_pairs_without_folding() {
    let batch =
        BatchSlot::from_evaluation_positions(InputFeatureSet::P, &positions()[..1], Color::Black)
            .unwrap();
    let mut model = zero_model(InputFeatureSet::P, 8);
    for (k, z) in model
        .parameter_mut(Parameter::Hidden1Biases)
        .iter_mut()
        .enumerate()
    {
        *z = [-0.5, 0.0, 0.5, 1.0, 1.5][k % 5];
    }
    for (k, a) in model
        .parameter_mut(Parameter::Hidden2Biases)
        .iter_mut()
        .enumerate()
    {
        *a = [-0.5, 0.0, 0.5, 1.0][k % 4];
    }
    // 第1視点は4積中2積、第2視点は4積中1積の両因子が正。
    let f = Forward {
        acc: vec![
            0.5, 0.0, 0.25, 0.5, 2.0, -1.0, 0.5, 0.5, 0.5, 0.0, 0.25, 0.5, -1.0, 0.5, 0.5, -1.0,
        ],
        lin: vec![[0.0; 2]],
    };
    let v = reference::validation_forward(&model, &batch, &f, &[0.5]).unwrap();
    assert_eq!(v.hidden1_saturation(), [6.0 / 15.0, 6.0 / 15.0]);
    assert_eq!(v.hidden2_saturation(), [16.0 / 32.0, 8.0 / 32.0]);
    assert_eq!(v.pairwise_both_positive(), 3.0 / 8.0);
}

#[test]
fn initialization_and_sharing_match_the_specified_shapes() {
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let shape = Shape::new(set, 8, 1000.0, HeadKind::PairwiseL16L32).unwrap();
        let mut model =
            Model::initialize(shape, &vec![[0.0; 2]; 13680], all_observed(set), 71, 0.05).unwrap();
        for (p, bound) in [
            (Parameter::Hidden1Weights, 1.0 / 8.0_f32.sqrt()),
            (Parameter::Hidden1Biases, 1.0 / 8.0_f32.sqrt()),
            (Parameter::Hidden2Weights, 1.0 / 30.0_f32.sqrt()),
            (Parameter::Hidden2Biases, 1.0 / 30.0_f32.sqrt()),
        ] {
            assert!(model.parameters()[p].iter().all(|x| x.abs() <= bound));
            assert!(model.parameters()[p].iter().any(|&x| x != 0.0));
        }
        for p in [
            Parameter::OutputWeights,
            Parameter::OutputBias,
            Parameter::Hidden2Output,
        ] {
            assert!(model.parameters()[p].iter().all(|&x| x == 0.0));
        }
        if set == InputFeatureSet::P {
            for (i, value) in model
                .parameter_mut(Parameter::OutputWeights)
                .iter_mut()
                .enumerate()
            {
                *value = i as f32 / 16.0;
            }
            for values in [
                model.expanded_output_weights(),
                model.expanded_hidden1_weights(),
            ] {
                for view in values.as_chunks::<4>().0 {
                    assert_eq!(&view[..2], &view[2..]);
                }
            }
        }
    }
}
