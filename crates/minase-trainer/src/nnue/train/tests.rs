use crate::nn::checkpoint::MismatchField;
#[path = "../../../tests/common/mnsd.rs"]
mod mnsd;
use super::*;
use crate::data::{Outcome, Sample};
use crate::nn::{BestEpoch, sha256};
use crate::{
    data::Dataset,
    nnue::{
        data::Observations,
        model::{Checkpoint, Masks, Model, Progress, Shape},
        reference::{AdamW, AdamWConfig},
        schedule::LrSchedule,
    },
};
use minase::training::records::{GAME_HEADER_LEN, HEADER_LEN, Header};
use minase_core::{Color, PieceCode, PieceKind, PositionBuilder, Square};
use std::{
    cell::Cell,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, UNIX_EPOCH},
};

fn args() -> Args {
    Args {
        device: crate::nnue::options::DeviceKind::Cuda,
        training: crate::nnue::options::TrainingOptions::new(1.0),
        lr_schedule: LrSchedule::default(),
        feature_set: crate::nnue::data::InputFeatureSet::P,
        width: 8,
        data: vec!["records.bin".into()],
        init_pst: "pst-float.npz".into(),
        base: "pst.bin".into(),
        output_dir: "output".into(),
        lookahead_gamma: None,
        lookahead_plies: None,
        batch: 33,
        epochs: 50,
        patience: 5,
        seed: 1,
        time_limit_hours: 24.0,
        validation_sample: 24,
        validation_interval: 0,
        threads: Some(2),
        resume: None,
    }
}

fn initial_progress() -> Progress {
    Progress {
        epoch: 0,
        best: None,
        elapsed_seconds: 0.0,
        seed: 1,
        config: crate::nn::checkpoint::Configuration {
            settings: serde_json::json!({}),
            inputs: serde_json::json!({}),
            preprocessing: serde_json::json!({}),
        },
    }
}

#[test]
fn best_epoch_rejects_non_advancing_epochs_without_changing_progress() {
    // 完了済みエポック以前の結果では、進捗も最良値も変更しない。
    for epoch in [1, 2] {
        let mut progress = initial_progress();
        update_best(&mut progress, 2, 0.5).unwrap();
        let saved = progress.clone();
        let error = update_best(&mut progress, epoch, 0.4).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<Error>(),
                Some(Error::Invalid("epoch must advance"))
            ),
            "epoch={epoch}: {error}"
        );
        assert_eq!(progress, saved, "epoch={epoch}");
    }
}

#[test]
fn non_finite_validation_cannot_replace_a_completed_best_epoch() {
    for loss in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut progress = initial_progress();
        update_best(&mut progress, 1, 0.5).unwrap();
        let saved = progress.clone();
        assert!(update_best(&mut progress, 2, loss).is_err());
        assert_eq!(progress, saved);
    }
}

#[test]
fn early_stopping_waits_for_five_unimproved_epochs_and_resets_on_improvement() {
    let args = args();
    let mut progress = initial_progress();
    update_best(&mut progress, 1, 0.5).unwrap();
    for epoch in 2..=5 {
        update_best(&mut progress, epoch, 0.5).unwrap();
        assert_eq!(stop_reason(&progress, &args), None, "epoch={epoch}");
    }
    update_best(&mut progress, 6, 0.6).unwrap();
    assert_eq!(
        stop_reason(&progress, &args),
        Some(StopReason::EarlyStopping)
    );

    let mut progress = initial_progress();
    update_best(&mut progress, 1, 0.5).unwrap();
    update_best(&mut progress, 5, 0.4).unwrap();
    for epoch in 6..=9 {
        update_best(&mut progress, epoch, 0.5).unwrap();
        assert_eq!(stop_reason(&progress, &args), None, "epoch={epoch}");
    }
    update_best(&mut progress, 10, 0.4).unwrap();
    assert_eq!(
        stop_reason(&progress, &args),
        Some(StopReason::EarlyStopping)
    );
}

#[test]
fn budgets_stop_at_the_boundary_without_selecting_epoch_zero() {
    let args = args();
    let mut progress = initial_progress();
    progress.elapsed_seconds = 24.0 * 3600.0 - 1.0;
    assert_eq!(stop_reason(&progress, &args), None);
    progress.elapsed_seconds += 1.0;
    assert_eq!(stop_reason(&progress, &args), Some(StopReason::TimeLimit));
    assert_eq!(progress.best, None);

    let mut progress = initial_progress();
    update_best(&mut progress, 49, 0.5).unwrap();
    assert_eq!(stop_reason(&progress, &args), None);
    update_best(&mut progress, 50, 0.6).unwrap();
    assert_eq!(stop_reason(&progress, &args), Some(StopReason::MaxEpochs));
    progress.elapsed_seconds = 24.0 * 3600.0;
    assert_eq!(stop_reason(&progress, &args), Some(StopReason::TimeLimit));
    assert_eq!(progress.best.unwrap().epoch, 49);
}

fn validation_statistics() -> Validation {
    Validation {
        loss_sum: 0.25,
        correct: 0,
        decisive: 0,
        nonlinear_sum: 0.0,
        nonlinear_square_sum: 0.0,
        lower: 8,
        upper: 0,
        positions: 1,
        activations: 16,
    }
}

#[test]
fn validation_deadline_includes_the_final_batch_and_stops_further_evaluation() {
    // 各バッチの前後で期限を検査し、期限未満で全件を終えた場合だけ集計を返す。
    for (case, initial_time, completion_times, expected_evaluations, completed) in [
        ("final batch before deadline", 0, [3, 9], 2, true),
        ("final batch at deadline", 0, [3, 10], 2, false),
        ("final batch after deadline", 0, [3, 11], 2, false),
        ("first batch at deadline", 0, [10, 11], 1, false),
        ("deadline before first batch", 10, [10, 11], 0, false),
    ] {
        let now = Cell::new(initial_time);
        let evaluated = Cell::new(0);
        let result = validate_batches(
            completion_times,
            |completed_at| {
                evaluated.set(evaluated.get() + 1);
                now.set(completed_at);
                Ok(validation_statistics())
            },
            || now.get() >= 10,
        )
        .unwrap_or_else(|error| panic!("{case}: {error}"));
        assert_eq!(evaluated.get(), expected_evaluations, "{case}");
        if completed {
            let result = result.unwrap_or_else(|| panic!("{case}: missing validation"));
            assert_eq!(result.positions, 2, "{case}");
            assert_eq!(result.loss(), 0.25, "{case}");
        } else {
            assert!(result.is_none(), "{case}: unexpected validation");
        }
    }
}

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "nnue-training-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn training_args(&self) -> Args {
        let mut args = args();
        args.data = vec![self.0.join("records.bin")];
        args.base = self.0.join("pst.bin");
        args.init_pst = self.0.join("pst-float.npz");
        args.output_dir = self.0.join("continuous");
        args.epochs = 2;
        args.lookahead_gamma = Some(0.9);
        args.lookahead_plies = Some(40);
        fs::write(
            &args.base,
            include_bytes!("../../../../minase/nets/pst.bin"),
        )
        .unwrap();
        let initial: Vec<_> = base::read(&args.base)
            .unwrap()
            .linear
            .iter()
            .map(|pair| pair.map(|weight| f32::from(weight) / 8.0))
            .collect();
        fs::write(
            &args.init_pst,
            crate::pst::encode_pst_npz(&initial).unwrap(),
        )
        .unwrap();

        // seed=11では0..40のうち4、15、22が検証対局である。
        // 40対局×8局面のうち訓練296局面を33件ずつに分け、末尾を32件にする。
        let header = Header::new(
            "L0,P0,R1,E0".into(),
            "1".repeat(40),
            [2; 32],
            minase::training::records::TeacherKind::AlphaBeta,
            200,
            11,
        )
        .unwrap();
        let mut samples = Vec::new();
        for game in 0..40_u32 {
            for ply in 0..8_u16 {
                let side = if ply % 2 == 0 {
                    Color::Black
                } else {
                    Color::White
                };
                let mut builder = PositionBuilder::new(side);
                for (file, rank, color, kind) in [
                    (4, 0, Color::Black, PieceKind::King),
                    (7, 11, Color::White, PieceKind::King),
                    ((game % 12) as u8, 3, Color::Black, PieceKind::Pawn),
                    (((game + 6) % 12) as u8, 8, Color::White, PieceKind::Pawn),
                    (((game + 3) % 12) as u8, 6, Color::Black, PieceKind::Rook),
                ] {
                    builder
                        .put(
                            Square::new(file, rank).unwrap(),
                            PieceCode::new(color, kind).unwrap(),
                        )
                        .unwrap();
                }
                let score = if side == Color::Black { 300 } else { -300 };
                let outcome = match (game % 3, side) {
                    (0, _) => Outcome::Draw,
                    (1, Color::Black) | (2, Color::White) => Outcome::Win,
                    _ => Outcome::Loss,
                };
                samples.push(
                    Sample::from_position(
                        &builder.finish().unwrap(),
                        score,
                        outcome,
                        game,
                        ply,
                        minase::training::records::Origin::RandomSelfplay,
                    )
                    .unwrap(),
                );
            }
        }
        fs::write(&args.data[0], mnsd::independent(header, &samples)).unwrap();
        args
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn initialized(args: &Args, dataset: &Dataset) -> (Model, Observations) {
    let base = base::read(&args.base).unwrap();
    let observations = Observations::compute(dataset, args.feature_set).unwrap();
    let model = Model::initialize(
        Shape::new(args.feature_set, args.width, base.cp_per_logit).unwrap(),
        &crate::nnue::model::load_initial(&args.init_pst).unwrap(),
        Masks::from_observations(args.feature_set, &observations).unwrap(),
        args.seed,
        0.05,
    )
    .unwrap();
    (model, observations)
}

#[test]
fn resume_config_allows_budget_and_worker_changes() {
    let fixture = Fixture::new();
    let mut args = fixture.training_args();
    args.lr_schedule = "cosine:3:0.1".parse().unwrap();

    let dataset = Dataset::open(
        &args.data,
        args.lookahead().unwrap(),
        std::num::NonZeroUsize::new(1).unwrap(),
    )
    .unwrap()
    .with_lambda(args.training.lambda)
    .unwrap();
    let (model, observations) = initialized(&args, &dataset);
    let teacher_cp_per_logit = dataset.estimate_teacher_cp_per_logit().unwrap();
    let configuration = record::configuration(
        &args,
        2,
        &dataset,
        &observations,
        &teacher_cp_per_logit,
        &model,
    )
    .unwrap();
    assert_eq!(
        configuration.document["preprocessing"]["training_positions"],
        296
    );
    assert_eq!(
        configuration.document["preprocessing"]["validation_positions"],
        24
    );
    assert_eq!(
        configuration.document["inputs"]["files"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // MNPT版3の検査和は80バイトのヘッダを除く本体全体のSHA-256。
    assert_eq!(
        configuration.document["inputs"]["base"]["checksum"],
        crate::nn::hex(&sha256(&fs::read(&args.base).unwrap()[80..]))
    );
    assert_eq!(
        configuration.document["inputs"]["init_pst"]["byte_length"],
        fs::metadata(&args.init_pst).unwrap().len()
    );
    let input = &configuration.document["inputs"]["files"][0];
    assert_eq!(
        input["path"],
        args.data[0].canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(
        input["byte_length"],
        fs::metadata(&args.data[0]).unwrap().len()
    );
    assert_eq!(input["header"]["entry_count"], 320);
    assert_eq!(configuration.document["settings"]["lambda"], 1.0);
    assert!(
        configuration.document["settings"]
            .get("validation_interval")
            .is_none()
    );
    assert_eq!(
        configuration.document["arguments"]["validation_interval"],
        0
    );
    for key in ["arguments", "settings"] {
        assert_eq!(
            configuration.document[key]["lr_schedule"],
            json!({"kind": "cosine", "horizon_epochs": 3.0, "floor": 0.1})
        );
    }
    let checkpoint = Checkpoint {
        removal_penalty: args.training.removal_penalty,
        lambda: args.training.lambda,
        optimizer: AdamW::new(model.shape(), AdamWConfig::default()).unwrap(),
        model: model.clone(),
        progress: Progress {
            config: crate::nn::checkpoint::Configuration::from_document(&configuration.document)
                .unwrap(),
            ..initial_progress()
        },
    };
    let restored = Checkpoint::decode(&checkpoint.encode().unwrap()).unwrap();
    restored
        .check_config(
            &crate::nn::checkpoint::Configuration::from_document(&configuration.document).unwrap(),
        )
        .unwrap();
    let mut resume = args.clone();
    resume.output_dir = fixture.0.join("resumed");
    resume.resume = Some(resume.output_dir.clone());
    resume.epochs = 10;
    resume.patience = 2;
    resume.time_limit_hours = 12.0;
    resume.threads = Some(3);
    resume.validation_interval = 2;
    let same = record::configuration(
        &resume,
        3,
        &dataset,
        &observations,
        &teacher_cp_per_logit,
        &model,
    )
    .unwrap();
    restored
        .check_config(&crate::nn::checkpoint::Configuration::from_document(&same.document).unwrap())
        .unwrap();
}

#[test]
fn each_resume_persists_its_time_budget_workers_and_starting_epoch() {
    let fixture = Fixture::new();
    let mut args = fixture.training_args();

    let dataset = Dataset::open(
        &args.data,
        args.lookahead().unwrap(),
        std::num::NonZeroUsize::new(1).unwrap(),
    )
    .unwrap()
    .with_lambda(args.training.lambda)
    .unwrap();
    let (model, observations) = initialized(&args, &dataset);
    let teacher_cp_per_logit = dataset.estimate_teacher_cp_per_logit().unwrap();
    let configuration = record::configuration(
        &args,
        2,
        &dataset,
        &observations,
        &teacher_cp_per_logit,
        &model,
    )
    .unwrap();
    let original = configuration.document;
    assert_eq!(original["resumes"], json!([]));
    let path = fixture.0.join("config.json");
    record::write_json(&path, &original).unwrap();
    for (epoch, epochs, patience, hours, workers, timestamp) in [
        (1, 4, 2, 6.0, 3, 1_700_000_000),
        (3, 8, 4, 12.0, 1, 1_700_000_123),
    ] {
        args.epochs = epochs;
        args.patience = patience;
        args.time_limit_hours = hours;
        args.threads = Some(workers);
        let mut saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        record::append_resume(
            &path,
            &mut saved,
            &args,
            workers,
            epoch,
            UNIX_EPOCH + Duration::from_secs(timestamp),
        )
        .unwrap();
    }
    let mut saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let resumes = saved["resumes"].as_array().unwrap();
    assert_eq!(resumes.len(), 2);
    for (entry, epoch, epochs, patience, hours, workers, timestamp) in [
        (&resumes[0], 1, 4, 2, 6.0, 3, 1_700_000_000_u64),
        (&resumes[1], 3, 8, 4, 12.0, 1, 1_700_000_123_u64),
    ] {
        assert_eq!(entry["resumed_at_unix_seconds"], timestamp);
        assert_eq!(entry["epoch"], epoch);
        assert_eq!(entry["epochs"], epochs);
        assert_eq!(entry["patience"], patience);
        assert_eq!(entry["time_limit_hours"], hours);
        assert_eq!(entry["workers"], workers);
        assert_eq!(entry["validation_interval"], 0);
    }
    saved["resumes"] = json!([]);
    assert_eq!(saved, original);
}

#[test]
fn resume_rejects_each_changed_input_or_setting_before_loading_cuda() {
    for change in [
        "settings",
        "resumes",
        "device",
        "lr_linear",
        "lr_schedule",
        "weight_decay_linear",
        "lr_ft",
        "weight_decay_ft",
        "lr_head",
        "weight_decay_head",
        "removal_penalty",
        "lambda",
        "npz",
        "base",
        "data",
        "origin",
    ] {
        let fixture = Fixture::new();
        let mut args = fixture.training_args();
        {
            let dataset = Dataset::open(
                &args.data,
                args.lookahead().unwrap(),
                std::num::NonZeroUsize::new(1).unwrap(),
            )
            .unwrap()
            .with_lambda(args.training.lambda)
            .unwrap();
            let (model, observations) = initialized(&args, &dataset);
            let teacher_cp_per_logit = dataset.estimate_teacher_cp_per_logit().unwrap();
            let configuration = record::configuration(
                &args,
                2,
                &dataset,
                &observations,
                &teacher_cp_per_logit,
                &model,
            )
            .unwrap();
            fs::create_dir(&args.output_dir).unwrap();
            record::write_json(
                &args.output_dir.join("config.json"),
                &configuration.document,
            )
            .unwrap();
            record::append_epoch(
                &args.output_dir.join("training.jsonl"),
                &epoch_record(&model, 0),
            )
            .unwrap();
            Checkpoint {
                removal_penalty: args.training.removal_penalty,
                lambda: args.training.lambda,
                optimizer: AdamW::new(model.shape(), AdamWConfig::default()).unwrap(),
                model,
                progress: Progress {
                    config: crate::nn::checkpoint::Configuration::from_document(
                        &configuration.document,
                    )
                    .unwrap(),
                    ..initial_progress()
                },
            }
            .write(args.output_dir.join("latest.npz"))
            .unwrap();
        }
        args.resume = Some(args.output_dir.clone());
        if change == "resumes" {
            let path = args.output_dir.join("config.json");
            let mut saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            saved.as_object_mut().unwrap().remove("resumes");
            record::write_json(&path, &saved).unwrap();
            let error = match Session::new(args) {
                Ok(_) => panic!("resume accepted missing resumes"),
                Err(error) => error,
            };
            assert!(
                matches!(
                    error.downcast_ref::<Error>(),
                    Some(Error::Invalid("configuration resumes must be an array"))
                ),
                "{error}"
            );
            continue;
        }
        match change {
            "settings" => args.seed += 1,
            "device" => args.device = crate::nnue::options::DeviceKind::Cpu,
            "lr_linear" => args.training.lr_linear *= 2.0,
            "lr_schedule" => args.lr_schedule = "constant:1".parse().unwrap(),
            "weight_decay_linear" => args.training.weight_decay_linear = 0.001,
            "lr_ft" => args.training.lr_ft *= 2.0,
            "weight_decay_ft" => args.training.weight_decay_ft *= 2.0,
            "lr_head" => args.training.lr_head *= 2.0,
            "weight_decay_head" => args.training.weight_decay_head *= 2.0,
            "removal_penalty" => args.training.removal_penalty *= 2.0,
            "lambda" => args.training.lambda = 0.75,
            "npz" => {
                let replacement = fixture.0.join("other.npz");
                fs::copy(&args.init_pst, &replacement).unwrap();
                args.init_pst = replacement;
            }
            "base" => {
                let mut bytes = fs::read(&args.base).unwrap();
                let last = bytes.len() - 1;
                bytes[last] ^= 1;
                let checksum = sha256(&bytes[80..]);
                bytes[48..80].copy_from_slice(&checksum);
                fs::write(&args.base, bytes).unwrap();
            }
            "data" => {
                let mut bytes = fs::read(&args.data[0]).unwrap();
                bytes[120..128].copy_from_slice(&201_u64.to_le_bytes());
                fs::write(&args.data[0], bytes).unwrap();
            }
            "origin" => {
                let mut bytes = fs::read(&args.data[0]).unwrap();
                for record in bytes[HEADER_LEN..]
                    .as_chunks_mut::<{ GAME_HEADER_LEN + 7 }>()
                    .0
                {
                    record[4] = 1;
                }
                fs::write(&args.data[0], bytes).unwrap();
            }
            _ => unreachable!(),
        }
        let error = match Session::new(args) {
            Ok(_) => panic!("resume accepted changed {change}"),
            Err(error) => error,
        };
        let expected = match change {
            "origin" => MismatchField::Preprocessing,
            "npz" | "base" | "data" => MismatchField::Inputs,
            "settings" => MismatchField::Seed,
            "device" => MismatchField::Device,
            "lr_linear" => MismatchField::LrLinear,
            "lr_schedule" => MismatchField::LrSchedule,
            "weight_decay_linear" => MismatchField::WeightDecayLinear,
            "lr_ft" => MismatchField::LrFt,
            "weight_decay_ft" => MismatchField::WeightDecayFt,
            "lr_head" => MismatchField::LrHead,
            "weight_decay_head" => MismatchField::WeightDecayHead,
            "removal_penalty" => MismatchField::RemovalPenalty,
            "lambda" => MismatchField::Lambda,
            _ => unreachable!(),
        };
        assert!(
            matches!(error.downcast_ref::<crate::nn::Error>(), Some(crate::nn::Error::CheckpointMismatch(field)) if *field == expected),
            "{change} reached an unexpected error: {error}"
        );
    }
}

fn epoch_record(model: &Model, epoch: u64) -> EpochRecord {
    EpochRecord {
        device: crate::nnue::options::DeviceKind::Cuda,
        epoch,
        train_loss: (epoch > 0).then_some(0.5),
        removal_loss: (epoch > 0).then_some(0.1),
        total_loss: (epoch > 0).then_some(0.6),
        validation_loss: 0.4,
        sign_accuracy: Some(0.75),
        samples: 24,
        decisive_samples: 16,
        nonlinear_std_cp: 3.5,
        saturation: Saturation {
            acc_le_zero: 0.25,
            acc_ge_one: 0.125,
        },
        linear_movement: record::linear_movement(model),
        training_seconds: (epoch > 0).then_some(3.0),
        validation_seconds: 0.5,
        interval_validation_seconds: 0.0,
        training_positions_per_second: (epoch > 0).then_some(296.0 / 3.0),
        update_seconds: (epoch > 0).then_some(0.1),
        gpu_memory_used_bytes: Some(1024 * 1024),
        gpu_memory_total_bytes: Some(8 * 1024 * 1024),
        elapsed_seconds: 3.5 * epoch as f64 + 0.5,
        steps: if epoch == 0 { 0 } else { 9 },
        total_steps: 9 * epoch,
        lr_multiplier_first: (epoch > 0).then_some(1.0),
        lr_multiplier_last: (epoch > 0).then_some(0.5),
    }
}

#[test]
fn epoch_log_uses_one_json_line_per_epoch_and_preserves_metrics() {
    let fixture = Fixture::new();
    let args = fixture.training_args();

    let dataset = Dataset::open(
        &args.data,
        args.lookahead().unwrap(),
        std::num::NonZeroUsize::new(1).unwrap(),
    )
    .unwrap()
    .with_lambda(args.training.lambda)
    .unwrap();
    let (model, _) = initialized(&args, &dataset);
    let path = fixture.0.join("training.jsonl");
    for epoch in 0..=1 {
        record::append_epoch(&path, &epoch_record(&model, epoch)).unwrap();
    }
    let bytes = fs::read(&path).unwrap();
    assert_eq!(bytes.iter().filter(|&&byte| byte == b'\n').count(), 2);
    assert_eq!(bytes.last(), Some(&b'\n'));
    let rows = logs(&path);
    assert_eq!(rows[0]["epoch"], 0);
    assert!(rows[0]["train_loss"].is_null());
    assert!(rows[0]["removal_loss"].is_null());
    assert_eq!(rows[0].get("lr_multiplier_first"), Some(&Value::Null));
    assert_eq!(rows[0].get("lr_multiplier_last"), Some(&Value::Null));
    assert_eq!(rows[1]["epoch"], 1);
    for (key, expected) in [
        ("train_loss", 0.5),
        ("removal_loss", 0.1),
        ("total_loss", 0.6),
        ("validation_loss", 0.4),
        ("sign_accuracy", 0.75),
        ("nonlinear_std_cp", 3.5),
        ("training_seconds", 3.0),
        ("validation_seconds", 0.5),
        ("interval_validation_seconds", 0.0),
        ("lr_multiplier_first", 1.0),
        ("lr_multiplier_last", 0.5),
    ] {
        assert_eq!(rows[1][key].as_f64(), Some(expected), "{key}");
    }
    assert_eq!(rows[1]["saturation"]["acc_le_zero"], 0.25);
    assert_eq!(rows[1]["saturation"]["acc_ge_one"], 0.125);
    assert_eq!(
        rows[1]["linear_movement"]["middlegame"]["mean_absolute_cp"],
        0.0
    );
    assert_eq!(
        rows[1]["linear_movement"]["endgame"]["max_absolute_cp"],
        0.0
    );
    assert_eq!(rows[1]["gpu_memory_used_bytes"], 1024 * 1024);
    assert_eq!(rows[1]["steps"], 9);
    assert_eq!(rows[1]["total_steps"], 9);
}

#[test]
fn resume_keeps_completed_log_epochs_and_discards_an_uncommitted_tail() {
    let fixture = Fixture::new();
    let args = fixture.training_args();

    let dataset = Dataset::open(
        &args.data,
        args.lookahead().unwrap(),
        std::num::NonZeroUsize::new(1).unwrap(),
    )
    .unwrap()
    .with_lambda(args.training.lambda)
    .unwrap();
    let (model, _) = initialized(&args, &dataset);
    let path = fixture.0.join("training.jsonl");
    for epoch in 0..=2 {
        record::append_epoch(&path, &epoch_record(&model, epoch)).unwrap();
    }
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend(b"{\"epoch\":3");
    fs::write(&path, bytes).unwrap();
    assert_eq!(record::reconcile_log(&path, 1).unwrap(), 0.4);
    assert_eq!(
        logs(&path)
            .iter()
            .map(|row| row["epoch"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(record::reconcile_log(&path, 2).is_err());
}

fn interval_record(step: u64) -> IntervalRecord {
    IntervalRecord {
        step,
        epoch: 1,
        epoch_fraction: step as f64 / 9.0,
        validation_loss: 0.4,
        sign_accuracy: None,
        samples: 24,
        decisive_samples: 0,
    }
}

#[test]
fn interval_log_reconciliation_keeps_the_checkpoint_step_and_discards_incomplete_tails() {
    let fixture = Fixture::new();
    let path = fixture.0.join("validation-interval.jsonl");
    record::reconcile_interval_log(&path, 4).unwrap();
    assert!(!path.exists());
    for step in [2, 4] {
        record::append_interval(&path, &interval_record(step)).unwrap();
    }
    let kept = fs::read(&path).unwrap();
    let rows = logs(&path);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1]["sign_accuracy"], Value::Null);
    assert_eq!(rows[1]["decisive_samples"], 0);
    assert_eq!(kept.last(), Some(&b'\n'));

    for future in [false, true] {
        for tail in [
            b"{\"step\":8".as_slice(),
            serde_json::to_vec(&interval_record(8)).unwrap().as_slice(),
        ] {
            fs::write(&path, &kept).unwrap();
            if future {
                record::append_interval(&path, &interval_record(6)).unwrap();
            }
            let mut bytes = fs::read(&path).unwrap();
            bytes.extend(tail);
            fs::write(&path, bytes).unwrap();
            record::reconcile_interval_log(&path, 4).unwrap();
            assert_eq!(fs::read(&path).unwrap(), kept);
            record::reconcile_interval_log(&path, 4).unwrap();
            assert_eq!(fs::read(&path).unwrap(), kept);
        }
    }
    record::reconcile_interval_log(&path, 0).unwrap();
    assert!(fs::read(&path).unwrap().is_empty());
}

#[test]
fn interval_log_reconciliation_rejects_non_increasing_steps_without_modifying_the_file() {
    let fixture = Fixture::new();
    let path = fixture.0.join("validation-interval.jsonl");
    // 切り捨てる区間も含めて、重複と逆順を拒否する。
    for steps in [&[4, 2][..], &[2, 2], &[2, 6, 5], &[2, 6, 6]] {
        fs::write(&path, []).unwrap();
        for &step in steps {
            record::append_interval(&path, &interval_record(step)).unwrap();
        }
        let before = fs::read(&path).unwrap();
        assert!(record::reconcile_interval_log(&path, 4).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

fn completed_checkpoint(model: &Model, epoch: u64, best: BestEpoch) -> Checkpoint {
    let mut model = model.clone();
    model.parameter_mut(Parameter::OutputBias)[0] = epoch as f32 / 8.0;
    let mut optimizer = AdamW::new(model.shape(), AdamWConfig::default()).unwrap();
    optimizer.step = 9 * epoch;
    optimizer.m[Parameter::OutputBias][0] = epoch as f32 / 32.0;
    optimizer.v[Parameter::OutputBias][0] = epoch as f32 / 64.0;
    Checkpoint {
        removal_penalty: 1000.0,
        lambda: 1.0,
        model,
        optimizer,
        progress: Progress {
            epoch,
            best: Some(best),
            elapsed_seconds: 10.0 * epoch as f64,
            ..initial_progress()
        },
    }
}

#[test]
fn committed_latest_recovers_an_interrupted_best_checkpoint_save() {
    let fixture = Fixture::new();
    let args = fixture.training_args();

    let dataset = Dataset::open(
        &args.data,
        args.lookahead().unwrap(),
        std::num::NonZeroUsize::new(1).unwrap(),
    )
    .unwrap()
    .with_lambda(args.training.lambda)
    .unwrap();
    let (model, _) = initialized(&args, &dataset);
    let old = completed_checkpoint(
        &model,
        1,
        BestEpoch {
            epoch: 1,
            validation_loss: 0.5,
        },
    );
    let latest = completed_checkpoint(
        &model,
        2,
        BestEpoch {
            epoch: 2,
            validation_loss: 0.4,
        },
    );
    let path = fixture.0.join("best.npz");
    old.write(&path).unwrap();
    recover_best_checkpoint(&fixture.0, &latest).unwrap();
    assert!(fs::read(&path).unwrap() == latest.encode().unwrap());

    fs::remove_file(&path).unwrap();
    recover_best_checkpoint(&fixture.0, &latest).unwrap();
    assert!(fs::read(&path).unwrap() == latest.encode().unwrap());
}

#[test]
fn resume_preserves_an_older_best_and_rejects_its_loss_or_corruption() {
    let fixture = Fixture::new();
    let args = fixture.training_args();

    let dataset = Dataset::open(
        &args.data,
        args.lookahead().unwrap(),
        std::num::NonZeroUsize::new(1).unwrap(),
    )
    .unwrap()
    .with_lambda(args.training.lambda)
    .unwrap();
    let (model, _) = initialized(&args, &dataset);
    let best_epoch = BestEpoch {
        epoch: 1,
        validation_loss: 0.5,
    };
    let best = completed_checkpoint(&model, 1, best_epoch);
    let latest = completed_checkpoint(&model, 2, best_epoch);
    let path = fixture.0.join("best.npz");
    best.write(&path).unwrap();
    let original = fs::read(&path).unwrap();
    recover_best_checkpoint(&fixture.0, &latest).unwrap();
    assert!(fs::read(&path).unwrap() == original);

    fs::remove_file(&path).unwrap();
    assert!(recover_best_checkpoint(&fixture.0, &latest).is_err());
    fs::write(&path, b"truncated checkpoint").unwrap();
    assert!(recover_best_checkpoint(&fixture.0, &latest).is_err());
    let wrong = completed_checkpoint(
        &model,
        1,
        BestEpoch {
            epoch: 1,
            validation_loss: 0.6,
        },
    );
    wrong.write(&path).unwrap();
    assert!(recover_best_checkpoint(&fixture.0, &latest).is_err());
}

fn logs(path: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn compare_checkpoint(left: &Path, right: &Path) {
    let mut left = Checkpoint::read(left).unwrap();
    let mut right = Checkpoint::read(right).unwrap();
    // 実時間だけを除き、重み、AdamWの全状態、進捗、マスク、固定値をビット比較する。
    left.progress.elapsed_seconds = 0.0;
    right.progress.elapsed_seconds = 0.0;
    assert!(
        left.encode().unwrap() == right.encode().unwrap(),
        "checkpoint differs after resume"
    );
}

#[test]
#[ignore = "CUDAが必要。学習環境で明示的に実行する"]
fn gpu_training_two_epochs_match_an_epoch_boundary_resume_for_both_feature_sets() {
    training_two_epochs_match_resume(crate::nnue::options::DeviceKind::Cuda);
}

#[test]
fn cpu_training_two_epochs_match_an_epoch_boundary_resume_for_both_feature_sets() {
    training_two_epochs_match_resume(crate::nnue::options::DeviceKind::Cpu);
}

#[test]
fn cpu_validation_interval_does_not_change_the_trajectory() {
    validation_interval_does_not_change_the_trajectory(crate::nnue::options::DeviceKind::Cpu);
}

#[test]
#[ignore = "CUDAが必要。学習環境で明示的に実行する"]
fn gpu_validation_interval_does_not_change_the_trajectory() {
    validation_interval_does_not_change_the_trajectory(crate::nnue::options::DeviceKind::Cuda);
}

fn validation_interval_does_not_change_the_trajectory(device: crate::nnue::options::DeviceKind) {
    for feature_set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let fixture = Fixture::new();
        let mut args = fixture.training_args();
        args.device = device;
        args.feature_set = feature_set;
        args.lr_schedule = "exponential:1:0.1".parse().unwrap();
        args.training.lambda = 0.75;
        args.training.removal_penalty = 250.0;
        // 途中検証は量子化検査の標本数にかかわらず全24局面を使う。
        args.validation_sample = 1;
        run(args.clone()).unwrap();
        assert!(!args.output_dir.join("validation-interval.jsonl").exists());

        let mut diagnostic = args.clone();
        diagnostic.output_dir = fixture.0.join("interval");
        // 296局面を33件ずつ学習するのでU = 9。2は9の約数ではない。
        diagnostic.validation_interval = 2;
        run(diagnostic.clone()).unwrap();
        assert_eq!(
            fs::read(args.output_dir.join("nnue.bin")).unwrap(),
            fs::read(diagnostic.output_dir.join("nnue.bin")).unwrap()
        );
        for file in ["latest.npz", "best.npz"] {
            compare_checkpoint(
                &args.output_dir.join(file),
                &diagnostic.output_dir.join(file),
            );
        }
        let baseline = logs(&args.output_dir.join("training.jsonl"));
        let measured = logs(&diagnostic.output_dir.join("training.jsonl"));
        assert_eq!(baseline.len(), 3);
        assert_eq!(measured.len(), baseline.len());
        for (epoch, (left, right)) in baseline.iter().zip(&measured).enumerate() {
            for key in [
                "train_loss",
                "removal_loss",
                "total_loss",
                "validation_loss",
            ] {
                assert_eq!(
                    left[key].as_f64().map(f64::to_bits),
                    right[key].as_f64().map(f64::to_bits),
                    "{feature_set:?}: {key} at epoch {epoch}"
                );
            }
            for key in [
                "steps",
                "total_steps",
                "lr_multiplier_first",
                "lr_multiplier_last",
            ] {
                assert_eq!(left[key], right[key], "{feature_set:?}: {key}");
            }
            assert_eq!(left["interval_validation_seconds"], 0.0);
            if epoch == 0 {
                assert_eq!(right["interval_validation_seconds"], 0.0);
            }
        }
        let rows = logs(&diagnostic.output_dir.join("validation-interval.jsonl"));
        let expected = [
            (2, 1, 2.0 / 9.0),
            (4, 1, 4.0 / 9.0),
            (6, 1, 6.0 / 9.0),
            (8, 1, 8.0 / 9.0),
            (10, 2, 1.0 / 9.0),
            (12, 2, 3.0 / 9.0),
            (14, 2, 5.0 / 9.0),
            (16, 2, 7.0 / 9.0),
        ];
        assert_eq!(rows.len(), expected.len());
        let mut stopped_args = args.clone();
        stopped_args.output_dir = fixture.0.join("stopped-at-update");
        let mut stopped = Session::new(stopped_args).unwrap();
        let (mut queue, mut buffers) = stopped.backend.queue(feature_set, args.batch).unwrap();
        let mut batches = (1..=2)
            .flat_map(|epoch| {
                epoch_permutation(stopped.dataset.training_indices(), args.seed, epoch)
                    .chunks(args.batch)
                    .map(<[u64]>::to_vec)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
            .into_iter();
        for (row, (step, epoch, fraction)) in rows.iter().zip(expected) {
            assert_eq!(row["step"], step);
            assert_eq!(row["epoch"], epoch);
            assert_eq!(row["epoch_fraction"].as_f64().unwrap(), fraction);
            // 記録とは独立に、指定更新まで進めて停止した重みを全検証集合で評価する。
            while stopped.backend.step() < step {
                let indices = batches.next().unwrap();
                let batch = build_batches(&stopped.dataset, feature_set, &[&indices])
                    .unwrap()
                    .remove(0);
                let targets = build_targets(
                    &batch,
                    &stopped.teacher_cp_per_logit,
                    stopped.dataset.teacher_lambdas(),
                )
                .unwrap();
                buffers[0].fill(batch).unwrap();
                let multiplier = args.lr_schedule.multiplier(stopped.backend.step(), 9);
                stopped
                    .backend
                    .enqueue(&mut queue, &mut buffers[0], &targets, multiplier)
                    .unwrap()
                    .collect()
                    .unwrap();
            }
            let (parameters, _) = stopped.backend.snapshot().unwrap();
            let mut model = stopped.checkpoint.model.clone();
            for parameter in Parameter::ALL {
                model
                    .parameter_mut(parameter)
                    .copy_from_slice(&parameters[parameter]);
            }
            let validation = stopped.validate(&model).unwrap().unwrap();
            assert_eq!(
                row["validation_loss"],
                json!(validation.loss()),
                "{feature_set:?}: step={step}"
            );
            assert_eq!(
                row["sign_accuracy"],
                json!(validation.sign_accuracy()),
                "{feature_set:?}: step={step}"
            );
            assert_eq!(row["samples"], 24);
            assert_eq!(row["decisive_samples"], 16);
        }
    }
}

#[test]
fn resume_truncates_interval_records_after_the_checkpoint() {
    let fixture = Fixture::new();
    let mut args = fixture.training_args();
    args.device = crate::nnue::options::DeviceKind::Cpu;
    args.lr_schedule = "exponential:1:0.1".parse().unwrap();
    args.validation_interval = 2;

    let mut split = args.clone();
    split.output_dir = fixture.0.join("resumed");
    split.epochs = 1;
    run(split.clone()).unwrap();
    let checkpoint = Checkpoint::read(split.output_dir.join("latest.npz")).unwrap();
    assert_eq!(checkpoint.optimizer.step(), 9);
    let path = split.output_dir.join("validation-interval.jsonl");
    let kept = fs::read(&path).unwrap();
    record::append_interval(
        &path,
        &IntervalRecord {
            step: 20,
            epoch: 3,
            epoch_fraction: 2.0 / 9.0,
            ..interval_record(20)
        },
    )
    .unwrap();
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend(b"{\"step\":22");
    fs::write(&path, bytes).unwrap();

    split.resume = Some(split.output_dir.clone());
    split.epochs = 2;
    split.threads = Some(5);
    let _session = Session::new(split.clone()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), kept);
}

#[test]
fn resume_allows_a_changed_validation_interval() {
    for (first, second, expected_steps) in [
        (0, 2, vec![10, 12, 14, 16]),
        (2, 0, vec![2, 4, 6, 8]),
        (2, 4, vec![2, 4, 6, 8, 12, 16]),
    ] {
        let fixture = Fixture::new();
        let mut args = fixture.training_args();
        args.device = crate::nnue::options::DeviceKind::Cpu;
        args.epochs = 1;
        args.validation_interval = first;
        run(args.clone()).unwrap();
        args.epochs = 2;
        args.resume = Some(args.output_dir.clone());
        args.validation_interval = second;
        run(args.clone()).unwrap();
        let config: Value =
            serde_json::from_slice(&fs::read(args.output_dir.join("config.json")).unwrap())
                .unwrap();
        assert!(config["settings"].get("validation_interval").is_none());
        assert_eq!(config["arguments"]["validation_interval"], first);
        assert_eq!(config["resumes"].as_array().unwrap().len(), 1);
        assert_eq!(config["resumes"][0]["validation_interval"], second);
        assert_eq!(config["resumes"][0]["epoch"], 1);
        let rows = logs(&args.output_dir.join("validation-interval.jsonl"));
        assert_eq!(
            rows.iter()
                .map(|row| row["step"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            expected_steps
        );
    }
}

/// 320局面の合成MNSDで両特徴集合を2エポック学習し、連続実行とエポック境界での再開を比べる。
/// 累積秒数を除いたチェックポイントのバイト列と整数ネットが一致することを検査する。
/// CUDA版の呼び出し元には`#[ignore]`を付ける。
fn training_two_epochs_match_resume(device: crate::nnue::options::DeviceKind) {
    for feature_set in [
        crate::nnue::data::InputFeatureSet::P,
        crate::nnue::data::InputFeatureSet::HalfKpHmB8,
    ] {
        let fixture = Fixture::new();
        let mut args = fixture.training_args();
        args.feature_set = feature_set;
        args.device = device;
        args.training.lambda = 0.75;
        args.training.removal_penalty = 250.0;
        args.training.lr_linear = 0.012;
        args.training.weight_decay_linear = 1e-5;
        args.training.lr_ft = 0.002;
        args.training.weight_decay_ft = 0.0003;
        args.training.lr_head = 0.0015;
        args.training.weight_decay_head = 0.0002;
        run(args.clone()).unwrap();

        let mut split = args.clone();
        split.output_dir = fixture.0.join("resumed");
        let mut session = Session::new(split.clone()).unwrap();
        assert!(session.advance().unwrap());
        drop(session);
        let first = Checkpoint::read(split.output_dir.join("latest.npz")).unwrap();
        assert_eq!(first.progress.epoch, 1);
        assert_eq!(first.optimizer.step(), 9);
        split.resume = Some(split.output_dir.clone());
        split.threads = Some(5);
        run(split.clone()).unwrap();
        let configuration: Value =
            serde_json::from_slice(&fs::read(split.output_dir.join("config.json")).unwrap())
                .unwrap();
        assert_eq!(configuration["resumes"].as_array().unwrap().len(), 1);
        assert_eq!(configuration["resumes"][0]["epoch"], 1);
        assert_eq!(configuration["resumes"][0]["epochs"], split.epochs);
        assert_eq!(configuration["resumes"][0]["workers"], 5);
        assert_eq!(
            configuration["device"],
            serde_json::to_value(device).unwrap()
        );
        assert_eq!(configuration["settings"]["lambda"], 0.75);
        assert_eq!(configuration["settings"]["removal_penalty"], 250.0);

        for file in ["latest.npz", "best.npz"] {
            compare_checkpoint(&args.output_dir.join(file), &split.output_dir.join(file));
        }
        assert_eq!(
            fs::read(args.output_dir.join("nnue.bin")).unwrap(),
            fs::read(split.output_dir.join("nnue.bin")).unwrap()
        );
        let latest = Checkpoint::read(split.output_dir.join("latest.npz")).unwrap();
        assert_eq!(latest.progress.epoch, 2);
        assert_eq!(latest.optimizer.step(), 18);
        assert!(latest.progress.best.unwrap().epoch >= 1);
        let continuous = logs(&args.output_dir.join("training.jsonl"));
        let resumed = logs(&split.output_dir.join("training.jsonl"));
        assert_eq!(
            continuous
                .iter()
                .map(|row| row["epoch"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(resumed.len(), continuous.len());
        for (left, right) in continuous.iter().zip(&resumed) {
            for row in [left, right] {
                assert_eq!(row["device"], serde_json::to_value(device).unwrap());
                if device == crate::nnue::options::DeviceKind::Cpu {
                    assert!(row["gpu_memory_used_bytes"].is_null());
                    assert!(row["gpu_memory_total_bytes"].is_null());
                }
            }
            for key in [
                "epoch",
                "train_loss",
                "removal_loss",
                "total_loss",
                "validation_loss",
                "steps",
                "total_steps",
                "lr_multiplier_first",
                "lr_multiplier_last",
            ] {
                assert_eq!(left[key], right[key], "{feature_set:?}: {key}");
                if key.ends_with("loss") {
                    assert_eq!(
                        left[key].as_f64().map(f64::to_bits),
                        right[key].as_f64().map(f64::to_bits),
                        "{feature_set:?}: {key}"
                    );
                }
            }
            assert!(left["validation_loss"].as_f64().unwrap().is_finite());
            if left["epoch"] != 0 {
                for key in ["train_loss", "removal_loss", "total_loss"] {
                    assert!(
                        left[key].as_f64().unwrap().is_finite(),
                        "{feature_set:?}: {key}"
                    );
                }
            }
        }
    }
}

#[test]
fn cpu_exponential_schedule_records_multipliers_and_preserves_base_coefficients() {
    let fixture = Fixture::new();
    let mut args = fixture.training_args();
    args.device = crate::nnue::options::DeviceKind::Cpu;
    args.lr_schedule = "exponential:1:0.1".parse().unwrap();
    let mut session = Session::new(args.clone()).unwrap();
    // 訓練296局面を33件ずつ処理するため、U = 9。
    for epoch in 1..=2 {
        assert!(session.advance().unwrap());
        let latest = Checkpoint::read(args.output_dir.join("latest.npz")).unwrap();
        assert_eq!(latest.progress.epoch, epoch);
        assert_eq!(latest.optimizer.step(), 9 * epoch);
        assert_eq!(latest.optimizer.config(), args.training.optimizer());
    }
    let rows = logs(&args.output_dir.join("training.jsonl"));
    let expected = [
        (None, None),
        (Some(1.0), Some(2.0_f64.powf(-8.0 / 9.0))),
        (Some(0.5), Some(2.0_f64.powf(-17.0 / 9.0))),
    ];
    assert_eq!(rows.len(), expected.len());
    for (epoch, (row, (first, last))) in rows.iter().zip(expected).enumerate() {
        assert_eq!(row["epoch"], epoch);
        for (key, expected) in [("lr_multiplier_first", first), ("lr_multiplier_last", last)] {
            let value = row.get(key).unwrap();
            match expected {
                Some(expected) => assert!(
                    (value.as_f64().unwrap() - expected).abs() < 1e-15,
                    "epoch={epoch}: {key}"
                ),
                None => assert!(value.is_null(), "epoch={epoch}: {key}"),
            }
        }
    }
}

fn parse_training_args(extra: &[&str]) -> std::result::Result<Args, clap::Error> {
    use clap::Parser;
    #[derive(Parser)]
    struct Command {
        #[command(flatten)]
        args: Args,
    }
    Command::try_parse_from(
        [
            "train",
            "--feature-set",
            "P",
            "--width",
            "8",
            "--lambda",
            "1",
            "--data",
            "missing.bin",
            "--init-pst",
            "missing.npz",
            "--base",
            "missing.mnpt",
            "--output-dir",
            "missing-output",
        ]
        .into_iter()
        .chain(extra.iter().copied()),
    )
    .map(|command| command.args)
}

#[test]
fn cosine_schedule_is_the_clap_default() {
    let default = parse_training_args(&[]).unwrap();
    let explicit = parse_training_args(&["--lr-schedule", "cosine:3:0.1"]).unwrap();
    assert_eq!(
        default.lr_schedule,
        LrSchedule::Cosine {
            horizon_epochs: 3.0,
            floor: 0.1,
        }
    );
    assert_eq!(explicit.lr_schedule, default.lr_schedule);
}

#[test]
fn constant_multiplier_matches_training_with_scaled_base_learning_rates() {
    let fixture = Fixture::new();
    let mut scheduled = fixture.training_args();
    scheduled.device = crate::nnue::options::DeviceKind::Cpu;
    scheduled.lr_schedule = "constant:0.25".parse().unwrap();
    scheduled.training.weight_decay_linear = 0.001;
    run(scheduled.clone()).unwrap();

    let mut scaled = scheduled.clone();
    scaled.output_dir = fixture.0.join("scaled");
    scaled.lr_schedule = "constant:1".parse().unwrap();
    scaled.training.lr_linear *= 0.25;
    scaled.training.lr_ft *= 0.25;
    scaled.training.lr_head *= 0.25;
    run(scaled.clone()).unwrap();

    let left = Checkpoint::read(scheduled.output_dir.join("latest.npz")).unwrap();
    let right = Checkpoint::read(scaled.output_dir.join("latest.npz")).unwrap();
    assert_eq!(left.optimizer.config(), scheduled.training.optimizer());
    assert_eq!(right.optimizer.config(), scaled.training.optimizer());
    assert_eq!(left.optimizer.step(), right.optimizer.step());
    for p in Parameter::ALL {
        for (a, b) in [
            (left.model.parameters(), right.model.parameters()),
            (left.optimizer.m(), right.optimizer.m()),
            (left.optimizer.v(), right.optimizer.v()),
        ] {
            assert!(
                a[p].iter()
                    .zip(&b[p])
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{p:?}"
            );
        }
    }
    assert_eq!(
        fs::read(scheduled.output_dir.join("nnue.bin")).unwrap(),
        fs::read(scaled.output_dir.join("nnue.bin")).unwrap()
    );
}

#[test]
fn arguments_reject_invalid_lr_schedules_before_accessing_files() {
    let fixture = Fixture::new();
    let files = args();
    for (schedule, reason) in [
        (
            LrSchedule::Constant { multiplier: 0.0 },
            "learning-rate multiplier must be finite and positive",
        ),
        (
            LrSchedule::Exponential {
                half_life_epochs: f64::NAN,
                floor: 0.1,
            },
            "learning-rate schedule duration must be finite and positive",
        ),
        (
            LrSchedule::Cosine {
                horizon_epochs: 3.0,
                floor: 1.1,
            },
            "learning-rate schedule floor must be finite and in (0, 1]",
        ),
    ] {
        let mut args = files.clone();
        args.data = vec![fixture.0.join("missing-data.bin")];
        args.base = fixture.0.join("missing-base.bin");
        args.init_pst = fixture.0.join("missing-pst.npz");
        for path in [&args.data[0], &args.base, &args.init_pst] {
            assert!(!path.exists());
        }
        args.lr_schedule = schedule;
        let error = match Session::new(args) {
            Ok(_) => panic!("accepted an invalid learning-rate schedule: {schedule}"),
            Err(error) => error,
        };
        assert!(
            matches!(error.downcast_ref::<crate::nnue::model::Error>(),
            Some(crate::nnue::model::Error::Invalid(message)) if *message == reason),
            "{schedule}: {error}"
        );
    }
}

#[test]
fn cpu_result_only_training_records_null_teacher_cp_per_logit_and_resumes() {
    let fixture = Fixture::new();
    let mut args = fixture.training_args();
    args.device = crate::nnue::options::DeviceKind::Cpu;
    args.width = 2;
    args.batch = 128;
    args.epochs = 1;
    args.training.lambda = 0.0;
    run(args.clone()).unwrap();
    let config: Value =
        serde_json::from_slice(&fs::read(args.output_dir.join("config.json")).unwrap()).unwrap();
    assert_eq!(
        config["preprocessing"]["teacher_cp_per_logit"],
        json!([null])
    );
    assert_eq!(config["preprocessing"]["teacher_lambdas"], json!([0.0]));
    args.resume = Some(args.output_dir.clone());
    args.epochs = 2;
    run(args.clone()).unwrap();
    let checkpoint = Checkpoint::read(args.output_dir.join("latest.npz")).unwrap();
    assert_eq!(checkpoint.progress.epoch, 2);
    assert_eq!(checkpoint.lambda, 0.0);
}

#[test]
fn resume_requires_an_existing_array_without_changing_the_configuration() {
    let fixture = Fixture::new();
    let args = fixture.training_args();
    let path = fixture.0.join("config.json");
    for mut config in [json!({}), json!({"resumes": null}), json!({"resumes": {}})] {
        let original = config.clone();
        record::write_json(&path, &config).unwrap();
        let bytes = fs::read(&path).unwrap();
        let error = record::append_resume(&path, &mut config, &args, 1, 1, UNIX_EPOCH).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<Error>(),
                Some(Error::Invalid("configuration resumes must be an array"))
            ),
            "{error}"
        );
        assert_eq!(config, original);
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn cpu_resume_rejects_changed_checkpoint_meta_fields() {
    let fixture = Fixture::new();
    let mut args = fixture.training_args();
    args.device = crate::nnue::options::DeviceKind::Cpu;
    let session = Session::new(args.clone()).unwrap();
    drop(session);
    args.resume = Some(args.output_dir.clone());
    let path = args.output_dir.join("latest.npz");
    let original = fs::read(&path).unwrap();
    let arrays = crate::nn::npz::decode(&original).unwrap();
    for (field, expected) in [
        ("settings", MismatchField::Settings),
        ("inputs", MismatchField::Inputs),
        ("preprocessing", MismatchField::Preprocessing),
    ] {
        let mut changed = arrays.clone();
        let meta = changed.get_mut("meta").unwrap();
        let mut value: Value = serde_json::from_slice(&meta.data).unwrap();
        value["config"][field] = serde_json::json!({"changed": true});
        meta.data = serde_json::to_vec(&value).unwrap();
        meta.shape = vec![meta.data.len()];
        let bytes = crate::nn::npz::encode(
            &changed
                .iter()
                .map(|(k, v)| (k.as_str(), v.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        fs::write(&path, &bytes).unwrap();
        let error = match Session::new(args.clone()) {
            Ok(_) => panic!("accepted changed {field}"),
            Err(error) => error,
        };
        assert!(
            matches!(error.downcast_ref::<crate::nnue::model::Error>(), Some(crate::nnue::model::Error::CheckpointMismatch(field)) if *field == expected),
            "{field}: {error}"
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn interval_log_rejects_malformed_complete_json_without_modifying_the_file() {
    let fixture = Fixture::new();
    let path = fixture.0.join("validation-interval.jsonl");
    for checkpoint_step in [0, 4] {
        fs::write(&path, []).unwrap();
        record::append_interval(&path, &interval_record(2)).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(b"{invalid json}\n");
        fs::write(&path, &bytes).unwrap();
        let error = record::reconcile_interval_log(&path, checkpoint_step).unwrap_err();
        assert!(
            error.downcast_ref::<serde_json::Error>().is_some(),
            "step={checkpoint_step}: {error}"
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn clap_accepts_validation_intervals_and_rejects_invalid_numbers() {
    for (text, expected) in [
        ("0", 0),
        ("1", 1),
        ("2", 2),
        ("18446744073709551615", u64::MAX),
    ] {
        let parsed = parse_training_args(&[&format!("--validation-interval={text}")]).unwrap();
        assert_eq!(parsed.validation_interval, expected, "{text}");
    }
    for text in ["-1", "abc", "18446744073709551616"] {
        let error = parse_training_args(&[&format!("--validation-interval={text}")]).unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::ValueValidation,
            "{text}: {error}"
        );
    }
    for text in ["constant:１", "定数:1", "constant:1e400"] {
        let error = parse_training_args(&[&format!("--lr-schedule={text}")]).unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::ValueValidation,
            "{text}: {error}"
        );
    }
}

#[test]
fn constant_one_records_unit_multipliers_for_every_update() {
    let fixture = Fixture::new();
    let mut args = fixture.training_args();
    args.device = crate::nnue::options::DeviceKind::Cpu;
    args.lr_schedule = "constant:1".parse().unwrap();
    // 1エポックを1更新にして、記録の両端で全更新を検査する。
    args.batch = 296;
    run(args.clone()).unwrap();
    let rows = logs(&args.output_dir.join("training.jsonl"));
    assert_eq!(rows.len(), 3);
    for (epoch, row) in rows.iter().enumerate() {
        assert_eq!(row["total_steps"], epoch);
        for key in ["lr_multiplier_first", "lr_multiplier_last"] {
            assert_eq!(
                row[key],
                if epoch == 0 { Value::Null } else { json!(1.0) },
                "epoch={epoch}: {key}"
            );
        }
    }
}

#[test]
fn interrupted_interval_validation_discards_the_epoch_without_appending_logs() {
    let fixture = Fixture::new();
    let mut args = fixture.training_args();
    args.device = crate::nnue::options::DeviceKind::Cpu;
    args.validation_interval = 2;
    let mut session = Session::new(args.clone()).unwrap();
    let checkpoint = fs::read(args.output_dir.join("latest.npz")).unwrap();
    let epoch_log = fs::read(session.log_path()).unwrap();
    let interval_path = args.output_dir.join("validation-interval.jsonl");
    let indices = epoch_permutation(session.dataset.training_indices(), args.seed, 1);
    let mut interrupted_at = None;
    let training = pipeline::train_epoch(
        pipeline::Supply {
            dataset: &session.dataset,
            feature_set: args.feature_set,
            indices: &indices,
            validation_indices: session.dataset.validation_indices(),
            teacher_cp_per_logit: &session.teacher_cp_per_logit,
            batch: args.batch,
            removal_penalty: args.training.removal_penalty,
            validation_interval: args.validation_interval,
            schedule: args.lr_schedule,
            steps_per_epoch: 9,
            workers: session.workers,
            deadline: session.deadline(),
        },
        &mut session.backend,
        |_, backend| {
            interrupted_at = Some(backend.step());
            Ok(false)
        },
    )
    .unwrap();
    assert_eq!(interrupted_at, Some(2));
    assert!(training.is_none());
    assert_eq!(session.backend.step(), 2);
    assert_eq!(session.checkpoint.progress.epoch, 0);
    assert_eq!(session.checkpoint.progress.best, None);
    assert_eq!(
        fs::read(args.output_dir.join("latest.npz")).unwrap(),
        checkpoint
    );
    assert_eq!(fs::read(session.log_path()).unwrap(), epoch_log);
    assert!(!interval_path.exists());
}

#[test]
fn validation_set_returns_none_when_the_deadline_has_already_expired() {
    let fixture = Fixture::new();
    let mut args = fixture.training_args();
    args.device = crate::nnue::options::DeviceKind::Cpu;
    let mut session = Session::new(args).unwrap();
    let validation = ValidationSet {
        dataset: &session.dataset,
        feature_set: session.args.feature_set,
        indices: session.dataset.validation_indices(),
        teacher_cp_per_logit: &session.teacher_cp_per_logit,
        batch: session.args.batch,
        removal_penalty: session.args.training.removal_penalty,
        deadline: Instant::now(),
    }
    .evaluate(
        &session.backend,
        &session.checkpoint.model,
        &mut session.gpu_used_bytes,
        &mut session.gpu_total_bytes,
    )
    .unwrap();
    assert!(validation.is_none());
    assert!(
        !session
            .args
            .output_dir
            .join("validation-interval.jsonl")
            .exists()
    );
}

#[test]
fn training_timing_excludes_interval_validation() {
    // 296局面、経過10秒、途中検証2秒なら学習8秒、296/8 = 37局面/秒。
    let timing = record::training_timing(296, 10.0, 2.0);
    assert!((timing.training_seconds - 8.0).abs() < 1e-12);
    assert!((timing.interval_validation_seconds - 2.0).abs() < 1e-12);
    assert!((timing.training_positions_per_second - 37.0).abs() < 1e-12);
}
