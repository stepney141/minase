//! pst trainの引数、端数バッチ、再開と成果物を合成MNSDで検証する。
#[path = "common/checkpoint.rs"]
#[allow(dead_code)]
mod checkpoint_contract;
mod common;
use clap::Parser;
use common::training::{compare_checkpoints, document, logs};
use minase::eval::{
    features::FEATURE_COUNT,
    pst::{FM_RANK, Pst},
};
use minase_trainer::error::ContractError;
use minase_trainer::nn::checkpoint::MismatchField;
use minase_trainer::{
    data::{Dataset, build_targets_from_arrays, epoch_permutation},
    pst::{
        self, Adam, Args, Batch, Checkpoint, Model, RemovalReference, RemovalReferenceKind,
        v0_piece_values,
    },
};
use std::{
    fs,
    path::PathBuf,
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
        assert_eq!(
            Arguments::try_parse_from(values).err().unwrap().kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
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
        let args = Arguments::try_parse_from(values).unwrap().args;
        let error = args.validate().unwrap_err();
        common::training::assert_invalid_argument(&error, flag);
    }
    for extra in [
        vec!["--lookahead-gamma", "0.9"],
        vec!["--lookahead-plies", "40"],
    ] {
        assert_eq!(
            Arguments::try_parse_from(argv().into_iter().chain(extra))
                .err()
                .unwrap()
                .kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }
    let args = Arguments::try_parse_from(argv().into_iter().chain([
        "--lookahead-gamma",
        "1",
        "--lookahead-plies",
        "40",
    ]))
    .unwrap()
    .args;
    assert!(matches!(
        args.validate(),
        Err(pst::Error::Data(
            minase_trainer::data::Error::LookaheadParameters
        ))
    ));
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
    fn new(large_evaluation: bool, with_fm: bool) -> (Self, Args) {
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
        if with_fm {
            initial = Pst::from_weights_with_fm(
                &weights,
                v0_piece_values().map(|value| value * 2),
                317.25,
                "L0,P0,R1,E0",
                minase::eval::pst::FmWeights {
                    embeddings: &vec![[771; FM_RANK]; FEATURE_COUNT],
                    signs: &[1; FM_RANK],
                    exponent: 7,
                },
            )
            .unwrap()
            .encode();
        }
        fs::write(&args.init, initial).unwrap();
        fs::write(
            &args.data[0],
            common::training::records(large_evaluation, false),
        )
        .unwrap();
        (fixture, args)
    }
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
        fs::remove_file(split.output_dir.join("best.npz")).unwrap();
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
        for file in ["latest.npz", "best.npz"] {
            compare_checkpoints(&args.output_dir.join(file), &split.output_dir.join(file));
        }
        for file in ["pst.bin", "pst-float.npz"] {
            assert_eq!(
                fs::read(args.output_dir.join(file)).unwrap(),
                fs::read(split.output_dir.join(file)).unwrap()
            );
        }
        let ck = Checkpoint::read(args.output_dir.join("latest.npz")).unwrap();
        assert_eq!(ck.optimizer.step(), 14); // 111訓練局面、16件×6バッチ＋15件の端数、2エポック。
        let rows = logs(&args.output_dir);
        let resumed = logs(&split.output_dir);
        assert_eq!(rows.len(), 3);
        assert_eq!(resumed.len(), rows.len());
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
                if field.ends_with("loss") {
                    assert_eq!(
                        left[field].as_f64().map(f64::to_bits),
                        right[field].as_f64().map(f64::to_bits),
                        "{field}"
                    );
                }
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
        assert_eq!(written.cp_per_logit(), 317.25);
        assert_eq!(written.rule_set(), "L0,P0,R1,E0");
        for (state, value) in v0_piece_values().into_iter().enumerate() {
            assert_eq!(written.piece_value_of_state(state), value * 2);
        }
        assert_eq!(summary.quantization.unwrap().samples, 7);
    }
}

#[test]
fn training_wires_permutation_and_tail_batches_to_updates_and_logs() {
    // 共有の数値部品を組み合わせ、順列・端数バッチ・更新への配線を検査する。
    let (_fixture, mut args) = Fixture::new(false, false);
    args.epochs = 1;
    args.removal_reference = RemovalReferenceKind::V0;

    let dataset = Dataset::open(
        &args.data,
        Some(minase_trainer::data::Lookahead::new(0.9, 40).unwrap()),
        std::num::NonZeroUsize::new(1).unwrap(),
    )
    .unwrap();
    let dataset = dataset.with_lambda(args.lambda).unwrap();
    let teacher_cp_per_logit = dataset.estimate_teacher_cp_per_logit().unwrap();
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
            &teacher_cp_per_logit,
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
    let ck = Checkpoint::read(args.output_dir.join("latest.npz")).unwrap();
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
    let before = fs::read(args.output_dir.join("latest.npz")).unwrap();
    checkpoint_contract::rejects_changed_meta(&args.output_dir, || {
        pst::run(args.clone()).unwrap_err().into()
    });
    type Change = fn(&mut Args);
    let changes: Vec<(MismatchField, Change)> = vec![
        (MismatchField::Lr, |a| a.lr = 0.02),
        (MismatchField::Batch, |a| a.batch = 17),
        (MismatchField::Seed, |a| a.seed = 2),
        (MismatchField::RemovalPenalty, |a| a.removal_penalty = 0.0),
        (MismatchField::RemovalReference, |a| {
            a.removal_reference = RemovalReferenceKind::V0
        }),
        (MismatchField::Lambda, |a| a.lambda = 0.5),
        (MismatchField::LookaheadGamma, |a| {
            a.lookahead_gamma = Some(0.8)
        }),
        (MismatchField::LookaheadPlies, |a| {
            a.lookahead_plies = Some(20)
        }),
        (MismatchField::ValidationSample, |a| a.validation_sample = 8),
    ];
    for (field, change) in changes {
        let mut changed = args.clone();
        change(&mut changed);
        let error = pst::run(changed).unwrap_err();
        checkpoint_contract::assert_mismatch(&error, field);
    }
    let base = Pst::decode(&fs::read(&args.init).unwrap()).unwrap();
    let modified =
        Pst::from_weights(base.weights(), v0_piece_values(), 318.0, base.rule_set()).unwrap();
    fs::write(&args.init, modified.encode()).unwrap();
    let error = pst::run(args.clone()).unwrap_err();
    checkpoint_contract::assert_mismatch(&error, MismatchField::Inputs);
    assert_eq!(
        fs::read(args.output_dir.join("latest.npz")).unwrap(),
        before
    );
}

#[test]
fn quantization_failure_keeps_outputs_and_records_failure() {
    let (_fixture, mut args) = Fixture::new(true, true);
    args.epochs = 1;
    args.removal_penalty = 0.0;
    args.patience = 1;
    args.batch = 16384;
    args.validation_sample = 10000;
    args.lookahead_gamma = None;
    args.lookahead_plies = None;
    assert!(pst::run(args.clone()).is_err());
    for file in ["pst.bin", "pst-float.npz", "latest.npz", "best.npz"] {
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
    assert!(matches!(
        pst::run(args.clone()).unwrap_err(),
        pst::Error::Contract(ContractError::AsymmetricReference)
    ));
    args.removal_penalty = 0.0;
    pst::run(args.clone()).unwrap();
    let ck = Checkpoint::read(args.output_dir.join("latest.npz")).unwrap();
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
