use super::super::commands::{ATOL, RTOL};
use super::engine::{BackwardBuffers, ForwardBuffers, Targets};
use super::*;
use super::{
    engine::{Evaluator, memory_bytes},
    step::Step,
};
use crate::data::{Outcome, Sample};
use crate::nnue::{
    data::{BatchSlot, FeatureSet},
    gpu::HostRing,
    simple::{
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
                if b % 7 == 0 {
                    PieceKind::CrownPrince
                } else {
                    PieceKind::King
                }
            } else {
                kinds[(j + b) % kinds.len()]
            };
            let color = if j % 2 == 0 { side } else { side.opposite() };
            let piece = if kind == PieceKind::CrownPrince {
                PieceCode::new_promoted(color, kind).unwrap()
            } else {
                PieceCode::new(color, kind).unwrap()
            };
            builder.put(square, piece).unwrap();
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
    BatchSlot::from_records(FeatureSet::HalfKpVm, &records, &vec![0.0; n], &vec![0; n]).unwrap()
}
fn weights() -> Parameters {
    let mut w = Parameters::initialize(19, 16).unwrap();
    let mut rng = Uniform(71);
    for p in Parameter::ALL {
        for v in &mut w[p] {
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
    assert_eq!(a.width(), b.width());
    for p in Parameter::ALL {
        for (i, (&a, &b)) in a[p].iter().zip(&b[p]).enumerate() {
            if !a.is_finite() || !b.is_finite() || a.to_bits() != b.to_bits() {
                close(&format!("{}[{i}]", p.name()), f64::from(a), f64::from(b));
            }
        }
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
use super::super::commands::forward_values;

#[test]
fn cuda_sources_compile_or_report_missing_nvrtc() {
    // A missing compiler must return the typed error. A compile error is never
    // treated as absence, and GPU tests never skip themselves at runtime.
    match crate::nnue::gpu::device::compile_sources(&SOURCES, (8, 6)) {
        Ok(_) => (),
        Err(InfrastructureError::CompilerUnavailable) => {
            eprintln!("NVRTC unavailable: compile_sources returned CompilerUnavailable");
        }
        Err(e) => panic!("Simple CUDA compilation: {e}"),
    }
}
#[test]
fn memory_16384_fits_budget_without_folded_weights_or_sparse_partials() {
    // From the phase 4b contract: W=[964224,256], V=[13392,256],
    // FT bias=256, L1=[32,512]+32, L2=[32,32]+32, L3=32+1.
    let params = (964224 + 13392) * 256 + 256 + 32 * 512 + 32 + 1024 + 32 + 32 + 1;
    assert_eq!(parameter_count(256), params);
    let ring = crate::nnue::gpu::ring::device_bytes(FeatureSet::HalfKpVm, 16384).unwrap();
    let forward = 4 * 16384 * 1153 + 8 * (128 * 7 + 7) + 4;
    let backward = 4 * 16384 * 577 + 8 * 128 * 17761;
    let train = 16 * params + ring + forward + backward + 8 * 16384;
    let validation = 4 * params + ring + forward + 4 * 16384;
    assert_eq!(memory_bytes(256, 16384, true).unwrap(), train);
    assert_eq!(memory_bytes(256, 16384, false).unwrap(), validation);
    assert!(train as f64 <= 5.5 * 1073741824.0);
    println!("Simple H=256 B=16384: train={train}, validation={validation} bytes");
    println!(
        "train: {:?}, validation: {:?}",
        engine::memory(256, 16384, true).unwrap(),
        engine::memory(256, 16384, false).unwrap()
    );
    assert!(memory_bytes(256, 0, true).is_err());
    assert!(memory_bytes(256, crate::nnue::data::MAX_BATCH_SIZE + 1, true).is_err());
    assert!(memory_bytes(3, 16384, true).is_err());
}

#[test]
#[ignore = "requires a CUDA GPU and NVRTC"]
fn gpu_forward_backward_validation_and_repeat_bits() {
    let device = Device::new(0).unwrap();
    let w = weights();
    let mut evaluator = Evaluator::new(device.clone(), &w, scale(), 4096).unwrap();
    let k = Kernels::new(&device).unwrap();
    let dw = upload(&device, &w).unwrap();
    let mut f = ForwardBuffers::new(&device, 4096, 16).unwrap();
    let mut d = BackwardBuffers::new(&device, 4096, 16).unwrap();
    let mut t = Targets::new(&device, 4096).unwrap();
    let mut ring = HostRing::new(&device, FeatureSet::HalfKpVm, 4096).unwrap();
    let repeated = BatchSlot::from_evaluation_positions(
        FeatureSet::HalfKpVm,
        &vec![minase_core::Position::initial(); 257],
        Color::Black,
    )
    .unwrap();
    for batch in [batch(100), batch(4096), repeated] {
        let n = batch.len();
        let targets: Vec<_> = (0..n).map(|i| (i % 11) as f32 / 10.0).collect();
        let features = Features::from_batch(&batch).unwrap();
        let (loss, gradient) = backward(&w, scale(), &features, &targets).unwrap();
        let first = evaluator.evaluate(&batch, &targets).unwrap();
        let second = evaluator.evaluate(&batch, &targets).unwrap();
        close("loss", first.validation.loss_sum / n as f64, loss);
        let statistics = evaluator.validate(&batch, &targets).unwrap();
        assert_eq!(statistics.loss_sum, first.validation.loss_sum);
        assert_eq!(statistics.sums, first.validation.sums);
        let cpu_stats = super::super::trainer::validation(&w, scale(), &batch, &targets).unwrap();
        for (key, expected) in cpu_stats.sums {
            close(&key, first.validation.sums[&key], expected);
            assert_eq!(
                first.validation.sums[&key].to_bits(),
                second.validation.sums[&key].to_bits()
            );
        }
        for ((a, b), feature) in first.forward.iter().zip(&second.forward).zip(&features) {
            let cpu = forward_validated(&w, feature);
            for ((a, b), c) in forward_values(a)
                .into_iter()
                .zip(forward_values(b))
                .zip(forward_values(&cpu))
            {
                close("forward", a, c);
                assert_eq!(a.to_bits(), b.to_bits());
            }
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
            let actual = download(&device, &d.gradients, 16).unwrap();
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
                w[p][2] = limit;
                w[p][3] = -limit;
                w[p][4] = limit - 0.01;
                w[p][5] = -limit + 0.01;
            }
        }
        let mut optimizer = AdamW::new(
            OptimizerConfig {
                ft: crate::nn::AdamWGroup {
                    lr: 0.002,
                    weight_decay: 0.1,
                },
                head: crate::nn::AdamWGroup {
                    lr: 0.004,
                    weight_decay: 0.2,
                },
            },
            16,
        )
        .unwrap();
        let mut gpu = Step::new(device.clone(), &w, &optimizer, scale(), n).unwrap();
        let mut repeat = Step::new(device.clone(), &w, &optimizer, scale(), n).unwrap();
        let mut ring = HostRing::new(&device, FeatureSet::HalfKpVm, n).unwrap();
        let mut repeated_ring = HostRing::new(&device, FeatureSet::HalfKpVm, n).unwrap();
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
            compare(&gpu.gradients().unwrap(), &g);
            assert_eq!(actual.step, iteration);
            let ticket = repeated_ring.upload(&batch).unwrap();
            repeat
                .enqueue(&mut repeated_ring, &ticket, &values, 0.7)
                .unwrap()
                .collect()
                .unwrap();
            let (gw, go) = gpu.snapshot().unwrap();
            assert_eq!(gpu.weights().unwrap(), gw);
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
                    HostRing::new(&device, FeatureSet::HalfKpVm, n).unwrap(),
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
    let mut ring = HostRing::new(&device, FeatureSet::HalfKpVm, 100).unwrap();
    let w = weights();
    let opt = AdamW::new(OptimizerConfig::default(), 16).unwrap();
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
    let mut gpu = Step::new(device, &bad, &opt, scale(), 100).unwrap();
    let ticket = ring.upload(&batch).unwrap();
    let result = gpu.enqueue(&mut ring, &ticket, &values, 1.0).unwrap();
    assert!(gpu.weights().is_err());
    assert!(gpu.snapshot().is_err());
    assert!(result.collect().is_err());
    assert!(gpu.weights().is_err());
    assert!(gpu.snapshot().is_err());
}

#[test]
fn equivalence_inputs_include_king_buckets_sparse_dense_boards_and_lion_flags() {
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
            .any(|f| f.views().iter().any(|v| v.is_empty()))
    );
    let buckets: std::collections::BTreeSet<_> = b
        .features()
        .iter()
        .filter(|&&r| r != u32::MAX)
        .map(|&r| r as usize / SPEC.piece_inputs())
        .collect();
    assert!(buckets.len() > 1);
    assert!(
        b.features()
            .iter()
            .any(|&r| r != u32::MAX && r as usize % SPEC.piece_inputs() >= 13248)
    );
    assert!(b.features().contains(&u32::MAX));
    assert!(b.first_layer_inverse().entries().len() > 128);
}

#[test]
#[ignore = "requires a CUDA GPU and NVRTC"]
fn gpu_activation_derivatives_at_zero_and_one_match_reference() {
    let device = Device::new(0).unwrap();
    let batch = batch(129);
    let targets = vec![0.25; batch.len()];
    for boundary in [
        Parameter::FtBiases,
        Parameter::L1Biases,
        Parameter::L2Biases,
    ] {
        let mut w = Parameters::zeros(16).unwrap();
        let desired = |p, j: usize| {
            if p == boundary {
                [0.0, 1.0, 0.5][j % 3]
            } else {
                0.5
            }
        };
        for j in 0..16 {
            w[Parameter::FtBiases][j] = desired(Parameter::FtBiases, j);
        }
        // Binary-exact compensation keeps the chosen layer at 0, 1 or 0.5,
        // while all other layers remain interior with nonzero downstream slopes.
        for k in 0..32 {
            w[Parameter::L1Weights][k * 32 + k % 16] = 0.125;
            w[Parameter::L1Biases][k] =
                desired(Parameter::L1Biases, k) - 0.125 * desired(Parameter::FtBiases, k % 16);
            w[Parameter::L2Weights][k * 32 + k] = 0.125;
            w[Parameter::L2Biases][k] =
                desired(Parameter::L2Biases, k) - 0.125 * desired(Parameter::L1Biases, k);
        }
        w[Parameter::L3Weights].fill(0.01);
        let features = Features::from_batch(&batch).unwrap();
        let (_, expected) = backward(&w, scale(), &features, &targets).unwrap();
        let opt = AdamW::new(OptimizerConfig::default(), 16).unwrap();
        let mut gpu = Step::new(device.clone(), &w, &opt, scale(), batch.len()).unwrap();
        let mut ring = HostRing::new(&device, FeatureSet::HalfKpVm, batch.len()).unwrap();
        let ticket = ring.upload(&batch).unwrap();
        gpu.enqueue(&mut ring, &ticket, &targets, 1.0)
            .unwrap()
            .collect()
            .unwrap();
        let actual = gpu.gradients().unwrap();
        compare(&actual, &expected);
        for (j, value) in actual[boundary].iter().enumerate() {
            if j % 3 < 2 {
                assert_eq!(*value, 0.0, "{boundary:?}[{j}] boundary derivative");
            } else {
                assert_ne!(*value, 0.0, "{boundary:?}[{j}] interior derivative");
            }
        }
    }
}

#[test]
#[ignore = "requires a CUDA GPU with at least 5.5 GiB free and NVRTC"]
fn gpu_width256_batch16384_memory_and_update() {
    let device = Device::new(0).unwrap();
    let w = Parameters::initialize(71, 256).unwrap();
    let opt = AdamW::new(OptimizerConfig::default(), 256).unwrap();
    device.compute.synchronize().unwrap();
    let before = device.memory_info().unwrap().0;
    let mut step = Step::new(device.clone(), &w, &opt, scale(), 16384).unwrap();
    drop(w);
    drop(opt);
    let mut ring = HostRing::new(&device, FeatureSet::HalfKpVm, 16384).unwrap();
    let batch = batch(16384);
    let targets = vec![0.25; batch.len()];
    let ticket = ring.upload(&batch).unwrap();
    let update = step
        .enqueue(&mut ring, &ticket, &targets, 1.0)
        .unwrap()
        .collect()
        .unwrap();
    assert_eq!(update.step, 1);
    assert!(update.loss.is_finite());
    let after = device.memory_info().unwrap().0;
    let bytes = step.device_bytes().unwrap();
    assert!(bytes as f64 <= 5.5 * 1073741824.0);
    println!(
        "Simple H=256 B=16384: payload={bytes}, device usage increment={} bytes",
        before as i128 - after as i128
    );
    println!(
        "training={:?}; validation={:?}",
        engine::memory(256, 16384, true).unwrap(),
        engine::memory(256, 16384, false).unwrap()
    );
    // Forward-only validation uses the same weights without a host snapshot.
    let v = step.validate(&mut ring, &batch, &targets).unwrap();
    assert!(v.loss_sum.is_finite());
    assert_eq!(step.step(), 1);
}
