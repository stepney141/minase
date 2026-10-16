//! 学習コマンドの引数契約を、ファイルやGPUを使わず検査する。

use clap::Parser;
use minase_trainer::nnue::{
    data::InputFeatureSet,
    options::{DeviceKind, TrainingOptions},
    train::Args,
};

#[derive(Parser)]
struct TrainArguments {
    #[command(flatten)]
    args: Args,
}

fn argv() -> Vec<&'static str> {
    vec![
        "minase-trainer",
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
        "--teacher-cp-per-logit",
        "calibrate",
        "--lambda",
        "1.0",
    ]
}

#[test]
fn train_defaults_match_the_training_contract() {
    let args = TrainArguments::try_parse_from(argv()).unwrap().args;
    args.validate().unwrap();
    assert_eq!(args.device, DeviceKind::Cuda);
    assert_eq!(args.training, TrainingOptions::new(1.0));
    assert_eq!(args.training.lr_linear, 0.03);
    assert_eq!(args.training.weight_decay_linear, 0.0);
    assert_eq!(args.training.lr_ft, 1e-3);
    assert_eq!(args.training.weight_decay_ft, 1e-4);
    assert_eq!(args.training.lr_head, 1e-3);
    assert_eq!(args.training.weight_decay_head, 1e-4);
    assert_eq!(args.training.lambda, 1.0);
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
        assert_eq!(
            TrainArguments::try_parse_from(missing)
                .err()
                .unwrap()
                .kind(),
            clap::error::ErrorKind::MissingRequiredArgument
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
fn train_validates_numeric_arguments_and_paired_lookahead_before_opening_inputs() {
    let max_batch = minase_trainer::nnue::data::MAX_BATCH_SIZE.to_string();
    let oversized_batch = (minase_trainer::nnue::data::MAX_BATCH_SIZE + 1).to_string();
    let cases: &[(&[&str], bool)] = &[
        (&["--width", "0"], false),
        (&["--width", "3"], false),
        (&["--width", "514"], false),
        (&["--batch", "0"], false),
        (&["--batch", "32769"], true),
        (&["--batch", &max_batch], true),
        (&["--batch", &oversized_batch], false),
        (&["--epochs", "0"], false),
        (&["--epochs", "51"], false),
        (&["--patience", "0"], false),
        (&["--time-limit-hours", "0"], false),
        (&["--time-limit-hours", "25"], false),
        (&["--time-limit-hours", "NaN"], false),
        (&["--time-limit-hours", "inf"], false),
        (&["--validation-sample", "0"], false),
        (&["--threads", "0"], false),
        (&["--lookahead-gamma", "0.9"], false),
        (&["--lookahead-plies", "40"], false),
        (
            &["--lookahead-gamma", "0", "--lookahead-plies", "40"],
            false,
        ),
        (
            &["--lookahead-gamma", "1", "--lookahead-plies", "40"],
            false,
        ),
        (
            &["--lookahead-gamma", "NaN", "--lookahead-plies", "40"],
            false,
        ),
        (
            &["--lookahead-gamma", "0.9", "--lookahead-plies", "0"],
            false,
        ),
        (
            &["--lookahead-gamma", "0.9", "--lookahead-plies", "4097"],
            false,
        ),
    ];
    for &(arguments, accepted) in cases {
        let mut values = argv();
        for &[flag, value] in arguments.as_chunks::<2>().0 {
            if let Some(index) = values.iter().position(|&arg| arg == flag) {
                values[index + 1] = value;
            } else {
                values.extend([flag, value]);
            }
        }
        let valid = match TrainArguments::try_parse_from(values) {
            Ok(parsed) => parsed.args.validate().is_ok(),
            Err(_) => false,
        };
        assert_eq!(valid, accepted, "arguments={arguments:?}");
    }
}

#[test]
fn coefficient_ranges_are_checked_before_opening_inputs() {
    for flag in [
        "--lr-linear",
        "--lr-ft",
        "--lr-head",
        "--weight-decay-linear",
        "--weight-decay-ft",
        "--weight-decay-head",
    ] {
        for value in ["NaN", "inf", "-0.1", "0"] {
            let mut values = argv().into_iter().map(str::to_owned).collect::<Vec<_>>();
            values.push(format!("{flag}={value}"));
            let args = TrainArguments::try_parse_from(values).unwrap().args;
            let valid = value == "0" && !flag.starts_with("--lr-");
            assert_eq!(args.validate().is_ok(), valid, "{flag}={value}");
        }
    }
    for device in ["cpu", "cuda"] {
        let mut values = argv();
        values.extend(["--device", device]);
        let i = values
            .iter()
            .position(|&value| value == "--lambda")
            .unwrap();
        values[i + 1] = "0.75";
        TrainArguments::try_parse_from(values)
            .unwrap()
            .args
            .validate()
            .unwrap();
    }
}

#[test]
fn bucket_defaults_and_explicit_limits_are_preserved_by_clap() {
    // 仕様1: B=1が既定で、容量と作業ディレクトリには既定値を補わない。
    let default = TrainArguments::try_parse_from(argv()).unwrap().args;
    assert_eq!(default.buckets, 1);
    assert_eq!(default.bucket_positions, None);
    assert_eq!(default.scratch_dir, None);
    let mut input = argv();
    input.extend([
        "--buckets",
        "3",
        "--bucket-positions",
        "1000000",
        "--scratch-dir",
        "/tmp",
    ]);
    let explicit = TrainArguments::try_parse_from(input).unwrap().args;
    explicit.validate().unwrap();
    assert_eq!(explicit.buckets, 3);
    assert_eq!(explicit.bucket_positions, Some(1_000_000));
    assert_eq!(
        explicit.scratch_dir.as_deref(),
        Some(std::path::Path::new("/tmp"))
    );
}

#[test]
fn bucket_arguments_are_paired_with_the_bucket_count_before_opening_inputs() {
    // 仕様1: B=1では容量と作業ディレクトリを受け付けず、B≥2では両方を必須とし、Bは1以上4096以下。
    let cases: &[(&[&str], bool)] = &[
        (&["--buckets", "1"], true),
        (&["--buckets", "1", "--bucket-positions", "10"], false),
        (&["--buckets", "1", "--scratch-dir", "/tmp"], false),
        (
            &[
                "--buckets",
                "1",
                "--bucket-positions",
                "10",
                "--scratch-dir",
                "/tmp",
            ],
            false,
        ),
        (&["--bucket-positions", "10"], false),
        (&["--scratch-dir", "/tmp"], false),
        (&["--buckets", "2"], false),
        (&["--buckets", "2", "--bucket-positions", "10"], false),
        (&["--buckets", "2", "--scratch-dir", "/tmp"], false),
        (
            &[
                "--buckets",
                "2",
                "--bucket-positions",
                "10",
                "--scratch-dir",
                "/tmp",
            ],
            true,
        ),
        (
            &[
                "--buckets",
                "4096",
                "--bucket-positions",
                "1",
                "--scratch-dir",
                "/tmp",
            ],
            true,
        ),
        (
            &[
                "--buckets",
                "0",
                "--bucket-positions",
                "10",
                "--scratch-dir",
                "/tmp",
            ],
            false,
        ),
        (&["--buckets", "0"], false),
        (
            &[
                "--buckets",
                "4097",
                "--bucket-positions",
                "10",
                "--scratch-dir",
                "/tmp",
            ],
            false,
        ),
        (
            &[
                "--buckets",
                "2",
                "--bucket-positions",
                "0",
                "--scratch-dir",
                "/tmp",
            ],
            false,
        ),
        (&["--buckets", "-1"], false),
    ];
    for &(arguments, accepted) in cases {
        let mut values = argv();
        values.extend_from_slice(arguments);
        let valid = match TrainArguments::try_parse_from(values) {
            Ok(parsed) => parsed.args.validate().is_ok(),
            Err(_) => false,
        };
        assert_eq!(valid, accepted, "arguments={arguments:?}");
    }
}
