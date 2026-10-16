//! 設計書nnue-hidden-layer.mdの出力部と整数手順を独立に検査する。
mod common;
use common::*;
use minase::eval::nnue::{
    self, Network,
    feature_set::{FeatureSet, HalfKpHmB8, P},
    head::{DirectScrelu, DirectScreluPhase2, Head, ScreluHidden16, ScreluHidden16Phase2},
};
use minase_core::{Color, Position};
use minase_trainer::nnue::{
    data::{BatchSlot, InputFeatureSet, SENTINEL, decode_p_feature},
    model::{Error, HeadKind, Model, Parameter, Quantized, Shape, quantize},
    reference::{self, AdamW, AdamWConfig},
};

// 仕様の整数計算だけを使う。推論側の活性化、補間、累算器は呼ばない。
fn integer_hidden(a: &[i32], weights: &[i8], biases: &[i32]) -> Vec<i32> {
    weights
        .chunks_exact(a.len())
        .zip(biases)
        .map(|(row, &bias)| {
            let z = row
                .iter()
                .zip(a)
                .fold(bias, |sum, (&w, &x)| sum + i32::from(w) * x);
            (z >> 6).clamp(0, 127)
        })
        .collect()
}
fn integer_endpoint(a: &[i32], o: &[i16], bias: i32, h: &[i32], w2: &[i16]) -> i32 {
    let direct = a
        .iter()
        .zip(o)
        .fold(bias, |sum, (&x, &w)| sum + x * i32::from(w));
    h.iter()
        .zip(w2)
        .fold(direct, |sum, (&x, &w)| sum + x * i32::from(w))
}
fn integer_interpolate(open: i32, end: i32, q: i64) -> i64 {
    let numerator = q * i64::from(open) + (90 - q) * i64::from(end);
    let quotient = numerator / 90;
    quotient - i64::from(numerator % 90 < 0)
}
fn integer_reference(qm: &Quantized, shape: Shape, batch: &BatchSlot, b: usize) -> i32 {
    let width = shape.width();
    let mut linear = [0_i64; 2];
    for &f in &batch.linear_features()[b * 145..(b + 1) * 145] {
        if f == SENTINEL {
            continue;
        }
        // 正準行は各12筋の左6筋だけを格納する。
        let f = f as usize;
        let base = f / 72 * 144 + f % 72 / 6 * 12 + f % 6;
        for (sum, w) in linear.iter_mut().zip(qm.linear[base]) {
            *sum += i64::from(w);
        }
    }
    let mut a = Vec::new();
    for view in 0..2 {
        let features = &batch.features()[(b * 2 + view) * 145..(b * 2 + view + 1) * 145];
        for j in 0..width {
            let mut acc = qm.biases[j];
            for &f in features {
                if f == SENTINEL {
                    continue;
                }
                let row = match shape.feature_set() {
                    InputFeatureSet::P => decode_p_feature(f).unwrap() as usize,
                    InputFeatureSet::HalfKpHmB8 => f as usize,
                };
                acc += i32::from(qm.weights[row * width + j]);
            }
            let c = acc.clamp(0, 127);
            a.push(c * c / 128);
        }
    }
    let h = integer_hidden(&a, &qm.hidden_weights, &qm.hidden_biases);
    let open = integer_endpoint(
        &a,
        &qm.output_weights,
        qm.output_bias,
        &h,
        &qm.hidden_output,
    );
    let q = (i64::from(batch.piece_counts()[b]) - 2).clamp(0, 90);
    let out = if shape.head().phase2() {
        integer_interpolate(
            open,
            integer_endpoint(
                &a,
                &qm.output_weights_end,
                qm.output_bias_end[0],
                &h,
                &qm.hidden_output_end,
            ),
            q,
        )
    } else {
        i64::from(open)
    };
    let scale =
        (f64::from(shape.cp_per_logit()) * 65536.0 / (127.0 * 4096.0)).round_ties_even() as i64;
    let nonlinear_cp = (out * scale + 32768) >> 16;
    let linear_cp = (q * linear[0] + (90 - q) * linear[1]) / 720;
    (linear_cp + nonlinear_cp).clamp(-28999, 28999) as i32
}

fn assert_network<F: FeatureSet, H: Head>(model: &Model, positions: &[Position]) {
    let quantized = quantize(model).unwrap();
    let bytes = quantized
        .encode(&reference::v0_piece_values(), "L0,P0,R1,E0")
        .unwrap();
    let network = Network::<F, H>::decode(&bytes).unwrap();
    for side in Color::ALL {
        let batch =
            BatchSlot::from_evaluation_positions(model.shape().feature_set(), positions, side)
                .unwrap();
        for (b, position) in positions.iter().enumerate() {
            assert_eq!(
                integer_reference(&quantized, model.shape(), &batch, b),
                nnue::evaluate_for_diagnostics(&network, position, side),
                "{:?} {:?} {side:?} position {b}",
                model.shape().feature_set(),
                model.shape().head()
            );
        }
    }
}
fn assert_head<F: FeatureSet>(model: &Model, positions: &[Position]) {
    match model.shape().head() {
        HeadKind::DirectScrelu => assert_network::<F, DirectScrelu>(model, positions),
        HeadKind::DirectScreluPhase2 => assert_network::<F, DirectScreluPhase2>(model, positions),
        HeadKind::ScreluHidden16 => assert_network::<F, ScreluHidden16>(model, positions),
        HeadKind::ScreluHidden16Phase2 => {
            assert_network::<F, ScreluHidden16Phase2>(model, positions)
        }
    }
}

#[test]
fn integer_reference_matches_all_heads_and_features_before_and_after_training() {
    let mut positions = positions();
    positions.push(Position::initial());
    positions.extend(positions.clone().iter().map(mirror));
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        for head in HeadKind::ALL {
            let mut model = model_with_head(set, 6, head);
            let mut state = 39_u64;
            for p in Parameter::ALL {
                let scale = match p {
                    Parameter::Linear => 24.0,
                    Parameter::HiddenWeights => 0.5,
                    Parameter::HiddenBiases | Parameter::Biases => 0.75,
                    _ => 0.06,
                };
                for x in model.parameter_mut(p) {
                    state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                    *x = ((state >> 32) as f64 / f64::from(u32::MAX) * 2.0 - 1.0) as f32 * scale;
                }
            }
            model.constrain().unwrap();
            let batch =
                BatchSlot::from_evaluation_positions(set, &positions, Color::Black).unwrap();
            let mut optimizer = AdamW::new(model.shape(), AdamWConfig::default()).unwrap();
            for step in 0..=2 {
                match set {
                    InputFeatureSet::P => assert_head::<P>(&model, &positions),
                    InputFeatureSet::HalfKpHmB8 => assert_head::<HalfKpHmB8>(&model, &positions),
                }
                if step < 2 {
                    let gradients =
                        reference::loss_and_gradients(&model, &batch, &vec![0.7; batch.len()])
                            .unwrap();
                    optimizer.update(&mut model, &gradients.parameters).unwrap();
                }
            }
        }
    }
}

#[test]
fn all_heads_preserve_seeded_first_layer_and_epoch_zero_evaluations() {
    let positions = positions();
    let mut expected = None;
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let batch = BatchSlot::from_evaluation_positions(set, &positions, Color::Black).unwrap();
        let initialized = |head| {
            Model::initialize(
                Shape::new(set, 4, 1000.0, head).unwrap(),
                &vec![[1.0, 2.0]; 13680],
                all_observed(set),
                71,
                0.05,
            )
            .unwrap()
        };
        let base = initialized(HeadKind::DirectScrelu);
        for head in HeadKind::ALL {
            let model = initialized(head);
            assert_eq!(
                model.parameters()[Parameter::FirstLayer],
                base.parameters()[Parameter::FirstLayer]
            );
            assert_eq!(
                model.parameters()[Parameter::Virtual],
                base.parameters()[Parameter::Virtual]
            );
            let evaluations: Vec<_> = reference::evaluate(&model, &batch)
                .unwrap()
                .into_iter()
                .map(f32::to_bits)
                .collect();
            if let Some(ref expected) = expected {
                assert_eq!(&evaluations, expected);
            } else {
                expected = Some(evaluations);
            }
            let bound = 1.0 / 8.0_f32.sqrt();
            for p in [Parameter::HiddenWeights, Parameter::HiddenBiases] {
                assert!(model.parameters()[p].iter().all(|x| x.abs() <= bound));
                if head.hidden() {
                    assert!(model.parameters()[p].iter().any(|&x| x != 0.0));
                }
            }
            for p in [
                Parameter::OutputWeightsEnd,
                Parameter::OutputBiasEnd,
                Parameter::HiddenOutput,
                Parameter::HiddenOutputEnd,
            ] {
                assert!(model.parameters()[p].iter().all(|&x| x == 0.0));
            }
        }
    }
}

#[test]
fn hidden_quantization_rounds_ties_even_and_rejects_overflow() {
    let mut model = model_with_head(
        InputFeatureSet::HalfKpHmB8,
        2,
        HeadKind::ScreluHidden16Phase2,
    );
    for (p, scale, limit) in [
        (Parameter::HiddenWeights, 64.0, 127),
        (Parameter::HiddenBiases, 8128.0, 65024),
        (Parameter::HiddenOutput, 4096.0, 8192),
        (Parameter::HiddenOutputEnd, 4096.0, 8192),
        (Parameter::OutputWeightsEnd, 4096.0, 8192),
        (Parameter::OutputBiasEnd, 520192.0, 4161536),
    ] {
        let saved = model.parameters()[p][0];
        for sign in [-1.0, 1.0] {
            model.parameter_mut(p)[0] = sign * (limit as f32 + 1.0) / scale;
            assert!(matches!(quantize(&model), Err(Error::Quantization(actual)) if actual == p));
        }
        model.parameter_mut(p)[0] = saved;
    }
    model.parameter_mut(Parameter::HiddenWeights)[..4].copy_from_slice(&[
        0.5 / 64.0,
        1.5 / 64.0,
        -0.5 / 64.0,
        -1.5 / 64.0,
    ]);
    assert_eq!(
        &quantize(&model).unwrap().hidden_weights[..4],
        &[0, 2, 0, -2]
    );
}

#[test]
fn integer_head_boundaries_use_floor_saturation_and_safe_width_512_sums() {
    assert_eq!(integer_interpolate(-1, 0, 45), -1);
    let phase = DirectScreluPhase2::new(2, [&[0; 4], &[0; 4]], [-1, 0], false).unwrap();
    assert_eq!(phase.output(&[0; 2], &[0; 2], 45), -1);
    let a = [0; 4];
    let mut biases = [0; 16];
    biases[..6].copy_from_slice(&[-65, -1, 0, 63, 64, 8192]);
    assert_eq!(
        &integer_hidden(&a, &[0; 64], &biases)[..6],
        &[0, 0, 0, 0, 1, 127]
    );
    for k in 0..6 {
        let mut w2 = [0; 16];
        w2[k] = 1;
        let head = ScreluHidden16::new(2, [&[0; 4]], [0], &[0; 64], biases, [w2], false).unwrap();
        assert_eq!(head.output(&[0; 2], &[0; 2], 45), [0, 0, 0, 0, 1, 127][k]);
    }
    let a = vec![126; 1024];
    let w1 = vec![127; 16 * 1024];
    let h = integer_hidden(&a, &w1, &[65024; 16]);
    assert_eq!(h, vec![127; 16]);
    let expected = integer_endpoint(&a, &[8192; 1024], 4161536, &h, &[8192; 16]);
    assert_eq!(expected, 1_077_772_288);
    let head = ScreluHidden16Phase2::new(
        512,
        [&[8192; 1024]; 2],
        [4161536; 2],
        &w1,
        [65024; 16],
        [[8192; 16]; 2],
        true,
    )
    .unwrap();
    assert_eq!(head.output(&[127; 512], &[127; 512], 45), expected);
}

#[test]
fn hidden_saturation_counts_clipped_values_at_both_boundaries() {
    let batch =
        BatchSlot::from_evaluation_positions(InputFeatureSet::P, &positions(), Color::Black)
            .unwrap();
    for head in HeadKind::ALL {
        let mut model = model_with_head(InputFeatureSet::P, 4, head);
        model.parameter_mut(Parameter::HiddenWeights).fill(0.0);
        for (i, b) in model
            .parameter_mut(Parameter::HiddenBiases)
            .iter_mut()
            .enumerate()
        {
            *b = [-1.0, 0.0, 0.5, 1.0, 2.0, 0.5, 0.5, 0.5][i % 8];
        }
        let forward =
            reference::forward(&model, &batch, &reference::fold(&model).unwrap()).unwrap();
        let stats =
            reference::validation_forward(&model, &batch, &forward, &vec![0.5; batch.len()])
                .unwrap();
        if head.hidden() {
            assert_eq!(stats.hidden_activations, batch.len() * 16);
            assert_eq!(stats.hidden_lower, (batch.len() * 4) as u64);
            assert_eq!(stats.hidden_upper, (batch.len() * 4) as u64);
            assert_eq!(stats.hidden_saturation(), Some([0.25, 0.25]));
        } else {
            assert_eq!(stats.hidden_activations, 0);
            assert_eq!(stats.hidden_saturation(), None);
        }
    }
}

#[test]
fn new_parameters_use_head_optimizer_group_and_their_own_constraints() {
    use minase_trainer::nn::AdamWGroup;
    use minase_trainer::nnue::model::Parameters;
    let limits = [
        (Parameter::OutputWeightsEnd, 1.98),
        (Parameter::OutputBiasEnd, 8.0),
        (Parameter::HiddenWeights, 127.0 / 64.0),
        (Parameter::HiddenBiases, 8.0),
        (Parameter::HiddenOutput, 1.98),
        (Parameter::HiddenOutputEnd, 1.98),
    ];
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let mut model = model_with_head(set, 4, HeadKind::ScreluHidden16Phase2);
        let mut gradient = Parameters::zeros(model.shape());
        for &(p, _) in &limits {
            model.parameter_mut(p).fill(0.5);
            gradient[p].fill(1.0);
        }
        let mut optimizer = AdamW::new(
            model.shape(),
            AdamWConfig {
                linear: AdamWGroup {
                    lr: 0.01,
                    weight_decay: 0.0,
                },
                ft: AdamWGroup {
                    lr: 0.02,
                    weight_decay: 0.0,
                },
                head: AdamWGroup {
                    lr: 0.125,
                    weight_decay: 0.5,
                },
            },
        )
        .unwrap();
        optimizer.update(&mut model, &gradient).unwrap();
        // 最初のAdam更新では補正済み平均と分散がともに1。
        let expected = 0.5 * (1.0 - 0.125 * 0.5) - 0.125 / (1.0 + 1e-8);
        for &(p, _) in &limits {
            assert!(
                (f64::from(model.parameters()[p][0]) - expected).abs() < 1e-7,
                "{p:?}"
            );
        }
        for sign in [-1.0, 1.0] {
            for &(p, _) in &limits {
                model.parameter_mut(p).fill(sign * 100.0);
            }
            let mut constrained = model.clone();
            constrained.constrain().unwrap();
            gradient = Parameters::zeros(model.shape());
            optimizer.update(&mut model, &gradient).unwrap();
            for &(p, limit) in &limits {
                assert!(
                    model.parameters()[p].iter().all(|&x| x == sign * limit),
                    "{p:?}"
                );
                assert_eq!(model.parameters()[p], constrained.parameters()[p]);
            }
        }
    }
}

#[test]
fn clipped_relu_backward_includes_zero_and_one() {
    let batch =
        BatchSlot::from_evaluation_positions(InputFeatureSet::P, &positions()[..1], Color::Black)
            .unwrap();
    for head in [HeadKind::ScreluHidden16, HeadKind::ScreluHidden16Phase2] {
        let mut model = model_with_head(InputFeatureSet::P, 4, head);
        model.parameter_mut(Parameter::HiddenWeights).fill(0.0);
        for (i, x) in model
            .parameter_mut(Parameter::HiddenBiases)
            .iter_mut()
            .enumerate()
        {
            *x = [-0.01, 0.0, 1.0, 1.01][i % 4];
        }
        model.parameter_mut(Parameter::HiddenOutput).fill(0.04);
        model.parameter_mut(Parameter::HiddenOutputEnd).fill(0.04);
        let forward =
            reference::forward(&model, &batch, &reference::fold(&model).unwrap()).unwrap();
        let output = reference::output_backward(&model, &batch, &forward, &[0.0]).unwrap();
        for (k, &gradient) in output.d_hidden_biases.iter().enumerate() {
            let expected = if matches!(k % 4, 1 | 2) {
                output.g[0] * 0.04
            } else {
                0.0
            };
            assert!((gradient - expected).abs() < 1e-8, "{head:?} unit={k}");
        }
    }
}
