//! 較正JSONの契約、教師分類の照合、記録と再開を小さなMNSDで検証する。
#[path = "common/mnsd.rs"]
mod mnsd;
use minase::{
    eval::{features::FEATURE_COUNT, pst::Pst},
    training::records::{Header, Origin, TeacherKind},
};
use minase_core::{Color, PieceCode, PieceKind, PositionBuilder, Square};
use minase_trainer::{
    data::{
        self, CalibrateArgs, Dataset, Error, Lookahead, Outcome, Sample, TeacherCpPerLogitSource,
    },
    nn::checkpoint::MismatchField,
    pst::{self, Args, RemovalReferenceKind},
};
use serde_json::{Value, json};
use std::{
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "teacher-calibration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let pst = Pst::from_weights(
            &vec![[0; 2]; FEATURE_COUNT],
            pst::v0_piece_values(),
            317.25,
            "L0,P0,R1,E0",
        )
        .unwrap();
        fs::write(root.join("init.bin"), pst.encode()).unwrap();
        Self(root)
    }

    fn input(&self, name: &str, class: u8, score: i16, origin: Origin, training: bool) -> PathBuf {
        let seed = 11 * u64::from(class);
        let training_game = (0..)
            .find(|&game| !data::is_validation(seed, game))
            .unwrap();
        let validation_game = (0..).find(|&game| data::is_validation(seed, game)).unwrap();
        let mut builder = PositionBuilder::new(Color::Black);
        for (file, rank, color, kind) in [
            (0, 0, Color::Black, PieceKind::King),
            (11, 11, Color::White, PieceKind::King),
            (2, 4, Color::Black, PieceKind::Pawn),
        ] {
            builder
                .put(
                    Square::new(file, rank).unwrap(),
                    PieceCode::new(color, kind).unwrap(),
                )
                .unwrap();
        }
        let position = builder.finish().unwrap();
        let mut samples = Vec::new();
        // 3勝1敗を3組。非0の同一探索値に対して有限な内点の尺度を持つ。
        for i in 0..14 {
            let game = if training && i < 12 {
                training_game
            } else {
                validation_game
            };
            let outcome = if i % 4 == 3 {
                Outcome::Loss
            } else {
                Outcome::Win
            };
            samples
                .push(Sample::from_position(&position, score, outcome, game, 0, origin).unwrap());
        }
        let header = Header::new(
            "L0,P0,R1,E0".into(),
            format!("{class:040x}"),
            [class; 32],
            TeacherKind::AlphaBeta,
            20_000,
            seed,
        )
        .unwrap();
        let path = self.0.join(name);
        fs::write(&path, mnsd::independent(header, &samples)).unwrap();
        path
    }

    fn calibrate_args(&self, data: Vec<PathBuf>, name: &str) -> CalibrateArgs {
        CalibrateArgs {
            data,
            lookahead_gamma: None,
            lookahead_plies: None,
            output: self.0.join(name),
            threads: Some(1),
        }
    }

    fn train_args(&self, data: Vec<PathBuf>, name: &str) -> Args {
        Args {
            data,
            teacher_cp_per_logit: TeacherCpPerLogitSource::Calibrate,
            init: self.0.join("init.bin"),
            output_dir: self.0.join(name),
            lr: 0.03,
            epochs: 1,
            patience: 2,
            batch: 4,
            seed: 1,
            removal_penalty: 0.0,
            removal_reference: RemovalReferenceKind::Init,
            lambda: 0.75,
            lookahead_gamma: None,
            lookahead_plies: None,
            validation_sample: 2,
            threads: Some(1),
            resume: None,
        }
    }

    fn write_json(&self, name: &str, document: &Value) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, serde_json::to_vec_pretty(document).unwrap()).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn read(path: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn calibration_cli(args: &CalibrateArgs) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_minase-trainer"));
    command
        .args(["data", "calibrate", "--data"])
        .args(&args.data)
        .arg("--output")
        .arg(&args.output)
        .args(["--threads", "1"]);
    if let (Some(gamma), Some(plies)) = (args.lookahead_gamma, args.lookahead_plies) {
        command
            .arg("--lookahead-gamma")
            .arg(gamma.to_string())
            .arg("--lookahead-plies")
            .arg(plies.to_string());
    }
    command.output().unwrap()
}
fn train_cli(args: &Args) -> Output {
    Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
        .args(["pst", "train", "--data"])
        .args(&args.data)
        .arg("--init")
        .arg(&args.init)
        .arg("--output-dir")
        .arg(&args.output_dir)
        .args([
            "--epochs",
            "1",
            "--patience",
            "2",
            "--removal-reference",
            "init",
            "--threads",
            "1",
            "--lambda",
            "0.75",
            "--teacher-cp-per-logit",
            "calibrate",
        ])
        .output()
        .unwrap()
}

#[test]
fn command_and_training_record_identical_scales_and_file_values_round_trip() {
    let fixture = Fixture::new();
    let input = fixture.input("input.mnsd", 1, 1000, Origin::RandomSelfplay, true);
    for lookahead in [false, true] {
        let mut calibration = fixture.calibrate_args(
            vec![input.clone()],
            &format!("calibration-{lookahead}.json"),
        );
        let mut args = fixture.train_args(vec![input.clone()], &format!("estimated-{lookahead}"));
        if lookahead {
            calibration.lookahead_gamma = Some(0.9);
            calibration.lookahead_plies = Some(40);
            args.lookahead_gamma = Some(0.9);
            args.lookahead_plies = Some(40);
        }
        let output = calibration_cli(&calibration);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut document = read(&calibration.output);
        assert_eq!(document["format"], "minase-trainer-teacher-calibration");
        assert_eq!(document["trainer_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(
            document["conditions"]["validation_split"],
            "hash64(seed, game) % 20 == 0"
        );
        assert_eq!(document["conditions"]["precision"], "f64");
        assert_eq!(
            document["conditions"]["search_range"],
            json!([50.0, 4000.0])
        );
        assert_eq!(document["conditions"]["iterations"], 64);
        assert_eq!(
            document["conditions"]["inputs"][0],
            serde_json::to_value(minase_trainer::identity::file_identity(&input).unwrap()).unwrap()
        );
        assert_eq!(document["classes"][0]["training_positions"], 12);
        assert_eq!(document["classes"][0]["network_checksum"], "01".repeat(32));
        let dataset = Dataset::open(
            &args.data,
            if lookahead {
                Some(Lookahead::new(0.9, 40).unwrap())
            } else {
                None
            },
            NonZeroUsize::new(1).unwrap(),
        )
        .unwrap()
        .with_lambda(0.75)
        .unwrap();
        let value = dataset.estimate_teacher_cp_per_logit().unwrap()[0]
            .unwrap()
            .value;
        assert_eq!(
            document["classes"][0]["teacher_cp_per_logit"]
                .as_f64()
                .unwrap()
                .to_bits(),
            value.to_bits()
        );
        pst::run(args.clone()).unwrap();
        let config = read(args.output_dir.join("config.json"));
        let preprocessing = &config["preprocessing"];
        assert_eq!(preprocessing["teacher_cp_per_logit_source"], "calibrate");
        assert_eq!(config["arguments"]["teacher_cp_per_logit"], "calibrate");
        assert_eq!(
            document["classes"][0]["teacher_cp_per_logit"].to_string(),
            preprocessing["teacher_cp_per_logit"][0].to_string()
        );
        assert_eq!(
            preprocessing["teacher_score_multiplier"][0],
            json!(317.25_f64 / value)
        );
        assert_eq!(
            preprocessing["teacher_cp_per_logit_at_search_bound"],
            json!([false])
        );
        // 記録した値を較正ファイルへ写し、入力一覧が異なっても固定値を使える。
        document["classes"][0]["teacher_cp_per_logit"] =
            preprocessing["teacher_cp_per_logit"][0].clone();
        document["conditions"]["inputs"] = json!([]);
        document["annotation"] = json!("preserve the full calibration document");
        let path = fixture.write_json(&format!("copied-{lookahead}.json"), &document);
        let mut fixed = args.clone();
        fixed.output_dir = fixture.0.join(format!("fixed-{lookahead}"));
        fixed.teacher_cp_per_logit = TeacherCpPerLogitSource::File(path.clone());
        pst::run(fixed.clone()).unwrap();
        let config_fixed = read(fixed.output_dir.join("config.json"));
        for key in [
            "teacher_cp_per_logit",
            "teacher_score_multiplier",
            "teacher_cp_per_logit_at_search_bound",
        ] {
            assert_eq!(
                config_fixed["preprocessing"][key], preprocessing[key],
                "{key}"
            );
        }
        assert_eq!(
            config_fixed["preprocessing"]["teacher_cp_per_logit_source"],
            json!({"calibration_file": path.canonicalize().unwrap(), "calibration": document})
        );
        assert_eq!(
            fs::read(args.output_dir.join("pst.bin")).unwrap(),
            fs::read(fixed.output_dir.join("pst.bin")).unwrap()
        );
    }
}

#[test]
fn invalid_calibrations_fail_before_training_and_keep_typed_causes() {
    let fixture = Fixture::new();
    let input = fixture.input("input.mnsd", 1, 1000, Origin::RandomSelfplay, true);
    let calibration = fixture.calibrate_args(vec![input.clone()], "valid.json");
    data::calibrate(calibration.clone()).unwrap();
    let valid = read(&calibration.output);
    type RejectionCase = (&'static str, fn(&mut Value), fn(&Error) -> bool);
    let cases: &[RejectionCase] = &[
        (
            "format",
            |v| v["format"] = json!("unknown"),
            |e| matches!(e, Error::CalibrationFormat),
        ),
        (
            "missing",
            |v| v["classes"] = json!([]),
            |e| matches!(e, Error::MissingCalibrationClass { class: 0 }),
        ),
        (
            "extra",
            |v| {
                let mut extra = v["classes"][0].clone();
                extra["search_budget"] = json!(20_001);
                v["classes"].as_array_mut().unwrap().push(extra);
            },
            |e| matches!(e, Error::UnexpectedCalibrationClass { entry: 1 }),
        ),
        (
            "duplicate",
            |v| {
                let extra = v["classes"][0].clone();
                v["classes"].as_array_mut().unwrap().push(extra);
            },
            |e| matches!(e, Error::DuplicateCalibrationClass { entry: 1 }),
        ),
        (
            "zero",
            |v| v["classes"][0]["teacher_cp_per_logit"] = json!(0.0),
            |e| matches!(e, Error::CalibrationScale { entry: 0 }),
        ),
        (
            "negative",
            |v| v["classes"][0]["teacher_cp_per_logit"] = json!(-1.0),
            |e| matches!(e, Error::CalibrationScale { entry: 0 }),
        ),
        (
            "nan",
            |v| v["classes"][0]["teacher_cp_per_logit"] = json!(f64::NAN),
            |e| matches!(e, Error::CalibrationScale { entry: 0 }),
        ),
        (
            "infinity",
            |v| v["classes"][0]["teacher_cp_per_logit"] = json!("Infinity"),
            |e| matches!(e, Error::CalibrationScale { entry: 0 }),
        ),
    ];
    for (name, change, expected) in cases {
        let mut document = valid.clone();
        change(&mut document);
        let mut args = fixture.train_args(vec![input.clone()], name);
        args.teacher_cp_per_logit =
            TeacherCpPerLogitSource::File(fixture.write_json(&format!("{name}.json"), &document));
        let error = pst::run(args.clone()).unwrap_err();
        assert!(
            matches!(&error, pst::Error::Data(error) if expected(error)),
            "{name}: {error:?}"
        );
        assert!(!args.output_dir.exists(), "{name}");
    }
    // JSON自体に表現できない非有限数も構文エラーの原因を保つ。
    for literal in ["NaN", "Infinity", "1e999"] {
        let path = fixture.0.join("nonfinite.json");
        let mut document = valid.clone();
        document["classes"][0]["teacher_cp_per_logit"] = json!("REPLACE");
        fs::write(&path, document.to_string().replace("\"REPLACE\"", literal)).unwrap();
        let mut args = fixture.train_args(vec![input.clone()], "nonfinite");
        args.teacher_cp_per_logit = TeacherCpPerLogitSource::File(path);
        assert!(matches!(
            pst::run(args.clone()),
            Err(pst::Error::Data(Error::CalibrationJson(_)))
        ));
        assert!(!args.output_dir.exists());
    }
}

#[test]
fn all_six_class_attributes_are_required_to_match() {
    let fixture = Fixture::new();
    let input = fixture.input("input.mnsd", 1, 1000, Origin::RandomSelfplay, true);
    let calibration = fixture.calibrate_args(vec![input.clone()], "valid.json");
    data::calibrate(calibration.clone()).unwrap();
    for (field, changed) in [
        ("generation_commit", json!("a".repeat(40))),
        ("network_checksum", json!("ff".repeat(32))),
        ("search_budget", json!(20_001)),
        ("rule_set", json!("L0,P0,R1,E1")),
        ("origin", json!("human-opening-selfplay")),
        ("lookahead", json!({"gamma": 0.9, "plies": 40})),
    ] {
        let mut document = read(&calibration.output);
        document["classes"][0][field] = changed;
        let mut args = fixture.train_args(vec![input.clone()], field);
        args.teacher_cp_per_logit =
            TeacherCpPerLogitSource::File(fixture.write_json(&format!("{field}.json"), &document));
        assert!(
            matches!(
                pst::run(args.clone()),
                Err(pst::Error::Data(Error::UnexpectedCalibrationClass {
                    entry: 0
                }))
            ),
            "{field}"
        );
        assert!(!args.output_dir.exists());
    }
}

#[test]
fn reordered_inputs_match_by_attributes_and_allow_zero_lambda_without_entries() {
    let fixture = Fixture::new();
    let first = fixture.input("first.mnsd", 1, 1000, Origin::RandomSelfplay, true);
    let second = fixture.input("second.mnsd", 2, 2000, Origin::RandomSelfplay, true);
    let human = fixture.input("human.mnsd", 3, 0, Origin::HumanGame, true);
    let calibration = fixture.calibrate_args(
        vec![first.clone(), second.clone(), human.clone()],
        "calibration.json",
    );
    data::calibrate(calibration.clone()).unwrap();
    let document = read(&calibration.output);
    assert_eq!(document["classes"].as_array().unwrap().len(), 2);
    assert!(
        document["classes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["origin"] != "human-game")
    );
    let expected = [
        document["classes"][0]["teacher_cp_per_logit"].clone(),
        document["classes"][1]["teacher_cp_per_logit"].clone(),
    ];
    assert_ne!(expected[0], expected[1]);
    for reverse in [false, true] {
        let inputs = if reverse {
            vec![second.clone(), human.clone(), first.clone()]
        } else {
            vec![first.clone(), human.clone(), second.clone()]
        };
        let mut args = fixture.train_args(inputs, &format!("order-{reverse}"));
        args.teacher_cp_per_logit = TeacherCpPerLogitSource::File(calibration.output.clone());
        pst::run(args.clone()).unwrap();
        let p = read(args.output_dir.join("config.json"))["preprocessing"].clone();
        assert_eq!(
            p["teacher_cp_per_logit"],
            if reverse {
                json!([expected[1], null, expected[0]])
            } else {
                json!([expected[0], null, expected[1]])
            }
        );
        assert!(p["teacher_score_multiplier"][1].is_null());
        assert!(p["teacher_cp_per_logit_at_search_bound"][1].is_null());
    }
    let human_calibration = fixture.calibrate_args(vec![human.clone()], "human.json");
    data::calibrate(human_calibration.clone()).unwrap();
    assert_eq!(read(&human_calibration.output)["classes"], json!([]));
    for (name, inputs, lambda) in [
        ("human-only", vec![human], 0.75),
        ("result-only", vec![first], 0.0),
    ] {
        let mut args = fixture.train_args(inputs, name);
        args.lambda = lambda;
        args.teacher_cp_per_logit = TeacherCpPerLogitSource::File(human_calibration.output.clone());
        pst::run(args.clone()).unwrap();
        let p = read(args.output_dir.join("config.json"))["preprocessing"].clone();
        for field in [
            "teacher_cp_per_logit",
            "teacher_score_multiplier",
            "teacher_cp_per_logit_at_search_bound",
        ] {
            assert_eq!(p[field], json!([null]));
        }
    }
}

#[test]
fn zero_scores_are_undetermined_and_existing_calibration_outputs_are_untouched() {
    let fixture = Fixture::new();
    let input = fixture.input("zero.mnsd", 1, 0, Origin::RandomSelfplay, true);
    let calibration = fixture.calibrate_args(vec![input.clone()], "zero.json");
    let error = data::calibrate(calibration.clone()).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<Error>(),
        Some(Error::UndeterminedScale)
    ));
    let output = calibration_cli(&calibration);
    assert_eq!(output.status.code(), Some(1));
    assert!(!calibration.output.exists());
    let args = fixture.train_args(vec![input], "zero-train");
    assert!(matches!(
        pst::run(args.clone()),
        Err(pst::Error::Data(Error::UndeterminedScale))
    ));
    assert_eq!(train_cli(&args).status.code(), Some(1));
    assert!(!args.output_dir.exists());
    fs::write(&calibration.output, b"keep these bytes").unwrap();
    let error = data::calibrate(calibration.clone()).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<Error>(),
        Some(Error::CalibrationOutputExists { .. })
    ));
    assert_eq!(fs::read(&calibration.output).unwrap(), b"keep these bytes");
}

#[test]
fn active_class_without_training_positions_is_rejected_even_with_a_file() {
    let fixture = Fixture::new();
    let first = fixture.input("first.mnsd", 1, 1000, Origin::RandomSelfplay, true);
    let second = fixture.input("second.mnsd", 2, 1000, Origin::RandomSelfplay, true);
    let calibration = fixture.calibrate_args(vec![first.clone(), second], "calibration.json");
    data::calibrate(calibration.clone()).unwrap();
    let validation_only = fixture.input("validation.mnsd", 2, 1000, Origin::RandomSelfplay, false);
    let inputs = vec![first, validation_only];
    let error = data::calibrate(fixture.calibrate_args(inputs.clone(), "empty.json")).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<Error>(),
        Some(Error::EmptyTeacherClass { class: 1 })
    ));
    for (name, source) in [
        ("estimate-empty", TeacherCpPerLogitSource::Calibrate),
        (
            "file-empty",
            TeacherCpPerLogitSource::File(calibration.output),
        ),
    ] {
        let mut args = fixture.train_args(inputs.clone(), name);
        args.teacher_cp_per_logit = source;
        assert!(matches!(
            pst::run(args.clone()),
            Err(pst::Error::Data(Error::EmptyTeacherClass { class: 1 }))
        ));
        assert!(!args.output_dir.exists());
    }
}

#[test]
fn resume_requires_the_same_calibration_source_and_rejects_earlier_preprocessing() {
    let fixture = Fixture::new();
    let input = fixture.input("input.mnsd", 1, 1000, Origin::RandomSelfplay, true);
    let calibration = fixture.calibrate_args(vec![input.clone()], "calibration.json");
    data::calibrate(calibration.clone()).unwrap();
    let other_file = fixture.0.join("other.json");
    fs::copy(&calibration.output, &other_file).unwrap();
    let assert_rejected = |args: Args| {
        let before = fs::read(args.output_dir.join("config.json")).unwrap();
        assert!(matches!(
            pst::run(args.clone()),
            Err(pst::Error::Nn(
                minase_trainer::nn::Error::CheckpointMismatch(MismatchField::Preprocessing)
            ))
        ));
        assert_eq!(
            fs::read(args.output_dir.join("config.json")).unwrap(),
            before
        );
    };
    for (name, source) in [
        ("estimated", TeacherCpPerLogitSource::Calibrate),
        (
            "fixed",
            TeacherCpPerLogitSource::File(calibration.output.clone()),
        ),
    ] {
        let mut args = fixture.train_args(vec![input.clone()], name);
        args.teacher_cp_per_logit = source;
        pst::run(args.clone()).unwrap();
        args.resume = Some(args.output_dir.clone());
        pst::run(args.clone()).unwrap();
        let mut changed = args.clone();
        changed.teacher_cp_per_logit = TeacherCpPerLogitSource::File(other_file.clone());
        assert_rejected(changed);
        if name == "estimated" {
            // フェーズ1の記録は固定文字列trainingと新しい配列の欠落で拒否する。
            let path = args.output_dir.join("config.json");
            let mut saved = read(&path);
            let preprocessing = saved["preprocessing"].as_object_mut().unwrap();
            preprocessing.insert("teacher_cp_per_logit_source".into(), json!("training"));
            preprocessing.remove("teacher_score_multiplier");
            preprocessing.remove("teacher_cp_per_logit_at_search_bound");
            fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
            assert_rejected(args.clone());
            let preprocessing = saved["preprocessing"].as_object_mut().unwrap();
            let value = preprocessing.remove("teacher_cp_per_logit").unwrap();
            preprocessing.insert("teacher_ks".into(), value);
            fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
            assert_rejected(args);
        } else {
            // 同じファイルへのパス表記の違いは正規化後に同じ設定になる。
            args.teacher_cp_per_logit =
                TeacherCpPerLogitSource::File(fixture.0.join(".").join("calibration.json"));
            pst::run(args.clone()).unwrap();
            let mut changed = read(&calibration.output);
            changed["trainer_version"] = json!("different-provenance");
            fs::write(&calibration.output, serde_json::to_vec(&changed).unwrap()).unwrap();
            assert_rejected(args);
        }
    }
}

#[test]
fn calibration_cli_requires_inputs_output_and_paired_lookahead() {
    use clap::Parser;
    #[derive(Parser)]
    struct Parsed {
        #[command(flatten)]
        args: CalibrateArgs,
    }
    let required = [
        "calibrate",
        "--data",
        "missing.mnsd",
        "--output",
        "calibration.json",
    ];
    let parsed = Parsed::try_parse_from(required).unwrap().args;
    assert_eq!(parsed.threads, None);
    for flag in ["--data", "--output"] {
        let mut missing = required.to_vec();
        let index = missing.iter().position(|&value| value == flag).unwrap();
        missing.drain(index..index + 2);
        assert_eq!(
            Parsed::try_parse_from(missing).err().unwrap().kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }
    for extra in [["--lookahead-gamma", "0.9"], ["--lookahead-plies", "40"]] {
        assert_eq!(
            Parsed::try_parse_from(required.into_iter().chain(extra))
                .err()
                .unwrap()
                .kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }
    for (gamma, plies) in [(0.0, 40), (1.0, 40), (f64::NAN, 40), (0.9, 0), (0.9, 4097)] {
        let mut args = Parsed::try_parse_from(required).unwrap().args;
        args.lookahead_gamma = Some(gamma);
        args.lookahead_plies = Some(plies);
        assert!(matches!(
            data::calibrate(args).unwrap_err().downcast_ref::<Error>(),
            Some(Error::LookaheadParameters)
        ));
    }
    let mut args = Parsed::try_parse_from(required).unwrap().args;
    args.threads = Some(0);
    assert!(data::calibrate(args).is_err());
}

#[test]
fn supplied_scales_keep_every_bit_and_do_not_reestimate_zero_scores() {
    let fixture = Fixture::new();
    let input = fixture.input("input.mnsd", 1, 1000, Origin::RandomSelfplay, true);
    let calibration = fixture.calibrate_args(vec![input.clone()], "calibration.json");
    data::calibrate(calibration.clone()).unwrap();
    let mut document = read(&calibration.output);
    // 同じ6属性で探索値だけが0の別集合にも、外部で固定した尺度を使える。
    let zero = fixture.input("zero.mnsd", 1, 0, Origin::RandomSelfplay, true);
    for (index, value) in [
        1217.6196418053848_f64,
        910.2392016071356,
        f64::from_bits(1000.0_f64.to_bits() + 1),
    ]
    .into_iter()
    .enumerate()
    {
        document["classes"][0]["teacher_cp_per_logit"] = json!(value);
        document["classes"][0]["at_search_bound"] = json!(true);
        let path = fixture.write_json(&format!("precise-{index}.json"), &document);
        assert_eq!(
            read(&path)["classes"][0]["teacher_cp_per_logit"]
                .as_f64()
                .unwrap()
                .to_bits(),
            value.to_bits()
        );
        let mut args = fixture.train_args(vec![zero.clone()], &format!("precise-{index}"));
        args.teacher_cp_per_logit = TeacherCpPerLogitSource::File(path);
        pst::run(args.clone()).unwrap();
        let p = read(args.output_dir.join("config.json"))["preprocessing"].clone();
        assert_eq!(
            p["teacher_cp_per_logit"][0].as_f64().unwrap().to_bits(),
            value.to_bits()
        );
        assert_eq!(
            p["teacher_score_multiplier"][0].as_f64().unwrap().to_bits(),
            (317.25_f64 / value).to_bits()
        );
        assert_eq!(p["teacher_cp_per_logit_at_search_bound"], json!([true]));
    }
}

#[test]
fn search_bounds_warn_once_per_class_and_are_recorded_without_stopping() {
    let fixture = Fixture::new();
    for (name, score, bound) in [("lower", 1, 50.0), ("upper", 32767, 4000.0)] {
        let input = fixture.input(
            &format!("{name}.mnsd"),
            1,
            score,
            Origin::RandomSelfplay,
            true,
        );
        let args = fixture.calibrate_args(vec![input.clone()], &format!("{name}.json"));
        let output = calibration_cli(&args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let document = read(&args.output);
        let value = document["classes"][0]["teacher_cp_per_logit"]
            .as_f64()
            .unwrap();
        assert!((value - bound).abs() < 1e-9);
        assert_eq!(document["classes"][0]["at_search_bound"], true);
        let warning = format!(
            "warning: teacher class 0: teacher_cp_per_logit={value} reached a search bound"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .filter(|line| *line == warning)
                .count(),
            1
        );
        let train = fixture.train_args(vec![input], &format!("{name}-train"));
        let output = train_cli(&train);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .filter(|line| *line == warning)
                .count(),
            1
        );
        let config = read(train.output_dir.join("config.json"));
        assert_eq!(
            config["preprocessing"]["teacher_cp_per_logit_at_search_bound"],
            json!([true])
        );
    }
}

#[test]
fn external_scores_use_the_same_training_order_without_rounding_to_f32() {
    let fixture = Fixture::new();
    let first = fixture.input("first.mnsd", 1, 1000, Origin::RandomSelfplay, true);
    let second = fixture.input("second.mnsd", 2, 2000, Origin::RandomSelfplay, true);
    let human = fixture.input("human.mnsd", 3, 0, Origin::HumanGame, true);
    let dataset = Dataset::open(&[first, second, human], None, NonZeroUsize::new(1).unwrap())
        .unwrap()
        .with_lambda(0.75)
        .unwrap();
    let stored: Vec<_> = (0..dataset.record_count())
        .map(|index| f64::from(dataset.teacher_score(index).unwrap()))
        .collect();
    assert_eq!(
        dataset.estimate_teacher_cp_per_logit().unwrap(),
        dataset
            .estimate_teacher_cp_per_logit_with_scores(&stored)
            .unwrap()
    );
    assert!(matches!(
        dataset.estimate_teacher_cp_per_logit_with_scores(&stored[1..]),
        Err(Error::TeacherSamples)
    ));
    let mut external = vec![1_000.000_03; 14];
    external.extend([2_000.000_06; 14]);
    external.extend([f64::NAN; 14]);
    // 検証局面とλが0の分類の探索値は、較正の標本に含めない。
    for &index in dataset.validation_indices() {
        external[index as usize] = f64::NAN;
    }
    let actual = dataset
        .estimate_teacher_cp_per_logit_with_scores(&external)
        .unwrap();
    let outcomes = [2, 2, 2, 0, 2, 2, 2, 0, 2, 2, 2, 0];
    for (class, score) in [1_000.000_03, 2_000.000_06].into_iter().enumerate() {
        let expected = data::estimate_cp_per_logit(&[score; 12], &outcomes).unwrap();
        assert_eq!(
            actual[class].unwrap().value.to_bits(),
            expected.value.to_bits()
        );
        assert_ne!(
            actual[class].unwrap().value.to_bits(),
            dataset.estimate_teacher_cp_per_logit().unwrap()[class]
                .unwrap()
                .value
                .to_bits()
        );
    }
    assert!(actual[2].is_none());
    external[0] = f64::INFINITY;
    assert!(matches!(
        dataset.estimate_teacher_cp_per_logit_with_scores(&external),
        Err(Error::TeacherSamples)
    ));
}
