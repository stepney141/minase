//! NNUEの数式を、独立なf64の密な評価と有限差分で検証する。
//! 逆索引や後退の中間値を使わず、ネット全体を再評価する。

use minase_trainer::nn::AdamWGroup;
mod common;
use common::*;
use minase::eval::features::{FEATURE_COUNT, feature_index, mirror_feature};
use minase_core::{Color, PieceKind, Square};
use minase_trainer::nnue::{
    data::{BatchSlot, InputFeatureSet, SENTINEL, build_targets, decode_p_feature},
    model::{HeadKind, Masks, Model, Parameter, Parameters},
    reference::{self, AdamW, AdamWConfig, RemovalReference, loss_and_gradients},
};

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "actual={actual}, expected={expected}, difference={}",
        (actual - expected).abs()
    );
}
fn nonzero_model(set: InputFeatureSet) -> Model {
    nonzero_model_with_head(set, HeadKind::DirectScrelu)
}
fn nonzero_model_with_head(set: InputFeatureSet, head: HeadKind) -> Model {
    let mut result = model_with_head(set, 4, head);
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
    for (p, value) in [
        (Parameter::OutputWeightsEnd, -0.013),
        (Parameter::OutputBiasEnd, -0.02),
        (Parameter::HiddenWeights, 0.07),
        (Parameter::HiddenBiases, 0.2),
        (Parameter::HiddenOutput, 0.03),
        (Parameter::HiddenOutputEnd, -0.01),
    ] {
        result.parameter_mut(p).fill(value);
    }
    result
}

// 独立な損失の定義: ネット全体をf64で再評価し、二値交差エントロピーを求める。
fn dense_loss(model: &Model, batch: &BatchSlot) -> f64 {
    let width = model.shape().width();
    let table = model.expanded_first_layer();
    let linear = model.expanded_linear();
    let bias = model.expanded_biases();

    let base = |b: usize, col: usize| {
        let row = batch.linear_features()[b * 145 + col];
        if row == SENTINEL {
            FEATURE_COUNT
        } else {
            let row = row as usize;
            row / 72 * 144 + row % 72 / 6 * 12 + row % 6
        }
    };
    let value = |b: usize| {
        let q = (i32::from(batch.piece_counts()[b]) - 2).clamp(0, 90) as f64;
        let mut linear_sums = [0.0; 2];
        let mut hidden = vec![bias.iter().map(|&v| f64::from(v)).collect::<Vec<_>>(); 2];
        for col in 0..145 {
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
        let s: Vec<_> = hidden
            .iter()
            .flatten()
            .map(|x| x.clamp(0.0, 1.0).powi(2))
            .collect();
        let p = model.parameters();
        let u = model.shape().units();
        let index = |i: usize| i / width * u + i % width % u;
        let hidden: Vec<_> = (0..if model.shape().head().hidden() { 16 } else { 0 })
            .map(|k| {
                (s.iter()
                    .enumerate()
                    .map(|(i, &x)| x * f64::from(p[Parameter::HiddenWeights][k * 2 * u + index(i)]))
                    .sum::<f64>()
                    + f64::from(p[Parameter::HiddenBiases][k]))
                .clamp(0.0, 1.0)
            })
            .collect();
        let endpoint = |o, w, b| {
            f64::from(p[b][0])
                + s.iter()
                    .enumerate()
                    .map(|(i, &x)| x * f64::from(p[o][index(i)]))
                    .sum::<f64>()
                + hidden
                    .iter()
                    .zip(&p[w])
                    .map(|(&x, &w)| x * f64::from(w))
                    .sum::<f64>()
        };
        let open = endpoint(
            Parameter::OutputWeights,
            Parameter::HiddenOutput,
            Parameter::OutputBias,
        );
        let nonlinear = if model.shape().head().phase2() {
            (q * open
                + (90.0 - q)
                    * endpoint(
                        Parameter::OutputWeightsEnd,
                        Parameter::HiddenOutputEnd,
                        Parameter::OutputBiasEnd,
                    ))
                / 90.0
        } else {
            open
        };
        (q * linear_sums[0] + (90.0 - q) * linear_sums[1])
            / 90.0
            / f64::from(model.shape().cp_per_logit())
            + nonlinear
    };
    let targets = build_targets(batch, &[700.0], &[1.0]).unwrap();
    let mut loss = 0.0;
    for (b, &target) in targets.iter().enumerate() {
        let before = value(b);
        loss += before.max(0.0) - before * f64::from(target) + (-before.abs()).exp().ln_1p();
    }
    loss / batch.len() as f64
}

#[test]
fn finite_differences_cover_all_parameter_groups() {
    let fixture = Fixture::new(&positions());
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let batch = fixture.batch(set, &[0, 1, 2]);
        for head in HeadKind::ALL {
            let mut model = nonzero_model_with_head(set, head);
            let gradients = loss_and_gradients(
                &model,
                &batch,
                &build_targets(&batch, &[700.0], &[1.0]).unwrap(),
            )
            .unwrap();
            close(gradients.losses.bce, dense_loss(&model, &batch), 3e-5);
            for p in Parameter::ALL {
                let mut indices: Vec<_> = gradients.parameters[p]
                    .iter()
                    .enumerate()
                    .filter(|(_, g)| g.abs() > 1e-7)
                    .map(|(i, _)| i)
                    .collect();
                if matches!(
                    p,
                    Parameter::Linear | Parameter::FirstLayer | Parameter::Virtual
                ) && indices.len() > 16
                {
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
                    let plus = dense_loss(&model, &batch);
                    model.parameter_mut(p)[i] = original - epsilon;
                    let minus_value = model.parameters()[p][i];
                    let minus = dense_loss(&model, &batch);
                    model.parameter_mut(p)[i] = original;
                    let expected = (plus - minus) / f64::from(plus_value - minus_value);
                    let actual = f64::from(gradients.parameters[p][i]);
                    assert!(
                        (actual - expected).abs() <= 8e-5 + expected.abs() * 3e-4,
                        "{set:?} {head:?} {p:?}[{i}] actual={actual}, finite difference={expected}"
                    );
                }
            }
        }
    }
}

#[test]
fn quantization_evaluation_uses_double_precision_at_every_stage() {
    let fixture = Fixture::new(&[position(
        &[
            (0, 0, Color::Black, PieceKind::Pawn),
            (1, 0, Color::Black, PieceKind::Pawn),
            (2, 0, Color::Black, PieceKind::Pawn),
        ],
        Color::Black,
        None,
    )]);
    let epsilon = 2.0_f32.powi(-24);
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let batch = fixture.batch(set, &[0]);
        let mut zero = model(set, 2);
        for p in Parameter::ALL {
            zero.parameter_mut(p).fill(0.0);
        }
        let mut linear = zero.clone();
        let mut hidden = zero.clone();
        for (col, value) in [1.0, epsilon, -1.0].into_iter().enumerate() {
            let row = batch.linear_features()[col] as usize;
            linear.parameter_mut(Parameter::Linear)[row * 2..row * 2 + 2].fill(value);
            let feature = batch.features()[col];
            let row = match set {
                InputFeatureSet::P => feature as usize / 2,
                InputFeatureSet::HalfKpHmB8 => feature as usize,
            };
            hidden.parameter_mut(Parameter::FirstLayer)[row * 2..row * 2 + 2].fill(value);
        }
        let units = hidden.shape().units();
        hidden.parameter_mut(Parameter::OutputWeights)[..units].fill(1.0);
        // 1 + 2^-24 - 1は、線形和でも隠れ和でも倍精度なら2^-24を保つ。
        for (model, expected) in [
            (linear, f64::from(epsilon)),
            (hidden, 2.0 * f64::from(epsilon).powi(2) * 1000.0),
        ] {
            assert_eq!(reference::evaluate_f64(&model, &batch).unwrap(), [expected]);
            assert_eq!(reference::evaluate(&model, &batch).unwrap(), [0.0]);
        }

        let mut phased = zero.clone();
        let row = batch.linear_features()[0] as usize;
        phased.parameter_mut(Parameter::Linear)[row * 2] = 3.0;
        // 駒数3なので中盤比は1/90。位相比も倍精度で求める。
        close(
            reference::evaluate_f64(&phased, &batch).unwrap()[0],
            1.0 / 30.0,
            1e-17,
        );

        let mut nonlinear = zero;
        nonlinear.parameter_mut(Parameter::Biases).fill(0.3);
        nonlinear.parameter_mut(Parameter::OutputWeights).fill(0.11);
        nonlinear.parameter_mut(Parameter::OutputBias)[0] = 0.04;
        // 2視点×2ユニットについて、活性化・出力積・バイアス・cp_per_logit換算をf64で行う。
        let head = 2.0 * f64::from(0.3_f32).powi(2) * f64::from(0.11_f32);
        let expected = ((f64::from(0.04_f32) + head) + head) * 1000.0;
        assert_eq!(
            reference::evaluate_f64(&nonlinear, &batch).unwrap(),
            [expected]
        );
        assert!(
            (f64::from(reference::evaluate(&nonlinear, &batch).unwrap()[0]) - expected).abs()
                > 1e-6
        );
    }
}

#[test]
fn reference_entrypoints_reject_all_malformed_intermediate_array_lengths() {
    // 公開の中間配列は、欠落と余分な要素のどちらも各消費入口でErrにする。
    macro_rules! malformed_fields {
        ($value:ident, $check:ident, $($field:ident),+ $(,)?) => {
            $(
                let saved = std::mem::take(&mut $value.$field);
                $check(&$value);
                $value.$field = saved.clone();
                $value.$field.push(saved[0]);
                $check(&$value);
                $value.$field = saved;
            )+
        };
    }
    let fixture = Fixture::new(&positions()[..1]);
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let batch = fixture.batch(set, &[0]);
        let model = model_with_head(set, 2, HeadKind::ScreluHidden16Phase2);
        let weights = reference::fold(&model).unwrap();
        let mut forward = reference::forward(&model, &batch, &weights).unwrap();
        let reference = RemovalReference::v0();
        let mut output = reference::output_backward(&model, &batch, &forward, &[0.5]).unwrap();
        let mut gradient = Parameters::zeros(model.shape());
        let unchanged = gradient.clone();
        let check_forward = |bad: &reference::Forward| {
            assert!(reference::output_backward(&model, &batch, bad, &[0.5]).is_err());
            assert!(reference::validation_forward(&model, &batch, bad, &[0.5]).is_err());
        };
        malformed_fields!(forward, check_forward, acc, lin);
        let mut check_output = |bad: &reference::Output| {
            assert!(reference::reduce_partials(&model, bad, &mut gradient).is_err());
            assert!(reference::first_layer_backward(&model, &batch, bad, &mut gradient).is_err());
            assert!(reference::linear_backward(&model, &batch, bad, &mut gradient).is_err());
        };
        malformed_fields!(
            output,
            check_output,
            f,
            h,
            logits,
            loss,
            g,
            d_acc,
            d_output,
            d_output_bias,
            d_output_end,
            d_output_bias_end,
            d_hidden_weights,
            d_hidden_biases,
            d_hidden_output,
            d_hidden_output_end,
            c
        );
        assert_eq!(gradient, unchanged);

        let short_weights = &weights[..weights.len() - 1];
        assert!(reference::forward(&model, &batch, short_weights).is_err());
        for targets in [vec![], vec![0.5; 2]] {
            assert!(reference::output_backward(&model, &batch, &forward, &targets).is_err());
            assert!(reference::validation_forward(&model, &batch, &forward, &targets).is_err());
        }
        assert!(build_targets(&batch, &[], &[1.0]).is_err());
        assert!(reference.deltas(&batch, batch.len()).is_err());
        let shape = minase_trainer::nnue::model::Shape::new(
            set,
            4,
            1000.0,
            minase_trainer::nnue::model::HeadKind::DirectScrelu,
        )
        .unwrap();
        let mut wrong_gradient = Parameters::zeros(shape);
        assert!(reference::reduce_partials(&model, &output, &mut wrong_gradient).is_err());
        assert!(
            reference::first_layer_backward(&model, &batch, &output, &mut wrong_gradient).is_err()
        );
        assert!(reference::linear_backward(&model, &batch, &output, &mut wrong_gradient).is_err());
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
            let w = reference::fold(&model).unwrap();
            let f = reference::forward(&model, batch, &w).unwrap();
            let o = reference::output_backward(&model, batch, &f, &[0.7]).unwrap();
            let mut g = minase_trainer::nnue::model::Parameters::zeros(model.shape());
            reference::reduce_partials(&model, &o, &mut g).unwrap();
            reference::first_layer_backward(&model, batch, &o, &mut g).unwrap();
            reference::linear_backward(&model, batch, &o, &mut g).unwrap();
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
    use minase_trainer::nn::AdamCoefficients;
    // AdamCoefficientsの契約式から、g=(1/2,-1/4), 0, (-1/2,1/4)の
    // モーメントを手計算する。減衰はモーメントに混ぜず、重みだけに掛かる。
    for decay in [0.0, 0.1] {
        let mut p = [2.0_f32, -3.0];
        let mut m = [0.0; 2];
        let mut v = [0.0; 2];
        let group = AdamWGroup {
            lr: 0.01,
            weight_decay: decay,
        };
        let cases = [
            (
                [0.5, -0.25],
                [0.05, -0.025],
                [0.00025_f64, 0.0000625],
                0.1,
                0.001,
            ),
            (
                [0.0, 0.0],
                [0.045, -0.0225],
                [0.00024975, 0.0000624375],
                0.19,
                0.001999,
            ),
            (
                [-0.5, 0.25],
                [-0.0095, 0.00475],
                [0.00049950025, 0.0001248750625],
                0.271,
                0.002997001,
            ),
        ];
        for (step, (gradient, expected_m, expected_v, c1, c2)) in cases.into_iter().enumerate() {
            let coefficients = AdamCoefficients::new(step as u64 + 1, group).unwrap();
            for i in 0..2 {
                let old = f64::from(p[i]);
                let expected = old * (1.0 - 0.01 * decay)
                    - 0.01 * (expected_m[i] / c1) / ((expected_v[i] / c2).sqrt() + 1e-8);
                coefficients.update(&mut p[i], gradient[i], &mut m[i], &mut v[i]);
                assert!(
                    (f64::from(p[i]) - expected).abs() < [2e-7, 3e-7, 4e-7][step],
                    "decay={decay}, step={step}, element={i}"
                );
                assert!(
                    (f64::from(m[i]) - expected_m[i]).abs() < 1e-8,
                    "decay={decay}, step={step}, element={i}: m"
                );
                assert!(
                    (f64::from(v[i]) - expected_v[i]).abs() < 1e-10,
                    "decay={decay}, step={step}, element={i}: v"
                );
                if step == 1 {
                    assert!((f64::from(p[i]) - old * (1.0 - 0.01 * decay)) * expected_m[i] < 0.0);
                }
            }
        }
    }
}

#[test]
fn adamw_preserves_unobserved_parameters_and_moments_without_clipping() {
    // AdamW::updateは訓練集合で未観測の行のp,m,vを保持する一方、
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
        let folded = reference::fold(&model).unwrap();
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
fn integer_reference_uses_material_and_phase_truncation() {
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
    let (delta, _) = RemovalReference::v0().deltas(&batch, 0).unwrap();
    assert_eq!(delta[25], -100);
    assert_eq!(delta[38], 100);
    assert_eq!(delta[0], -2600);
    let mut endpoints = vec![[0; 2]; FEATURE_COUNT];
    // 3駒なので除去前q=1、除去後q=0。mg=8,eg=1の歩を1枚だけ置く。
    let p = fixture.dataset.record(0).unwrap().to_position().unwrap();
    let sq = Square::new(1, 2).unwrap();
    let feature = feature_index(Color::Black, p.piece_at(sq).unwrap(), sq);
    endpoints[feature] = [8, 1];
    endpoints[mirror_feature(feature)] = [8, 1];
    let (delta, magnitudes) = RemovalReference::new(&endpoints)
        .unwrap()
        .deltas(&batch, 0)
        .unwrap();
    assert_eq!(delta[25], 0);
    close(magnitudes[25], 97.0 / 720.0, 1e-15);
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
            let (deltas, _) = RemovalReference::new(&endpoints)
                .unwrap()
                .deltas(&batch, 0)
                .unwrap();
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
        let (deltas, magnitudes) = RemovalReference::v0().deltas(&batch, 0).unwrap();
        assert_eq!(deltas[0], expected);
        assert_eq!(magnitudes[0], 750.0);
    }
}

fn assert_parameter_bits(actual: &Parameters, expected: &Parameters) {
    assert_eq!(actual.shape(), expected.shape());
    for parameter in Parameter::ALL {
        for (index, (&actual, &expected)) in actual[parameter]
            .iter()
            .zip(&expected[parameter])
            .enumerate()
        {
            assert_eq!(
                actual.to_bits(),
                expected.to_bits(),
                "{parameter:?}[{index}]"
            );
        }
    }
}

#[test]
fn cpu_threads_preserve_every_loss_gradient_parameter_and_moment_bit_for_three_updates() {
    // loss_and_gradients_parallelでは、処理の分割数を変えても縮約と3更新の軌跡は変わらない。
    // 7局面を2本と5本に分け、余りのある分割と同じ行の反復出現を含める。
    let fixture = Fixture::new(&positions());
    let config = AdamWConfig {
        linear: AdamWGroup {
            lr: 0.017,
            weight_decay: 0.021,
        },
        ft: AdamWGroup {
            lr: 0.0007,
            weight_decay: 0.004,
        },
        head: AdamWGroup {
            lr: 0.0013,
            weight_decay: 0.009,
        },
    };
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let mut serial = nonzero_model(set);
        let mut serial_optimizer = AdamW::new(serial.shape(), config).unwrap();
        let mut parallel: Vec<_> = [1, 2, 5]
            .into_iter()
            .map(|threads| (threads, serial.clone(), serial_optimizer.clone()))
            .collect();
        for indices in [
            [0, 1, 2, 0, 1, 2, 1],
            [2, 1, 0, 1, 2, 0, 2],
            [1, 0, 2, 2, 1, 0, 0],
        ] {
            let batch = fixture.batch(set, &indices);
            let targets = build_targets(&batch, &[700.0], &[0.75]).unwrap();
            let expected = loss_and_gradients(&serial, &batch, &targets).unwrap();
            serial_optimizer
                .update(&mut serial, &expected.parameters)
                .unwrap();
            for (threads, model, optimizer) in &mut parallel {
                let actual =
                    reference::loss_and_gradients_parallel(model, &batch, &targets, *threads)
                        .unwrap();
                assert_eq!(actual.losses.bce.to_bits(), expected.losses.bce.to_bits());
                assert_parameter_bits(&actual.parameters, &expected.parameters);
                optimizer
                    .update_parallel(model, &actual.parameters, *threads, 1.0)
                    .unwrap();
                assert_eq!(optimizer.step(), serial_optimizer.step());
                assert_parameter_bits(model.parameters(), serial.parameters());
                assert_parameter_bits(optimizer.m(), serial_optimizer.m());
                assert_parameter_bits(optimizer.v(), serial_optimizer.v());
            }
        }
    }
}

#[test]
fn cpu_rejects_zero_threads_without_changing_state() {
    let fixture = Fixture::new(&[position(
        &[(1, 2, Color::Black, PieceKind::Pawn)],
        Color::Black,
        None,
    )]);
    let batch = fixture.batch(InputFeatureSet::P, &[0]);
    let mut model = model(InputFeatureSet::P, 2);
    for parameter in Parameter::ALL {
        model.parameter_mut(parameter).fill(0.0);
    }
    assert!(reference::loss_and_gradients_parallel(&model, &batch, &[0.5], 0).is_err());
    let mut optimizer = AdamW::new(model.shape(), AdamWConfig::default()).unwrap();
    let unchanged = model.clone();
    let before = optimizer.clone();
    let gradient = Parameters::zeros(model.shape());
    assert!(
        optimizer
            .update_parallel(&mut model, &gradient, 0, 1.0)
            .is_err()
    );
    assert_eq!(model, unchanged);
    assert_eq!(optimizer, before);
}
