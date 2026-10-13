//! PST学習の契約を、独立な損失の定義、鏡映、更新、保存の性質から検証する。
mod common;

use common::{Fixture, mirror, position, positions};
use minase::{
    eval::{
        features::{FEATURE_COUNT, feature_index, lion_feature_index, mirror_feature},
        pst::Pst,
    },
    training::records::{Outcome, Record},
};
use minase_core::{Color, PieceKind, Position, Square};
use minase_trainer::pst::{
    self, Adam, Batch, Checkpoint, Model, Progress, RemovalReference, encode_pst_npz,
    export::quantize, load_initial, v0_piece_values,
};

fn batch(positions: &[Position]) -> Batch {
    let records: Vec<_> = positions
        .iter()
        .map(|p| {
            Record::from_position(
                p,
                0,
                Outcome::Draw,
                1,
                0,
                minase::training::records::Origin::RandomSelfplay,
            )
        })
        .collect();
    Batch::from_records(&records, &vec![0.0; records.len()], &vec![0; records.len()]).unwrap()
}

fn endpoints() -> Vec<[i16; 2]> {
    let values = v0_piece_values();
    (0..FEATURE_COUNT)
        .map(|f| {
            if f < 13536 {
                [((if f < 6768 { 1 } else { -1 }) * values[f / 144 % 47] * 8) as i16; 2]
            } else {
                [0; 2]
            }
        })
        .collect()
}

fn close(a: f64, b: f64, tolerance: f64) {
    assert!(
        (a - b).abs() <= tolerance,
        "{a} != {b}, error={}",
        (a - b).abs()
    );
}

// 仕様の損失を独立に定義する。各除去で全特徴を再評価し、共有係数や差分関数は使わない。
fn oracle(
    model: &Model,
    positions: &[Position],
    targets: &[f32],
    reference: &[[i16; 2]],
    penalty: f64,
) -> (f64, usize, usize) {
    let weights = model.expanded();
    let mut loss = 0.0;
    let mut lower_count = 0;
    let mut upper_count = 0;
    for (position, &target) in positions.iter().zip(targets) {
        let mut features: Vec<_> = Square::all()
            .filter_map(|sq| {
                position
                    .piece_at(sq)
                    .map(|p| feature_index(position.side_to_move(), p, sq))
            })
            .collect();
        let count = features.len();
        if let Some(trigger) = position.lion_taken_by_non_lion() {
            features.push(lion_feature_index(position.side_to_move(), trigger.square));
        }
        let evaluate = |removed: Option<usize>| {
            let phi = (count as i32 - 2 - i32::from(removed.is_some())).clamp(0, 90) as f64 / 90.0;
            features
                .iter()
                .enumerate()
                .filter(|(col, _)| Some(*col) != removed)
                .map(|(_, &f)| {
                    phi * f64::from(weights[f][0]) + (1.0 - phi) * f64::from(weights[f][1])
                })
                .sum::<f64>()
        };
        let base = |removed: Option<usize>| {
            let q = (count as i64 - 2 - i64::from(removed.is_some())).clamp(0, 90);
            let numerator: i64 = features
                .iter()
                .enumerate()
                .filter(|(col, _)| Some(*col) != removed)
                .map(|(_, &f)| {
                    q * i64::from(reference[f][0]) + (90 - q) * i64::from(reference[f][1])
                })
                .sum();
            ((numerator / 720).clamp(-28999, 28999), numerator)
        };
        let before = evaluate(None);
        let z = before / f64::from(model.k());
        loss += z.max(0.0) - z * f64::from(target) + (-z.abs()).exp().ln_1p();
        let (integer_before, numerator_before) = base(None);
        let mut removal = 0.0;
        let mut eligible = 0;
        for (col, &f) in features[..count].iter().enumerate() {
            if [11, 21].contains(&(f / 144 % 47)) {
                continue;
            }
            let (integer_after, numerator_after) = base(Some(col));
            let sign = (integer_after - integer_before).signum();
            if sign == 0 {
                continue;
            }
            eligible += 1;
            let signed = sign as f64 * (evaluate(Some(col)) - before);
            let magnitude = (numerator_after - numerator_before).abs() as f64 / 720.0;
            let lower = (magnitude.min(0.25) - signed).max(0.0);
            let upper = if (f < 6768 && sign > 0) || (f >= 6768 && sign < 0) {
                (signed - magnitude).max(0.0)
            } else {
                0.0
            };
            lower_count += usize::from(lower > 0.0);
            upper_count += usize::from(upper > 0.0);
            removal += lower + upper;
        }
        loss += penalty * removal / eligible.max(1) as f64 / f64::from(model.k());
    }
    (loss / positions.len() as f64, lower_count, upper_count)
}

#[test]
fn pst_gradients_match_independent_differences_at_both_penalties_phase_edges_and_lion() {
    let mut cases = positions();
    for count in [0, 1, 2, 3, 91, 92, 93, 144] {
        let pieces: Vec<_> = (0..count)
            .map(|i| {
                (
                    (i % 12) as u8,
                    (i / 12) as u8,
                    Color::Black,
                    PieceKind::Pawn,
                )
            })
            .collect();
        cases.push(position(&pieces, Color::Black, None));
    }
    let mut saw_lower = false;
    let mut saw_upper = false;
    for inverted in [false, true] {
        let mut reference = endpoints();
        if inverted {
            reference[29 * 144..30 * 144].fill([-8; 2]);
            reference[(47 + 29) * 144..(48 + 29) * 144].fill([8; 2]);
        }
        let removal = RemovalReference::new(&reference).unwrap();
        // 逆向き1cpの歩を3cpにし、上側の罰則を確実に発火させる。
        let mut initial = vec![[0.02, -0.03]; FEATURE_COUNT];
        for (f, row) in initial.iter_mut().enumerate() {
            if inverted && f < 13536 && f / 144 % 47 == 29 {
                *row = reference[f].map(|v| f32::from(v) / 8.0 * 3.0);
            }
            if f >= 13536 {
                *row = [1.2, -0.7];
            }
        }
        let mut model = Model::initialize(&initial, 317.25).unwrap();
        for p in &cases {
            let input = batch(std::slice::from_ref(p));
            let targets = [0.3];
            let g = pst::gradients(&model, &input, &targets, Some(&removal), 73.0, 2).unwrap();
            let (loss, lower, upper) =
                oracle(&model, std::slice::from_ref(p), &targets, &reference, 73.0);
            saw_lower |= lower > 0;
            saw_upper |= upper > 0;
            close(g.losses.bce + 73.0 * g.losses.removal, loss, 2e-5);
            let mut rows: Vec<_> = input
                .columns()
                .iter()
                .copied()
                .filter(|&r| r != u32::MAX)
                .map(|r| r as usize)
                .collect();
            rows.sort_unstable();
            rows.dedup();
            for row in rows {
                for e in 0..2 {
                    let i = row * 2 + e;
                    let original = model.weights()[i];
                    model.weights_mut()[i] = original + 0.01;
                    let hi = model.weights()[i];
                    let plus =
                        oracle(&model, std::slice::from_ref(p), &targets, &reference, 73.0).0;
                    model.weights_mut()[i] = original - 0.01;
                    let lo = model.weights()[i];
                    let minus =
                        oracle(&model, std::slice::from_ref(p), &targets, &reference, 73.0).0;
                    model.weights_mut()[i] = original;
                    let numerical = (plus - minus) / f64::from(hi - lo);
                    close(
                        f64::from(g.weights[i]),
                        numerical,
                        2e-6 + numerical.abs() * 2e-5,
                    );
                }
            }
        }
    }
    assert!(saw_lower && saw_upper);
}

#[test]
fn mirror_initialization_averages_pairs_and_mirrored_positions_evaluate_identically() {
    let initial: Vec<_> = (0..FEATURE_COUNT)
        .map(|f| [(f % 144) as f32 / 8.0, -((f % 97) as f32) / 16.0])
        .collect();
    let model = Model::initialize(&initial, 400.0).unwrap();
    let expanded = model.expanded();
    for f in 0..FEATURE_COUNT {
        for e in 0..2 {
            assert_eq!(
                expanded[f][e],
                (initial[f][e] + initial[mirror_feature(f)][e]) / 2.0
            );
        }
    }
    let original = positions();
    let mirrored: Vec<_> = original.iter().map(mirror).collect();
    bits(
        &pst::evaluate(&model, &batch(&original)).unwrap(),
        &pst::evaluate(&model, &batch(&mirrored)).unwrap(),
    );
}

fn bits(a: &[f32], b: &[f32]) {
    assert_eq!(a.len(), b.len());
    for (i, (&a, &b)) in a.iter().zip(b).enumerate() {
        assert_eq!(a.to_bits(), b.to_bits(), "element {i}");
    }
}

#[test]
fn threads_preserve_losses_gradients_weights_and_both_adam_moments() {
    let cases = positions();
    let input = batch(&[
        cases[0].clone(),
        cases[1].clone(),
        cases[2].clone(),
        cases[0].clone(),
        cases[2].clone(),
    ]);
    let reference = RemovalReference::v0();
    let mut model = Model::initialize(&vec![[0.02, -0.03]; FEATURE_COUNT], 700.0).unwrap();
    let mut optimizer = Adam::new(0.03).unwrap();
    for _ in 0..3 {
        let expected =
            pst::gradients(&model, &input, &[0.3; 5], Some(&reference), 1000.0, 1).unwrap();
        let mut updated = model.clone();
        let mut updated_opt = optimizer.clone();
        updated_opt.update(&mut updated, &expected.weights).unwrap();
        for threads in [2, 3, 8] {
            let g = pst::gradients(&model, &input, &[0.3; 5], Some(&reference), 1000.0, threads)
                .unwrap();
            assert_eq!(g.losses.bce.to_bits(), expected.losses.bce.to_bits());
            assert_eq!(
                g.losses.removal.to_bits(),
                expected.losses.removal.to_bits()
            );
            bits(&g.weights, &expected.weights);
            let mut actual = model.clone();
            let mut opt = optimizer.clone();
            opt.update(&mut actual, &g.weights).unwrap();
            bits(actual.weights(), updated.weights());
            bits(opt.m(), updated_opt.m());
            bits(opt.v(), updated_opt.v());
        }
        model = updated;
        optimizer = updated_opt;
    }
}

#[test]
fn removal_cancellation_is_exact_before_adam_and_reference_zero_is_excluded() {
    // 同じ鏡映行の歩2枚の一方を除く。残存する全行の係数が相殺される例を構成する。
    // 両色の除去符号が逆で、先獅子と王の係数和は厳密に0になる。
    let p = position(
        &[
            (4, 0, Color::Black, PieceKind::King),
            (7, 11, Color::White, PieceKind::King),
            (2, 4, Color::Black, PieceKind::Pawn),
            (9, 7, Color::White, PieceKind::Pawn),
        ],
        Color::Black,
        Some((5, 5)),
    );
    let input = batch(&[p]);
    let mut model = Model::initialize(&vec![[0.0; 2]; FEATURE_COUNT], 1000.0).unwrap();
    let g = pst::gradients(
        &model,
        &input,
        &[0.5],
        Some(&RemovalReference::v0()),
        1000.0,
        1,
    )
    .unwrap();
    for col in [4, 139, 144] {
        let row = input.columns()[col] as usize;
        assert_eq!(&g.weights[row * 2..row * 2 + 2], &[0.0, 0.0]);
    }
    let lion = input.columns()[144] as usize;
    let mut opt = Adam::new(0.03).unwrap();
    opt.update(&mut model, &g.weights).unwrap();
    assert_eq!(&model.weights()[lion * 2..lion * 2 + 2], &[0.0, 0.0]);
    let zero = RemovalReference::new(&vec![[0; 2]; FEATURE_COUNT]).unwrap();
    let g = pst::gradients(&model, &input, &[0.5], Some(&zero), 1000.0, 1).unwrap();
    assert_eq!(g.losses.removal, 0.0);
}

#[test]
fn adam_matches_bias_corrected_formula_updates_absent_rows_and_clamps() {
    let mut model = Model::initialize(&vec![[0.0; 2]; FEATURE_COUNT], 500.0).unwrap();
    let mut opt = Adam::new(0.03).unwrap();
    let mut gradient = vec![0.0; model.weights().len()];
    let mut m = 0.0_f64;
    let mut v = 0.0_f64;
    let mut p = 0.0_f64;
    for (step, g) in [2.0, 0.0, -1.0].into_iter().enumerate() {
        gradient[0] = g as f32;
        opt.update(&mut model, &gradient).unwrap();
        m = 0.9 * m + 0.1 * g;
        v = 0.999 * v + 0.001 * g * g;
        let t = (step + 1) as i32;
        p -= 0.03 * (m / (1.0 - 0.9_f64.powi(t))) / ((v / (1.0 - 0.999_f64.powi(t))).sqrt() + 1e-8);
        close(f64::from(model.weights()[0]), p, 1e-7);
        close(f64::from(opt.m()[0]), m, 1e-7);
        close(f64::from(opt.v()[0]), v, 1e-8);
    }
    model.weights_mut()[0] = 4095.875;
    model.weights_mut()[1] = -4096.0;
    gradient[0] = -100.0;
    gradient[1] = 100.0;
    opt.update(&mut model, &gradient).unwrap();
    assert_eq!(&model.weights()[..2], &[4095.875, -4096.0]);
    model.weights_mut()[0] = f32::INFINITY;
    assert!(model.constrain().is_err());
}

#[test]
fn ties_to_even_mnpt_v2_and_npz_preserve_endpoints_and_metadata() {
    let fixture = Fixture::new(&positions());
    let mut model = Model::initialize(&vec![[0.0; 2]; FEATURE_COUNT], 317.25).unwrap();
    let values = [
        0.0625, 0.1875, 0.3125, -0.0625, -0.1875, -0.3125, -4096.0, 4095.875,
    ];
    let expected = [0, 2, 2, 0, -2, -2, i16::MIN, i16::MAX];
    for (row, value) in values.into_iter().enumerate() {
        model.weights_mut()[row * 2] = value;
        model.weights_mut()[row * 2 + 1] = values[values.len() - 1 - row];
    }
    let quantized = quantize(&model).unwrap();
    for (row, want) in expected.into_iter().enumerate() {
        assert_eq!(
            quantized[row / 6 * 12 + row % 6],
            [want, expected[expected.len() - 1 - row]]
        );
    }
    let values = v0_piece_values().map(|value| value * 2);
    let pst = Pst::from_weights(&quantized, values, 317.25, "L1,P0,R1,E0").unwrap();
    let bytes = pst.encode();
    assert_eq!(&bytes[4..8], &2_u32.to_le_bytes());
    let written = Pst::decode(&bytes).unwrap();
    assert_eq!(written.weights().as_slice(), quantized);
    assert_eq!(written.k(), 317.25);
    assert_eq!(written.rule_set(), "L1,P0,R1,E0");
    for (state, value) in values.into_iter().enumerate() {
        assert_eq!(written.piece_value_of_state(state), value);
    }
    let path = fixture.root.join("pst.npz");
    std::fs::write(&path, encode_pst_npz(&model.expanded()).unwrap()).unwrap();
    assert_eq!(load_initial(path).unwrap(), model.expanded());
    model.weights_mut()[0] = 4095.9375;
    assert!(quantize(&model).is_err());
    model.weights_mut()[0] = f32::NAN;
    assert!(quantize(&model).is_err());
    assert!(Pst::from_weights(&quantized, values, 0.0, "L0").is_err());
    assert!(Pst::from_weights(&quantized, values, 1.0, "bad\0rule").is_err());
    assert!(Pst::from_weights(&quantized[..10], values, 1.0, "L0").is_err());
}

#[test]
fn pst_checkpoint_roundtrip_validates_progress_padding_and_corruption() {
    let model = Model::initialize(&vec![[0.0; 2]; FEATURE_COUNT], 400.0).unwrap();
    let mut ck = Checkpoint {
        optimizer: Adam::new(0.03).unwrap(),
        model,
        progress: Progress {
            epoch: 0,
            best: None,
            elapsed_seconds: 0.0,
            seed: 1,
            config_hash: [1; 32],
            input_hash: [2; 32],
        },
    };
    ck.optimizer
        .update(
            &mut ck.model,
            &vec![0.1; FEATURE_COUNT]
                .into_iter()
                .chain([0.0; 2])
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let bytes = ck.encode().unwrap();
    assert_eq!(Checkpoint::decode(&bytes).unwrap(), ck);
    assert!(minase_trainer::nnue::model::Checkpoint::decode(&bytes).is_err());
    for cut in [0, 8, 100, bytes.len() - 1] {
        assert!(Checkpoint::decode(&bytes[..cut]).is_err());
    }
    let mut corrupt = bytes;
    corrupt[160] ^= 1;
    assert!(Checkpoint::decode(&corrupt).is_err());
    assert!(ck.progress.check_hashes(&[3; 32], &[2; 32]).is_err());
}
