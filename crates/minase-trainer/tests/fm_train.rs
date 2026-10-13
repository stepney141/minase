//! FM学習の数式、固定PST、再現性、出力と拒否条件を検証する。
#[path = "fm_train/fixture.rs"]
mod support;
use clap::Parser;
use minase::{
    eval::{
        features::{FEATURE_COUNT, active_features, mirror_feature},
        pst::{self, FM_RANK, Pst},
    },
    training::records::{Outcome, Record},
};
use minase_core::{Color, PieceCode, PieceKind, Position, PositionBuilder, Square};
use minase_trainer::{
    data::Dataset,
    fm::{self, Adam, Args, Batch, Checkpoint, Model, batch::PADDING},
    nn::npz,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
use support::*;

#[derive(Parser)]
struct Arguments {
    #[command(flatten)]
    args: Args,
}
fn argv() -> Vec<&'static str> {
    vec![
        "trainer",
        "--lambda",
        "0.75",
        "--data",
        "records.bin",
        "--init",
        "pst.bin",
        "--output-dir",
        "output",
        "--epochs",
        "2",
        "--patience",
        "5",
    ]
}
fn position(
    pieces: &[(u8, u8, Color, PieceKind)],
    side: Color,
    lion: Option<(u8, u8)>,
) -> Position {
    let mut builder = PositionBuilder::new(side);
    for &(f, r, c, p) in pieces {
        builder
            .put(Square::new(f, r).unwrap(), PieceCode::new(c, p).unwrap())
            .unwrap();
    }
    let mut position = builder.finish().unwrap();
    if let Some((f, r)) = lion {
        position
            .set_lion_capture(Some(Square::new(f, r).unwrap()))
            .unwrap();
    }
    position
}
fn mirror(position: &Position) -> Position {
    let mut builder = PositionBuilder::new(position.side_to_move());
    for square in Square::all() {
        if let Some(piece) = position.piece_at(square) {
            builder
                .put(
                    Square::new(11 - square.file(), square.rank()).unwrap(),
                    piece,
                )
                .unwrap();
        }
    }
    let mut mirrored = builder.finish().unwrap();
    if let Some(lion) = position.lion_taken_by_non_lion() {
        mirrored
            .set_lion_capture(Some(
                Square::new(11 - lion.square.file(), lion.square.rank()).unwrap(),
            ))
            .unwrap();
    }
    mirrored
}
fn small_positions() -> Vec<Position> {
    [Color::Black, Color::White]
        .into_iter()
        .map(|side| {
            position(
                &[
                    (4, 0, Color::Black, PieceKind::King),
                    (7, 11, Color::White, PieceKind::King),
                    (2, 4, Color::Black, PieceKind::Pawn),
                    (8, 7, Color::White, PieceKind::Rook),
                ],
                side,
                Some((3, 5)),
            )
        })
        .collect()
}
fn small_batch() -> (Model, Batch) {
    let positions = small_positions();
    let records: Vec<_> = positions
        .iter()
        .map(|p| {
            Record::from_position(
                p,
                200,
                Outcome::Win,
                1,
                0,
                minase::training::records::Origin::RandomSelfplay,
            )
        })
        .collect();
    let base = Pst::from_weights(
        &vec![[72, -91]; FEATURE_COUNT],
        minase_trainer::pst::v0_piece_values(),
        317.25,
        "L0,P0,R1,E0",
    )
    .unwrap();
    let batch = Batch::from_records(&base, &records, &[0, 1], &[0.2, 0.8]).unwrap();
    let mut mask = vec![false; FEATURE_COUNT];
    for &f in batch.columns().iter().flatten().filter(|&&f| f != PADDING) {
        mask[f as usize] = true;
    }
    let model = Model::initialize(mask.clone(), 317.25, 42).unwrap();
    (
        Model::from_weights(
            model.v(),
            std::array::from_fn(|f| (f as f32 - 14.0) * 0.03),
            mask,
            317.25,
        )
        .unwrap(),
        batch,
    )
}

#[test]
fn cli_enforces_required_budgets_and_ranges() {
    let args = Arguments::try_parse_from(argv()).unwrap().args;
    args.validate().unwrap();
    assert_eq!(
        (
            args.batch,
            args.seed,
            args.lr,
            args.weight_decay,
            args.lambda_res,
            args.validation_sample
        ),
        (4096, 1, 1e-3, 1e-4, 1e-4, 10000)
    );
    assert_eq!(args.lambda, 0.75);
    for flag in [
        "--data",
        "--init",
        "--output-dir",
        "--epochs",
        "--patience",
        "--lambda",
    ] {
        let mut v = argv();
        let i = v.iter().position(|&v| v == flag).unwrap();
        v.drain(i..i + 2);
        assert!(Arguments::try_parse_from(v).is_err());
    }
    for (flag, value) in [
        ("--epochs", "0"),
        ("--epochs", "201"),
        ("--patience", "0"),
        ("--batch", "0"),
        ("--threads", "0"),
        ("--lr", "0"),
        ("--lr", "NaN"),
        ("--weight-decay", "-1"),
        ("--lambda-res", "inf"),
        ("--validation-sample", "0"),
    ] {
        let mut v: Vec<String> = argv().into_iter().map(str::to_owned).collect();
        if let Some(i) = v.iter().position(|v| v == flag) {
            v[i + 1] = value.into();
        } else {
            v.push(format!("{flag}={value}"));
        }
        let result = Arguments::try_parse_from(v);
        assert!(
            result.is_err() || result.unwrap().args.validate().is_err(),
            "{flag}"
        );
    }
    for extra in [
        vec!["--rank", "16"],
        vec!["--lookahead-gamma", "0.9"],
        vec!["--lookahead-plies", "40"],
    ] {
        let mut v = argv();
        v.extend(extra);
        assert!(Arguments::try_parse_from(v).is_err());
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
        .args(["fm", "train", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for flag in ["--resume", "--lambda-res", "--weight-decay", "--threads"] {
        assert!(help.contains(flag));
    }
}

#[test]
fn mirrored_features_and_integer_pst_match_a_physically_reflected_board() {
    let weights: Vec<_> = (0..FEATURE_COUNT)
        .map(|i| [((i % 12).pow(2) * 8) as i16, -((i % 12).pow(2) as i16) * 8])
        .collect();
    let base = Pst::from_weights(
        &weights,
        minase_trainer::pst::v0_piece_values(),
        317.25,
        "L0,P0,R1,E0",
    )
    .unwrap();
    for p in small_positions() {
        let reflected = mirror(&p);
        let record = Record::from_position(
            &p,
            0,
            Outcome::Draw,
            1,
            0,
            minase::training::records::Origin::RandomSelfplay,
        );
        let batch = Batch::from_records(&base, &[record], &[1], &[0.5]).unwrap();
        let mut expected = Vec::new();
        active_features(&reflected, |f| expected.push(f as u32));
        assert_eq!(
            batch.columns()[0]
                .iter()
                .copied()
                .filter(|&f| f != PADDING)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(batch.baseline()[0], pst::evaluate_pst(&base, &reflected));
        assert_ne!(batch.baseline()[0], pst::evaluate_pst(&base, &p));
    }
}

#[test]
fn observations_include_all_training_mirrors_and_exclude_validation_only_features() {
    let (_fixture, args) = Fixture::new(false, false);
    // SAFETY: fixture専用のファイルをDatasetの生存中は変更しない。
    let dataset = unsafe { Dataset::open(&args.data, None).unwrap() };
    let counts = fm::feature_observations(&dataset).unwrap();
    let mut expected = vec![0; FEATURE_COUNT];
    for &i in dataset.training_indices() {
        let p = dataset.record(i).unwrap().to_position().unwrap();
        for p in [&p, &mirror(&p)] {
            active_features(p, |f| expected[f] += 1);
        }
    }
    assert_eq!(counts, expected);
    let mut validation_only = Vec::new();
    for &i in dataset.validation_indices() {
        active_features(&dataset.record(i).unwrap().to_position().unwrap(), |f| {
            if counts[f] == 0 {
                validation_only.push(f)
            }
        });
    }
    assert!(!validation_only.is_empty());
    let model = Model::initialize(counts.iter().map(|&c| c > 0).collect(), 317.25, 7).unwrap();
    assert!(model.a().iter().all(|&v| v == 0.0));
    for f in validation_only {
        assert_eq!(model.v()[f], [0.0; FM_RANK]);
    }
    assert!(
        model
            .v()
            .iter()
            .enumerate()
            .any(|(f, v)| counts[f] > 0 && v != &model.v()[mirror_feature(f)])
    );
    assert_eq!(
        model,
        Model::initialize(model.mask().to_vec(), 317.25, 7).unwrap()
    );
}

#[test]
fn mirror_hash_is_stateless_and_uses_full_epoch_and_position_numbers() {
    let indices = [0, 1, 2, 17, u32::MAX as u64 + 1, u64::MAX];
    let bits: Vec<_> = indices.iter().map(|&i| fm::mirror_bit(7, 3, i)).collect();
    for (&i, &bit) in indices.iter().zip(&bits).rev() {
        assert_eq!(bit, fm::mirror_bit(7, 3, i));
    }
    // mirror_bitのSplitMix64の合成を独立な整数演算で求めた固定値である。
    assert_eq!(bits, vec![1, 1, 0, 0, 1, 1]);
    for (seed, epoch) in [(8, 3), (7, 4)] {
        assert!((0..100).any(|i| fm::mirror_bit(7, 3, i) != fm::mirror_bit(seed, epoch, i)));
    }
}

// Independent oracle: enumerate unordered feature pairs, without the squared-sum identity.
fn pair_loss(model: &Model, batch: &Batch, coefficient: f64) -> f64 {
    let mut total = 0.0;
    for (b, columns) in batch.columns().iter().enumerate() {
        let features: Vec<_> = columns.iter().copied().filter(|&f| f != PADDING).collect();
        let mut phi = 0.0;
        for (i, &left) in features.iter().enumerate() {
            for &right in &features[i + 1..] {
                for f in 0..FM_RANK {
                    phi += f64::from(model.a()[f])
                        * f64::from(model.v()[left as usize][f])
                        * f64::from(model.v()[right as usize][f]);
                }
            }
        }
        let z = f64::from(batch.baseline()[b] as f32 / model.k()) + phi;
        let target = [0.2_f32, 0.8][b] as f64;
        total += z.exp().ln_1p() - z * target + coefficient * phi * phi;
    }
    total / batch.len() as f64
}

#[test]
fn both_gradients_including_regularization_match_pairwise_numerical_derivatives() {
    let (model, batch) = small_batch();
    for lambda in [0.0, 0.7] {
        let g = fm::gradients(&model, &batch, lambda, 3).unwrap();
        assert!((g.losses.total() - pair_loss(&model, &batch, lambda)).abs() < 1e-14);
        let features: Vec<_> = batch
            .columns()
            .iter()
            .flatten()
            .copied()
            .filter(|&f| f != PADDING)
            .collect();
        for &feature in &features {
            for f in [0, 17, 31] {
                let mut v = model.v().to_vec();
                let x = v[feature as usize][f];
                let plus = x + 1e-3;
                let minus = x - 1e-3;
                v[feature as usize][f] = plus;
                let high =
                    Model::from_weights(&v, *model.a(), model.mask().to_vec(), model.k()).unwrap();
                v[feature as usize][f] = minus;
                let low =
                    Model::from_weights(&v, *model.a(), model.mask().to_vec(), model.k()).unwrap();
                let numerical = (pair_loss(&high, &batch, lambda)
                    - pair_loss(&low, &batch, lambda))
                    / f64::from(plus - minus);
                assert!(
                    (f64::from(g.weights[feature as usize * FM_RANK + f]) - numerical).abs()
                        < 1e-6 + numerical.abs() * 5e-4
                );
            }
        }
        for f in 0..FM_RANK {
            let mut a = *model.a();
            let x = a[f];
            let plus = x + 1e-3;
            let minus = x - 1e-3;
            a[f] = plus;
            let high = Model::from_weights(model.v(), a, model.mask().to_vec(), model.k()).unwrap();
            a[f] = minus;
            let low = Model::from_weights(model.v(), a, model.mask().to_vec(), model.k()).unwrap();
            let numerical = (pair_loss(&high, &batch, lambda) - pair_loss(&low, &batch, lambda))
                / f64::from(plus - minus);
            assert!(
                (f64::from(g.weights[(FEATURE_COUNT + 1) * FM_RANK + f]) - numerical).abs()
                    < 1e-6 + numerical.abs() * 5e-4
            );
        }
    }
}

#[test]
fn gradients_and_all_adam_elements_are_bitwise_independent_of_threads() {
    let (model, batch) = small_batch();
    let expected = fm::gradients(&model, &batch, 0.7, 1).unwrap();
    let mut reference = model.clone();
    let mut adam = Adam::new(1e-3, 1e-4).unwrap();
    adam.update(&mut reference, &expected.weights, 1).unwrap();
    for threads in [2, 3, 8] {
        let g = fm::gradients(&model, &batch, 0.7, threads).unwrap();
        assert_eq!(expected.losses, g.losses);
        assert_eq!(
            expected
                .weights
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            g.weights.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
        let mut current = model.clone();
        let mut opt = Adam::new(1e-3, 1e-4).unwrap();
        opt.update(&mut current, &g.weights, threads).unwrap();
        assert_eq!(current, reference);
        assert_eq!(opt, adam);
        let bits = |model: &Model, optimizer: &Adam| {
            model
                .v()
                .iter()
                .flatten()
                .chain(model.a())
                .chain(optimizer.m())
                .chain(optimizer.v())
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        };
        assert_eq!(bits(&current, &opt), bits(&reference, &adam));
    }
    // A row missing from the next batch still decays its moments and its weight.
    let old = reference.clone();
    let old_m = adam.m().to_vec();
    let old_v = adam.v().to_vec();
    adam.update(&mut reference, &vec![0.0; expected.weights.len()], 3)
        .unwrap();
    let i = expected.weights.iter().position(|&g| g != 0.0).unwrap();
    assert_eq!(adam.m()[i], old_m[i] + (0.0 - old_m[i]) * 0.1);
    assert_eq!(adam.v()[i], old_v[i] * 0.999);
    assert_ne!(reference.v(), old.v());
    for (f, &observed) in reference.mask().iter().enumerate() {
        if !observed {
            assert_eq!(reference.v()[f], [0.0; FM_RANK]);
        }
    }
}

#[test]
fn continuous_and_resumed_training_match_weights_moments_losses_and_exports() {
    for version3 in [false, true] {
        let (fixture, args) = Fixture::new(false, version3);
        let summary = fm::train(args.clone()).unwrap();
        assert_eq!(summary.last_epoch, 2);
        assert_eq!(summary.gradients.len(), 5);
        assert_eq!(summary.gradients[0].embedding, 0.0);
        assert!(summary.gradients[1..].iter().any(|g| g.embedding > 0.0));
        let mut split = args.clone();
        split.output_dir = fixture.root.join("resumed");
        split.epochs = 1;
        fm::train(split.clone()).unwrap();
        fs::remove_file(split.output_dir.join("best.bin")).unwrap();
        use std::io::Write;
        fs::OpenOptions::new()
            .append(true)
            .open(split.output_dir.join("training.jsonl"))
            .unwrap()
            .write_all(b"{\"epoch\":2")
            .unwrap();
        split.resume = Some(split.output_dir.clone());
        split.epochs = 2;
        split.patience = 7;
        split.threads = Some(3);
        fm::train(split.clone()).unwrap();
        for file in ["latest.bin", "best.bin"] {
            compare_checkpoints(&args.output_dir.join(file), &split.output_dir.join(file));
        }
        for file in ["fm.bin", "fm-float.npz"] {
            assert_eq!(
                fs::read(args.output_dir.join(file)).unwrap(),
                fs::read(split.output_dir.join(file)).unwrap()
            );
        }
        for (a, b) in logs(&args.output_dir).iter().zip(logs(&split.output_dir)) {
            for key in [
                "epoch",
                "train_loss",
                "total_loss",
                "validation_loss",
                "phi",
                "steps",
                "total_steps",
            ] {
                assert_eq!(a[key], b[key], "{key}");
            }
        }
        let init = Pst::decode(&fs::read(&args.init).unwrap()).unwrap();
        let output = Pst::decode(&fs::read(args.output_dir.join("fm.bin")).unwrap()).unwrap();
        // 初期局面の記録には、保存したPSTとFMの両方を含める。
        let initial_score = pst::evaluate(&output, &Position::initial());
        assert_eq!(summary.initial_position_score, Some(initial_score));
        assert_eq!(
            document(args.output_dir.join("summary.json"))["initial_position_score"],
            initial_score
        );
        assert!(output.fm_weights().is_some());
        assert_eq!(init.weights(), output.weights());
        assert_eq!(init.k().to_bits(), output.k().to_bits());
        assert_eq!(init.rule_set(), output.rule_set());
        for state in 0..pst::PIECE_STATE_COUNT {
            assert_eq!(
                init.piece_value_of_state(state),
                output.piece_value_of_state(state)
            );
        }
        let arrays = npz::decode(&fs::read(args.output_dir.join("fm-float.npz")).unwrap()).unwrap();
        assert_eq!(arrays.len(), 3);
        assert_eq!(arrays["V"].shape, [FEATURE_COUNT, FM_RANK]);
        assert_eq!(arrays["V"].dtype, "<f4");
        assert_eq!(arrays["a"].shape, [FM_RANK]);
        assert_eq!(arrays["mask"].dtype, "|b1");
        assert_eq!(arrays["mask"].shape, [FEATURE_COUNT]);
        assert_eq!(
            document(args.output_dir.join("config.json"))["preprocessing"]["teacher_lambdas"],
            json!([0.75])
        );
    }
}

#[test]
fn resume_rejects_trajectory_changes_and_input_changes_before_writing() {
    let (_fixture, mut args) = Fixture::new(false, false);
    args.epochs = 1;
    args.lookahead_gamma = Some(0.9);
    args.lookahead_plies = Some(40);
    fm::train(args.clone()).unwrap();
    let before = fs::read(args.output_dir.join("latest.bin")).unwrap();
    let config = fs::read(args.output_dir.join("config.json")).unwrap();
    args.resume = Some(args.output_dir.clone());
    args.epochs = 2;
    let changes: Vec<fn(&mut Args)> = vec![
        |a| a.lr = 0.02,
        |a| a.weight_decay = 0.0,
        |a| a.lambda_res = 0.0,
        |a| a.batch = 17,
        |a| a.seed = 2,
        |a| a.lambda = 0.5,
        |a| a.lookahead_gamma = Some(0.8),
        |a| a.lookahead_plies = Some(20),
        |a| a.validation_sample = 8,
    ];
    for change in changes {
        let mut changed = args.clone();
        change(&mut changed);
        let error = fm::train(changed).unwrap_err().to_string();
        assert!(error.contains("hash mismatch"), "{error}");
    }
    let init = Pst::decode(&fs::read(&args.init).unwrap()).unwrap();
    fs::write(
        &args.init,
        Pst::from_weights(
            init.weights(),
            std::array::from_fn(|s| init.piece_value_of_state(s)),
            318.0,
            init.rule_set(),
        )
        .unwrap()
        .encode(),
    )
    .unwrap();
    assert!(
        fm::train(args.clone())
            .unwrap_err()
            .to_string()
            .contains("hash mismatch")
    );
    assert_eq!(
        before,
        fs::read(args.output_dir.join("latest.bin")).unwrap()
    );
    assert_eq!(
        config,
        fs::read(args.output_dir.join("config.json")).unwrap()
    );
}

#[test]
fn equal_losses_keep_epoch_one_and_stop_at_patience_with_zero_output_exclusion() {
    let (_fixture, mut args) = Fixture::new(false, false);
    let initial = Pst::decode(&fs::read(&args.init).unwrap()).unwrap();
    let baseline = Pst::from_weights(
        &vec![[8; 2]; FEATURE_COUNT],
        std::array::from_fn(|state| initial.piece_value_of_state(state)),
        initial.k(),
        initial.rule_set(),
    )
    .unwrap();
    fs::write(&args.init, baseline.encode()).unwrap();
    args.epochs = 10;
    args.patience = 1;
    args.batch = 1000;
    args.lr = 1e-300;
    let summary = fm::train(args.clone()).unwrap();
    assert_eq!(summary.best_epoch, Some(1));
    assert_eq!(summary.last_epoch, 2);
    assert_eq!(summary.stopped_by, fm::StopReason::EarlyStopping);
    assert_eq!(summary.improved_over_epoch_zero, Some(false));
    assert_eq!(summary.status, "excluded_zero_correction");
    // 1枚1 cpのPSTと補正0のFMなので、初期局面の92枚の評価は92 cpとなる。
    assert_eq!(summary.initial_position_score, Some(92));
    assert_eq!(
        document(args.output_dir.join("summary.json"))["initial_position_score"],
        92
    );
    let output = Pst::decode(&fs::read(args.output_dir.join("fm.bin")).unwrap()).unwrap();
    let fm = output.fm_weights().unwrap();
    assert_eq!(fm.exponent, 0);
    assert!(fm.embeddings.iter().flatten().all(|&v| v == 0));
    let rows = logs(&args.output_dir);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["phi"]["mean"], 0.0);
    assert_eq!(rows[0]["phi"]["samples"], 9);
    assert!(rows[0]["train_loss"].is_null());
    assert!(rows[0]["total_loss"].is_null());
    assert!(summary.quarter_zero_check.unwrap().identically_zero);
}

#[test]
fn five_zero_embedding_gradients_fail_and_preserve_their_record() {
    let (_fixture, mut args) = Fixture::new(false, false);
    args.lr = 1e-300;
    assert!(
        fm::train(args.clone())
            .unwrap_err()
            .to_string()
            .contains("stayed zero for 5")
    );
    let summary = document(args.output_dir.join("summary.json"));
    assert_eq!(summary["status"], "error");
    assert_eq!(summary["gradients"].as_array().unwrap().len(), 5);
    assert!(
        summary["gradients"]
            .as_array()
            .unwrap()
            .iter()
            .all(|g| g["embedding"] == 0.0)
    );
    assert!(!args.output_dir.join("fm.bin").exists());
}

#[test]
fn quantization_failure_keeps_both_exports_and_cli_returns_one() {
    let (_fixture, mut args) = Fixture::new(true, false);
    args.epochs = 1;
    fm::train(args.clone()).unwrap();
    // 有効なチェックポイントに正のFM補正を置く。PSTは既に評価上限に達しているため、
    // 実数の合計と切り詰めた整数評価の誤差は2 cpを超える。
    let mut checkpoint = Checkpoint::read(args.output_dir.join("latest.bin")).unwrap();
    let mask = checkpoint.model.mask().to_vec();
    let v = mask
        .iter()
        .map(|&observed| [if observed { 0.5 } else { 0.0 }; FM_RANK])
        .collect::<Vec<_>>();
    checkpoint.model = Model::from_weights(&v, [0.5; FM_RANK], mask, checkpoint.model.k()).unwrap();
    for name in ["latest.bin", "best.bin"] {
        fs::write(args.output_dir.join(name), checkpoint.encode().unwrap()).unwrap();
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
        .args(["fm", "train", "--lambda", "0.75", "--data"])
        .args(&args.data)
        .arg("--init")
        .arg(&args.init)
        .arg("--output-dir")
        .arg(&args.output_dir)
        .arg("--resume")
        .arg(&args.output_dir)
        .args([
            "--batch",
            "16",
            "--epochs",
            "1",
            "--patience",
            "5",
            "--validation-sample",
            "7",
            "--threads",
            "1",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for file in ["fm.bin", "fm-float.npz", "latest.bin", "best.bin"] {
        assert!(args.output_dir.join(file).exists(), "{file}");
    }
    let summary = document(args.output_dir.join("summary.json"));
    assert_eq!(summary["status"], "quantization_error");
    assert_eq!(summary["quantization"]["passed"], false);
    assert!(
        summary["quantization"]["mean_absolute_error_cp"]
            .as_f64()
            .unwrap()
            > 2.0
    );
    assert_eq!(summary["initial_position_score"], 28999);
}

#[test]
fn checkpoint_rejects_corruption_and_output_directory_must_be_empty() {
    let (fixture, mut args) = Fixture::new(false, true);
    args.epochs = 1;
    fm::train(args.clone()).unwrap();
    let bytes = fs::read(args.output_dir.join("latest.bin")).unwrap();
    assert_eq!(bytes, Checkpoint::decode(&bytes).unwrap().encode().unwrap());
    for index in [0, 8, 12, bytes.len() / 2, bytes.len() - 1] {
        let mut changed = bytes.clone();
        changed[index] ^= 1;
        assert!(Checkpoint::decode(&changed).is_err());
    }
    for end in [0, 8, 112, bytes.len() - 1] {
        assert!(Checkpoint::decode(&bytes[..end]).is_err());
    }
    assert!(minase_trainer::pst::Checkpoint::decode(&bytes).is_err());
    assert!(
        fm::train(args.clone())
            .unwrap_err()
            .to_string()
            .contains("not empty")
    );
    args.output_dir = fixture.root.join("empty");
    fs::create_dir(&args.output_dir).unwrap();
    fm::train(args).unwrap();
}

// Phase 4 may expose components, but the existing trainers must retain their bit patterns.
#[test]
fn phase4_preserves_existing_trainer_weights_and_adam_states() {
    let (_fixture, mut args) = Fixture::new(false, true);
    args.epochs = 1;
    fm::train(args.clone()).unwrap();
    let checkpoint = Checkpoint::read(args.output_dir.join("latest.bin")).unwrap();
    let mut hash = Sha256::new();
    for value in checkpoint
        .model
        .v()
        .iter()
        .flatten()
        .chain(checkpoint.model.a())
        .chain(checkpoint.optimizer.m())
        .chain(checkpoint.optimizer.v())
    {
        hash.update(value.to_le_bytes());
    }
    assert_eq!(
        format!("{:x}", hash.finalize()),
        "d26cc3c6ffcf10b8e873ce7aba9c582e86e344d53da8424df0160a098057bca9"
    );
    let output = args.output_dir.with_file_name("pst");
    minase_trainer::pst::run(minase_trainer::pst::Args {
        data: args.data,
        init: args.init,
        output_dir: output.clone(),
        lr: 0.03,
        epochs: 1,
        patience: 5,
        batch: 16,
        seed: 1,
        removal_penalty: 1000.0,
        removal_reference: minase_trainer::pst::RemovalReferenceKind::V0,
        lambda: 0.75,
        lookahead_gamma: None,
        lookahead_plies: None,
        validation_sample: 7,
        threads: Some(1),
        resume: None,
    })
    .unwrap();
    let checkpoint = minase_trainer::pst::Checkpoint::read(output.join("latest.bin")).unwrap();
    let mut hash = Sha256::new();
    for value in checkpoint
        .model
        .weights()
        .iter()
        .chain(checkpoint.optimizer.m())
        .chain(checkpoint.optimizer.v())
    {
        hash.update(value.to_le_bytes());
    }
    assert_eq!(
        format!("{:x}", hash.finalize()),
        "45f03c5a8cc011690326d3501e6281d9efdcb67091f871eefdc1b3dcb83ce8f9"
    );
}
