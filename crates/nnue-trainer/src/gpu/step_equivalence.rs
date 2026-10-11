//! カーネル8の契約を、CPU参照の1更新と3更新、固定行、切り詰めで検査する。

use super::{Device, Error, HostRing, Result, Step, StepResult, equivalence::Report};
use crate::{
    data::{BatchSlot, CANONICAL_ROWS, InputFeatureSet},
    model::{Masks, Model, Parameter, Parameters, Shape},
    reference::{self, AdamW, AdamWConfig, AdamWGroup, Losses, RemovalReference},
};
use minase::eval::features::FEATURE_COUNT;

use super::equivalence::Random;

struct IdleProbe {
    parameter: Parameter,
    index: usize,
    decay_only: bool,
}

struct Fixture {
    model: Model,
    optimizer: AdamW,
    idle: Vec<IdleProbe>,
}

/// CLIとGPUテストで同じ全更新の検査を使う。
pub(super) fn run(device: Device, shape: Shape, batch: &BatchSlot, seed: u64) -> Result<Report> {
    let fixture = fixture(shape, batch, seed)?;
    let targets = reference::targets(batch, &[500.0, 1000.0, 1500.0])?;
    let mut report = check(device.clone(), &fixture, batch, &targets)?;
    drop(fixture);
    report.merge(check_clipping(device, shape, seed)?);
    Ok(report)
}

fn fixture(shape: Shape, batch: &BatchSlot, seed: u64) -> Result<Fixture> {
    let mut random = Random(seed ^ 0x4b31_79de_725f_c608);
    let first_present: Vec<_> = batch
        .first_layer_inverse()
        .starts()
        .windows(2)
        .map(|range| range[0] != range[1])
        .collect();
    let linear_present: Vec<_> = batch
        .linear_inverse()
        .starts()
        .windows(2)
        .map(|range| range[0] != range[1])
        .collect();
    let mut base_present = vec![false; FEATURE_COUNT];
    if shape.feature_set() == InputFeatureSet::HalfKpHmB8 {
        for (row, &present) in first_present[..shape.feature_set().dimensions()]
            .iter()
            .enumerate()
        {
            base_present[row % FEATURE_COUNT] |= present;
        }
    }
    // このバッチで使う行は観測済みに保つ。空の逆索引には観測済みと未観測を混ぜる。
    let canonical: Vec<_> = (0..CANONICAL_ROWS)
        .map(|row| {
            row + 1 < CANONICAL_ROWS
                && (linear_present[row]
                    || shape.feature_set() == InputFeatureSet::P && first_present[row]
                    || row % 7 != 0)
        })
        .collect();
    let dimensions = shape.feature_set().dimensions();
    let input: Vec<_> = (0..=dimensions)
        .map(|row| {
            row < dimensions
                && (shape.feature_set() == InputFeatureSet::P
                    || first_present[row]
                    || row % 7 != 0
                        && (base_present[row % FEATURE_COUNT]
                            || !(row % FEATURE_COUNT).is_multiple_of(19)))
        })
        .collect();
    let masks = Masks::new(shape.feature_set(), canonical, input)?;
    let initial: Vec<_> = (0..FEATURE_COUNT)
        .map(|_| {
            let value = random.centered();
            [value.signum() * (2.0 + value.abs() * 4.0); 2]
        })
        .collect();
    let mut model = Model::initialize(shape, &initial, masks, seed, 0.001)?;
    for pair in model
        .parameter_mut(Parameter::Linear)
        .as_chunks_mut::<2>()
        .0
    {
        let value = random.centered();
        pair.fill(value.signum() * (2.0 + value.abs() * 4.0));
    }
    for parameter in [Parameter::FirstLayer, Parameter::Virtual] {
        for value in model.parameter_mut(parameter) {
            *value = random.centered() * 0.0006;
        }
    }
    for value in model.parameter_mut(Parameter::Biases) {
        *value = 0.4 + random.centered() * 0.15;
    }
    for value in model.parameter_mut(Parameter::OutputWeights) {
        *value = random.centered() * 0.003 / shape.width() as f32;
    }
    model.parameter_mut(Parameter::OutputBias)[0] = random.centered() * 0.1;
    model.fix_unobserved();

    let mut optimizer = AdamW::new(
        shape,
        AdamWConfig {
            linear: AdamWGroup {
                lr: 0.03,
                weight_decay: 0.02,
            },
            ..AdamWConfig::default()
        },
    )?;
    optimizer.step = 7;
    for parameter in Parameter::ALL {
        for value in &mut optimizer.m[parameter] {
            let draw = random.centered();
            *value = draw.signum() * (0.005 + draw.abs() * 0.01);
        }
        for value in &mut optimizer.v[parameter] {
            *value = 0.02 + random.centered().abs() * 0.02;
        }
    }
    let mut idle = Vec::new();
    for parameter in [Parameter::Linear, Parameter::FirstLayer, Parameter::Virtual] {
        let (mask, present, width) = match parameter {
            Parameter::Linear => (model.masks().canonical(), &linear_present, 2),
            Parameter::FirstLayer => (model.masks().first_layer(), &first_present, shape.width()),
            Parameter::Virtual => (model.masks().virtual_rows(), &base_present, shape.width()),
            _ => unreachable!(),
        };
        let rows: Vec<_> = mask
            .iter()
            .zip(present)
            .enumerate()
            .filter_map(|(row, (&seen, &used))| (seen && !used).then_some(row))
            .take(2)
            .collect();
        if rows.len() != 2
            && (parameter != Parameter::Virtual
                || shape.feature_set() == InputFeatureSet::HalfKpHmB8)
        {
            return Err(Error::Equivalence(
                "missing zero-gradient fixture rows".into(),
            ));
        }
        for (probe, row) in rows.into_iter().enumerate() {
            let index = row * width;
            model.parameter_mut(parameter)[index] = 1.5;
            optimizer.m[parameter][index] = if probe == 0 { 0.25 } else { 0.0 };
            optimizer.v[parameter][index] = 0.125;
            idle.push(IdleProbe {
                parameter,
                index,
                decay_only: probe == 1,
            });
        }
    }
    Ok(Fixture {
        model,
        optimizer,
        idle,
    })
}

fn check(device: Device, fixture: &Fixture, batch: &BatchSlot, targets: &[f32]) -> Result<Report> {
    let initial = &fixture.model;
    let shape = initial.shape();
    let removal = RemovalReference::v0();
    let mut cpu = initial.clone();
    let mut cpu_optimizer = fixture.optimizer.clone();
    let mut first = Step::new(
        device.clone(),
        initial,
        &fixture.optimizer,
        &removal,
        batch.len(),
    )?;
    let mut repeated = Step::new(
        device.clone(),
        initial,
        &fixture.optimizer,
        &removal,
        batch.len(),
    )?;
    let mut first_ring = HostRing::new(&device, shape.feature_set(), batch.len())?;
    let mut repeated_ring = HostRing::new(&device, shape.feature_set(), batch.len())?;
    let mut report = Report {
        device_name: device.name().into(),
        comparisons: 0,
        max_absolute_error: 0.0,
        max_relative_error: 0.0,
    };
    let mut pending = None;
    for update in 0..3 {
        let gradients = cpu_gradients(&cpu, batch, targets, &removal)?;
        cpu_optimizer.update(&mut cpu, &gradients.parameters)?;
        let first_ticket = first_ring.upload(batch)?;
        let first_result = first.enqueue(&mut first_ring, &first_ticket, targets)?;
        let repeated_ticket = repeated_ring.upload(batch)?;
        let repeated_result = repeated.enqueue(&mut repeated_ring, &repeated_ticket, targets)?;
        // 前の損失は次の更新を投入してから回収する。末尾だけループ外で回収する。
        if let Some(previous) = pending.take() {
            collect(&mut report, previous)?;
        }
        let (parameters, optimizer) = first.snapshot()?;
        let (again, again_optimizer) = repeated.snapshot()?;
        compare_state(&mut report, &cpu, &cpu_optimizer, &parameters, &optimizer)?;
        compare_state_bits(&parameters, &optimizer, &again, &again_optimizer)?;
        frozen_rows(fixture, &parameters, &optimizer)?;
        if update == 0 {
            idle_rows(fixture, &gradients.parameters, &parameters, &optimizer)?;
        }
        compare_expanded(&mut report, &cpu, &first, &repeated)?;
        pending = Some((
            first_result,
            repeated_result,
            gradients.losses,
            cpu_optimizer.step(),
        ));
    }
    if let Some(last) = pending {
        collect(&mut report, last)?;
    }
    Ok(report)
}

fn cpu_gradients(
    model: &Model,
    batch: &BatchSlot,
    targets: &[f32],
    removal_reference: &RemovalReference,
) -> Result<reference::Gradients> {
    let weights = reference::fold(model);
    let forward = reference::forward(model, batch, &weights)?;
    let removal = reference::removal_forward(model, batch, &weights, &forward, removal_reference)?;
    let output = reference::output_backward(model, batch, &forward, &removal, targets)?;
    let mut parameters = Parameters::zeros(model.shape());
    let losses = reference::reduce_partials(model, &output, &removal, &mut parameters);
    reference::first_layer_backward(
        model,
        batch,
        &weights,
        &forward,
        &removal,
        &output,
        &mut parameters,
    );
    reference::linear_backward(model, batch, &removal, &output, &mut parameters);
    reference::reduce_virtual(&mut parameters);
    Ok(reference::Gradients { losses, parameters })
}

fn clipping_fixture(shape: Shape, batch: &BatchSlot, seed: u64, sign: f32) -> Result<Fixture> {
    let mut fixture = fixture(shape, batch, seed)?;
    fixture.idle.clear();
    let model = &mut fixture.model;
    let unused = |mask: &[bool], starts: &[u32]| {
        mask.iter()
            .zip(starts.windows(2))
            .position(|(&seen, range)| seen && range[0] == range[1])
            .ok_or_else(|| Error::Equivalence("missing clipping fixture row".into()))
    };
    let linear = unused(model.masks().canonical(), batch.linear_inverse().starts())?;
    model.parameter_mut(Parameter::Linear)[2 * linear..2 * linear + 2]
        .copy_from_slice(&[4100.0, -4100.0]);
    let first = unused(
        model.masks().first_layer(),
        batch.first_layer_inverse().starts(),
    )?;
    for (j, value) in model.parameter_mut(Parameter::FirstLayer)
        [first * shape.width()..(first + 1) * shape.width()]
        .iter_mut()
        .enumerate()
    {
        *value = if j % 2 == 0 { 12.0 } else { -12.0 };
    }
    if shape.feature_set() == InputFeatureSet::HalfKpHmB8 {
        // Wの前にVを±8へ切り詰める必要がある。古いVでWを制限すると和が±7になる。
        let base = first % FEATURE_COUNT;
        for (j, value) in model.parameter_mut(Parameter::Virtual)
            [base * shape.width()..(base + 1) * shape.width()]
            .iter_mut()
            .enumerate()
        {
            *value = if j % 2 == 0 { 9.0 } else { -9.0 };
        }
    }
    for parameter in [
        Parameter::Biases,
        Parameter::OutputWeights,
        Parameter::OutputBias,
    ] {
        let limit = if parameter == Parameter::OutputWeights {
            2.5
        } else {
            9.0
        };
        for (j, value) in model.parameter_mut(parameter).iter_mut().enumerate() {
            *value = sign * if j % 2 == 0 { limit } else { -limit };
        }
    }
    // マスクした実行行は範囲外でもp/m/vを保持する。埋め草はこの操作の対象外。
    for parameter in [Parameter::Linear, Parameter::FirstLayer, Parameter::Virtual] {
        let mask = match parameter {
            Parameter::Linear => model.masks().canonical(),
            Parameter::FirstLayer => model.masks().first_layer(),
            Parameter::Virtual => model.masks().virtual_rows(),
            _ => unreachable!(),
        };
        if mask.is_empty() {
            continue;
        }
        let row = mask[..mask.len() - 1]
            .iter()
            .position(|&seen| !seen)
            .ok_or_else(|| Error::Equivalence("missing frozen clipping fixture row".into()))?;
        let width = if parameter == Parameter::Linear {
            2
        } else {
            shape.width()
        };
        let value = if parameter == Parameter::Linear {
            5000.0
        } else {
            12.0
        };
        model.parameter_mut(parameter)[row * width..(row + 1) * width].fill(sign * value);
    }
    Ok(fixture)
}

fn check_clipping(device: Device, shape: Shape, seed: u64) -> Result<Report> {
    let batch = super::equivalence::synthetic_batch(shape.feature_set(), 3, seed)?;
    let targets = reference::targets(&batch, &[500.0, 1000.0, 1500.0])?;
    let positive = clipping_fixture(shape, &batch, seed, 1.0)?;
    let mut report = check(device.clone(), &positive, &batch, &targets)?;
    drop(positive);
    let negative = clipping_fixture(shape, &batch, seed, -1.0)?;
    report.merge(check(device, &negative, &batch, &targets)?);
    Ok(report)
}

fn collect(report: &mut Report, pending: (StepResult, StepResult, Losses, u64)) -> Result<()> {
    let (first, repeated, expected, step) = pending;
    let first = first.collect()?;
    let repeated = repeated.collect()?;
    if first.step != step || repeated.step != step {
        return Err(Error::Equivalence("collected update number".into()));
    }
    for (name, expected, actual, again) in [
        (
            "step BCE",
            expected.bce,
            first.losses.bce,
            repeated.losses.bce,
        ),
        (
            "step auxiliary",
            expected.auxiliary,
            first.losses.auxiliary,
            repeated.losses.auxiliary,
        ),
    ] {
        report.number(name, step as usize, expected, actual)?;
        if actual.to_bits() != again.to_bits() {
            return Err(Error::Equivalence(format!(
                "{name}: repeated updates differ in bits"
            )));
        }
    }
    Ok(())
}

fn compare_state(
    report: &mut Report,
    cpu: &Model,
    cpu_optimizer: &AdamW,
    parameters: &Parameters,
    optimizer: &AdamW,
) -> Result<()> {
    if optimizer.step() != cpu_optimizer.step() || optimizer.config() != cpu_optimizer.config() {
        return Err(Error::Equivalence(
            "AdamW update number or configuration".into(),
        ));
    }
    for parameter in Parameter::ALL {
        for (kind, expected, actual) in [
            ("parameter", cpu.parameters(), parameters),
            ("first moment", cpu_optimizer.m(), optimizer.m()),
            ("second moment", cpu_optimizer.v(), optimizer.v()),
        ] {
            report.compare(
                &format!("step {} {kind} {}", optimizer.step(), parameter.name()),
                &expected[parameter],
                &actual[parameter],
            )?;
        }
    }
    Ok(())
}

fn compare_state_bits(a: &Parameters, a_opt: &AdamW, b: &Parameters, b_opt: &AdamW) -> Result<()> {
    if a_opt.step() != b_opt.step() || a_opt.config() != b_opt.config() {
        return Err(Error::Equivalence("repeated AdamW metadata".into()));
    }
    for parameter in Parameter::ALL {
        for (kind, first, second) in [
            ("parameter", a, b),
            ("first moment", a_opt.m(), b_opt.m()),
            ("second moment", a_opt.v(), b_opt.v()),
        ] {
            bits(
                &format!("repeated {kind} {}", parameter.name()),
                &first[parameter],
                &second[parameter],
            )?;
        }
    }
    Ok(())
}

fn compare_expanded(report: &mut Report, cpu: &Model, first: &Step, repeated: &Step) -> Result<()> {
    let (weights, biases, output) = first.snapshot_expanded()?;
    let (again_weights, again_biases, again_output) = repeated.snapshot_expanded()?;
    for (name, expected, actual, again) in [
        (
            "folded first layer",
            reference::fold(cpu),
            weights,
            again_weights,
        ),
        (
            "expanded biases",
            cpu.expanded_biases(),
            biases,
            again_biases,
        ),
        (
            "expanded output weights",
            cpu.expanded_output_weights(),
            output,
            again_output,
        ),
    ] {
        report.compare(name, &expected, &actual)?;
        bits(name, &actual, &again)?;
    }
    Ok(())
}

fn bits(name: &str, first: &[f32], second: &[f32]) -> Result<()> {
    if first.len() != second.len()
        || first
            .iter()
            .zip(second)
            .any(|(a, b)| a.to_bits() != b.to_bits())
    {
        return Err(Error::Equivalence(format!("{name}: values differ in bits")));
    }
    Ok(())
}

fn frozen_rows(fixture: &Fixture, parameters: &Parameters, optimizer: &AdamW) -> Result<()> {
    let model = &fixture.model;
    for (parameter, mask, width) in [
        (Parameter::Linear, model.masks().canonical(), 2),
        (
            Parameter::FirstLayer,
            model.masks().first_layer(),
            model.shape().width(),
        ),
        (
            Parameter::Virtual,
            model.masks().virtual_rows(),
            model.shape().width(),
        ),
    ] {
        for (kind, initial, updated) in [
            ("parameter", model.parameters(), parameters),
            ("first moment", fixture.optimizer.m(), optimizer.m()),
            ("second moment", fixture.optimizer.v(), optimizer.v()),
        ] {
            for (row, &observed) in mask.iter().enumerate() {
                if !observed {
                    let range = row * width..(row + 1) * width;
                    bits(
                        &format!("unobserved {kind} {} row {row}", parameter.name()),
                        &initial[parameter][range.clone()],
                        &updated[parameter][range],
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn idle_rows(
    fixture: &Fixture,
    gradient: &Parameters,
    parameters: &Parameters,
    optimizer: &AdamW,
) -> Result<()> {
    for probe in &fixture.idle {
        let p = probe.parameter;
        let i = probe.index;
        if gradient[p][i] != 0.0 {
            return Err(Error::Equivalence(
                "idle fixture has a nonzero gradient".into(),
            ));
        }
        if parameters[p][i] >= fixture.model.parameters()[p][i] {
            return Err(Error::Equivalence(format!(
                "zero-gradient {} did not update",
                p.name()
            )));
        }
        if probe.decay_only && optimizer.m()[p][i] != 0.0 {
            return Err(Error::Equivalence("decay-only first moment changed".into()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::equivalence::synthetic_batch;

    // 指示書の幅と、縮約ブロックに満たない/またがるバッチで同じ契約を検査する。
    fn matrix(set: InputFeatureSet, width: usize) {
        let device = Device::new(0).unwrap();
        let shape = Shape::new(set, width, 1000.0).unwrap();
        for batch_size in [1, 64, 129, 4096] {
            let batch = synthetic_batch(set, batch_size, 137 + batch_size as u64).unwrap();
            let fixture = fixture(shape, &batch, 293).unwrap();
            let targets = reference::targets(&batch, &[500.0, 1000.0, 1500.0]).unwrap();
            check(device.clone(), &fixture, &batch, &targets).unwrap();
        }
        check_clipping(device, shape, 293).unwrap();
    }

    #[test]
    fn step_inputs_cover_zero_gradients_and_unobserved_moments() {
        for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
            let shape = Shape::new(set, 6, 1000.0).unwrap();
            let batch = synthetic_batch(set, 129, 71).unwrap();
            let fixture = fixture(shape, &batch, 293).unwrap();
            let targets = reference::targets(&batch, &[500.0, 1000.0, 1500.0]).unwrap();
            let gradients =
                cpu_gradients(&fixture.model, &batch, &targets, &RemovalReference::v0()).unwrap();
            assert!(gradients.losses.auxiliary > 0.0);
            assert!(
                gradients.parameters[Parameter::FirstLayer]
                    .iter()
                    .any(|&g| g != 0.0)
            );
            assert!(
                gradients.parameters[Parameter::Biases]
                    .iter()
                    .any(|&g| g != 0.0)
            );
            let mut updated = fixture.model.clone();
            let mut optimizer = fixture.optimizer.clone();
            optimizer
                .update(&mut updated, &gradients.parameters)
                .unwrap();
            idle_rows(
                &fixture,
                &gradients.parameters,
                updated.parameters(),
                &optimizer,
            )
            .unwrap();
            frozen_rows(&fixture, updated.parameters(), &optimizer).unwrap();
            for (parameter, mask, width) in [
                (Parameter::Linear, fixture.model.masks().canonical(), 2),
                (
                    Parameter::FirstLayer,
                    fixture.model.masks().first_layer(),
                    6,
                ),
                (Parameter::Virtual, fixture.model.masks().virtual_rows(), 6),
            ] {
                if !mask.is_empty() {
                    let row = mask[..mask.len() - 1]
                        .iter()
                        .position(|&seen| !seen)
                        .unwrap();
                    assert_ne!(fixture.optimizer.m()[parameter][row * width], 0.0);
                    assert_ne!(fixture.optimizer.v()[parameter][row * width], 0.0);
                }
            }
        }
    }

    #[test]
    #[ignore = "requires a CUDA GPU and NVRTC"]
    fn gpu_step_queues_distinct_targets_and_preserves_uncollected_losses() {
        let device = Device::new(0).unwrap();
        for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
            let shape = Shape::new(set, 6, 1000.0).unwrap();
            let batches: Vec<_> = [129, 64, 3]
                .into_iter()
                .map(|n| synthetic_batch(set, n, 71 + n as u64).unwrap())
                .collect();
            let targets: Vec<_> = batches
                .iter()
                .zip([0.1, 0.9, 0.35])
                .map(|(batch, value)| vec![value; batch.len()])
                .collect();
            let fixture = fixture(shape, &batches[0], 293).unwrap();
            let removal = RemovalReference::v0();
            let mut cpu = fixture.model.clone();
            let mut cpu_optimizer = fixture.optimizer.clone();
            let mut expected = Vec::new();
            for (batch, targets) in batches.iter().zip(&targets) {
                let gradient = cpu_gradients(&cpu, batch, targets, &removal).unwrap();
                cpu_optimizer
                    .update(&mut cpu, &gradient.parameters)
                    .unwrap();
                expected.push(gradient.losses);
            }
            let mut step = Step::new(
                device.clone(),
                &fixture.model,
                &fixture.optimizer,
                &removal,
                129,
            )
            .unwrap();
            let mut ring = HostRing::new(&device, set, 129).unwrap();
            let first_ticket = ring.upload(&batches[0]).unwrap();
            let first = step.enqueue(&mut ring, &first_ticket, &targets[0]).unwrap();
            let second_ticket = ring.upload(&batches[1]).unwrap();
            let second = step
                .enqueue(&mut ring, &second_ticket, &targets[1])
                .unwrap();
            let third_ticket = ring.upload(&batches[2]).unwrap();
            assert!(step.enqueue(&mut ring, &third_ticket, &targets[2]).is_err());
            assert_eq!(step.step(), fixture.optimizer.step() + 2);
            let first = first.collect().unwrap();
            let third = step.enqueue(&mut ring, &third_ticket, &targets[2]).unwrap();
            let actual = [first, second.collect().unwrap(), third.collect().unwrap()];
            let mut report = Report {
                device_name: device.name().into(),
                comparisons: 0,
                max_absolute_error: 0.0,
                max_relative_error: 0.0,
            };
            for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
                assert_eq!(actual.step, fixture.optimizer.step() + index as u64 + 1);
                report
                    .number("queued BCE", index, expected.bce, actual.losses.bce)
                    .unwrap();
                report
                    .number(
                        "queued auxiliary",
                        index,
                        expected.auxiliary,
                        actual.losses.auxiliary,
                    )
                    .unwrap();
            }
            let (parameters, optimizer) = step.snapshot().unwrap();
            compare_state(&mut report, &cpu, &cpu_optimizer, &parameters, &optimizer).unwrap();
        }
    }

    macro_rules! gpu_case {
        ($name:ident, $set:expr, $width:expr) => {
            #[test]
            #[ignore = "requires a CUDA GPU and NVRTC"]
            fn $name() {
                matrix($set, $width);
            }
        };
    }
    gpu_case!(gpu_step_p_width_2, InputFeatureSet::P, 2);
    gpu_case!(gpu_step_p_width_6, InputFeatureSet::P, 6);
    gpu_case!(gpu_step_p_width_8, InputFeatureSet::P, 8);
    gpu_case!(gpu_step_p_width_128, InputFeatureSet::P, 128);
    gpu_case!(gpu_step_p_width_130, InputFeatureSet::P, 130);
    gpu_case!(gpu_step_halfkp_width_2, InputFeatureSet::HalfKpHmB8, 2);
    gpu_case!(gpu_step_halfkp_width_6, InputFeatureSet::HalfKpHmB8, 6);
    gpu_case!(gpu_step_halfkp_width_8, InputFeatureSet::HalfKpHmB8, 8);
    gpu_case!(gpu_step_halfkp_width_128, InputFeatureSet::HalfKpHmB8, 128);
    gpu_case!(gpu_step_halfkp_width_130, InputFeatureSet::HalfKpHmB8, 130);
}
