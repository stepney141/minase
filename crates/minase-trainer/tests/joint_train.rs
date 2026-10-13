//! 同時学習の損失、初期値照合、再現性、保存物と再開の拒否条件を検証する。
#[path = "joint_train/algebra.rs"]
mod algebra;
#[path = "common/checkpoint.rs"]
#[allow(dead_code)]
mod checkpoint_contract;
#[path = "joint_train/fixture.rs"]
mod support;
use clap::Parser;
use minase::{
    eval::{
        features::FEATURE_COUNT,
        pst::{self, FM_RANK, Pst},
    },
    training::records::{Outcome, Record},
};
use minase_core::{Color, PieceCode, PieceKind, Position, PositionBuilder, Square};
use minase_trainer::{
    data::Dataset,
    fm,
    joint::{self, Args, Checkpoint, Model},
    nn::npz,
};
use serde_json::{Value, json};
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
        "--base",
        "base.bin",
        "--pst-init",
        "pst.npz",
        "--fm-init",
        "fm.npz",
        "--fm-reference",
        "reference.bin",
        "--output-dir",
        "output",
    ]
}
fn position(
    pieces: &[(u8, u8, Color, PieceKind)],
    side: Color,
    lion: Option<(u8, u8)>,
) -> Position {
    let mut b = PositionBuilder::new(side);
    for &(f, r, c, p) in pieces {
        b.put(Square::new(f, r).unwrap(), PieceCode::new(c, p).unwrap())
            .unwrap();
    }
    let mut p = b.finish().unwrap();
    if let Some((f, r)) = lion {
        p.set_lion_capture(Some(Square::new(f, r).unwrap()))
            .unwrap();
    }
    p
}
fn float_array(values: &[f32], shape: &[usize]) -> npz::Array {
    npz::Array {
        dtype: "<f4".into(),
        shape: shape.to_vec(),
        data: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
    }
}
fn fm_npz(model: &fm::Model) -> Vec<u8> {
    npz::encode(&[
        (
            "V",
            float_array(
                &model.v().iter().flatten().copied().collect::<Vec<_>>(),
                &[FEATURE_COUNT, FM_RANK],
            ),
        ),
        ("a", float_array(model.a(), &[FM_RANK])),
        (
            "mask",
            npz::Array {
                dtype: "|b1".into(),
                shape: vec![FEATURE_COUNT],
                data: model.mask().iter().map(|&v| u8::from(v)).collect(),
            },
        ),
    ])
    .unwrap()
}
fn dataset(args: &Args) -> Dataset {
    // SAFETY: Each test owns these files, and leaves them unchanged while the Dataset is alive.
    unsafe {
        Dataset::open(&args.data, None)
            .unwrap()
            .with_lambda(args.lambda)
            .unwrap()
    }
}
fn initialize(args: &Args) -> (Model, usize) {
    let ds = dataset(args);
    let mask = fm::feature_observations(&ds)
        .unwrap()
        .iter()
        .map(|&v| v > 0)
        .collect();
    joint::initialize(
        &Pst::decode(&fs::read(&args.base).unwrap()).unwrap(),
        &args.pst_init,
        &args.fm_init,
        &Pst::decode(&fs::read(&args.fm_reference).unwrap()).unwrap(),
        mask,
    )
    .unwrap()
}

#[test]
fn cli_defaults_and_invalid_arguments() {
    let args = Arguments::try_parse_from(argv()).unwrap().args;
    args.validate().unwrap();
    assert_eq!(
        (args.epochs, args.patience, args.batch, args.seed),
        (200, 5, 16384, 1)
    );
    assert_eq!(
        (
            args.pst_lr,
            args.fm_lr,
            args.fm_weight_decay,
            args.lambda_res,
            args.removal_penalty
        ),
        (0.03, 0.001, 0.0001, 0.0001, 1000.0)
    );
    assert_eq!(args.lambda, 0.75);
    for flag in [
        "--base",
        "--pst-init",
        "--fm-init",
        "--fm-reference",
        "--data",
        "--output-dir",
    ] {
        let mut v = argv();
        let i = v.iter().position(|&v| v == flag).unwrap();
        v.drain(i..i + 2);
        assert!(Arguments::try_parse_from(v).is_err());
    }
    for (flag, value) in [
        ("--epochs", "201"),
        ("--epochs", "0"),
        ("--patience", "0"),
        ("--batch", "0"),
        ("--threads", "0"),
        ("--pst-lr", "NaN"),
        ("--fm-lr", "0"),
        ("--fm-weight-decay", "-1"),
        ("--lambda-res", "inf"),
        ("--removal-penalty", "-1"),
        ("--validation-sample", "0"),
    ] {
        let mut v: Vec<_> = argv().into_iter().map(str::to_owned).collect();
        v.push(format!("{flag}={value}"));
        let result = Arguments::try_parse_from(v);
        assert!(
            result.is_err() || result.unwrap().args.validate().is_err(),
            "{flag}"
        );
    }
    let help = std::process::Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
        .args(["joint", "train", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
}

#[test]
fn initialization_verifies_raw_endpoints_and_unscaled_fm_then_recounts_mask() {
    let (_fixture, args) = Fixture::new(false);
    let (model, mismatches) = initialize(&args);
    assert!(mismatches > 0);
    assert_eq!(mismatches, model.fm.mask().iter().filter(|&&v| !v).count());
    for (v, &observed) in model.fm.v().iter().zip(model.fm.mask()) {
        if !observed {
            assert_eq!(*v, [0.0; FM_RANK]);
        }
    }
    let base = Pst::decode(&fs::read(&args.base).unwrap()).unwrap();
    let reference = Pst::decode(&fs::read(&args.fm_reference).unwrap()).unwrap();
    let mask = model.fm.mask().to_vec();
    for key in ["middlegame", "endgame", "V", "a"] {
        let path = if ["V", "a"].contains(&key) {
            &args.fm_init
        } else {
            &args.pst_init
        };
        let original = fs::read(path).unwrap();
        let mut arrays = npz::decode(&original).unwrap();
        arrays.get_mut(key).unwrap().data[..4].copy_from_slice(&3.0_f32.to_le_bytes());
        fs::write(
            path,
            npz::encode(
                &arrays
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.clone()))
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            joint::initialize(
                &base,
                &args.pst_init,
                &args.fm_init,
                &reference,
                mask.clone()
            )
            .is_err(),
            "{key}"
        );
        fs::write(path, original).unwrap();
    }
    // Each raw value quantizes to zero, but the mirrored floating average is nonzero.
    let mut arrays = npz::decode(&fs::read(&args.pst_init).unwrap()).unwrap();
    arrays.get_mut("middlegame").unwrap().data[..4].copy_from_slice(&0.04_f32.to_le_bytes());
    fs::write(
        &args.pst_init,
        npz::encode(
            &arrays
                .iter()
                .map(|(k, v)| (k.as_str(), v.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap(),
    )
    .unwrap();
    let (model, _) =
        joint::initialize(&base, &args.pst_init, &args.fm_init, &reference, mask).unwrap();
    assert_eq!(model.pst.expanded()[0][0].to_bits(), 0.02_f32.to_bits());
    assert_eq!(model.pst.expanded()[11][0].to_bits(), 0.02_f32.to_bits());
}

#[test]
fn continuous_and_resumed_training_match_all_weights_states_losses_and_outputs() {
    let (fixture, mut args) = Fixture::new(false);
    args.epochs = 2;
    let summary = joint::train(args.clone()).unwrap();
    assert_eq!(summary.last_epoch, 2);
    assert_eq!(summary.gradients.len(), 5);
    for group in [0, 1, 2] {
        assert!(
            summary
                .gradients
                .iter()
                .any(|g| [g.pst, g.embedding, g.a][group] > 0.0)
        );
    }
    assert!(
        summary.initial_quantization.as_ref().unwrap().passed
            && summary.quantization.as_ref().unwrap().passed
    );
    let mut split = args.clone();
    split.output_dir = fixture.root.join("resumed");
    split.epochs = 1;
    joint::train(split.clone()).unwrap();
    fs::remove_file(split.output_dir.join("best.npz")).unwrap();
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(split.output_dir.join("training.jsonl"))
        .unwrap()
        .write_all(b"{\"epoch\":2")
        .unwrap();
    split.epochs = 2;
    split.patience = 7;
    split.threads = Some(3);
    split.resume = Some(split.output_dir.clone());
    joint::train(split.clone()).unwrap();
    for name in ["latest.npz", "best.npz"] {
        compare_checkpoints(&args.output_dir.join(name), &split.output_dir.join(name));
    }
    for name in ["pst.bin", "pst-float.npz"] {
        assert_eq!(
            fs::read(args.output_dir.join(name)).unwrap(),
            fs::read(split.output_dir.join(name)).unwrap()
        );
    }
    let continuous = logs(&args.output_dir);
    let resumed = logs(&split.output_dir);
    assert_eq!(continuous.len(), 3);
    assert_eq!(resumed.len(), continuous.len());
    for (a, b) in continuous.iter().zip(resumed) {
        for key in [
            "epoch",
            "train_loss",
            "removal_loss",
            "phi_squared_mean",
            "total_loss",
            "validation_loss",
            "phi",
            "total_steps",
        ] {
            assert_eq!(a[key], b[key], "{key}");
            if key.ends_with("loss") || key == "phi_squared_mean" {
                assert_eq!(
                    a[key].as_f64().map(f64::to_bits),
                    b[key].as_f64().map(f64::to_bits),
                    "{key}"
                );
            }
        }
    }
    let base = Pst::decode(&fs::read(&args.base).unwrap()).unwrap();
    let output = Pst::decode(&fs::read(args.output_dir.join("pst.bin")).unwrap()).unwrap();
    assert!(output.fm_weights().is_some());
    assert_eq!(base.k().to_bits(), output.k().to_bits());
    assert_eq!(base.rule_set(), output.rule_set());
    for s in 0..pst::PIECE_STATE_COUNT {
        assert_eq!(base.piece_value_of_state(s), output.piece_value_of_state(s));
    }
    let bytes = fs::read(args.output_dir.join("pst-float.npz")).unwrap();
    let arrays = npz::decode(&bytes).unwrap();
    assert_eq!(arrays.len(), 5);
    assert_eq!(arrays["V"].shape, [FEATURE_COUNT, FM_RANK]);
    assert_eq!(arrays["mask"].shape, [FEATURE_COUNT]);
    let mut previous = 0;
    for key in [
        "middlegame.npy",
        "endgame.npy",
        "V.npy",
        "a.npy",
        "mask.npy",
    ] {
        let offset = bytes
            .windows(key.len())
            .position(|b| b == key.as_bytes())
            .unwrap();
        assert!(offset > previous);
        previous = offset;
    }
    let config = document(args.output_dir.join("config.json"));
    assert_eq!(config["preprocessing"]["teacher_lambdas"], json!([0.75]));
    assert!(config["preprocessing"]["mask_mismatches"].as_u64().unwrap() > 0);
}

#[test]
fn resume_rejects_trajectory_changes_before_writing() {
    let (_fixture, mut args) = Fixture::new(false);
    args.epochs = 1;
    joint::train(args.clone()).unwrap();
    args.resume = Some(args.output_dir.clone());
    args.epochs = 2;
    let original = fs::read(args.output_dir.join("latest.npz")).unwrap();
    let config = fs::read(args.output_dir.join("config.json")).unwrap();
    checkpoint_contract::rejects_changed_meta(&args.output_dir, || {
        joint::train(args.clone()).unwrap_err().to_string()
    });
    let bytes = fs::read(args.output_dir.join("latest.npz")).unwrap();
    checkpoint_contract::rejects_invalid_schema(&bytes, |bytes| Checkpoint::decode(bytes).is_ok());
    let changes: Vec<fn(&mut Args)> = vec![
        |a| a.pst_lr = 0.04,
        |a| a.fm_lr = 0.02,
        |a| a.fm_weight_decay = 0.0,
        |a| a.lambda_res = 0.0,
        |a| a.removal_penalty = 0.0,
        |a| a.batch = 17,
        |a| a.seed = 2,
        |a| a.lambda = 0.5,
        |a| a.validation_sample = 8,
        |a| {
            a.lookahead_gamma = Some(0.9);
            a.lookahead_plies = Some(40);
        },
    ];
    for change in changes {
        let mut changed = args.clone();
        change(&mut changed);
        assert!(
            joint::train(changed)
                .unwrap_err()
                .to_string()
                .contains("mismatch")
        );
    }
    assert_eq!(
        original,
        fs::read(args.output_dir.join("latest.npz")).unwrap()
    );
    assert_eq!(
        config,
        fs::read(args.output_dir.join("config.json")).unwrap()
    );
}

#[test]
fn initial_quantization_failure_is_recorded_and_exits_with_one() {
    let (_fixture, args) = Fixture::new(true);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
        .args(["joint", "train", "--lambda", "0.75", "--data"])
        .arg(&args.data[0])
        .arg("--base")
        .arg(&args.base)
        .arg("--pst-init")
        .arg(&args.pst_init)
        .arg("--fm-init")
        .arg(&args.fm_init)
        .arg("--fm-reference")
        .arg(&args.fm_reference)
        .arg("--output-dir")
        .arg(&args.output_dir)
        .args(["--epochs", "1", "--threads", "1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let summary = document(args.output_dir.join("summary.json"));
    assert_eq!(summary["status"], "quantization_error");
    assert_eq!(summary["initial_quantization"]["passed"], false);
    assert!(!args.output_dir.join("pst.bin").exists());
}

#[test]
fn exported_quantization_failure_keeps_weights_and_records_error() {
    let (_fixture, mut args) = Fixture::new(false);
    args.epochs = 1;
    args.batch = 1000;
    // A large learned score on a dense board exceeds the integer evaluation limit.
    // The initial zero PST has no such clipping error.
    let mut pieces = Vec::new();
    for i in 0..100 {
        pieces.push((
            (i % 12) as u8,
            (i / 12) as u8,
            Color::Black,
            PieceKind::Pawn,
        ));
    }
    let p = position(&pieces, Color::Black, None);
    let original = fs::read(&args.data[0]).unwrap();
    let mut bytes = original[..136].to_vec();
    for game in 0..40 {
        for ply in 0..3 {
            bytes.extend(
                Record::from_position(
                    &p,
                    300,
                    Outcome::Win,
                    game,
                    ply,
                    minase::training::records::Origin::RandomSelfplay,
                )
                .encode(),
            );
        }
    }
    fs::write(&args.data[0], &bytes).unwrap();
    args.pst_lr = 4095.875;
    args.removal_penalty = 0.0;
    let result = joint::train(args.clone());
    assert!(result.is_err());
    let summary = document(args.output_dir.join("summary.json"));
    assert_eq!(summary["status"], "quantization_error", "{summary}");
    assert_eq!(summary["quantization"]["passed"], false);
    for name in ["pst.bin", "pst-float.npz", "best.npz", "latest.npz"] {
        assert!(args.output_dir.join(name).exists(), "{name}");
    }
}

#[test]
fn five_zero_coefficient_gradients_fail_with_norms_preserved() {
    let (_fixture, mut args) = Fixture::new(false);
    // Only the friendly king carries V, so there is no nonzero feature pair.
    // Phi and every dL/da are zero, while V can still receive a gradient.
    let mut v = vec![[0.0; FM_RANK]; FEATURE_COUNT];
    let piece = PieceCode::new(Color::Black, PieceKind::King).unwrap();
    let f = minase::eval::features::feature_index(Color::Black, piece, Square::new(4, 0).unwrap());
    v[f][0] = 0.01;
    v[minase::eval::features::mirror_feature(f)][0] = 0.01;
    let a = [0.3; FM_RANK];
    let model = fm::Model::from_weights(&v, a, vec![true; FEATURE_COUNT], 317.25).unwrap();
    fs::write(&args.fm_init, fm_npz(&model)).unwrap();
    let q = fm::quantize(
        &v.iter().map(|r| r.map(f64::from)).collect::<Vec<_>>(),
        &a.map(f64::from),
        317.25,
    )
    .unwrap();
    let base = Pst::decode(&fs::read(&args.base).unwrap()).unwrap();
    let reference = Pst::from_weights_with_fm(
        base.weights(),
        std::array::from_fn(|s| base.piece_value_of_state(s)),
        base.k(),
        base.rule_set(),
        q.as_weights(),
    )
    .unwrap();
    fs::write(&args.fm_reference, reference.encode()).unwrap();
    // Subnormal learning rates round every FM update to zero, preserving zero a gradients.
    args.fm_lr = f64::MIN_POSITIVE;
    args.fm_weight_decay = 0.0;
    args.epochs = 1;
    let error = joint::train(args.clone()).unwrap_err().to_string();
    assert!(error.contains("a loss gradient stayed zero"), "{error}");
    let summary = document(args.output_dir.join("summary.json"));
    assert_eq!(summary["status"], "error");
    assert_eq!(summary["gradients"].as_array().unwrap().len(), 5);
}
