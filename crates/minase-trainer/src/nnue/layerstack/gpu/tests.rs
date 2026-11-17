use super::engine::{BackwardBuffers, ForwardBuffers, Targets};
use super::*;
use super::{
    engine::{Evaluator, memory_bytes},
    step::Step,
};
use crate::data::{Outcome, Sample};
use crate::nnue::{
    data::{BatchSlot, InputFeatureSet},
    gpu::{
        HostRing,
        equivalence::{ATOL, RTOL},
    },
    layerstack::{
        AdamW, Features, OptimizerConfig, Scale, backward, model::Uniform,
        reference::forward_validated,
    },
};
use minase_core::{Color, PieceCode, PieceKind, PositionBuilder, Square};

fn scale() -> Scale {
    Scale::new(1072.65).unwrap()
}
fn batch(n: usize) -> BatchSlot {
    let mut rng = Uniform(493);
    let mut records = Vec::new();
    for b in 0..n {
        let side = Color::ALL[b % 2];
        let mut squares: Vec<_> = Square::all().collect();
        for j in (1..squares.len()).rev() {
            let k = (((rng.next() + 1.0) / 2.0) * (j + 1) as f64) as usize;
            squares.swap(j, k);
        }
        let mut builder = PositionBuilder::new(side);
        for (j, &square) in squares[..[2, 3, 45, 90, 144][b % 5]].iter().enumerate() {
            let kinds = [
                PieceKind::Pawn,
                PieceKind::Rook,
                PieceKind::Lion,
                PieceKind::SilverGeneral,
                PieceKind::Bishop,
            ];
            let kind = if j < 2 {
                PieceKind::King
            } else {
                kinds[(j + b) % kinds.len()]
            };
            let color = if j % 2 == 0 { side } else { side.opposite() };
            builder
                .put(square, PieceCode::new(color, kind).unwrap())
                .unwrap();
        }
        let mut position = builder.finish().unwrap();
        if b % 2 == 0 {
            position.set_lion_capture(Some(squares[1])).unwrap();
        }
        records.push(
            Sample::from_position(
                &position,
                0,
                [Outcome::Loss, Outcome::Draw, Outcome::Win][b % 3],
                0,
                b as u16,
                minase::training::records::Origin::RandomSelfplay,
            )
            .unwrap(),
        );
    }
    BatchSlot::from_records(InputFeatureSet::P, &records, &vec![0.0; n], &vec![0; n]).unwrap()
}
fn weights() -> Parameters {
    let mut w = Parameters::initialize(19, scale());
    let mut rng = Uniform(71);
    for p in Parameter::ALL {
        for v in &mut w[p][..p.active_len()] {
            *v += (rng.next()
                * if p == Parameter::FtWeights {
                    0.005
                } else {
                    0.1
                }) as f32;
        }
    }
    for v in &mut w[Parameter::FtBiases] {
        *v += 0.4;
    }
    w[Parameter::L1Biases][0] = -0.5;
    w[Parameter::L1Biases][1] = 1.5;
    w[Parameter::L2Biases][0] = -0.5;
    w[Parameter::L2Biases][1] = 1.5;
    w
}
fn close(name: &str, actual: f64, expected: f64) {
    assert!(
        actual.is_finite()
            && expected.is_finite()
            && (actual - expected).abs() <= ATOL + RTOL * expected.abs(),
        "{name}: GPU={actual:.15e}, CPU={expected:.15e}, abs={:.3e}",
        (actual - expected).abs()
    );
}
fn compare(a: &Parameters, b: &Parameters) {
    for p in Parameter::ALL {
        for (i, (&a, &b)) in a[p].iter().zip(&b[p]).enumerate() {
            close(&format!("{}[{i}]", p.name()), f64::from(a), f64::from(b));
        }
        assert!(a[p][p.active_len()..].iter().all(|v| v.to_bits() == 0));
    }
}
fn bits(a: &Parameters, b: &Parameters) {
    for p in Parameter::ALL {
        assert_eq!(
            a[p].iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            b[p].iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            "{p:?}"
        );
    }
}
fn forward_values(f: &super::super::Forward) -> Vec<f64> {
    f.a.iter()
        .flatten()
        .chain(&f.p)
        .chain(&f.y)
        .chain(&f.z)
        .chain(&f.u)
        .copied()
        .chain([f.n, f.psqt[0], f.psqt[1], f.output])
        .collect()
}

#[test]
fn cuda_sources_compile_or_report_missing_nvrtc() {
    // A missing compiler must return the typed error. A compile error is never
    // treated as absence, and GPU tests never skip themselves at runtime.
    match crate::nnue::gpu::device::compile_sources(&SOURCES, (8, 6)) {
        Ok(_) => (),
        Err(InfrastructureError::CompilerUnavailable) => {
            eprintln!("NVRTC unavailable: compile_sources returned CompilerUnavailable");
        }
        Err(e) => panic!("LayerStack CUDA compilation: {e}"),
    }
}
#[test]
fn memory_16384_uses_block_partials_and_forward_only_evaluation() {
    let n = 16384;
    let params = 1_767_514;
    assert_eq!(parameter_count(), params);
    let segments = 43_907;
    let head = 128 * 5_265 * 8;
    // Independent specification count: forward=850 floats, backward=561 floats;
    // the shared L1 contributes one gradient, copied to both parameter arrays.
    let expected = crate::nnue::gpu::ring::device_bytes(InputFeatureSet::P, n).unwrap()
        + 16 * params
        + 4 * n * (850 + 561 + 2)
        + head
        + 8 * segments * 257
        + 8 * (128 * 14 + 14)
        + 4;
    assert_eq!(memory_bytes(n, true).unwrap(), expected);
    let evaluation = memory_bytes(n, false).unwrap();
    assert_eq!(
        expected - evaluation,
        12 * params + 4 * n * (561 + 1) + head + 8 * segments * 257
    );
    assert!(head < n * 16 * 256 * 4 / 32);
    println!("LayerStack B=16384: train={expected} bytes, validation={evaluation} bytes");
    assert!(memory_bytes(0, true).is_err());
    assert!(memory_bytes(crate::nnue::data::MAX_BATCH_SIZE + 1, true).is_err());
}

#[test]
#[ignore = "requires a CUDA GPU and NVRTC"]
fn gpu_forward_backward_validation_and_repeat_bits() {
    let device = Device::new(0).unwrap();
    let w = weights();
    let mut evaluator = Evaluator::new(device.clone(), &w, scale(), 4096).unwrap();
    let k = Kernels::new(&device).unwrap();
    let dw = upload(&device, &w).unwrap();
    let mut f = ForwardBuffers::new(&device, 4096).unwrap();
    let mut d = BackwardBuffers::new(&device, 4096).unwrap();
    let mut t = Targets::new(&device, 4096).unwrap();
    let mut ring = HostRing::new(&device, InputFeatureSet::P, 4096).unwrap();
    for n in [100, 4096] {
        let batch = batch(n);
        let targets: Vec<_> = (0..n).map(|i| (i % 11) as f32 / 10.0).collect();
        let features = Features::from_batch(&batch).unwrap();
        let (loss, gradient) = backward(&w, scale(), &features, &targets).unwrap();
        let first = evaluator.evaluate(&batch, &targets).unwrap();
        let second = evaluator.evaluate(&batch, &targets).unwrap();
        close("loss", first.validation.loss(), loss);
        let (statistics, square_lower, square_upper) =
            evaluator.validate(&batch, &targets).unwrap();
        assert_eq!(statistics, first.validation);
        assert_eq!(
            (square_lower, square_upper),
            (first.square_lower, first.square_upper)
        );
        let mut stats = [0.0; 14];
        stats[0] = loss * n as f64;
        for (i, ((a, b), feature)) in first
            .forward
            .iter()
            .zip(&second.forward)
            .zip(&features)
            .enumerate()
        {
            let cpu = forward_validated(&w, feature);
            for (j, ((a, b), c)) in forward_values(a)
                .into_iter()
                .zip(forward_values(b))
                .zip(forward_values(&cpu))
                .enumerate()
            {
                close(&format!("forward[{i}][{j}]"), a, c);
                assert_eq!(a.to_bits(), b.to_bits());
            }
            if batch.outcomes()[i] != 1 {
                stats[1] += f64::from((cpu.output > 0.0) == (batch.outcomes()[i] == 2));
                stats[2] += 1.0;
            }
            let cp = cpu.n * scale().output();
            stats[3] += cp;
            stats[4] += cp * cp;
            for x in cpu.a.iter().flatten() {
                stats[5] += f64::from(*x <= 0.0);
                stats[6] += f64::from(*x >= 1.0);
            }
            for x in &cpu.y[..15] {
                stats[7] += f64::from(*x <= 0.0);
                stats[8] += f64::from(*x >= 1.0);
                let square = 127.0 / 128.0 * x * x;
                stats[12] += f64::from(square <= 0.0);
                stats[13] += f64::from(square >= 1.0);
            }
            for x in cpu.u {
                stats[9] += f64::from(x <= 0.0);
                stats[10] += f64::from(x >= 1.0);
            }
            for v in cpu.a {
                for j in 0..128 {
                    stats[11] += f64::from(v[j] > 0.0 && v[j + 128] > 0.0);
                }
            }
        }
        for ((a, b), expected) in first
            .validation
            .sums()
            .into_iter()
            .chain([first.square_lower as f64, first.square_upper as f64])
            .zip(
                second
                    .validation
                    .sums()
                    .into_iter()
                    .chain([second.square_lower as f64, second.square_upper as f64]),
            )
            .zip(stats)
        {
            close("validation", a, expected);
            assert_eq!(a.to_bits(), b.to_bits());
        }
        let mut previous = None;
        for _ in 0..2 {
            t.upload(&device, &targets).unwrap();
            let ticket = ring.upload(&batch).unwrap();
            let input = ring.input(&ticket).unwrap();
            f.launch(&device, &k, &dw, input, &t.values, scale())
                .unwrap();
            d.launch(&device, &k, (&dw, scale()), input, &t.values, &mut f)
                .unwrap();
            ring.finish(ticket).unwrap();
            check_status(&device, &f.nonfinite).unwrap();
            let actual = download(&device, &d.gradients).unwrap();
            compare(&actual, &gradient);
            if let Some(previous) = previous {
                bits(&actual, &previous);
            }
            previous = Some(actual);
        }
    }
}

#[test]
#[ignore = "requires a CUDA GPU and NVRTC"]
fn gpu_one_three_updates_reproducibility_and_snapshot_resume() {
    let device = Device::new(0).unwrap();
    for n in [100, 4096] {
        let batch = batch(n);
        let features = Features::from_batch(&batch).unwrap();
        let values: Vec<_> = (0..n).map(|i| (i % 11) as f32 / 10.0).collect();
        let mut w = weights();
        // Saturation/clipping for both groups, with both signs. Preserve enough
        // unsaturated units to exercise every downstream derivative.
        for p in Parameter::ALL {
            if let Some(limit) = p.limit() {
                w[p][0] = limit + 0.01;
                w[p][1] = -limit - 0.01;
            }
        }
        let mut optimizer = AdamW::new(OptimizerConfig {
            ft: crate::nn::AdamWGroup {
                lr: 0.002,
                weight_decay: 0.1,
            },
            head: crate::nn::AdamWGroup {
                lr: 0.004,
                weight_decay: 0.2,
            },
        })
        .unwrap();
        let mut gpu = Step::new(device.clone(), &w, &optimizer, scale(), n).unwrap();
        let mut repeat = Step::new(device.clone(), &w, &optimizer, scale(), n).unwrap();
        let mut ring = HostRing::new(&device, InputFeatureSet::P, n).unwrap();
        let mut repeated_ring = HostRing::new(&device, InputFeatureSet::P, n).unwrap();
        let mut resumed = None;
        for iteration in 1..=3 {
            let (loss, g) = backward(&w, scale(), &features, &values).unwrap();
            optimizer.update(&mut w, &g, 0.7).unwrap();
            let ticket = ring.upload(&batch).unwrap();
            let actual = gpu
                .enqueue(&mut ring, &ticket, &values, 0.7)
                .unwrap()
                .collect()
                .unwrap();
            close("update loss", actual.loss, loss);
            assert_eq!(actual.step, iteration);
            let ticket = repeated_ring.upload(&batch).unwrap();
            repeat
                .enqueue(&mut repeated_ring, &ticket, &values, 0.7)
                .unwrap()
                .collect()
                .unwrap();
            let (gw, go) = gpu.snapshot().unwrap();
            let (rw, ro) = repeat.snapshot().unwrap();
            compare(&gw, &w);
            compare(&go.m, &optimizer.m);
            compare(&go.v, &optimizer.v);
            assert_eq!(go.step, optimizer.step);
            bits(&gw, &rw);
            bits(&go.m, &ro.m);
            bits(&go.v, &ro.v);
            if let Some(ref mut resumed) = resumed {
                let (state, resumed_ring): &mut (Step, HostRing) = resumed;
                let ticket = resumed_ring.upload(&batch).unwrap();
                state
                    .enqueue(resumed_ring, &ticket, &values, 0.7)
                    .unwrap()
                    .collect()
                    .unwrap();
                let (sw, so) = state.snapshot().unwrap();
                bits(&gw, &sw);
                bits(&go.m, &so.m);
                bits(&go.v, &so.v);
                assert_eq!(so.step, go.step);
            }
            if iteration == 1 {
                resumed = Some((
                    Step::new(device.clone(), &gw, &go, scale(), n).unwrap(),
                    HostRing::new(&device, InputFeatureSet::P, n).unwrap(),
                ));
            }
        }
    }
}

#[test]
#[ignore = "requires a CUDA GPU and NVRTC"]
fn gpu_asynchronous_results_reject_reuse_and_nonfinite_snapshot() {
    let device = Device::new(0).unwrap();
    let batch = batch(100);
    let mut ring = HostRing::new(&device, InputFeatureSet::P, 100).unwrap();
    let w = weights();
    let opt = AdamW::new(OptimizerConfig::default()).unwrap();
    let mut gpu = Step::new(device.clone(), &w, &opt, scale(), 100).unwrap();
    let values = vec![0.25; 100];
    let first_ticket = ring.upload(&batch).unwrap();
    for multiplier in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::from_bits(1)] {
        // The smallest positive value underflows when multiplied by lr.
        assert!(
            gpu.enqueue(&mut ring, &first_ticket, &values, multiplier)
                .is_err()
        );
        assert_eq!(gpu.step(), 0);
    }
    let first = gpu.enqueue(&mut ring, &first_ticket, &values, 1.0).unwrap();
    let ticket = ring.upload(&batch).unwrap();
    let second = gpu.enqueue(&mut ring, &ticket, &values, 1.0).unwrap();
    let third_ticket = ring.upload(&batch).unwrap();
    assert!(gpu.enqueue(&mut ring, &third_ticket, &values, 1.0).is_err());
    assert_eq!(first.collect().unwrap().step, 1);
    let third = gpu.enqueue(&mut ring, &third_ticket, &values, 1.0).unwrap();
    assert_eq!(second.collect().unwrap().step, 2);
    assert_eq!(third.collect().unwrap().step, 3);
    assert_eq!(gpu.snapshot().unwrap().1.step, 3);
    let mut bad = w;
    bad[Parameter::L1Weights].fill(f32::MAX);
    bad[Parameter::L1SharedWeights].fill(f32::MAX);
    let mut gpu = Step::new(device, &bad, &opt, scale(), 100).unwrap();
    let ticket = ring.upload(&batch).unwrap();
    let result = gpu.enqueue(&mut ring, &ticket, &values, 1.0).unwrap();
    assert!(gpu.snapshot().is_err());
    assert!(result.collect().is_err());
    assert!(gpu.snapshot().is_err());
}

#[test]
fn equivalence_inputs_include_both_views_rotations_sentinels_and_lion_flags() {
    let b = batch(100);
    let features = Features::from_batch(&b).unwrap();
    assert_eq!(features.len(), 100);
    assert!(
        features
            .iter()
            .any(|f| f.views().iter().any(|v| v.len() == 145))
    );
    assert!(
        features
            .iter()
            .any(|f| f.views().iter().any(|v| v.len() == 2))
    );
    for rotated in [0, 1] {
        assert!(
            b.features()
                .iter()
                .any(|&f| f != u32::MAX && f & 1 == rotated)
        );
    }
    assert!(b.features().contains(&u32::MAX));
    assert!(b.first_layer_inverse().entries().len() > 128);
    weights().validate().unwrap();
}

#[test]
#[ignore = "requires a CUDA GPU and NVRTC"]
fn gpu_activation_derivatives_at_zero_and_one_match_reference() {
    let device = Device::new(0).unwrap();
    let batch = batch(100);
    let targets = vec![0.25; batch.len()];
    let mut w = Parameters::zeros();
    for (j, v) in w[Parameter::FtBiases].iter_mut().enumerate() {
        *v = [0.0, 1.0, 0.5][j % 3];
    }
    for (j, v) in w[Parameter::L1Biases].iter_mut().enumerate() {
        *v = [0.0, 1.0, -0.5][j % 3];
    }
    w[Parameter::L1Weights][15 * H..16 * H].fill(0.001);
    for (j, v) in w[Parameter::L2Biases].iter_mut().enumerate() {
        *v = [0.0, 1.0, 0.5][j % 3] - 0.125;
    }
    for k in 0..32 {
        w[Parameter::L2Weights][k * 30 + 15] = 0.125;
        w[Parameter::L2Weights][k * 30 + 16] = 0.125;
    }
    w[Parameter::L3Weights].fill(0.01);
    let features = Features::from_batch(&batch).unwrap();
    let (_, expected) = backward(&w, scale(), &features, &targets).unwrap();
    let kernels = Kernels::new(&device).unwrap();
    let weights = upload(&device, &w).unwrap();
    let mut forward = ForwardBuffers::new(&device, 100).unwrap();
    let mut backward = BackwardBuffers::new(&device, 100).unwrap();
    let mut uploaded = Targets::new(&device, 100).unwrap();
    uploaded.upload(&device, &targets).unwrap();
    let mut ring = HostRing::new(&device, InputFeatureSet::P, 100).unwrap();
    let ticket = ring.upload(&batch).unwrap();
    let input = ring.input(&ticket).unwrap();
    forward
        .launch(
            &device,
            &kernels,
            &weights,
            input,
            &uploaded.values,
            scale(),
        )
        .unwrap();
    backward
        .launch(
            &device,
            &kernels,
            (&weights, scale()),
            input,
            &uploaded.values,
            &mut forward,
        )
        .unwrap();
    ring.finish(ticket).unwrap();
    check_status(&device, &forward.nonfinite).unwrap();
    let actual = download(&device, &backward.gradients).unwrap();
    compare(&actual, &expected);
    for p in [Parameter::FtBiases, Parameter::L2Biases] {
        for (j, value) in actual[p].iter().enumerate() {
            if j % 3 < 2 {
                assert_eq!(*value, 0.0, "{p:?}[{j}] boundary derivative");
            }
        }
    }
}
