//! pst trainの引数、端数バッチ、再開と成果物を合成MNSDで検証する。
mod common;
use clap::Parser;
use minase::eval::{
    features::FEATURE_COUNT,
    pst::{FM_RANK, Pst},
};
use minase::training::records::{Header, Outcome, Record};
use minase_core::{Color, PieceKind};
use minase_trainer::{
    data::{Dataset, build_targets_from_arrays, epoch_permutation},
    pst::{
        self, Adam, Args, Batch, Checkpoint, Model, RemovalReference, RemovalReferenceKind,
        v0_piece_values,
    },
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Parser)]
struct Arguments {
    #[command(flatten)]
    args: Args,
}
fn argv() -> Vec<&'static str> {
    vec![
        "minase-trainer",
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
        "--removal-reference",
        "init",
    ]
}

#[test]
fn pst_cli_requires_explicit_budgets_and_reference_and_accepts_only_valid_values() {
    let args = Arguments::try_parse_from(argv()).unwrap().args;
    args.validate().unwrap();
    assert_eq!(
        (
            args.lr,
            args.batch,
            args.seed,
            args.removal_penalty,
            args.validation_sample
        ),
        (0.03, 16384, 1, 1000.0, 10000)
    );
    assert_eq!(args.lambda, 0.75);
    assert_eq!(args.threads, None);
    let mut large = args.clone();
    large.batch = 1 << 24;
    large.validate().unwrap();
    large.batch = usize::MAX;
    assert!(large.validate().is_err());
    for flag in [
        "--data",
        "--init",
        "--output-dir",
        "--epochs",
        "--patience",
        "--removal-reference",
    ] {
        let mut values = argv();
        let i = values.iter().position(|&v| v == flag).unwrap();
        values.drain(i..i + 2);
        assert!(Arguments::try_parse_from(values).is_err(), "{flag}");
    }
    for (flag, value) in [
        ("--epochs", "0"),
        ("--epochs", "201"),
        ("--patience", "0"),
        ("--lr", "0"),
        ("--lr", "NaN"),
        ("--removal-penalty", "inf"),
        ("--removal-penalty", "-1"),
        ("--batch", "0"),
        ("--threads", "0"),
        ("--validation-sample", "0"),
    ] {
        let mut values: Vec<String> = argv().into_iter().map(str::to_owned).collect();
        if let Some(i) = values.iter().position(|v| v == flag) {
            values[i + 1] = value.into();
        } else {
            values.push(format!("{flag}={value}"));
        }
        let result = Arguments::try_parse_from(values);
        assert!(
            result.is_err() || result.unwrap().args.validate().is_err(),
            "{flag}={value}"
        );
    }
    for extra in [
        vec!["--lookahead-gamma", "0.9"],
        vec!["--lookahead-plies", "40"],
        vec!["--lookahead-gamma", "1", "--lookahead-plies", "40"],
    ] {
        let mut values = argv();
        values.extend(extra);
        let result = Arguments::try_parse_from(values);
        assert!(result.is_err() || result.unwrap().args.validate().is_err());
    }
    let mut values = argv();
    values[8] = "200";
    values.insert(
        values.iter().position(|&arg| arg == "--data").unwrap() + 2,
        "second.bin",
    );
    values.extend(["--lookahead-gamma", "0.9", "--lookahead-plies", "40"]);
    let args = Arguments::try_parse_from(values).unwrap().args;
    args.validate().unwrap();
    assert_eq!(args.data.len(), 2);
    let help = std::process::Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
        .args(["pst", "train", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    for flag in [
        "--removal-reference",
        "--epochs",
        "--resume",
        "--validation-sample",
    ] {
        assert!(help.contains(flag));
    }
}

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
impl Fixture {
    fn new(large_evaluation: bool, version3: bool) -> (Self, Args) {
        let root = std::env::temp_dir().join(format!(
            "pst-training-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self { root };
        let mut args = Arguments::try_parse_from(argv()).unwrap().args;
        args.data = vec![fixture.root.join("records.bin")];
        args.init = fixture.root.join("init.bin");
        args.output_dir = fixture.root.join("continuous");
        args.batch = 16;
        args.validation_sample = 7;
        args.threads = Some(1);
        args.lookahead_gamma = Some(0.9);
        args.lookahead_plies = Some(40);
        let weights = vec![
            if large_evaluation {
                [4000 * 8; 2]
            } else {
                [0; 2]
            };
            FEATURE_COUNT
        ];
        let mut initial = Pst::from_weights(
            &weights,
            v0_piece_values().map(|value| value * 2),
            317.25,
            "L0,P0,R1,E0",
        )
        .unwrap()
        .encode();
        if version3 {
            initial[4..8].copy_from_slice(&3_u32.to_le_bytes());
            initial.extend((FM_RANK as u32).to_le_bytes());
            initial.extend(7_u32.to_le_bytes());
            initial.extend([1_u8; FM_RANK]);
            initial.extend(vec![3; FEATURE_COUNT * FM_RANK * 2]);
            let checksum = Sha256::digest(&initial[80..]);
            initial[48..80].copy_from_slice(&checksum);
        }
        fs::write(&args.init, initial).unwrap();
        let header =
            Header::new("L0,P0,R1,E0".into(), "1".repeat(40), [2; 32], 200, 11, 120).unwrap();
        let mut bytes = header.encode().to_vec();
        for game in 0..40 {
            for ply in 0..3 {
                let side = if ply % 2 == 0 {
                    Color::Black
                } else {
                    Color::White
                };
                let mut pieces = vec![
                    (4, 0, Color::Black, PieceKind::King),
                    (7, 11, Color::White, PieceKind::King),
                    ((game % 12) as u8, 3, Color::Black, PieceKind::Pawn),
                    (((game + 6) % 12) as u8, 8, Color::White, PieceKind::Pawn),
                    (((game + 3) % 12) as u8, 6, Color::Black, PieceKind::Rook),
                ];
                if large_evaluation {
                    pieces.extend([
                        (0, 2, Color::Black, PieceKind::Pawn),
                        (1, 2, Color::Black, PieceKind::Pawn),
                        (2, 2, Color::Black, PieceKind::Pawn),
                    ]);
                }
                let p = common::position(&pieces, side, None);
                bytes.extend(
                    Record::from_position(
                        &p,
                        if side == Color::Black { 300 } else { -300 },
                        if game % 3 == 0 {
                            Outcome::Draw
                        } else {
                            Outcome::Win
                        },
                        game,
                        ply,
                        minase::training::records::Origin::RandomSelfplay,
                    )
                    .encode(),
                );
            }
        }
        fs::write(&args.data[0], &bytes).unwrap();
        (fixture, args)
    }
}
fn logs(directory: &Path) -> Vec<Value> {
    fs::read_to_string(directory.join("training.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn document(path: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn compare_checkpoints(left: &Path, right: &Path) {
    let mut left = Checkpoint::read(left).unwrap();
    let mut right = Checkpoint::read(right).unwrap();
    left.progress.elapsed_seconds = 0.0;
    right.progress.elapsed_seconds = 0.0;
    assert_eq!(left.encode().unwrap(), right.encode().unwrap());
}

#[test]
fn continuous_two_epochs_equal_one_plus_resume_in_weights_moments_and_logs() {
    for kind in [RemovalReferenceKind::V0, RemovalReferenceKind::Init] {
        let (fixture, mut args) = Fixture::new(false, kind == RemovalReferenceKind::Init);
        args.removal_reference = kind;
        let summary = pst::run(args.clone()).unwrap();
        assert_eq!(summary.last_epoch, 2);
        let mut split = args.clone();
        split.output_dir = fixture.root.join("resumed");
        split.epochs = 1;
        pst::run(split.clone()).unwrap();
        // 改善したlatestを公開した直後、bestの保存前に中断した場合も再開できる。
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
        pst::run(split.clone()).unwrap();
        for file in ["latest.bin", "best.bin"] {
            compare_checkpoints(&args.output_dir.join(file), &split.output_dir.join(file));
        }
        for file in ["pst.bin", "pst-float.npz"] {
            assert_eq!(
                fs::read(args.output_dir.join(file)).unwrap(),
                fs::read(split.output_dir.join(file)).unwrap()
            );
        }
        let ck = Checkpoint::read(args.output_dir.join("latest.bin")).unwrap();
        assert_eq!(ck.optimizer.step(), 14); // 111訓練局面、16件×6バッチ＋15件の端数、2エポック。
        let rows = logs(&args.output_dir);
        let resumed = logs(&split.output_dir);
        assert_eq!(rows.len(), 3);
        for (i, (left, right)) in rows.iter().zip(resumed).enumerate() {
            for field in [
                "epoch",
                "train_loss",
                "removal_loss",
                "total_loss",
                "validation_loss",
                "steps",
                "total_steps",
            ] {
                assert_eq!(left[field], right[field], "{field}");
            }
            for absent in ["saturation", "nonlinear_std_cp", "gpu_memory_used_bytes"] {
                assert!(left.get(absent).is_none());
            }
            if i == 0 {
                assert!(left["train_loss"].is_null());
            } else {
                let total = left["train_loss"].as_f64().unwrap()
                    + args.removal_penalty * left["removal_loss"].as_f64().unwrap();
                assert!((left["total_loss"].as_f64().unwrap() - total).abs() < 1e-12);
            }
        }
        let config = document(split.output_dir.join("config.json"));
        assert_eq!(
            config["preprocessing"]["teacher_lambdas"],
            serde_json::json!([0.75])
        );
        assert_eq!(config["resumes"][0]["workers"], 3);
        let written = Pst::decode(&fs::read(args.output_dir.join("pst.bin")).unwrap()).unwrap();
        // 初期局面の記録は、書き出した整数重みを手番側から評価した値とする。
        let initial_score =
            minase::eval::pst::evaluate(&written, &minase_core::Position::initial());
        assert_eq!(summary.initial_position_score, Some(initial_score));
        assert_eq!(
            document(args.output_dir.join("summary.json"))["initial_position_score"],
            initial_score
        );
        assert_eq!(written.k(), 317.25);
        assert_eq!(written.rule_set(), "L0,P0,R1,E0");
        for (state, value) in v0_piece_values().into_iter().enumerate() {
            assert_eq!(written.piece_value_of_state(state), value * 2);
        }
        assert_eq!(summary.quantization.unwrap().samples, 7);
    }
}

#[test]
fn training_log_weights_the_tail_by_positions_and_validation_has_no_removal_term() {
    let (_fixture, mut args) = Fixture::new(false, false);
    args.epochs = 1;
    args.removal_reference = RemovalReferenceKind::V0;
    // SAFETY: fixture専用の入力はDatasetの生存中に変更しない。
    let dataset = unsafe {
        Dataset::open(
            &args.data,
            Some(minase_trainer::data::Lookahead::new(0.9, 40).unwrap()),
        )
        .unwrap()
    };
    let dataset = dataset.with_lambda(args.lambda).unwrap();
    let ks = dataset.estimate_teacher_ks().unwrap();
    let mut model = Model::initialize(&vec![[0.0; 2]; FEATURE_COUNT], 317.25).unwrap();
    let mut opt = Adam::new(0.03).unwrap();
    let order = epoch_permutation(dataset.training_indices(), args.seed, 1);
    assert_eq!(order.len() % args.batch, 15);
    let mut bce = 0.0;
    let mut removal = 0.0;
    for indices in order.chunks(args.batch) {
        let batch = Batch::from_indices(&dataset, indices).unwrap();
        let targets = build_targets_from_arrays(
            batch.teacher_scores(),
            batch.teacher_classes(),
            batch.outcomes(),
            &ks,
            dataset.teacher_lambdas(),
        )
        .unwrap();
        let g = pst::gradients(
            &model,
            &batch,
            &targets,
            Some(&RemovalReference::v0()),
            1000.0,
            1,
        )
        .unwrap();
        bce += g.losses.bce * indices.len() as f64;
        removal += g.losses.removal * indices.len() as f64;
        opt.update(&mut model, &g.weights).unwrap();
    }
    pst::run(args.clone()).unwrap();
    let rows = logs(&args.output_dir);
    assert!((rows[1]["train_loss"].as_f64().unwrap() - bce / order.len() as f64).abs() < 1e-14);
    assert!(
        (rows[1]["removal_loss"].as_f64().unwrap() - removal / order.len() as f64).abs() < 1e-14
    );
    assert!((rows[0]["validation_loss"].as_f64().unwrap() - 2.0_f64.ln()).abs() < 1e-14);
    let ck = Checkpoint::read(args.output_dir.join("latest.bin")).unwrap();
    assert_eq!(ck.model, model);
    assert_eq!(ck.optimizer, opt);
}

#[test]
fn resume_rejects_changes_to_trajectory_settings_and_input_without_changing_saved_run() {
    let (_fixture, mut args) = Fixture::new(false, false);
    args.epochs = 1;
    pst::run(args.clone()).unwrap();
    args.epochs = 2;
    args.resume = Some(args.output_dir.clone());
    let before = fs::read(args.output_dir.join("latest.bin")).unwrap();
    let changes: Vec<fn(&mut Args)> = vec![
        |a| a.lr = 0.02,
        |a| a.batch = 17,
        |a| a.seed = 2,
        |a| a.removal_penalty = 0.0,
        |a| a.removal_reference = RemovalReferenceKind::V0,
        |a| a.lambda = 0.5,
        |a| a.lookahead_gamma = Some(0.8),
        |a| a.lookahead_plies = Some(20),
        |a| a.validation_sample = 8,
    ];
    for change in changes {
        let mut changed = args.clone();
        change(&mut changed);
        let error = pst::run(changed).unwrap_err().to_string();
        assert!(error.contains("hash mismatch"), "{error}");
    }
    let base = Pst::decode(&fs::read(&args.init).unwrap()).unwrap();
    let modified =
        Pst::from_weights(base.weights(), v0_piece_values(), 318.0, base.rule_set()).unwrap();
    fs::write(&args.init, modified.encode()).unwrap();
    assert!(
        pst::run(args.clone())
            .unwrap_err()
            .to_string()
            .contains("hash mismatch")
    );
    assert_eq!(
        fs::read(args.output_dir.join("latest.bin")).unwrap(),
        before
    );
}

#[test]
fn quantization_failure_keeps_outputs_and_records_failure_with_exit_code_one() {
    let (_fixture, mut args) = Fixture::new(true, true);
    args.epochs = 1;
    args.removal_penalty = 0.0;
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
        .args(["pst", "train", "--lambda", "0.75", "--data"])
        .arg(&args.data[0])
        .arg("--init")
        .arg(&args.init)
        .arg("--output-dir")
        .arg(&args.output_dir)
        .args([
            "--epochs",
            "1",
            "--patience",
            "1",
            "--removal-reference",
            "init",
            "--removal-penalty",
            "0",
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
    for file in ["pst.bin", "pst-float.npz", "latest.bin", "best.bin"] {
        assert!(args.output_dir.join(file).exists());
    }
    let summary = document(args.output_dir.join("summary.json"));
    assert_eq!(summary["status"], "quantization_error");
    assert_eq!(summary["quantization"]["passed"], false);
    // 92枚に各4000 cpの初期値を与えたため、学習後も整数評価は上限に達する。
    assert_eq!(summary["initial_position_score"], 28999);
    assert!(
        summary["quantization"]["mean_absolute_error_cp"]
            .as_f64()
            .unwrap()
            > 2.0
    );
}

#[test]
fn asymmetric_init_is_averaged_except_when_positive_init_reference_requires_symmetry() {
    let (fixture, mut args) = Fixture::new(false, false);
    args.epochs = 1;
    let mut weights = vec![[0; 2]; FEATURE_COUNT];
    weights[0] = [8, 16];
    fs::write(
        &args.init,
        Pst::from_weights(&weights, v0_piece_values(), 317.25, "L0,P0,R1,E0")
            .unwrap()
            .encode(),
    )
    .unwrap();
    assert!(
        pst::run(args.clone())
            .unwrap_err()
            .to_string()
            .contains("mirror sharing")
    );
    args.removal_penalty = 0.0;
    pst::run(args.clone()).unwrap();
    let ck = Checkpoint::read(args.output_dir.join("latest.bin")).unwrap();
    assert_eq!(&ck.model.weights()[..2], &[0.5, 1.0]);
    args.output_dir = fixture.root.join("v0");
    args.removal_penalty = 1000.0;
    args.removal_reference = RemovalReferenceKind::V0;
    pst::run(args).unwrap();
}

#[test]
fn epoch_zero_is_not_a_candidate_and_equal_losses_stop_at_patience() {
    let (_fixture, mut args) = Fixture::new(false, false);
    args.epochs = 10;
    args.patience = 1;
    // f32の更新量が0になる正の学習率。検証損失が厳密に同じエポックを作る。
    args.lr = 1e-300;
    let summary = pst::run(args).unwrap();
    assert_eq!(summary.best_epoch, Some(1));
    assert_eq!(summary.last_epoch, 2);
    assert_eq!(summary.stopped_by, pst::StopReason::EarlyStopping);
    assert_eq!(summary.improved_over_epoch_zero, Some(false));
}

fn compatibility_fixture() -> (Fixture, Args) {
    let (fixture, mut args) = Fixture::new(false, false);
    args.batch = 7;
    args.patience = 10;
    // Distinct mirrored endpoints exercise interpolation and nonzero init references.
    let weights: Vec<[i16; 2]> = (0..FEATURE_COUNT)
        .map(|f| {
            let f = f.min(minase::eval::features::mirror_feature(f));
            [(f % 43) as i16 - 21, (f % 59) as i16 - 29]
        })
        .collect();
    fs::write(
        &args.init,
        Pst::from_weights(&weights, v0_piece_values(), 317.25, "L0,P0,R1,E0")
            .unwrap()
            .encode(),
    )
    .unwrap();
    (fixture, args)
}
fn array_hash(values: &[f32]) -> String {
    let mut hash = Sha256::new();
    for value in values {
        hash.update(value.to_bits().to_le_bytes());
    }
    format!("{:x}", hash.finalize())
}
fn snapshot(directory: &Path) -> Value {
    let mut states = serde_json::Map::new();
    for file in ["latest.bin", "best.bin"] {
        let mut ck = Checkpoint::read(directory.join(file)).unwrap();
        ck.progress.elapsed_seconds = 0.0;
        ck.progress.config_hash = [0; 32];
        ck.progress.input_hash = [0; 32];
        let encoded = format!("{:x}", Sha256::digest(ck.encode().unwrap()));
        let best = ck.progress.best.unwrap();
        states.insert(file.into(), serde_json::json!({
            "encoded":encoded, "weights":array_hash(ck.model.weights()), "m":array_hash(ck.optimizer.m()),
            "v":array_hash(ck.optimizer.v()), "step":ck.optimizer.step(), "epoch":ck.progress.epoch,
            "best_epoch":best.epoch, "best_loss":best.validation_loss.to_bits()
        }));
    }
    let losses: Vec<_> = logs(directory)
        .iter()
        .map(|row| {
            [
                "train_loss",
                "removal_loss",
                "total_loss",
                "validation_loss",
            ]
            .map(|key| row[key].as_f64().map(f64::to_bits))
        })
        .collect();
    serde_json::json!({"states":states, "losses":losses,
        "pst":format!("{:x}",Sha256::digest(fs::read(directory.join("pst.bin")).unwrap())),
        "npz":format!("{:x}",Sha256::digest(fs::read(directory.join("pst-float.npz")).unwrap()))})
}

#[test]
fn phase_2a_weights_adam_losses_and_outputs_match_b0a245b_bit_for_bit() {
    // Expected values were captured by executing commit b0a245b, unpacked with
    // git archive, with compatibility_fixture() and epochs=1,2,3, threads=1.
    // Command: cargo test --offline -p minase-trainer --test pst_train
    // capture_b0a245b_compatibility -- --nocapture (temporary capture test).
    // Array hashes include every f32 bit in little-endian order, including padding.
    // Losses are the f64 bits read from training.jsonl; elapsed times and paths are excluded.
    // Checkpoint hashes set elapsed time and both path-dependent hashes to zero before encoding.
    let expected: Value = serde_json::from_str(include_str!("fixtures/pst-b0a245b.json")).unwrap();
    for kind in [RemovalReferenceKind::V0, RemovalReferenceKind::Init] {
        let (fixture, mut args) = compatibility_fixture();
        args.removal_reference = kind;
        for threads in [1, 3, 8] {
            args.threads = Some(threads);
            for epoch in 1..=3 {
                args.epochs = epoch;
                args.output_dir = fixture
                    .root
                    .join(format!("epoch-{epoch}-threads-{threads}"));
                pst::run(args.clone()).unwrap();
                assert_eq!(
                    snapshot(&args.output_dir),
                    expected[format!("{kind:?}")][epoch as usize - 1],
                    "reference={kind:?}, epoch={epoch}, threads={threads}"
                );
            }
        }
    }
}
