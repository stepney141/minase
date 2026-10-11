//! 学習コマンドの引数契約を、ファイルやGPUを使わず検査する。

use clap::Parser;
use nnue_trainer::{data::InputFeatureSet, train::Args};

#[derive(Parser)]
struct TrainArguments {
    #[command(flatten)]
    args: Args,
}

fn argv() -> Vec<&'static str> {
    vec![
        "nnue-trainer",
        "--feature-set",
        "P",
        "--width",
        "128",
        "--data",
        "first.bin",
        "--init-pst",
        "pst-float.npz",
        "--base",
        "pst.bin",
        "--output-dir",
        "output",
    ]
}

#[test]
fn train_defaults_match_the_training_contract() {
    let args = TrainArguments::try_parse_from(argv()).unwrap().args;
    args.validate().unwrap();
    assert_eq!(args.feature_set, InputFeatureSet::P);
    assert_eq!(args.width, 128);
    assert_eq!(args.batch, 16_384);
    assert_eq!(args.epochs, 50);
    assert_eq!(args.patience, 5);
    assert_eq!(args.seed, 1);
    assert_eq!(args.time_limit_hours, 24.0);
    assert_eq!(args.validation_sample, 10_000);
    assert_eq!(args.threads, None);
    assert_eq!(args.resume, None);
    assert_eq!(args.lookahead_gamma, None);
    assert_eq!(args.lookahead_plies, None);
}

#[test]
fn train_requires_the_model_inputs_and_output_directory() {
    for required in [
        "--feature-set",
        "--width",
        "--data",
        "--init-pst",
        "--base",
        "--output-dir",
    ] {
        let mut missing = argv();
        let index = missing.iter().position(|&arg| arg == required).unwrap();
        missing.drain(index..index + 2);
        assert!(
            TrainArguments::try_parse_from(missing).is_err(),
            "missing {required} must fail"
        );
    }
}

#[test]
fn train_accepts_both_feature_sets_multiple_files_and_paired_lookahead() {
    for feature_set in ["P", "HalfKP-hm-b8"] {
        let mut values = argv();
        values[2] = feature_set;
        values.insert(7, "second.bin");
        values.extend([
            "--lookahead-gamma",
            "0.9",
            "--lookahead-plies",
            "40",
            "--threads",
            "2",
            "--resume",
            "output",
        ]);
        let args = TrainArguments::try_parse_from(values).unwrap().args;
        args.validate().unwrap();
        assert_eq!(args.data.len(), 2);
        assert_eq!(args.lookahead_gamma, Some(0.9));
        assert_eq!(args.lookahead_plies, Some(40));
        assert_eq!(args.threads, Some(2));
        assert_eq!(args.resume.as_deref(), Some(std::path::Path::new("output")));
    }
}

#[test]
fn train_rejects_invalid_numeric_arguments_before_opening_inputs() {
    for (flag, value) in [
        ("--width", "0"),
        ("--width", "3"),
        ("--width", "514"),
        ("--batch", "0"),
        ("--batch", "4194305"),
        ("--epochs", "0"),
        ("--epochs", "51"),
        ("--patience", "0"),
        ("--time-limit-hours", "0"),
        ("--time-limit-hours", "25"),
        ("--time-limit-hours", "NaN"),
        ("--time-limit-hours", "inf"),
        ("--validation-sample", "0"),
        ("--threads", "0"),
    ] {
        let mut values = argv();
        if let Some(index) = values.iter().position(|&arg| arg == flag) {
            values[index + 1] = value;
        } else {
            values.extend([flag, value]);
        }
        let parsed = TrainArguments::try_parse_from(values);
        assert!(
            parsed.is_err() || parsed.unwrap().args.validate().is_err(),
            "{flag} {value} must fail"
        );
    }
}

#[test]
fn train_batch_bound_uses_the_packed_occurrence_limit() {
    let mut args = TrainArguments::try_parse_from(argv()).unwrap().args;
    for batch in [32_769, nnue_trainer::data::MAX_BATCH_SIZE] {
        args.batch = batch;
        args.validate().unwrap();
    }
    args.batch = nnue_trainer::data::MAX_BATCH_SIZE + 1;
    assert!(args.validate().is_err());
}

#[test]
fn train_rejects_unpaired_or_invalid_lookahead() {
    for extra in [
        vec!["--lookahead-gamma", "0.9"],
        vec!["--lookahead-plies", "40"],
        vec!["--lookahead-gamma", "0", "--lookahead-plies", "40"],
        vec!["--lookahead-gamma", "1", "--lookahead-plies", "40"],
        vec!["--lookahead-gamma", "NaN", "--lookahead-plies", "40"],
        vec!["--lookahead-gamma", "0.9", "--lookahead-plies", "0"],
        vec!["--lookahead-gamma", "0.9", "--lookahead-plies", "4097"],
    ] {
        let mut values = argv();
        values.extend(extra);
        let parsed = TrainArguments::try_parse_from(values);
        assert!(parsed.is_err() || parsed.unwrap().args.validate().is_err());
    }
}

#[test]
fn train_help_is_wired_and_learning_rates_are_fixed() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nnue-trainer"))
        .args(["train", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for required in ["--init-pst", "--base", "--resume", "--validation-sample"] {
        assert!(help.contains(required), "missing help for {required}");
    }
    for unsupported in ["--lr-linear", "--lr-nonlinear", "--weight-decay"] {
        let mut values = argv();
        values.extend([unsupported, "0.1"]);
        assert!(TrainArguments::try_parse_from(values).is_err());
    }
}
