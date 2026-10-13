use super::*;
use minase::eval::features::{active_features, mirror_feature};
use minase_trainer::joint::{Adam, Batch};

fn sample() -> (Model, Batch, Vec<Record>) {
    let records: Vec<_> = [Color::Black, Color::White]
        .into_iter()
        .map(|side| {
            Record::from_position(
                &position(
                    &[
                        (4, 0, Color::Black, PieceKind::King),
                        (7, 11, Color::White, PieceKind::King),
                        (2, 4, Color::Black, PieceKind::Pawn),
                        (9, 4, Color::Black, PieceKind::Pawn),
                        (8, 7, Color::White, PieceKind::Rook),
                        (3, 5, Color::White, PieceKind::Pawn),
                    ],
                    side,
                    Some((5, 6)),
                ),
                0,
                Outcome::Draw,
                1,
                0,
            )
        })
        .collect();
    let batch = Batch::from_records(&records, &[0, 1], &[0.2, 0.8]).unwrap();
    let initial: Vec<_> = (0..FEATURE_COUNT)
        .map(|i| {
            [
                ((i % 43) as f32 - 20.0) * 0.004,
                ((i % 37) as f32 - 18.0) * 0.006,
            ]
        })
        .collect();
    let fm = fm::Model::initialize(vec![true; FEATURE_COUNT], 317.25, 42).unwrap();
    let fm = fm::Model::from_weights(
        fm.v(),
        std::array::from_fn(|f| (f as f32 - 14.0) * 0.03),
        vec![true; FEATURE_COUNT],
        317.25,
    )
    .unwrap();
    (
        Model {
            pst: minase_trainer::pst::Model::initialize(&initial, 317.25).unwrap(),
            fm,
        },
        batch,
        records,
    )
}

// Oracle: recompute each removed position and enumerate unordered FM feature pairs.
// No squared-sum identity or analytical removal derivative is used here.
fn score(model: &Model, features: &[usize], count: usize) -> f64 {
    let rows = model.pst.expanded();
    let q = count.saturating_sub(2).min(90) as f64 / 90.0;
    let mut score = features
        .iter()
        .map(|&f| q * f64::from(rows[f][0]) + (1.0 - q) * f64::from(rows[f][1]))
        .sum::<f64>();
    for (i, &left) in features.iter().enumerate() {
        for &right in &features[i + 1..] {
            for f in 0..FM_RANK {
                score += f64::from(model.pst.k())
                    * f64::from(model.fm.a()[f])
                    * f64::from(model.fm.v()[left][f])
                    * f64::from(model.fm.v()[right][f]);
            }
        }
    }
    score
}
fn oracle(model: &Model, batch: &Batch, lambda: f64, removal: f64) -> f64 {
    let mut total = 0.0;
    let values = minase_trainer::pst::v0_piece_values();
    for (b, columns) in batch.columns().iter().enumerate() {
        let features: Vec<_> = columns
            .iter()
            .copied()
            .filter(|&f| f != fm::batch::PADDING)
            .map(|f| f as usize)
            .collect();
        let count = columns[..144]
            .iter()
            .filter(|&&f| f != fm::batch::PADDING)
            .count();
        let cp = score(model, &features, count);
        let z = cp / f64::from(model.pst.k());
        let mut phi = 0.0;
        for (i, &left) in features.iter().enumerate() {
            for &right in &features[i + 1..] {
                for f in 0..FM_RANK {
                    phi += f64::from(model.fm.a()[f])
                        * f64::from(model.fm.v()[left][f])
                        * f64::from(model.fm.v()[right][f]);
                }
            }
        }
        let reference = |fs: &[usize]| {
            fs.iter()
                .filter(|&&f| f < 94 * 144)
                .map(|&f| {
                    if f < 47 * 144 {
                        values[f / 144 % 47]
                    } else {
                        -values[f / 144 % 47]
                    }
                })
                .sum::<i32>()
                .clamp(-28999, 28999)
        };
        let before = reference(&features);
        let mut penalties = Vec::new();
        for (i, &feature) in features.iter().enumerate() {
            if feature >= 94 * 144 || [11, 21].contains(&(feature / 144 % 47)) {
                continue;
            }
            let mut after = features.clone();
            after.remove(i);
            let delta = reference(&after) - before;
            if delta == 0 {
                continue;
            }
            let signed = f64::from(delta.signum()) * (score(model, &after, count - 1) - cp);
            // v0 has equal endpoints, so the unclipped magnitude is the material value.
            let magnitude = f64::from(values[feature / 144 % 47]).abs();
            let wrong = (feature < 47 * 144 && delta > 0) || (feature >= 47 * 144 && delta < 0);
            penalties.push(
                (magnitude.min(0.25) - signed).max(0.0)
                    + if wrong {
                        (signed - magnitude).max(0.0)
                    } else {
                        0.0
                    },
            );
        }
        let penalty = penalties.iter().sum::<f64>()
            / penalties.len().max(1) as f64
            / f64::from(model.pst.k());
        total += z.exp().ln_1p() - z * f64::from([0.2_f32, 0.8][b])
            + lambda * phi * phi
            + removal * penalty;
    }
    total / batch.len() as f64
}
fn canonical(feature: usize) -> usize {
    feature / 144 * 72 + (feature % 144) / 12 * 6 + (feature % 12).min(11 - feature % 12)
}
fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-5 + expected.abs() * 2e-3,
        "actual={actual} expected={expected}"
    );
}

#[test]
fn all_three_gradients_match_independent_numerical_derivatives_with_removal_and_regularization() {
    let (model, batch, _) = sample();
    let features: Vec<_> = batch
        .columns()
        .iter()
        .flatten()
        .filter(|&&f| f != fm::batch::PADDING)
        .map(|&f| f as usize)
        .collect();
    for (lambda, removal) in [(0.0, 0.0), (0.7, 0.0), (0.0, 1000.0), (0.7, 1000.0)] {
        let g = joint::gradients(&model, &batch, lambda, removal, 3).unwrap();
        close(g.losses.total, oracle(&model, &batch, lambda, removal));
        for &feature in &features {
            for e in 0..2 {
                let i = 2 * canonical(feature) + e;
                let mut high = model.clone();
                let mut low = model.clone();
                high.pst.weights_mut()[i] += 0.001;
                low.pst.weights_mut()[i] -= 0.001;
                let dx = f64::from(high.pst.weights()[i] - low.pst.weights()[i]);
                close(
                    f64::from(g.pst[i]),
                    (oracle(&high, &batch, lambda, removal)
                        - oracle(&low, &batch, lambda, removal))
                        / dx,
                );
            }
            for f in [0, 17, 31] {
                let mut v = model.fm.v().to_vec();
                let x = v[feature][f];
                let plus = x + 0.0001;
                let minus = x - 0.0001;
                v[feature][f] = plus;
                let mut high = model.clone();
                high.fm = fm::Model::from_weights(
                    &v,
                    *model.fm.a(),
                    model.fm.mask().to_vec(),
                    model.fm.k(),
                )
                .unwrap();
                v[feature][f] = minus;
                let mut low = model.clone();
                low.fm = fm::Model::from_weights(
                    &v,
                    *model.fm.a(),
                    model.fm.mask().to_vec(),
                    model.fm.k(),
                )
                .unwrap();
                close(
                    f64::from(g.fm[feature * FM_RANK + f]),
                    (oracle(&high, &batch, lambda, removal)
                        - oracle(&low, &batch, lambda, removal))
                        / f64::from(plus - minus),
                );
            }
        }
        for f in 0..FM_RANK {
            let mut a = *model.fm.a();
            let x = a[f];
            let plus = x + 0.0001;
            let minus = x - 0.0001;
            a[f] = plus;
            let mut high = model.clone();
            high.fm =
                fm::Model::from_weights(model.fm.v(), a, model.fm.mask().to_vec(), model.fm.k())
                    .unwrap();
            a[f] = minus;
            let mut low = model.clone();
            low.fm =
                fm::Model::from_weights(model.fm.v(), a, model.fm.mask().to_vec(), model.fm.k())
                    .unwrap();
            close(
                f64::from(g.fm[(FEATURE_COUNT + 1) * FM_RANK + f]),
                (oracle(&high, &batch, lambda, removal) - oracle(&low, &batch, lambda, removal))
                    / f64::from(plus - minus),
            );
        }
    }
}

#[test]
fn threads_do_not_change_any_gradient_weight_or_adam_bit() {
    let (model, batch, _) = sample();
    let g = joint::gradients(&model, &batch, 0.7, 1000.0, 1).unwrap();
    let mut expected = model.clone();
    let mut optimizer = Adam::new(0.03, 0.001, 0.0001).unwrap();
    optimizer.update(&mut expected, &g, 1).unwrap();
    let bits = |v: &[f32]| v.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
    for threads in [2, 3, 8] {
        let actual = joint::gradients(&model, &batch, 0.7, 1000.0, threads).unwrap();
        assert_eq!(actual.losses, g.losses);
        assert_eq!(bits(&actual.pst), bits(&g.pst));
        assert_eq!(bits(&actual.fm), bits(&g.fm));
        let mut current = model.clone();
        let mut opt = Adam::new(0.03, 0.001, 0.0001).unwrap();
        opt.update(&mut current, &actual, threads).unwrap();
        assert_eq!(current, expected);
        assert_eq!(opt, optimizer);
        for (a, b) in [
            (current.pst.weights(), expected.pst.weights()),
            (opt.pst.m(), optimizer.pst.m()),
            (opt.pst.v(), optimizer.pst.v()),
            (opt.fm.m(), optimizer.fm.m()),
            (opt.fm.v(), optimizer.fm.v()),
        ] {
            assert_eq!(bits(a), bits(b));
        }
        for (a, b) in current.fm.v().iter().zip(expected.fm.v()) {
            assert_eq!(bits(a), bits(b));
        }
    }
    // 勾配が0でもモーメントを更新し、バッチに現れない行を含む全要素を更新する。
    let mut zero = g;
    zero.pst.fill(0.0);
    zero.fm.fill(0.0);
    let old = expected.clone();
    optimizer.update(&mut expected, &zero, 3).unwrap();
    assert_ne!(old, expected);
}

#[test]
fn mirrors_apply_to_both_models_and_keep_the_lion_in_removal_sums() {
    let (model, _, records) = sample();
    let batch = Batch::from_records(&records, &[1, 1], &[0.2, 0.8]).unwrap();
    for (b, record) in records.iter().enumerate() {
        let mut expected = Vec::new();
        active_features(&record.to_position().unwrap(), |f| {
            expected.push(mirror_feature(f) as u32)
        });
        expected.sort();
        let mut actual: Vec<_> = batch.columns()[b]
            .iter()
            .copied()
            .filter(|&f| f != fm::batch::PADDING)
            .collect();
        actual.sort();
        assert_eq!(actual, expected);
    }
    let gradient = joint::gradients(&model, &batch, 0.7, 1000.0, 2).unwrap();
    close(gradient.losses.total, oracle(&model, &batch, 0.7, 1000.0));
}

#[test]
fn fm_removal_rounds_in_embedding_precision_before_adding_pst_delta() {
    let p = position(
        &[
            (0, 0, Color::Black, PieceKind::Pawn),
            (1, 0, Color::Black, PieceKind::Pawn),
        ],
        Color::Black,
        None,
    );
    let record = Record::from_position(&p, 0, Outcome::Win, 1, 0);
    let batch = Batch::from_records(&[record], &[0], &[0.5]).unwrap();
    let mut v = vec![[0.0; FM_RANK]; FEATURE_COUNT];
    let (x, y, a) = (0.1234567_f32, 0.2345678_f32, -0.3456789_f32);
    v[batch.columns()[0][0] as usize][0] = x;
    v[batch.columns()[0][1] as usize][0] = y;
    let mut coefficients = [0.0; FM_RANK];
    coefficients[0] = a;
    let k = 317.25_f32;
    let model = Model {
        pst: minase_trainer::pst::Model::initialize(&vec![[0.0; 2]; FEATURE_COUNT], k).unwrap(),
        fm: fm::Model::from_weights(&v, coefficients, vec![true; FEATURE_COUNT], k).unwrap(),
    };
    // Two own pawns, zero PST: each removal is punished by 0.25 + K*DeltaPhi.
    // Compute the two f32 products explicitly, independently of the reduction code.
    let sum = x + y;
    let d0 = -(a * x) * (sum - x);
    let d1 = -(a * y) * (sum - y);
    let expected = ((0.25 + f64::from(k) * f64::from(d0)) + (0.25 + f64::from(k) * f64::from(d1)))
        / (2.0 * f64::from(k));
    let actual = joint::gradients(&model, &batch, 0.0, 1.0, 1)
        .unwrap()
        .losses
        .removal;
    assert!((actual - expected).abs() < 1e-15);
    let double = 0.25 / f64::from(k) - f64::from(a) * f64::from(x) * f64::from(y);
    assert!((actual - double).abs() > 1e-10);
}
