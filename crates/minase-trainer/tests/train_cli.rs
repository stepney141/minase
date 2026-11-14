//! 学習コマンドの引数契約と、CPUでの進捗表示・再開を検査する。

use clap::Parser;
use minase_trainer::nnue::{data::InputFeatureSet, options::DeviceKind, train::Args};

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
        "--base",
        "base.bin",
        "--output-dir",
        "output",
        "--superbatches",
        "3",
        "--batches-per-superbatch",
        "6104",
        "--validation-superbatches",
        "1",
        "--lr-schedule",
        "cosine:3:0.1",
        "--lambda",
        "1.0",
    ]
}

#[test]
fn train_defaults_match_the_training_contract() {
    let mut values = argv();
    let at = values
        .iter()
        .position(|&v| v == "--batches-per-superbatch")
        .unwrap();
    values.drain(at..at + 2);
    let args = TrainArguments::try_parse_from(values).unwrap().args;
    args.validate().unwrap();
    assert_eq!(args.device, DeviceKind::Cuda);
    assert_eq!(args.training.weight_decay_ft, 0.0);
    assert_eq!(args.training.weight_decay_head, 0.0);
    assert_eq!(args.training.lambda, 1.0);
    assert_eq!(args.feature_set, InputFeatureSet::P);
    assert_eq!(args.width, 128);
    assert_eq!(args.batch, 16_384);
    assert_eq!(args.batches_per_superbatch.get(), 6104);
    assert!(args.teacher_cp_per_logit.is_none());
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
        "--base",
        "--output-dir",
        "--superbatches",
        "--validation-superbatches",
        "--lr-schedule",
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
        (&["--superbatches", "0"], false),
        (&["--batches-per-superbatch", "0"], false),
        (&["--validation-superbatches", "0"], false),
        (
            &[
                "--superbatches",
                "18446744073709551615",
                "--batches-per-superbatch",
                "2",
            ],
            false,
        ),
        (
            &[
                "--superbatches",
                "18446744073709551615",
                "--batches-per-superbatch",
                "1",
            ],
            true,
        ),
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

#[test]
fn removed_epoch_and_interval_options_are_rejected() {
    for flag in [
        "--epochs",
        "--patience",
        "--export-epoch",
        "--validation-interval",
    ] {
        let mut values = argv();
        values.extend([flag, "1"]);
        assert_eq!(
            TrainArguments::try_parse_from(values).err().unwrap().kind(),
            clap::error::ErrorKind::UnknownArgument
        );
    }
}

#[test]
fn learning_rate_schedules_reject_invalid_numbers_and_shapes() {
    for spec in [
        "",
        "constant:0",
        "constant:NaN",
        "exponential:0:0.1",
        "cosine:3:0",
        "cosine:3:1.1",
        "cosine:inf:0.1",
    ] {
        let mut values = argv();
        let at = values.iter().position(|&v| v == "--lr-schedule").unwrap();
        values[at + 1] = spec;
        assert!(TrainArguments::try_parse_from(values).is_err());
    }
}

mod common;

#[test]
fn time_limit_exit_one_and_resume_export_match_a_complete_cli_run() {
    use std::{fs, process::Command};
    let fixture = common::Fixture::new(&common::positions());
    let data = fixture.root.join("records.bin");
    fs::write(&data, common::training::records(false, false)).unwrap();
    let base = fixture.root.join("base.bin");
    fs::write(&base, include_bytes!("../../minase/nets/nnue-epoch0.bin")).unwrap();
    let invoke = |directory: &str, superbatches: &str, resume: bool, limit: &str| {
        let output = fixture.root.join(directory);
        let mut command = Command::new(env!("CARGO_BIN_EXE_minase-trainer"));
        command
            .args([
                "nnue",
                "train",
                "--device",
                "cpu",
                "--feature-set",
                "P",
                "--width",
                "4",
                "--lambda",
                "1",
                "--threads",
                "1",
                "--batch",
                "33",
                "--superbatches",
                superbatches,
                "--batches-per-superbatch",
                "2",
                "--validation-superbatches",
                "2",
                "--lr-schedule",
                "cosine:3:0.1",
                "--time-limit-hours",
                limit,
                "--validation-sample",
                "8",
                "--data",
            ])
            .arg(&data)
            .arg("--base")
            .arg(&base)
            .arg("--output-dir")
            .arg(&output);
        if resume {
            command.arg("--resume").arg(&output);
        }
        command.output().unwrap()
    };
    for (directory, count) in [("continuous", "3"), ("resumed", "1")] {
        let result = invoke(directory, count, false, "24");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        // 指示書の表示契約に従い、検証なしは「-」、数値は小数点以下9桁とする。
        let stderr = String::from_utf8(result.stderr).unwrap();
        let progress: Vec<_> = stderr
            .lines()
            .filter(|line| line.starts_with("superbatch "))
            .collect();
        let records = common::training::logs(&fixture.root.join(directory));
        assert_eq!(progress.len(), count.parse::<usize>().unwrap());
        for (line, record) in progress.iter().zip(&records[1..]) {
            let validation = match record["validation_loss"].as_f64() {
                Some(loss) => format!("{loss:.9}"),
                None => "-".to_owned(),
            };
            assert_eq!(
                *line,
                format!(
                    "superbatch {}: updates={} train_loss={:.9} validation_loss={validation}",
                    record["superbatch"].as_u64().unwrap(),
                    record["total_steps"].as_u64().unwrap(),
                    record["train_loss"].as_f64().unwrap(),
                )
            );
        }
        if directory == "continuous" {
            assert!(progress[0].ends_with("validation_loss=-"));
            assert!(!progress[1].ends_with("validation_loss=-"));
            assert!(!progress[2].ends_with("validation_loss=-"));
        }
        let summary = common::training::document(fixture.root.join(directory).join("summary.json"));
        assert_eq!(summary["stopped_by"], "completed");
        assert_eq!(
            stderr.lines().last().unwrap(),
            format!(
                "superbatches: {count}; updates: {}; stopped_by: completed; status: candidate",
                summary["completed_steps"].as_u64().unwrap()
            )
        );
    }
    let interrupted = invoke("resumed", "3", true, "1e-20");
    assert_eq!(interrupted.status.code(), Some(1));
    assert!(!fixture.root.join("resumed/nnue.bin").exists());
    let summary = common::training::document(fixture.root.join("resumed/summary.json"));
    assert_eq!(summary["status"], "incomplete");
    assert_eq!(summary["stopped_by"], "time_limit");
    assert_eq!(summary["completed_steps"], 2);
    assert_eq!(summary["planned_steps"], 6);
    assert!(
        String::from_utf8(interrupted.stderr)
            .unwrap()
            .lines()
            .any(|line| {
                line == "superbatches: 1; updates: 2; stopped_by: time_limit; status: incomplete"
            })
    );
    let finished = invoke("resumed", "3", true, "24");
    assert!(
        finished.status.success(),
        "{}",
        String::from_utf8_lossy(&finished.stderr)
    );
    assert_eq!(
        fs::read(fixture.root.join("continuous/nnue.bin")).unwrap(),
        fs::read(fixture.root.join("resumed/nnue.bin")).unwrap()
    );
}
