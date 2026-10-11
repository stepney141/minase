//! 初期化、制約、保存形式、および整数推論との外部契約。
mod common;
use common::*;
use minase::eval::nnue::{
    self, Network,
    feature_set::{HalfKpHmB8, P},
    head::DirectScrelu,
};
use nnue_trainer::{
    data::{InputFeatureSet, Observations},
    model::{
        BestEpoch, Checkpoint, Masks, Model, Parameter, Parameters, Progress, Shape, quantize,
    },
    reference::{self, AdamW, AdamWConfig},
};

#[test]
fn initialization_is_seeded_and_consumes_only_observed_canonical_rows() {
    let mut initial = vec![[0.0; 2]; 13680];
    initial[0] = [10.0, 20.0];
    initial[11] = initial[0];
    let masks = |rows: &[usize]| {
        let mut canonical = vec![false; 6841];
        for &row in rows {
            canonical[row] = true;
        }
        Masks::new(InputFeatureSet::P, canonical, vec![false; 13681]).unwrap()
    };
    let shape = Shape::new(InputFeatureSet::P, 4, 1000.0).unwrap();
    let a = Model::initialize(shape, &initial, masks(&[3, 5]), 7, 0.05).unwrap();
    let b = Model::initialize(shape, &initial, masks(&[0, 3, 5]), 7, 0.05).unwrap();
    assert_eq!(&a.parameters()[Parameter::Linear][..2], &[10.0, 20.0]);
    assert_eq!(
        &a.parameters()[Parameter::FirstLayer][12..16],
        &b.parameters()[Parameter::FirstLayer][..4]
    );
    assert_eq!(
        &a.parameters()[Parameter::FirstLayer][20..24],
        &b.parameters()[Parameter::FirstLayer][12..16]
    );
    assert_eq!(
        a,
        Model::initialize(shape, &initial, masks(&[3, 5]), 7, 0.05).unwrap()
    );
    assert_ne!(
        a,
        Model::initialize(shape, &initial, masks(&[3, 5]), 8, 0.05).unwrap()
    );
    assert!(
        a.parameters()[Parameter::FirstLayer][..12]
            .iter()
            .all(|&v| v == 0.0)
    );
    for p in [
        Parameter::Biases,
        Parameter::OutputWeights,
        Parameter::OutputBias,
    ] {
        assert!(a.parameters()[p].iter().all(|&v| v == 0.0));
    }
    let full = model(InputFeatureSet::P, 4);
    let samples = &full.parameters()[Parameter::FirstLayer][..6840 * 4];
    let mean = samples.iter().map(|&x| f64::from(x)).sum::<f64>() / samples.len() as f64;
    let std = (samples
        .iter()
        .map(|&x| (f64::from(x) - mean).powi(2))
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt();
    assert!(mean.abs() < 0.001);
    assert!((std - 0.05).abs() < 0.001);
}

#[test]
fn initialization_requires_exact_mirror_symmetry_in_both_endpoints() {
    // Pythonのload_initialと同じく、未観測行も含む全特徴の左右一致を要求する。
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let shape = Shape::new(set, 2, 1000.0).unwrap();
        let masks = Masks::new(set, vec![false; 6841], vec![false; set.dimensions() + 1]).unwrap();
        for (left, right) in [(0, 11), (5, 6), (12, 23), (13668, 13679)] {
            for endpoint in 0..2 {
                let mut initial = vec![[0.0; 2]; 13680];
                initial[left][endpoint] = 1.0;
                initial[right][endpoint] = 1.0;
                let model = Model::initialize(shape, &initial, masks.clone(), 7, 0.05).unwrap();
                assert_eq!(model.expanded_linear(), initial);

                // 隣接するf32値の差にも許容誤差や平均化を適用しない。
                initial[right][endpoint] = f32::from_bits(1.0_f32.to_bits() + 1);
                assert!(Model::initialize(shape, &initial, masks.clone(), 7, 0.05).is_err());
            }
        }
    }
}

#[test]
fn initialization_standard_deviation_scales_first_layer_and_virtual_rows() {
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let shape = Shape::new(set, 6, 1000.0).unwrap();
        let initial = vec![[17.0, -23.0]; 13680];
        let make = |std| Model::initialize(shape, &initial, all_observed(set), 71, std);
        let a = make(0.05).unwrap();
        let b = make(0.1).unwrap();
        for p in [Parameter::FirstLayer, Parameter::Virtual] {
            for (&a, &b) in a.parameters()[p].iter().zip(&b.parameters()[p]) {
                assert_eq!(b, 2.0 * a);
            }
        }
        assert_eq!(
            a.parameters()[Parameter::Linear],
            b.parameters()[Parameter::Linear]
        );
        let zero = make(0.0).unwrap();
        assert!(reference::fold(&zero).unwrap().iter().all(|&x| x == 0.0));
        for std in [-0.1, f32::NAN, f32::INFINITY] {
            assert!(make(std).is_err());
        }
    }
}

#[test]
fn constraints_restore_unobserved_rows_and_bound_folded_weights() {
    let fixture = Fixture::new(&positions());
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let observed = Observations::compute(&fixture.dataset, set).unwrap();
        let masks = Masks::from_observations(set, &observed).unwrap();
        for sign in [-1.0, 1.0] {
            let mut model = Model::initialize(
                Shape::new(set, 4, 1000.0).unwrap(),
                &vec![[100.0, 120.0]; 13680],
                masks.clone(),
                41,
                0.05,
            )
            .unwrap();
            let initial_linear = model.initial_linear().to_vec();
            let initial_virtual = model.initial_virtual().to_vec();
            for p in Parameter::ALL {
                model.parameter_mut(p).fill(sign * 100000.0);
            }
            model
                .parameter_mut(Parameter::Virtual)
                .fill(-sign * 100000.0);
            model.constrain().unwrap();
            for (row, &seen) in masks.canonical().iter().enumerate() {
                for e in 0..2 {
                    let expected = if seen {
                        if sign < 0.0 { -4096.0 } else { 4095.875 }
                    } else {
                        initial_linear[row * 2 + e]
                    };
                    assert_eq!(model.parameters()[Parameter::Linear][row * 2 + e], expected);
                }
            }
            for (row, &seen) in masks.first_layer().iter().enumerate() {
                if !seen {
                    assert!(
                        model.parameters()[Parameter::FirstLayer][row * 4..row * 4 + 4]
                            .iter()
                            .all(|&x| x == 0.0)
                    );
                }
            }
            for (row, &seen) in masks.virtual_rows().iter().enumerate() {
                if !seen {
                    assert_eq!(
                        &model.parameters()[Parameter::Virtual][row * 4..row * 4 + 4],
                        &initial_virtual[row * 4..row * 4 + 4]
                    );
                }
            }
            assert!(model.expanded_first_layer().iter().all(|&x| x.abs() <= 8.0));
            assert!(
                model.parameters()[Parameter::Biases]
                    .iter()
                    .all(|&x| x == sign * 8.0)
            );
            assert!(
                model.parameters()[Parameter::OutputWeights]
                    .iter()
                    .all(|&x| x == sign * 1.98)
            );
            assert_eq!(model.parameters()[Parameter::OutputBias][0], sign * 8.0);
            quantize(&model).unwrap();
        }
    }
}

#[test]
fn halfkp_unobserved_feature_inherits_updated_virtual_row() {
    let mut canonical = vec![false; 6841];
    canonical[0] = true;
    let mut input = vec![false; 109441];
    input[0] = true;
    let masks = Masks::new(InputFeatureSet::HalfKpHmB8, canonical, input).unwrap();
    let mut model = Model::initialize(
        Shape::new(InputFeatureSet::HalfKpHmB8, 4, 1000.0).unwrap(),
        &vec![[0.0; 2]; 13680],
        masks,
        5,
        0.05,
    )
    .unwrap();
    model.parameter_mut(Parameter::Virtual)[0] = 0.75;
    model.parameter_mut(Parameter::FirstLayer)[13680 * 4] = 4.0;
    model.constrain().unwrap();
    assert_eq!(model.parameters()[Parameter::FirstLayer][13680 * 4], 0.0);
    assert_eq!(reference::fold(&model).unwrap()[13680 * 4], 0.75);
}

#[test]
fn quantization_ties_even_rotation_and_overflow_match_python_examples() {
    let mut model = model(InputFeatureSet::P, 4);
    for (row, value) in [0.0625, 0.1875, -0.0625, -0.1875].into_iter().enumerate() {
        model.parameter_mut(Parameter::Linear)[row * 2] = value;
    }
    model.parameter_mut(Parameter::FirstLayer)[..4].copy_from_slice(&[0.5, 1.5, -0.5, -1.5]);
    model
        .parameter_mut(Parameter::Biases)
        .copy_from_slice(&[-0.5, -1.5]);
    model
        .parameter_mut(Parameter::OutputWeights)
        .copy_from_slice(&[0.5 / 4096.0, 1.5 / 4096.0, -0.5 / 4096.0, -1.5 / 4096.0]);
    model.parameter_mut(Parameter::OutputBias)[0] = 1.0 / 8192.0;
    let q = quantize(&model).unwrap();
    assert_eq!(
        q.linear[..4].iter().map(|v| v[0]).collect::<Vec<_>>(),
        [0, 2, 0, -2]
    );
    assert_eq!(&q.weights[..4], &[64, 190, -64, -190]);
    assert_eq!(&q.weights[44..48], &[-64, -190, 64, 190]);
    assert_eq!(q.biases, [-64, -190, -64, -190]);
    assert_eq!(q.output_weights, [0, 2, 0, 2, 0, -2, 0, -2]);
    assert_eq!(q.output_bias, 64);
    for (p, v) in [
        (Parameter::Linear, 4096.0),
        (Parameter::FirstLayer, 9.0),
        (Parameter::Biases, -9.0),
        (Parameter::OutputWeights, 2.1),
        (Parameter::OutputBias, 9.0),
    ] {
        let mut bad = model.clone();
        bad.parameter_mut(p)[0] = v;
        assert!(quantize(&bad).is_err());
    }
    let mut bad = model;
    bad.parameter_mut(Parameter::OutputBias)[0] = f32::NAN;
    assert!(bad.constrain().is_err());
    assert!(quantize(&bad).is_err());
}

#[test]
fn quantized_mnue_v5_round_trip_has_small_mean_integer_evaluation_error() {
    let mut positions = positions();
    positions.push(minase_core::Position::initial());
    positions.extend(positions.clone().iter().map(mirror));
    let fixture = Fixture::new(&positions);
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let mut m = model(set, 6);
        m.parameter_mut(Parameter::Biases).fill(0.2);
        for (i, x) in m
            .parameter_mut(Parameter::OutputWeights)
            .iter_mut()
            .enumerate()
        {
            *x = (i as f32 % 3.0 - 1.0) * 0.004;
        }
        m.parameter_mut(Parameter::OutputBias)[0] = 0.01;
        let quantized = quantize(&m).unwrap();
        let bytes = quantized
            .encode(&reference::v0_piece_values(), "L0,P0,R1,E0")
            .unwrap();
        assert_eq!(&bytes[..8], b"MNUE\x05\0\0\0");
        let integer: Vec<_> = match set {
            InputFeatureSet::P => {
                let net = Network::<P, DirectScrelu>::decode(&bytes).unwrap();
                positions.iter().map(|p| nnue::evaluate(&net, p)).collect()
            }
            InputFeatureSet::HalfKpHmB8 => {
                let net = Network::<HalfKpHmB8, DirectScrelu>::decode(&bytes).unwrap();
                positions.iter().map(|p| nnue::evaluate(&net, p)).collect()
            }
        };
        let batch = fixture.batch(set, &(0..positions.len() as u64).collect::<Vec<_>>());
        let floating = reference::evaluate(&m, &batch).unwrap();
        let mean = floating
            .iter()
            .zip(integer)
            .map(|(&a, b)| (f64::from(a) - f64::from(b)).abs())
            .sum::<f64>()
            / positions.len() as f64;
        assert!(mean < 1.0, "{set:?} mean absolute error={mean}cp");
        assert!(mean < f64::from(m.shape().k()) / 40.0);
    }
}

#[test]
fn checkpoint_round_trip_restores_next_update_and_rejects_corruption_and_hash_mismatch() {
    let fixture = Fixture::new(&positions());
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let model = model(set, 4);
        let optimizer = AdamW::new(model.shape(), AdamWConfig::default()).unwrap();
        let mut state = Checkpoint {
            model,
            optimizer,
            progress: Progress {
                epoch: 4,
                best: Some(BestEpoch {
                    epoch: 3,
                    validation_loss: 0.4,
                }),
                elapsed_seconds: 123.5,
                seed: 71,
                config_hash: [1; 32],
                input_hash: [2; 32],
            },
        };
        let batch = fixture.batch(set, &[0, 1, 2]);
        for _ in 0..3 {
            let g = reference::loss_and_gradients(
                &state.model,
                &batch,
                &[700.0],
                &reference::RemovalReference::v0(),
            )
            .unwrap();
            state
                .optimizer
                .update(&mut state.model, &g.parameters)
                .unwrap();
        }
        let bytes = state.encode().unwrap();
        let mut restored = Checkpoint::decode(&bytes).unwrap();
        assert_eq!(state, restored);
        let path = fixture.root.join("checkpoint.bin");
        state.write(&path).unwrap();
        assert_eq!(Checkpoint::read(&path).unwrap(), state);
        restored.check_hashes(&[1; 32], &[2; 32]).unwrap();
        assert!(restored.check_hashes(&[0; 32], &[2; 32]).is_err());
        let g = reference::loss_and_gradients(
            &state.model,
            &batch,
            &[700.0],
            &reference::RemovalReference::v0(),
        )
        .unwrap();
        state
            .optimizer
            .update(&mut state.model, &g.parameters)
            .unwrap();
        restored
            .optimizer
            .update(&mut restored.model, &g.parameters)
            .unwrap();
        assert_eq!(state, restored);
        let mut corrupt = bytes.clone();
        corrupt[110] ^= 1;
        assert!(Checkpoint::decode(&corrupt).is_err());
        assert!(Checkpoint::decode(&bytes[..bytes.len() - 1]).is_err());
    }
}

#[test]
fn model_rejects_invalid_width_k_initial_and_mask_shapes() {
    for width in [0, 1, 3, 513, usize::MAX] {
        assert!(Shape::new(InputFeatureSet::P, width, 1000.0).is_err());
    }
    for width in [2, 6, 512] {
        assert!(Shape::new(InputFeatureSet::P, width, 1000.0).is_ok());
    }
    for k in [0.0, -1.0, f32::NAN, f32::INFINITY, 100001.0] {
        assert!(Shape::new(InputFeatureSet::P, 4, k).is_err());
    }
    assert!(Masks::new(InputFeatureSet::P, vec![false; 6840], vec![false; 13681]).is_err());
    assert!(
        Model::initialize(
            Shape::new(InputFeatureSet::P, 4, 1000.0).unwrap(),
            &[],
            all_observed(InputFeatureSet::P),
            7,
            0.05,
        )
        .is_err()
    );
    let mut m = model(InputFeatureSet::P, 4);
    let mut optimizer = AdamW::new(m.shape(), AdamWConfig::default()).unwrap();
    assert!(
        optimizer
            .update(
                &mut m,
                &Parameters::zeros(Shape::new(InputFeatureSet::P, 6, 1000.0).unwrap())
            )
            .is_err()
    );
}

#[test]
fn stored_npz_and_numpy_zip64_load_endpoints_and_reject_bad_archives() {
    let fixture = Fixture::new(&positions());
    let path = fixture.root.join("pst-float.npz");
    let initial: Vec<_> = (0..13680)
        .map(|i| {
            let row = i / 12;
            [row as f32 / 8.0, -(row as f32) / 16.0]
        })
        .collect();
    for zip64 in [false, true] {
        let bytes = npz_bytes(&initial, zip64);
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(nnue_trainer::model::load_initial(&path).unwrap(), initial);
        for change in [6, 8, 14, bytes.len() - 1] {
            let mut invalid = bytes.clone();
            invalid[change] ^= 1;
            std::fs::write(&path, invalid).unwrap();
            assert!(nnue_trainer::model::load_initial(&path).is_err());
        }
        for cut in [0, 10, 100, bytes.len() - 1] {
            std::fs::write(&path, &bytes[..cut]).unwrap();
            assert!(nnue_trainer::model::load_initial(&path).is_err());
        }
    }
}

#[test]
fn load_initial_rejects_asymmetric_endpoints() {
    let fixture = Fixture::new(&positions());
    let path = fixture.root.join("pst-float.npz");
    for zip64 in [false, true] {
        for endpoint in 0..2 {
            let mut initial = vec![[0.0; 2]; 13680];
            initial[13679][endpoint] = 1.0;
            std::fs::write(&path, npz_bytes(&initial, zip64)).unwrap();
            assert!(nnue_trainer::model::load_initial(&path).is_err());
        }
    }
}
