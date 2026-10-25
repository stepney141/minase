//! λの値域は共有パーサーで検査し、各引数型の必須性とCLIの終了コードを検証する。
use clap::Parser;
use minase_trainer::{diagnostics, fm, joint, nnue, pst};
use std::process::Command;

#[derive(Parser)]
struct Parsed<T: clap::Args> {
    #[command(flatten)]
    args: T,
}

fn requires_lambda<T: clap::Args>(args: &[&str]) {
    let missing =
        Parsed::<T>::try_parse_from(std::iter::once("trainer").chain(args.iter().copied()));
    let error = missing.err().expect("missing lambda must be rejected");
    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::MissingRequiredArgument,
        "{args:?}"
    );
    assert!(error.to_string().contains("--lambda"), "{args:?}: {error}");
    assert!(
        Parsed::<T>::try_parse_from(
            std::iter::once("trainer")
                .chain(args.iter().copied())
                .chain(["--lambda", "0.75"])
        )
        .is_ok(),
        "{args:?}"
    );
}

#[test]
fn teacher_commands_require_lambda_and_cli_rejects_missing_or_invalid_values() {
    let commands: &[&[&str]] = &[
        &[
            "pst",
            "train",
            "--data",
            "missing.mnsd",
            "--init",
            "missing.bin",
            "--teacher-cp-per-logit",
            "calibrate",
            "--output-dir",
            "unused",
            "--epochs",
            "1",
            "--patience",
            "1",
            "--removal-reference",
            "init",
        ],
        &[
            "fm",
            "train",
            "--data",
            "missing.mnsd",
            "--init",
            "missing.bin",
            "--teacher-cp-per-logit",
            "calibrate",
            "--output-dir",
            "unused",
            "--epochs",
            "1",
            "--patience",
            "1",
        ],
        &[
            "joint",
            "train",
            "--data",
            "missing.mnsd",
            "--base",
            "missing.bin",
            "--pst-init",
            "missing.npz",
            "--fm-init",
            "missing.npz",
            "--fm-reference",
            "missing.bin",
            "--teacher-cp-per-logit",
            "calibrate",
            "--output-dir",
            "unused",
            "--epochs",
            "1",
            "--patience",
            "1",
        ],
        &[
            "nnue",
            "train",
            "--feature-set",
            "P",
            "--width",
            "8",
            "--data",
            "missing.mnsd",
            "--base",
            "missing.bin",
            "--teacher-cp-per-logit",
            "calibrate",
            "--output-dir",
            "unused",
        ],
        &[
            "nnue",
            "bench-step",
            "--feature-set",
            "P",
            "--width",
            "8",
            "--batch",
            "2",
            "--steps",
            "2",
            "--data",
            "missing.mnsd",
        ],
        &[
            "diag",
            "pst-compare",
            "--data",
            "missing.mnsd",
            "--base",
            "missing.bin",
            "--candidate",
            "missing.bin",
            "--candidate-float",
            "missing.npz",
            "--teacher-cp-per-logit",
            "calibrate",
            "--output-dir",
            "unused",
        ],
    ];
    requires_lambda::<pst::Args>(&commands[0][2..]);
    requires_lambda::<fm::Args>(&commands[1][2..]);
    requires_lambda::<joint::Args>(&commands[2][2..]);
    requires_lambda::<nnue::train::Args>(&commands[3][2..]);
    requires_lambda::<nnue::gpu::bench::Args>(&commands[4][2..]);
    // check-equivalenceはmain.rs内の引数型にTrainingOptionsを平坦化する。
    requires_lambda::<nnue::options::TrainingOptions>(&[]);
    requires_lambda::<diagnostics::PstCompareArgs>(&commands[5][2..]);
    for value in [None, Some("NaN")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_minase-trainer"));
        command.args(commands[0]);
        if let Some(value) = value {
            command.arg(format!("--lambda={value}"));
        }
        let output = command.output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{value:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("--lambda"));
    }
}

#[test]
fn training_and_pst_comparison_require_an_explicit_teacher_scale_source() {
    let commands: &[&[&str]] = &[
        &[
            "pst",
            "train",
            "--data",
            "missing.mnsd",
            "--init",
            "missing.bin",
            "--output-dir",
            "unused",
            "--epochs",
            "1",
            "--patience",
            "1",
            "--removal-reference",
            "init",
        ],
        &[
            "fm",
            "train",
            "--data",
            "missing.mnsd",
            "--init",
            "missing.bin",
            "--output-dir",
            "unused",
            "--epochs",
            "1",
            "--patience",
            "1",
        ],
        &[
            "joint",
            "train",
            "--data",
            "missing.mnsd",
            "--base",
            "missing.bin",
            "--pst-init",
            "missing.npz",
            "--fm-init",
            "missing.npz",
            "--fm-reference",
            "missing.bin",
            "--output-dir",
            "unused",
            "--epochs",
            "1",
            "--patience",
            "1",
        ],
        &[
            "nnue",
            "train",
            "--feature-set",
            "P",
            "--width",
            "8",
            "--data",
            "missing.mnsd",
            "--base",
            "missing.bin",
            "--output-dir",
            "unused",
        ],
        &[
            "diag",
            "pst-compare",
            "--data",
            "missing.mnsd",
            "--base",
            "missing.bin",
            "--candidate",
            "missing.bin",
            "--candidate-float",
            "missing.npz",
            "--output-dir",
            "unused",
        ],
    ];
    fn check<T: clap::Args>(args: &[&str]) {
        let parse = |source: &[&str]| {
            Parsed::<T>::try_parse_from(
                std::iter::once("trainer")
                    .chain(args.iter().copied())
                    .chain(["--lambda", "0.75"])
                    .chain(source.iter().copied()),
            )
        };
        let error = parse(&[]).err().unwrap();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
        assert!(error.to_string().contains("--teacher-cp-per-logit"));
        for source in ["calibrate", "calibration.json"] {
            assert!(parse(&["--teacher-cp-per-logit", source]).is_ok());
        }
    }
    check::<pst::Args>(&commands[0][2..]);
    check::<fm::Args>(&commands[1][2..]);
    check::<joint::Args>(&commands[2][2..]);
    check::<nnue::train::Args>(&commands[3][2..]);
    check::<diagnostics::PstCompareArgs>(&commands[4][2..]);
    for args in commands {
        let output = Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
            .args(*args)
            .args(["--lambda", "0.75"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("--teacher-cp-per-logit"));
    }
}
