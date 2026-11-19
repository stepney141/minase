//! フェーズ3c-2のCLI、入力検査、進捗と再開の受け入れ契約。
mod common;
#[path = "common/removal.rs"]
mod removal_contract;
use clap::Parser;
use common::training::{document, logs};
use minase_trainer::nnue::{
    layerstack::{Checkpoint, Parameter, trainer::run},
    options::DeviceKind,
    train::{Args, Error},
};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
#[derive(Parser)]
struct Parsed {
    #[command(flatten)]
    args: Args,
}
fn argv() -> Vec<&'static str> {
    vec![
        "train",
        "--data",
        "missing.mnsd",
        "--output-dir",
        "output",
        "--rules",
        "L0,P0,R1,E0",
        "--teacher-cp-per-logit",
        "1072.65",
        "--lambda",
        "0.75",
        "--superbatches",
        "3",
        "--batches-per-superbatch",
        "2",
        "--validation-superbatches",
        "1",
        "--lr-schedule",
        "cosine:3:0.027",
    ]
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "layerstack-train-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join("data.mnsd"),
            common::training::records(false, true),
        )
        .unwrap();
        Self(root)
    }
    fn args(&self, output: &str, superbatches: u64) -> Args {
        let mut a = Parsed::parse_from(argv()).args;
        a.data = vec![self.0.join("data.mnsd")];
        a.output_dir = self.0.join(output);
        a.device = DeviceKind::Cpu;
        a.threads = Some(1);
        a.batch = 17;
        a.validation_sample = 5;
        a.superbatches = superbatches.try_into().unwrap();
        a
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn bits(a: &Checkpoint, b: &Checkpoint) {
    assert_eq!(a.optimizer.step, b.optimizer.step);
    for (a, b) in [
        (&a.parameters, &b.parameters),
        (&a.optimizer.m, &b.optimizer.m),
        (&a.optimizer.v, &b.optimizer.v),
    ] {
        for p in Parameter::ALL {
            assert!(
                a[p].iter()
                    .zip(&b[p])
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{p:?}"
            );
        }
    }
}
#[test]
fn required_arguments_and_numeric_domains_are_checked_before_input() {
    for required in [
        "--data",
        "--output-dir",
        "--rules",
        "--teacher-cp-per-logit",
        "--lambda",
        "--superbatches",
        "--validation-superbatches",
        "--lr-schedule",
    ] {
        let mut args = argv();
        let i = args.iter().position(|&v| v == required).unwrap();
        args.drain(i..i + 2);
        assert_eq!(
            Parsed::try_parse_from(args).err().unwrap().kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }
    for value in [
        0.0,
        -1.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        16256.1,
        f64::MIN_POSITIVE,
    ] {
        let mut args = Parsed::parse_from(argv()).args;
        args.teacher_cp_per_logit = value;
        assert!(args.validate().is_err(), "{value}");
    }
    let mut args = Parsed::parse_from(argv()).args;
    args.teacher_cp_per_logit = 16256.0;
    args.validate().unwrap();
    for hours in [0.0, -1.0, 24.1, f64::INFINITY] {
        args.time_limit_hours = hours;
        assert!(args.validate().is_err());
    }
    let args = Parsed::parse_from(argv()).args;
    assert_eq!(args.batch, 16384);
    assert_eq!(args.training.lr_ft, 0.001);
    assert_eq!(args.training.lr_head, 0.001);
    assert_eq!(args.training.weight_decay_ft, 0.0);
    assert_eq!(args.training.weight_decay_head, 0.0);
}
#[test]
fn cli_subcommands_require_scales_and_rules_and_reject_invalid_scales() {
    let bin = env!("CARGO_BIN_EXE_minase-trainer");
    for command in ["train", "check-equivalence", "bench-step"] {
        let output = Command::new(bin)
            .args(["nnue", command, "layerstack", "--help"])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .contains("--teacher-cp-per-logit")
        );
    }
    for value in ["0", "-1", "NaN", "inf", "16256.1", "0.0000000001"] {
        let mut args = argv();
        let at = args
            .iter()
            .position(|&v| v == "--teacher-cp-per-logit")
            .unwrap();
        args.drain(at..at + 2);
        let output = Command::new(bin)
            .args(["nnue", "train", "layerstack"])
            .args(&args[1..])
            .arg(format!("--teacher-cp-per-logit={value}"))
            .output()
            .unwrap();
        assert!(!output.status.success(), "{value}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("No such file"));
    }
}
#[test]
fn validation_plateau_keeps_first_trained_checkpoint_across_resume() {
    let fixture = Fixture::new();
    let mut args = fixture.args("plateau", 2);
    // f32の重みが変化しない学習率で、すべての検証時点を同率にする。
    args.training.lr_ft = 1e-100;
    args.training.lr_head = 1e-100;
    args.validation_patience = Some(2.try_into().unwrap());
    let first = run(args.clone()).unwrap();
    assert_eq!(first.selected_superbatch, Some(1));
    assert_eq!(first.completed_superbatches, 2);
    let exported = fs::read(args.output_dir.join("nnue.bin")).unwrap();

    args.superbatches = 6.try_into().unwrap();
    args.resume = Some(args.output_dir.clone());
    let resumed = run(args.clone()).unwrap();
    assert_eq!(resumed.stopped_by.to_string(), "validation_patience");
    assert_eq!(resumed.completed_superbatches, 3);
    assert_eq!(resumed.selected_superbatch, Some(1));
    assert_eq!(
        resumed.selected_validation_loss,
        first.selected_validation_loss
    );
    assert_eq!(
        fs::read(args.output_dir.join("nnue.bin")).unwrap(),
        exported
    );
    for (file, boundary) in [("best.npz", 1), ("latest.npz", 3)] {
        assert_eq!(
            Checkpoint::read(args.output_dir.join(file))
                .unwrap()
                .progress
                .completed_superbatches,
            boundary
        );
    }
}
#[test]
fn validation_exports_best_weights_even_when_training_moves_farther() {
    use minase::training::records::{Header, Origin, TeacherKind};
    use minase_core::Position;
    use minase_trainer::data::{Outcome, Sample, is_validation};
    use minase_trainer::nnue::layerstack::export;

    // 同じ盤面に訓練と検証で逆の探索値を与え、学習の進行とともに検証誤差を悪化させる。
    let f = Fixture::new();
    let mut args = f.args("opposed", 5);
    let header = Header::new(
        "JCSA".into(),
        "1".repeat(40),
        [2; 32],
        TeacherKind::AlphaBeta,
        200,
        11,
    )
    .unwrap();
    let samples: Vec<_> = (0..40)
        .map(|game| {
            let score = if is_validation(11, game) { -2000 } else { 2000 };
            Sample::from_position(
                &Position::initial(),
                score,
                Outcome::Draw,
                game,
                0,
                Origin::RandomSelfplay,
            )
            .unwrap()
        })
        .collect();
    fs::write(&args.data[0], common::mnsd::independent(header, &samples)).unwrap();
    args.training.lambda = 1.0;
    args.validation_patience = Some(2.try_into().unwrap());
    let result = run(args.clone()).unwrap();
    assert_eq!(result.selected_superbatch, Some(1));
    assert_eq!(result.completed_superbatches, 3);
    let network = fs::read(args.output_dir.join("nnue.bin")).unwrap();
    let quantized = |name| {
        let saved = Checkpoint::read(args.output_dir.join(name)).unwrap();
        export(&saved.parameters, saved.settings.scale).unwrap()
    };
    assert_eq!(network, quantized("best.npz"));
    assert_ne!(network, quantized("latest.npz"));
}
#[test]
fn cpu_records_initial_validation_planned_updates_and_tatara_export() {
    let f = Fixture::new();
    let args = f.args("run", 3);
    let dir = args.output_dir.clone();
    let summary = run(args.clone()).unwrap();
    assert_eq!(summary.completed_steps, 6);
    assert_eq!(summary.status, "candidate");
    let rows = logs(&dir);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0]["total_steps"], 0);
    assert!(rows[0]["validation_loss"].is_number());
    assert!(rows[0]["train_loss"].is_null());
    for (i, row) in rows.iter().enumerate().skip(1) {
        assert_eq!(row["steps"], 2);
        assert_eq!(row["total_steps"], i * 2);
    }
    let config = document(dir.join("config.json"));
    assert_eq!(config["architecture"]["kind"], "LayerStack");
    assert_eq!(config["architecture"]["H"], 256);
    assert_eq!(config["architecture"]["K_t"], 1072.65);
    assert_eq!(config["architecture"]["m"], 8);
    assert_eq!(config["architecture"]["K_o"], 1016.0);
    let summary = document(dir.join("summary.json"));
    assert_eq!(summary["removal_sign"]["positions"], 5);
    assert_eq!(summary["removal_sign"]["removals"], 20);
    assert_eq!(summary["removal_sign"]["reversals"]["total"], 20);
    assert_eq!(summary["removal_sign"]["terminal_removal_count"], 0);
    for key in [
        "mean_absolute_cp",
        "mean_signed_cp",
        "max_absolute_cp",
        "ft_saturation",
        "l1_square_saturation",
        "l1_linear_saturation",
        "l2_saturation",
    ] {
        assert!(summary["quantization"][key].is_number(), "{key}");
    }
    let mut checkpoint = Checkpoint::read(dir.join("latest.npz")).unwrap();
    let network =
        minase::eval::nnue::Network::decode(&fs::read(dir.join("nnue.bin")).unwrap()).unwrap();
    let dataset =
        minase_trainer::data::Dataset::open(&args.data, None, 1.try_into().unwrap()).unwrap();
    let selected = minase_trainer::data::floyd_sample(
        dataset.validation_indices().len() as u64,
        args.validation_sample,
        args.seed,
    )
    .unwrap();
    let samples: Vec<_> = selected
        .iter()
        .map(|&i| {
            let sample = dataset
                .record(dataset.validation_indices()[i as usize])
                .unwrap();
            (sample.to_position().unwrap(), sample.side_to_move())
        })
        .collect();
    removal_contract::assert_report(&summary["removal_sign"], &network, &samples);
    let q = minase_trainer::nnue::layerstack::quantize(
        &checkpoint.parameters,
        checkpoint.settings.scale,
    )
    .unwrap();
    for position in common::positions() {
        for side in minase_core::Color::ALL {
            assert_eq!(
                network.evaluate_for(&position, side).unwrap(),
                integer_reference(
                    &q,
                    &minase_trainer::nnue::layerstack::Features::from_position(&position, side)
                )
            );
        }
    }
    // 反転を確実に作り、記録値が学習の成否を変えないことを確かめる。
    for weight in
        &mut checkpoint.parameters[Parameter::Psqt][..minase_trainer::nnue::layerstack::ROWS - 1]
    {
        *weight = -*weight;
    }
    checkpoint.write(dir.join("latest.npz")).unwrap();
    let mut resumed = args;
    resumed.resume = Some(dir.clone());
    assert_eq!(run(resumed).unwrap().status, "candidate");
    let reversed = document(dir.join("summary.json"));
    assert_eq!(reversed["removal_sign"]["reversals"]["count"], 20);
    let network =
        minase::eval::nnue::Network::decode(&fs::read(dir.join("nnue.bin")).unwrap()).unwrap();
    removal_contract::assert_report(&reversed["removal_sign"], &network, &samples);
    // 診断は別の標本抽出方式を使うため、学習が選んだ標本だけを入力にする。
    let records: Vec<_> = selected
        .into_iter()
        .map(|i| {
            dataset
                .record(dataset.validation_indices()[i as usize])
                .unwrap()
        })
        .collect();
    let selected_path = f.0.join("selected.mnsd");
    fs::write(
        &selected_path,
        common::mnsd::independent(dataset.headers().next().unwrap().clone(), &records),
    )
    .unwrap();
    let diagnostic_path = f.0.join("removal.json");
    let output = Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
        .args(["diag", "nnue-removal", "--data"])
        .arg(selected_path)
        .arg("--float")
        .arg(dir.join("latest.npz"))
        .arg("--net")
        .arg(dir.join("nnue.bin"))
        .args(["--sample", "5", "--seed", "1", "--output"])
        .arg(&diagnostic_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let diagnostic = document(diagnostic_path);
    assert_eq!(
        reversed["removal_sign"]["reversals"],
        diagnostic["reversals"]
    );
}
fn resume_matches(device: DeviceKind) {
    let f = Fixture::new();
    let mut full = f.args("full", 3);
    full.device = device;
    run(full.clone()).unwrap();
    let mut first = f.args("resume", 1);
    first.device = device;
    run(first.clone()).unwrap();
    let before = Checkpoint::read(first.output_dir.join("latest.npz")).unwrap();
    assert!(
        before.progress.completed_steps < 7,
        "stop inside the first epoch"
    );
    let mut resumed = first;
    resumed.superbatches = 3.try_into().unwrap();
    resumed.resume = Some(resumed.output_dir.clone());
    resumed.threads = Some(2);
    resumed.time_limit_hours = 23.0;
    run(resumed.clone()).unwrap();
    bits(
        &Checkpoint::read(full.output_dir.join("latest.npz")).unwrap(),
        &Checkpoint::read(resumed.output_dir.join("latest.npz")).unwrap(),
    );
    assert_eq!(
        fs::read(full.output_dir.join("nnue.bin")).unwrap(),
        fs::read(resumed.output_dir.join("nnue.bin")).unwrap()
    );
}
#[test]
fn cpu_resume_inside_epoch_is_bit_identical_with_changed_budgets() {
    resume_matches(DeviceKind::Cpu);
}
#[test]
#[ignore = "requires a CUDA GPU"]
fn gpu_resume_inside_epoch_is_bit_identical() {
    resume_matches(DeviceKind::Cuda);
}
fn validation_interval(device: DeviceKind) {
    let f = Fixture::new();
    let mut a = f.args("frequent", 3);
    a.device = device;
    let mut b = a.clone();
    b.output_dir = f.0.join("sparse");
    b.validation_superbatches = 2.try_into().unwrap();
    run(a.clone()).unwrap();
    run(b.clone()).unwrap();
    let sparse = logs(&b.output_dir);
    assert!(sparse[1]["validation_loss"].is_null());
    assert!(sparse[2]["validation_loss"].is_number());
    assert!(sparse[3]["validation_loss"].is_number());
    bits(
        &Checkpoint::read(a.output_dir.join("latest.npz")).unwrap(),
        &Checkpoint::read(b.output_dir.join("latest.npz")).unwrap(),
    );
}
#[test]
fn cpu_validation_interval_does_not_change_training() {
    validation_interval(DeviceKind::Cpu);
}
#[test]
#[ignore = "requires a CUDA GPU"]
fn gpu_validation_interval_does_not_change_training() {
    validation_interval(DeviceKind::Cuda);
}
#[test]
#[ignore = "requires a CUDA GPU"]
fn gpu_one_superbatch_matches_cpu_with_existing_tolerance() {
    let f = Fixture::new();
    let cpu = f.args("cpu", 1);
    let mut cuda = f.args("cuda", 1);
    cuda.device = DeviceKind::Cuda;
    run(cpu.clone()).unwrap();
    run(cuda.clone()).unwrap();
    let cpu = Checkpoint::read(cpu.output_dir.join("latest.npz")).unwrap();
    let cuda = Checkpoint::read(cuda.output_dir.join("latest.npz")).unwrap();
    for p in Parameter::ALL {
        for (&a, &e) in cuda.parameters[p].iter().zip(&cpu.parameters[p]) {
            assert!(
                (f64::from(a) - f64::from(e)).abs() <= 1e-6 + 1e-5 * f64::from(e).abs(),
                "{p:?}"
            );
        }
    }
}
#[test]
fn resume_rejects_changed_trajectory_and_inputs_without_mutating_artifacts() {
    let f = Fixture::new();
    let a = f.args("run", 1);
    run(a.clone()).unwrap();
    let bytes = fs::read(a.output_dir.join("latest.npz")).unwrap();
    for change in 0..5 {
        let mut b = a.clone();
        b.resume = Some(b.output_dir.clone());
        b.superbatches = 2.try_into().unwrap();
        match change {
            0 => b.teacher_cp_per_logit = 679.03,
            1 => b.rules = "LISHOGI".into(),
            2 => b.training.lambda = 0.5,
            3 => b.seed += 1,
            _ => b.batch += 1,
        }
        assert!(run(b).is_err(), "{change}");
        assert_eq!(bytes, fs::read(a.output_dir.join("latest.npz")).unwrap());
    }
    let mut b = a.clone();
    b.resume = Some(b.output_dir.clone());
    let copy = f.0.join("copy.mnsd");
    fs::copy(&b.data[0], &copy).unwrap();
    b.data = vec![copy];
    assert!(run(b).is_err());
}
#[test]
fn rules_are_parsed_and_checked_for_every_teacher() {
    let f = Fixture::new();
    let mut a = f.args("mismatch", 1);
    a.rules = "LISHOGI".into();
    assert!(run(a).is_err());
    let lishogi = f.0.join("lishogi.mnsd");
    let header = minase::training::records::Header::new(
        "LISHOGI".into(),
        "1".repeat(40),
        [4; 32],
        minase::training::records::TeacherKind::AlphaBeta,
        200,
        19,
    )
    .unwrap();
    let samples: Vec<_> = (0..120)
        .map(|game| {
            minase_trainer::data::Sample::from_position(
                &common::positions()[0],
                100,
                minase_trainer::data::Outcome::Win,
                game,
                0,
                minase::training::records::Origin::RandomSelfplay,
            )
            .unwrap()
        })
        .collect();
    fs::write(&lishogi, common::mnsd::independent(header, &samples)).unwrap();
    let mut a = f.args("both", 1);
    a.data.push(lishogi.clone());
    assert!(run(a).is_err());
    let mut a = f.args("equivalent", 1);
    a.data = vec![lishogi];
    a.rules = minase_core::Rules::LISHOGI.to_string();
    run(a.clone()).unwrap();
    a.rules = "LISHOGI".into();
    a.resume = Some(a.output_dir.clone());
    run(a).unwrap();
}
#[test]
fn time_limit_before_initial_validation_exits_incomplete_without_network() {
    let f = Fixture::new();
    let mut a = f.args("timeout", 1);
    a.time_limit_hours = 1e-12;
    assert!(matches!(
        run(a.clone()).unwrap_err().downcast_ref::<Error>(),
        Some(Error::Incomplete)
    ));
    assert_eq!(
        document(a.output_dir.join("summary.json"))["status"],
        "incomplete"
    );
    assert!(!a.output_dir.join("nnue.bin").exists());
    assert!(!a.output_dir.join("latest.npz").exists());
}
#[test]
fn subsampling_changes_only_training_and_spill_resume_repeats_it() {
    let f = Fixture::new();
    let mut a = f.args("subsample", 1);
    a.train_subsample = Some("positions:2:0".parse().unwrap());
    a.buckets = 3;
    a.bucket_positions = Some(1000);
    a.scratch_dir = Some(f.0.clone());
    run(a.clone()).unwrap();
    let config = document(a.output_dir.join("config.json"));
    assert!(config["preprocessing"]["train_subsample"].is_object());
    let mut b = a.clone();
    b.output_dir = f.0.join("continuous");
    b.superbatches = 2.try_into().unwrap();
    run(b.clone()).unwrap();
    a.resume = Some(a.output_dir.clone());
    a.superbatches = 2.try_into().unwrap();
    run(a.clone()).unwrap();
    bits(
        &Checkpoint::read(a.output_dir.join("latest.npz")).unwrap(),
        &Checkpoint::read(b.output_dir.join("latest.npz")).unwrap(),
    );
}

fn integer_reference(
    q: &minase::eval::nnue::LayerStackParameters,
    features: &minase_trainer::nnue::layerstack::Features,
) -> i32 {
    let mut input = [0_i64; 256];
    let mut sums = [0_i64; 2];
    for (v, rows) in features.views().iter().enumerate() {
        let mut a: [i64; 256] = std::array::from_fn(|j| i64::from(q.ft_biases[j]));
        for &f in rows {
            for (j, x) in a.iter_mut().enumerate() {
                *x += i64::from(q.ft_weights[f * 256 + j]);
            }
            sums[v] += i64::from(q.psqt_weights[f]);
        }
        for j in 0..128 {
            input[v * 128 + j] = (a[j].clamp(0, 127) * a[j + 128].clamp(0, 127)) >> 7;
        }
    }
    let y: [i64; 16] = std::array::from_fn(|k| {
        input
            .iter()
            .enumerate()
            .fold(i64::from(q.l1_biases[k]), |s, (j, x)| {
                s + i64::from(q.l1_weights[k * 256 + j]) * x
            })
    });
    let z: [i64; 30] = std::array::from_fn(|j| {
        if j < 15 {
            ((y[j] * y[j]) >> 19).clamp(0, 127)
        } else {
            (y[j - 15] >> 6).clamp(0, 127)
        }
    });
    let u: [i64; 32] = std::array::from_fn(|k| {
        let pre = z
            .iter()
            .enumerate()
            .fold(i64::from(q.l2_biases[k]), |s, (j, x)| {
                s + i64::from(q.l2_weights[k * 30 + j]) * x
            });
        (pre >> 6).clamp(0, 127)
    });
    let n = u
        .iter()
        .enumerate()
        .fold(i64::from(q.output_bias) + y[15], |s, (j, x)| {
            s + i64::from(q.output_weights[j]) * x
        });
    ((n + (sums[0] - sums[1]) / 2) / i64::from(q.fv_scale)).clamp(-28999, 28999) as i32
}

#[test]
fn cli_time_limit_preserves_checkpoint_and_resume_matches_continuous_training() {
    let f = Fixture::new();
    let first = f.args("resume-limit", 1);
    run(first.clone()).unwrap();
    let before = Checkpoint::read(first.output_dir.join("latest.npz")).unwrap();
    let mut limited = first.clone();
    limited.superbatches = 3.try_into().unwrap();
    limited.resume = Some(limited.output_dir.clone());
    limited.time_limit_hours = before.progress.elapsed_seconds / 3600.0;
    assert!(matches!(
        run(limited).unwrap_err().downcast_ref::<Error>(),
        Some(Error::Incomplete)
    ));
    let after = Checkpoint::read(first.output_dir.join("latest.npz")).unwrap();
    bits(&before, &after);
    assert!(after.progress.elapsed_seconds > before.progress.elapsed_seconds);
    assert!(!first.output_dir.join("nnue.bin").exists());
    let mut resumed = first;
    resumed.superbatches = 3.try_into().unwrap();
    resumed.resume = Some(resumed.output_dir.clone());
    run(resumed.clone()).unwrap();
    let full = f.args("complete-limit", 3);
    run(full.clone()).unwrap();
    bits(
        &Checkpoint::read(resumed.output_dir.join("latest.npz")).unwrap(),
        &Checkpoint::read(full.output_dir.join("latest.npz")).unwrap(),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
        .args([
            "nnue",
            "train",
            "layerstack",
            "--device",
            "cpu",
            "--threads",
            "1",
            "--data",
        ])
        .arg(&resumed.data[0])
        .args(["--output-dir"])
        .arg(f.0.join("cli-timeout"))
        .args([
            "--rules",
            "JCSA",
            "--teacher-cp-per-logit",
            "1072.65",
            "--lambda",
            "0.75",
            "--superbatches",
            "1",
            "--batches-per-superbatch",
            "1",
            "--validation-superbatches",
            "1",
            "--lr-schedule",
            "constant:1",
            "--time-limit-hours",
            "0.000000000001",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        document(f.0.join("cli-timeout/summary.json"))["status"],
        "incomplete"
    );
}

#[test]
fn cpu_bench_uses_mnsd_and_reports_update_and_supply_times() {
    let f = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
        .args([
            "nnue",
            "bench-step",
            "layerstack",
            "--device",
            "cpu",
            "--threads",
            "2",
            "--batch",
            "8",
            "--steps",
            "2",
            "--seed",
            "1",
            "--lambda",
            "0.75",
            "--teacher-cp-per-logit",
            "1072.65",
            "--profile",
            "--data",
        ])
        .arg(f.0.join("data.mnsd"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["steps"], 2);
    for key in [
        "compute_ms",
        "supply_seconds",
        "worker_positions_per_second",
    ] {
        assert!(report[key].as_f64().unwrap() > 0.0, "{key}");
    }
    assert!(report["profile"]["backward_ms"].as_f64().unwrap() > 0.0);
    assert!(report["profile"]["optimizer_ms"].as_f64().unwrap() > 0.0);
}
