//! 設計式を独立なf64の密な評価で検証する。実装の分岐を期待値に写さない。
mod common;
use common::*;
use minase::eval::features::{FEATURE_COUNT, feature_index, mirror_feature};
use minase_core::{Color, PieceKind, Square};
use nnue_trainer::{
    data::{BatchSlot, InputFeatureSet, SENTINEL, decode_p_feature},
    model::{Masks, Model, Parameter, Parameters},
    reference::{self, AdamW, AdamWConfig, AdamWGroup, RemovalReference, loss_and_gradients},
};

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "actual={actual}, expected={expected}, difference={}",
        (actual - expected).abs()
    );
}
fn nonzero_model(set: InputFeatureSet) -> Model {
    let mut result = model(set, 4);
    result
        .parameter_mut(Parameter::FirstLayer)
        .iter_mut()
        .for_each(|x| *x *= 0.2);
    result
        .parameter_mut(Parameter::Virtual)
        .iter_mut()
        .for_each(|x| *x *= 0.2);
    result.parameter_mut(Parameter::Biases).fill(0.3);
    for (i, v) in result
        .parameter_mut(Parameter::OutputWeights)
        .iter_mut()
        .enumerate()
    {
        *v = if i % 2 == 0 { 0.017 } else { -0.011 };
    }
    result.parameter_mut(Parameter::OutputBias)[0] = 0.04;
    result
}

// 独立な損失の定義: 行を1本ずつ消してからネット全体をf64で再評価し、
// BCEと片側絶対値損失を足す。逆索引、d_r、A_vは使わない。
fn dense_loss(model: &Model, batch: &BatchSlot, endpoints: &[[i16; 2]]) -> f64 {
    let width = model.shape().width();
    let table = model.expanded_first_layer();
    let linear = model.expanded_linear();
    let bias = model.expanded_biases();
    let output = model.expanded_output_weights();
    let base = |b: usize, col: usize| {
        let row = batch.linear_features()[b * 145 + col];
        if row == SENTINEL {
            FEATURE_COUNT
        } else {
            let row = row as usize;
            row / 72 * 144 + row % 72 / 6 * 12 + row % 6
        }
    };
    let value = |b: usize, removed: Option<usize>| {
        let q = (i32::from(batch.piece_counts()[b]) - 2 - i32::from(removed.is_some())).clamp(0, 90)
            as f64;
        let mut linear_sums = [0.0; 2];
        let mut hidden = vec![bias.iter().map(|&v| f64::from(v)).collect::<Vec<_>>(); 2];
        for col in 0..145 {
            if removed == Some(col) {
                continue;
            }
            if base(b, col) != FEATURE_COUNT {
                for (sum, w) in linear_sums.iter_mut().zip(linear[base(b, col)]) {
                    *sum += f64::from(w);
                }
            }
            for (v, hidden) in hidden.iter_mut().enumerate() {
                let f = batch.features()[(b * 2 + v) * 145 + col];
                if f == SENTINEL {
                    continue;
                }
                let feature = if batch.feature_set() == InputFeatureSet::P {
                    decode_p_feature(f).unwrap() as usize
                } else {
                    f as usize
                };
                for j in 0..width {
                    hidden[j] += f64::from(table[feature * width + j]);
                }
            }
        }
        let mut logit = (q * linear_sums[0] + (90.0 - q) * linear_sums[1])
            / 90.0
            / f64::from(model.shape().k())
            + f64::from(model.parameters()[Parameter::OutputBias][0]);
        for (v, hidden) in hidden.iter().enumerate() {
            for j in 0..width {
                logit += hidden[j].clamp(0.0, 1.0).powi(2) * f64::from(output[v * width + j]);
            }
        }
        logit
    };
    let integer_reference = |b: usize, removed: Option<usize>| {
        let q =
            (i64::from(batch.piece_counts()[b]) - 2 - i64::from(removed.is_some())).clamp(0, 90);
        let mut numerator = 0_i64;
        for col in 0..145 {
            if removed != Some(col) && base(b, col) != FEATURE_COUNT {
                let w = endpoints[base(b, col)];
                numerator += q * i64::from(w[0]) + (90 - q) * i64::from(w[1]);
            }
        }
        ((numerator / 720).clamp(-28999, 28999), numerator)
    };
    let targets = reference::targets(batch, &[700.0]).unwrap();
    let mut loss = 0.0;
    for (b, &target) in targets.iter().enumerate() {
        let before = value(b, None);
        loss += before.max(0.0) - before * f64::from(target) + (-before.abs()).exp().ln_1p();
        let (reference_before, numerator_before) = integer_reference(b, None);
        let mut penalties = 0.0;
        let mut count = 0;
        for col in 0..144 {
            let f = base(b, col);
            if f >= 13536 || [11, 21].contains(&(f / 144 % 47)) {
                continue;
            }
            let (reference_after, numerator_after) = integer_reference(b, Some(col));
            let sign = (reference_after - reference_before).signum() as f64;
            if sign == 0.0 {
                continue;
            }
            count += 1;
            let signed = sign * (value(b, Some(col)) - before) * f64::from(model.shape().k());
            let magnitude = ((numerator_after - numerator_before) as f64 / 720.0).abs();
            penalties += (magnitude.min(0.25) - signed).max(0.0);
            if (f < 6768 && sign > 0.0) || (f >= 6768 && sign < 0.0) {
                penalties += (signed - magnitude).max(0.0);
            }
        }
        loss += 1000.0 * penalties / f64::from(model.shape().k()) / count.max(1) as f64;
    }
    loss / batch.len() as f64
}

fn v0_endpoints() -> Vec<[i16; 2]> {
    let values = reference::v0_piece_values();
    (0..FEATURE_COUNT)
        .map(|f| {
            if f < 13536 {
                let sign = if f < 6768 { 1 } else { -1 };
                [(sign * values[f / 144 % 47] * 8) as i16; 2]
            } else {
                [0; 2]
            }
        })
        .collect()
}

#[test]
fn finite_differences_cover_all_parameter_groups_and_penalty_regions() {
    let fixture = Fixture::new(&positions());
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let batch = fixture.batch(set, &[0, 1, 2]);
        let mut model = nonzero_model(set);
        for inverted in [false, true] {
            let mut endpoints = v0_endpoints();
            if inverted {
                // v0は全駒が正なので上限制約は発火しない。反転した1cpの参照を
                // 用意し、3cpの候補差分で上限の独立な試験を作る。
                for color in 0..2 {
                    endpoints[(color * 47 + 29) * 144..(color * 47 + 30) * 144]
                        .fill([if color == 0 { -8 } else { 8 }; 2]);
                }
                for row in 29 * 72..30 * 72 {
                    model.parameter_mut(Parameter::Linear)[2 * row..2 * row + 2].fill(-3.0);
                }
            }
            let reference = RemovalReference::new(&endpoints).unwrap();
            let gradients = loss_and_gradients(&model, &batch, &[700.0], &reference).unwrap();
            close(
                gradients.losses.total(),
                dense_loss(&model, &batch, &endpoints),
                3e-5,
            );
            let weights = reference::fold(&model);
            let forward = reference::forward(&model, &batch, &weights).unwrap();
            let removal =
                reference::removal_forward(&model, &batch, &weights, &forward, &reference).unwrap();
            assert!(removal.eligible.iter().any(|&x| !x));
            assert!(removal.pen_r.iter().any(|&x| x > 0.0));
            if inverted {
                assert!(
                    (0..removal.d_r.len())
                        .any(|i| removal.wrong_r[i] && removal.d_r[i] * removal.s_r[i] > 0.0)
                );
            } else {
                assert!((0..removal.d_r.len()).any(|i| removal.d_r[i] * removal.s_r[i] < 0.0));
            }
            for p in Parameter::ALL {
                let mut indices: Vec<_> = gradients.parameters[p]
                    .iter()
                    .enumerate()
                    .filter(|(_, g)| g.abs() > 1e-7)
                    .map(|(i, _)| i)
                    .collect();
                if indices.len() > 16 {
                    let step = indices.len() / 16;
                    indices = indices.into_iter().step_by(step).take(16).collect();
                }
                if !model.parameters()[p].is_empty() {
                    assert!(!indices.is_empty(), "no tested gradient in {p:?}");
                    indices.push(model.parameters()[p].len() - 1);
                }
                for i in indices {
                    let original = model.parameters()[p][i];
                    let epsilon = if p == Parameter::Linear { 1e-3 } else { 1e-4 };
                    model.parameter_mut(p)[i] = original + epsilon;
                    let plus_value = model.parameters()[p][i];
                    let plus = dense_loss(&model, &batch, &endpoints);
                    model.parameter_mut(p)[i] = original - epsilon;
                    let minus_value = model.parameters()[p][i];
                    let minus = dense_loss(&model, &batch, &endpoints);
                    model.parameter_mut(p)[i] = original;
                    let expected = (plus - minus) / f64::from(plus_value - minus_value);
                    let actual = f64::from(gradients.parameters[p][i]);
                    assert!(
                        (actual - expected).abs() <= 8e-5 + expected.abs() * 3e-4,
                        "{set:?} inverted={inverted} {p:?}[{i}] actual={actual}, finite difference={expected}"
                    );
                }
            }
        }
    }
}

#[test]
fn p_mirror_preserves_evaluation_and_canonical_gradients() {
    let original = positions().remove(0);
    let fixture = Fixture::new(&[original.clone(), mirror(&original)]);
    let first = fixture.batch(InputFeatureSet::P, &[0]);
    let mirrored = fixture.batch(InputFeatureSet::P, &[1]);
    for width in [2, 4, 6] {
        let mut model = model(InputFeatureSet::P, width);
        model.parameter_mut(Parameter::Biases).fill(0.2);
        model.parameter_mut(Parameter::OutputWeights).fill(0.03);
        assert_eq!(
            reference::evaluate(&model, &first).unwrap(),
            reference::evaluate(&model, &mirrored).unwrap()
        );
        // 同じ教師値を明示して、鏡映以外の入力差を除く。
        let derivatives = |batch: &BatchSlot| {
            let w = reference::fold(&model);
            let f = reference::forward(&model, batch, &w).unwrap();
            let r =
                reference::removal_forward(&model, batch, &w, &f, &RemovalReference::v0()).unwrap();
            let o = reference::output_backward(&model, batch, &f, &r, &[0.7]).unwrap();
            let mut g = nnue_trainer::model::Parameters::zeros(model.shape());
            reference::reduce_partials(&model, &o, &r, &mut g);
            reference::first_layer_backward(&model, batch, &w, &f, &r, &o, &mut g);
            reference::linear_backward(&model, batch, &r, &o, &mut g);
            g
        };
        let a = derivatives(&first);
        let b = derivatives(&mirrored);
        for p in Parameter::ALL {
            for (&a, &b) in a[p].iter().zip(&b[p]) {
                close(f64::from(a), f64::from(b), 2e-5);
            }
        }
    }
}

#[test]
fn epoch_zero_halfkp_matches_p_and_nonzero_shared_head() {
    let fixture = Fixture::new(&positions());
    let p_batch = fixture.batch(InputFeatureSet::P, &[0, 1, 2]);
    let h_batch = fixture.batch(InputFeatureSet::HalfKpHmB8, &[0, 1, 2]);
    let mut p = model(InputFeatureSet::P, 4);
    let mut half = model(InputFeatureSet::HalfKpHmB8, 4);
    assert_eq!(
        reference::evaluate(&p, &p_batch).unwrap(),
        reference::evaluate(&half, &h_batch).unwrap()
    );
    p.parameter_mut(Parameter::Biases)
        .copy_from_slice(&[0.2, 0.3]);
    p.parameter_mut(Parameter::OutputWeights)
        .copy_from_slice(&[0.04, 0.02, -0.03, 0.01]);
    half.parameter_mut(Parameter::Biases)
        .copy_from_slice(&p.expanded_biases());
    half.parameter_mut(Parameter::OutputWeights)
        .copy_from_slice(&p.expanded_output_weights());
    for (a, b) in reference::evaluate(&p, &p_batch)
        .unwrap()
        .into_iter()
        .zip(reference::evaluate(&half, &h_batch).unwrap())
    {
        close(f64::from(a), f64::from(b), 1e-5);
    }
}

#[test]
fn screlu_derivative_includes_one_but_excludes_saturated_region() {
    for (x, f, h) in [
        (-1.0, 0.0, 0.0),
        (0.0, 0.0, 0.0),
        (0.5, 0.25, 1.0),
        (1.0, 1.0, 2.0),
        (1.01, 1.0, 0.0),
    ] {
        assert_eq!(reference::screlu(x), f);
        assert_eq!(reference::screlu_derivative(x), h);
    }
}

#[test]
fn adamw_bias_correction_decay_and_zero_gradient_momentum_match_hand_calculation() {
    let mut p = [2.0, -3.0];
    let mut m = [0.0; 2];
    let mut v = [0.0; 2];
    let group = AdamWGroup {
        lr: 0.01,
        weight_decay: 0.1,
    };
    reference::adamw(&mut p, &[0.5, -0.25], &mut m, &mut v, 1, group).unwrap();
    close(f64::from(p[0]), 1.998 - 0.01 * 0.5 / (0.5 + 1e-8), 2e-7);
    close(f64::from(p[1]), -2.997 + 0.01 * 0.25 / (0.25 + 1e-8), 2e-7);
    close(f64::from(m[0]), 0.05, 1e-8);
    close(f64::from(v[0]), 0.00025, 1e-10);
    let old = p[0];
    reference::adamw(&mut p, &[0.0; 2], &mut m, &mut v, 2, group).unwrap();
    let expected = f64::from(old) * 0.999
        - 0.01 * (0.045 / 0.19) / ((0.00024975_f64 / 0.001999).sqrt() + 1e-8);
    close(f64::from(p[0]), expected, 3e-7);
    assert!(p[0] < old * 0.999);
}

#[test]
fn adamw_preserves_unobserved_parameters_and_moments_without_clipping() {
    // カーネル8の契約: 訓練集合で未観測の行はp,m,vを保持する一方、
    // 今回だけ勾配0の観測行は過去のモーメントと減衰で動く。
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let mut warm = model(set, 2);
        let shape = warm.shape();
        let group = AdamWGroup {
            lr: 0.01,
            weight_decay: 0.1,
        };
        let mut optimizer = AdamW::new(
            shape,
            AdamWConfig {
                linear: group,
                ft: group,
                head: group,
            },
        )
        .unwrap();
        let mut gradient = Parameters::zeros(shape);
        for p in Parameter::ALL {
            gradient[p].fill(0.25);
        }
        optimizer.update(&mut warm, &gradient).unwrap();

        let mut canonical = vec![false; 6841];
        canonical[0] = true;
        let mut input = vec![false; set.dimensions() + 1];
        input[0] = true;
        let masks = Masks::new(set, canonical, input).unwrap();
        let mut model =
            Model::initialize(shape, &vec![[0.0; 2]; FEATURE_COUNT], masks, 0, 0.0).unwrap();
        for p in [Parameter::Linear, Parameter::FirstLayer, Parameter::Virtual] {
            // 初期値や範囲外の値を復元・切り詰めしていないことも検査する。
            model
                .parameter_mut(p)
                .fill(if p == Parameter::Linear { 5000.0 } else { 9.0 });
            model.parameter_mut(p)[..shape.len(p).min(2)].fill(0.5);
        }
        let before = model.parameters().clone();
        let m_before = optimizer.m().clone();
        let v_before = optimizer.v().clone();
        optimizer
            .update(&mut model, &Parameters::zeros(shape))
            .unwrap();
        assert_eq!(optimizer.step(), 2);
        for p in [Parameter::Linear, Parameter::FirstLayer, Parameter::Virtual] {
            for i in 0..shape.len(p) {
                if i < 2 {
                    assert!(model.parameters()[p][i] < before[p][i] * 0.999);
                    assert!(optimizer.m()[p][i] < m_before[p][i]);
                    assert!(optimizer.v()[p][i] < v_before[p][i]);
                } else {
                    assert_eq!(model.parameters()[p][i].to_bits(), before[p][i].to_bits());
                    assert_eq!(optimizer.m()[p][i].to_bits(), m_before[p][i].to_bits());
                    assert_eq!(optimizer.v()[p][i].to_bits(), v_before[p][i].to_bits());
                }
            }
        }
    }
}

#[test]
fn adamw_clips_all_groups_and_uses_updated_virtual_rows_for_the_fold() {
    // 仕様の保存範囲とW+Vの不変条件を、勾配や減衰のない更新で検査する。
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let mut model = model(set, 4);
        let shape = model.shape();
        for p in Parameter::ALL {
            for (i, value) in model.parameter_mut(p).iter_mut().enumerate() {
                *value = if i % 2 == 0 { 5000.0 } else { -5000.0 };
            }
        }
        let no_decay = AdamWGroup {
            lr: 0.01,
            weight_decay: 0.0,
        };
        let mut optimizer = AdamW::new(
            shape,
            AdamWConfig {
                linear: no_decay,
                ft: no_decay,
                head: no_decay,
            },
        )
        .unwrap();
        optimizer
            .update(&mut model, &Parameters::zeros(shape))
            .unwrap();
        for p in [
            Parameter::Linear,
            Parameter::Biases,
            Parameter::OutputWeights,
            Parameter::OutputBias,
        ] {
            let (upper, lower) = match p {
                Parameter::Linear => (4095.875, -4096.0),
                Parameter::OutputWeights => (1.98, -1.98),
                _ => (8.0, -8.0),
            };
            assert_eq!(model.parameters()[p][0], upper);
            if shape.len(p) > 1 {
                assert_eq!(model.parameters()[p][1], lower);
            }
        }
        let folded = reference::fold(&model);
        assert_eq!(&folded[..4], &[8.0, -8.0, 8.0, -8.0]);
        if set == InputFeatureSet::HalfKpHmB8 {
            assert_eq!(
                &model.parameters()[Parameter::Virtual][..4],
                &[8.0, -8.0, 8.0, -8.0]
            );
            assert_eq!(&model.parameters()[Parameter::FirstLayer][..4], &[0.0; 4]);
        }
    }
}

#[test]
fn adamw_fold_inherits_updated_virtual_rows_at_unobserved_features() {
    let set = InputFeatureSet::HalfKpHmB8;
    let shape = nnue_trainer::model::Shape::new(set, 2, 1000.0).unwrap();
    let mut canonical = vec![false; 6841];
    canonical[0] = true;
    let mut input = vec![false; set.dimensions() + 1];
    input[0] = true;
    let mut model = Model::initialize(
        shape,
        &vec![[0.0; 2]; FEATURE_COUNT],
        Masks::new(set, canonical, input).unwrap(),
        0,
        0.0,
    )
    .unwrap();
    let mut optimizer = AdamW::new(shape, AdamWConfig::default()).unwrap();
    let mut gradient = Parameters::zeros(shape);
    gradient[Parameter::Virtual][..2].fill(-1.0);
    optimizer.update(&mut model, &gradient).unwrap();
    let unobserved = FEATURE_COUNT * shape.width();
    assert_eq!(
        &model.parameters()[Parameter::FirstLayer][unobserved..unobserved + 2],
        &[0.0; 2]
    );
    let v = &model.parameters()[Parameter::Virtual][..2];
    assert!(v.iter().all(|&x| x > 0.0));
    assert_eq!(&reference::fold(&model)[unobserved..unobserved + 2], v);
}

#[test]
fn integer_reference_uses_phase_truncation_saturation_and_excludes_royals() {
    let position = position(
        &[
            (0, 0, Color::Black, PieceKind::King),
            (1, 2, Color::Black, PieceKind::Pawn),
            (2, 3, Color::White, PieceKind::Pawn),
        ],
        Color::Black,
        None,
    );
    let fixture = Fixture::new(&[position]);
    let batch = fixture.batch(InputFeatureSet::P, &[0]);
    let (delta, _) = RemovalReference::v0().deltas(&batch, 0);
    assert_eq!(delta[25], -100);
    assert_eq!(delta[38], 100);
    assert_eq!(delta[0], -2600);
    let m = model(InputFeatureSet::P, 4);
    let w = reference::fold(&m);
    let f = reference::forward(&m, &batch, &w).unwrap();
    let r = reference::removal_forward(&m, &batch, &w, &f, &RemovalReference::v0()).unwrap();
    assert!(!r.eligible[0]);
    assert!(!r.eligible[1]);
    assert!(r.eligible[25]);
    assert_eq!(r.eligible.iter().filter(|&&x| x).count(), 2);
    let mut endpoints = vec![[0; 2]; FEATURE_COUNT];
    // 3駒なので除去前q=1、除去後q=0。mg=8,eg=1の歩を1枚だけ置く。
    let p = fixture.dataset.record(0).unwrap().to_position().unwrap();
    let sq = Square::new(1, 2).unwrap();
    let feature = feature_index(Color::Black, p.piece_at(sq).unwrap(), sq);
    endpoints[feature] = [8, 1];
    endpoints[mirror_feature(feature)] = [8, 1];
    let (delta, magnitudes) = RemovalReference::new(&endpoints).unwrap().deltas(&batch, 0);
    assert_eq!(delta[25], 0);
    close(magnitudes[25], 97.0 / 720.0, 1e-15);
    let full = Fixture::new(&[common::position(&[], Color::Black, None)]); // 空局面の全除去は不適格。
    let empty = full.batch(InputFeatureSet::P, &[0]);
    let g = loss_and_gradients(&m, &empty, &[700.0], &RemovalReference::v0()).unwrap();
    assert_eq!(g.losses.auxiliary, 0.0);
}

#[test]
fn integer_reference_handles_phase_boundaries_negative_truncation_and_evaluation_limit() {
    for (count, expected) in [(1, -1), (2, -1), (3, -1), (91, -2), (92, -4), (93, -2)] {
        for color in [Color::Black, Color::White] {
            let pieces: Vec<_> = (0..count)
                .map(|i| ((i % 12) as u8, (i / 12) as u8, color, PieceKind::Pawn))
                .collect();
            let fixture = Fixture::new(&[position(&pieces, Color::Black, None)]);
            let batch = fixture.batch(InputFeatureSet::P, &[0]);
            let sign = if color == Color::Black { 1 } else { -1 };
            let mut endpoints = vec![[0; 2]; FEATURE_COUNT];
            let start = if color == Color::Black {
                29 * 144
            } else {
                (47 + 29) * 144
            };
            endpoints[start..start + 144].fill([sign * 16, sign * 8]);
            let (deltas, _) = RemovalReference::new(&endpoints).unwrap().deltas(&batch, 0);
            assert_eq!(
                deltas[0],
                i32::from(sign) * expected,
                "count={count} color={color:?}"
            );
        }
    }
    for (count, expected) in [(39, -499), (42, 0)] {
        let pieces: Vec<_> = (0..count)
            .map(|i| {
                (
                    (i % 12) as u8,
                    (i / 12) as u8,
                    Color::Black,
                    PieceKind::Rook,
                )
            })
            .collect();
        let fixture = Fixture::new(&[position(&pieces, Color::Black, None)]);
        let batch = fixture.batch(InputFeatureSet::P, &[0]);
        let (deltas, magnitudes) = RemovalReference::v0().deltas(&batch, 0);
        assert_eq!(deltas[0], expected);
        assert_eq!(magnitudes[0], 750.0);
    }
}
