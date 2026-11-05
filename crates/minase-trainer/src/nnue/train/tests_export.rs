use super::*;
use crate::nnue::{model::quantize, options::DeviceKind};
use minase::eval::{
    features::PIECE_STATE_COUNT,
    nnue::{
        EncodeInput, encode,
        feature_set::P,
        head::{Head, PairwiseL16L32},
    },
};

fn export_args(fixture: &Fixture, buckets: u32) -> Args {
    let mut args = fixture.training_args();
    args.device = DeviceKind::Cpu;
    args.width = 4;
    args.batch = 8;
    args.threads = Some(1);
    args.training.lambda = 0.0;
    args.training.lr_linear = 1.0;
    args.lr_schedule = "constant:1".parse().unwrap();
    args.lookahead_gamma = None;
    args.lookahead_plies = None;
    args.buckets = buckets;
    if buckets > 1 {
        args.bucket_positions = Some(40);
        args.scratch_dir = Some(fixture.0.clone());
    }

    write_base(&args.base, 320.0);

    let mut position = PositionBuilder::new(Color::Black);
    for (file, rank, color, kind) in [
        (4, 0, Color::Black, PieceKind::King),
        (7, 11, Color::White, PieceKind::King),
        (3, 5, Color::White, PieceKind::Pawn),
    ] {
        position
            .put(
                Square::new(file, rank).unwrap(),
                PieceCode::new(color, kind).unwrap(),
            )
            .unwrap();
    }
    let position = position.finish().unwrap();
    // 同じ局面を、訓練では勝ち、検証では負けとする。
    // 勝ちに近づく更新ほど検証損失は悪化するため、最良と最後を区別できる。
    let samples: Vec<_> = (0..20)
        .map(|game| {
            Sample::from_position(
                &position,
                0,
                if [4, 15].contains(&game) {
                    Outcome::Loss
                } else {
                    Outcome::Win
                },
                game,
                0,
                minase::training::records::Origin::RandomSelfplay,
            )
            .unwrap()
        })
        .collect();
    let header = Header::new(
        "L0,P0,R1,E0".into(),
        "1".repeat(40),
        [2; 32],
        minase::training::records::TeacherKind::AlphaBeta,
        200,
        11,
    )
    .unwrap();
    fs::write(&args.data[0], mnsd::independent(header, &samples)).unwrap();
    args
}

fn write_base(path: &Path, cp_per_logit: f32) {
    // 基準ネットも合成し、埋め込みのネットや学習済みの駒価値に依存させない。
    let mut values = [40; PIECE_STATE_COUNT];
    values[PieceKind::King.index()] = 80;
    values[PieceKind::CrownPrince.index()] = 80;
    let head = PairwiseL16L32::decode(&vec![0; PairwiseL16L32::byte_len(4)], 4, true).unwrap();
    fs::write(
        path,
        encode::<P, PairwiseL16L32>(&EncodeInput {
            width: 4,
            cp_per_logit,
            rule_set: "L0,P0,R1,E0",
            piece_values: &values,
            linear: &vec![[0; 2]; FEATURE_COUNT],
            weights: &vec![0; FEATURE_COUNT * 4],
            biases: &[0; 4],
            head: &head,
        })
        .unwrap(),
    )
    .unwrap();
}

fn source(session: &Session) -> export::ValidationSource<'_> {
    match &session.dataset {
        Store::Memory(dataset) => dataset.into(),
        Store::Spill(store) => store.into(),
    }
}

fn network_bytes(session: &Session, checkpoint: &Checkpoint) -> Vec<u8> {
    quantize(&checkpoint.model)
        .unwrap()
        .encode(&session.base.piece_values, &session.base.rule_set)
        .unwrap()
}

fn train_worsening_epochs(args: &Args) -> Session {
    let mut session = Session::new(args.clone()).unwrap();
    assert!(session.advance().unwrap());
    let best = fs::read(args.output_dir.join("best.npz")).unwrap();
    assert!(session.advance().unwrap());
    let records = logs(&session.log_path());
    assert!(
        records[2]["validation_loss"].as_f64().unwrap()
            > records[1]["validation_loss"].as_f64().unwrap()
    );
    assert_eq!(session.checkpoint.progress.epoch, 2);
    assert_eq!(session.checkpoint.progress.best.unwrap().epoch, 1);
    assert_eq!(fs::read(args.output_dir.join("best.npz")).unwrap(), best);
    session
}

fn check_export(export_epoch: ExportEpoch, expected_epoch: u64, file: &str) {
    for buckets in [1, 2] {
        let fixture = Fixture::new();
        let mut args = export_args(&fixture, buckets);
        args.export_epoch = export_epoch;
        let session = train_worsening_epochs(&args);
        let best_bytes = fs::read(args.output_dir.join("best.npz")).unwrap();
        let best = Checkpoint::decode(&best_bytes).unwrap();
        let selected = Checkpoint::read(args.output_dir.join(file)).unwrap();
        let expected = network_bytes(&session, &selected);
        let latest = Checkpoint::read(args.output_dir.join("latest.npz")).unwrap();
        assert_ne!(
            sha256(&network_bytes(&session, &best)),
            sha256(&network_bytes(&session, &latest))
        );
        // 診断も、選択したチェックポイントを直接渡した結果と一致する必要がある。
        let (quantization, removal_sign) = export::write_completed(
            source(&session),
            &selected,
            &fixture.0.join("expected"),
            &session.base.piece_values,
            &session.base.rule_set,
            args.validation_sample,
        )
        .unwrap();
        let summary = session.finish().unwrap();
        assert_eq!(summary.exported_epoch, Some(expected_epoch));
        assert_eq!(summary.best_epoch, Some(1));
        assert_eq!(
            summary.best_validation_loss,
            Some(best.progress.best.unwrap().validation_loss)
        );
        assert_eq!(summary.last_epoch, 2);
        assert_eq!(summary.quantization, Some(quantization));
        assert_eq!(
            serde_json::to_value(summary.removal_sign).unwrap(),
            json!(removal_sign)
        );
        assert!(fs::read(args.output_dir.join("nnue.bin")).unwrap() == expected);
        assert_eq!(
            fs::read(args.output_dir.join("best.npz")).unwrap(),
            best_bytes
        );
        let saved: Value =
            serde_json::from_slice(&fs::read(args.output_dir.join("summary.json")).unwrap())
                .unwrap();
        assert_eq!(saved["export_epoch"], json!(export_epoch));
        assert_eq!(saved["exported_epoch"], expected_epoch);
        assert_eq!(saved["best_epoch"], 1);
    }
}

#[test]
fn default_best_exports_the_best_model_after_validation_worsens() {
    let default = parse_training_args(&[]).unwrap();
    assert_eq!(default.export_epoch, ExportEpoch::Best);
    check_export(default.export_epoch, 1, "best.npz");
}

#[test]
fn last_exports_the_last_model_and_preserves_the_best_checkpoint() {
    check_export(ExportEpoch::Last, 2, "latest.npz");
}

#[test]
fn last_export_keeps_the_network_and_epoch_when_quantization_fails() {
    for buckets in [1, 2] {
        let fixture = Fixture::new();
        let mut args = export_args(&fixture, buckets);
        args.export_epoch = ExportEpoch::Last;
        write_base(&args.base, 1.0);
        let mut session = train_worsening_epochs(&args);
        let best = fs::read(args.output_dir.join("best.npz")).unwrap();
        // 3駒の線形寄与を各1/16 cpに固定する。浮動小数点評価は3/16 cp、
        // 整数評価は0 cpで、誤差は勝率尺度1 cpに対する閾値1/40 cpを超える。
        for parameter in Parameter::ALL {
            session.checkpoint.model.parameter_mut(parameter).fill(0.0);
        }
        session
            .checkpoint
            .model
            .parameter_mut(Parameter::Linear)
            .fill(1.0 / 16.0);
        session.checkpoint.model.constrain().unwrap();
        session.save_latest().unwrap();
        let expected = network_bytes(&session, &session.checkpoint);
        let error = session.finish().unwrap_err();
        assert!(matches!(
            error.downcast_ref::<Error>(),
            Some(Error::QuantizationError)
        ));
        let summary: Value =
            serde_json::from_slice(&fs::read(args.output_dir.join("summary.json")).unwrap())
                .unwrap();
        assert_eq!(summary["status"], "quantization_error");
        assert_eq!(summary["export_epoch"], "last");
        assert_eq!(summary["exported_epoch"], 2);
        assert_eq!(summary["best_epoch"], 1);
        assert_eq!(
            summary["quantization"]["mean_absolute_error_cp"],
            3.0 / 16.0
        );
        assert_eq!(summary["quantization"]["limit_cp"], 1.0 / 40.0);
        assert_eq!(summary["quantization"]["passed"], false);
        assert!(summary["removal_sign"].is_object());
        assert!(fs::read(args.output_dir.join("nnue.bin")).unwrap() == expected);
        assert_eq!(fs::read(args.output_dir.join("best.npz")).unwrap(), best);
    }
}

#[test]
fn export_epoch_arguments_accept_only_best_and_last() {
    for (name, expected) in [("best", ExportEpoch::Best), ("last", ExportEpoch::Last)] {
        assert_eq!(
            parse_training_args(&["--export-epoch", name])
                .unwrap()
                .export_epoch,
            expected
        );
    }
    for invalid in ["", "latest", "0", "Best"] {
        assert_eq!(
            parse_training_args(&["--export-epoch", invalid])
                .unwrap_err()
                .kind(),
            clap::error::ErrorKind::InvalidValue
        );
    }
}

fn interrupt_next_epoch(session: &mut Session) {
    let dataset = Dataset::open(
        &session.args.data,
        None,
        std::num::NonZeroUsize::new(1).unwrap(),
    )
    .unwrap()
    .with_lambda(session.args.training.lambda)
    .unwrap();
    let indices = epoch_permutation(
        dataset.training_indices(),
        session.args.seed,
        session.checkpoint.progress.epoch + 1,
    );
    // 1更新後に途中検証を打ち切り、バックエンドだけが保存済みモデルより先へ進んだ状態を作る。
    let training = pipeline::train_epoch(
        pipeline::Supply {
            dataset: &dataset,
            feature_set: session.args.feature_set,
            indices: &indices,
            teacher_cp_per_logit: &session.teacher_cp_per_logit,
            batch: session.args.batch,
            validation_interval: 1,
            schedule: session.args.lr_schedule,
            steps_per_epoch: indices.len().div_ceil(session.args.batch) as u64,
            workers: session.workers,
            deadline: session.deadline(),
        },
        &mut session.backend,
        |_, _| Ok(false),
    )
    .unwrap();
    assert!(training.is_none());
    assert_eq!(
        session.backend.step(),
        session.checkpoint.optimizer.step() + 1
    );
    let (parameters, _) = session.backend.snapshot().unwrap();
    assert_ne!(&parameters, session.checkpoint.model.parameters());
    session.previous_elapsed = session.args.time_limit_hours * 3600.0;
    assert!(!session.advance().unwrap());
}

#[test]
fn last_export_at_the_deadline_ignores_an_unfinished_epoch() {
    for buckets in [1, 2] {
        let fixture = Fixture::new();
        let mut args = export_args(&fixture, buckets);
        args.epochs = 3;
        args.export_epoch = ExportEpoch::Last;
        let mut session = train_worsening_epochs(&args);
        let expected = network_bytes(&session, &session.checkpoint);
        let best = fs::read(args.output_dir.join("best.npz")).unwrap();
        interrupt_next_epoch(&mut session);
        let summary = session.finish().unwrap();
        assert_eq!(summary.stopped_by, StopReason::TimeLimit);
        assert_eq!(summary.last_epoch, 2);
        assert_eq!(summary.exported_epoch, Some(2));
        assert_eq!(summary.best_epoch, Some(1));
        assert!(fs::read(args.output_dir.join("nnue.bin")).unwrap() == expected);
        assert_eq!(fs::read(args.output_dir.join("best.npz")).unwrap(), best);
        let saved = Checkpoint::read(args.output_dir.join("latest.npz")).unwrap();
        assert_eq!(saved.progress.epoch, 2);
        assert_eq!(saved.optimizer.step(), summary.total_steps);
        assert_eq!(logs(&args.output_dir.join("training.jsonl")).len(), 3);
    }
}

#[test]
fn last_export_requires_a_completed_epoch_at_the_deadline() {
    for buckets in [1, 2] {
        let fixture = Fixture::new();
        let mut args = export_args(&fixture, buckets);
        args.export_epoch = ExportEpoch::Last;
        let mut session = Session::new(args.clone()).unwrap();
        interrupt_next_epoch(&mut session);
        let error = session.finish().unwrap_err();
        assert!(matches!(
            error.downcast_ref::<Error>(),
            Some(Error::NoCompletedEpoch)
        ));
        let saved: Value =
            serde_json::from_slice(&fs::read(args.output_dir.join("summary.json")).unwrap())
                .unwrap();
        assert_eq!(saved["export_epoch"], "last");
        assert_eq!(saved["exported_epoch"], Value::Null);
        assert_eq!(saved["last_epoch"], 0);
        assert_eq!(saved["best_epoch"], Value::Null);
        assert!(!args.output_dir.join("nnue.bin").exists());
        assert!(!args.output_dir.join("best.npz").exists());
    }
}

#[test]
fn both_export_paths_reject_epoch_zero_and_best_rejects_a_later_epoch() {
    for buckets in [1, 2] {
        let fixture = Fixture::new();
        let args = export_args(&fixture, buckets);
        let mut session = Session::new(args.clone()).unwrap();
        let output = fixture.0.join("rejected");
        let error = export::write_completed(
            source(&session),
            &session.checkpoint,
            &output,
            &session.base.piece_values,
            &session.base.rule_set,
            args.validation_sample,
        )
        .unwrap_err();
        assert!(matches!(error, export::Error::Invalid(_)));
        assert!(!output.exists());
        assert!(session.advance().unwrap());
        assert!(session.advance().unwrap());
        assert_eq!(session.checkpoint.progress.best.unwrap().epoch, 1);
        let error = export::write_best(
            source(&session),
            &session.checkpoint,
            &output,
            &session.base.piece_values,
            &session.base.rule_set,
            args.validation_sample,
        )
        .unwrap_err();
        assert!(matches!(error, export::Error::Invalid(_)));
        assert!(!output.exists());
    }
}

#[test]
fn resume_can_change_export_epoch_in_both_directions_and_records_the_choice() {
    for buckets in [1, 2] {
        let fixture = Fixture::new();
        let mut args = export_args(&fixture, buckets);
        let session = train_worsening_epochs(&args);
        let best = session.best_checkpoint().unwrap();
        let expected_best = network_bytes(&session, &best);
        let expected_last = network_bytes(&session, &session.checkpoint);
        let original = session.checkpoint.clone();
        session.finish().unwrap();
        args.resume = Some(args.output_dir.clone());
        for (choice, expected_epoch, expected_network) in [
            (ExportEpoch::Last, 2, expected_last),
            (ExportEpoch::Best, 1, expected_best),
        ] {
            args.export_epoch = choice;
            let summary = run(args.clone()).unwrap();
            assert_eq!(summary.exported_epoch, Some(expected_epoch));
            assert!(fs::read(args.output_dir.join("nnue.bin")).unwrap() == expected_network);
            let latest = Checkpoint::read(args.output_dir.join("latest.npz")).unwrap();
            assert_eq!(latest.model, original.model);
            assert_eq!(latest.optimizer, original.optimizer);
            assert_eq!(latest.progress.best, original.progress.best);
            assert_eq!(
                Checkpoint::read(args.output_dir.join("best.npz")).unwrap(),
                best
            );
        }
        let config_path = args.output_dir.join("config.json");
        let saved = fs::read(&config_path).unwrap();
        let config: Value = serde_json::from_slice(&saved).unwrap();
        assert_eq!(config["arguments"]["export_epoch"], "best");
        assert!(config["settings"].get("export_epoch").is_none());
        assert_eq!(config["resumes"].as_array().unwrap().len(), 2);
        assert_eq!(config["resumes"][0]["export_epoch"], "last");
        assert_eq!(config["resumes"][1]["export_epoch"], "best");
        args.seed += 1;
        let error = run(args).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<crate::nn::Error>(),
            Some(crate::nn::Error::CheckpointMismatch(MismatchField::Seed))
        ));
        assert_eq!(fs::read(config_path).unwrap(), saved);
    }
}
