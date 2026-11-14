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
    // 勝ちに近づく更新ほど検証損失は悪化するため、途中の重みと最後の重みを区別できる。
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

#[test]
fn final_checkpoint_is_exported_even_when_an_earlier_validation_loss_is_lower() {
    for buckets in [1, 2] {
        let fixture = Fixture::new();
        let mut args = export_args(&fixture, buckets);
        args.batches_per_superbatch = nz(2);
        args.superbatches = nz(1);
        run(args.clone()).unwrap();
        let first = fs::read(args.output_dir.join("nnue.bin")).unwrap();
        args.resume = Some(args.output_dir.clone());
        args.superbatches = nz(3);
        let summary = run(args.clone()).unwrap();
        let records = logs(&args.output_dir.join("training.jsonl"));
        assert!(
            records[1]["validation_loss"].as_f64().unwrap()
                < records[3]["validation_loss"].as_f64().unwrap()
        );
        assert_eq!(
            summary.final_validation_loss,
            records[3]["validation_loss"].as_f64()
        );
        let latest = Checkpoint::read(args.output_dir.join("latest.npz")).unwrap();
        let base = base::read(&args.base).unwrap();
        let expected = quantize(&latest.model)
            .unwrap()
            .encode(&base.piece_values, &base.rule_set)
            .unwrap();
        let actual = fs::read(args.output_dir.join("nnue.bin")).unwrap();
        assert_eq!(actual, expected);
        assert_ne!(actual, first);
        assert!(!args.output_dir.join("best.npz").exists());
        assert!(!args.output_dir.join("validation-interval.jsonl").exists());
        let saved: Value =
            serde_json::from_slice(&fs::read(args.output_dir.join("summary.json")).unwrap())
                .unwrap();
        assert_eq!(saved["status"], "candidate");
        assert_eq!(saved["reason"], Value::Null);
        assert_eq!(saved["stopped_by"], "completed");
        assert_eq!(saved["completed_superbatches"], 3);
        assert_eq!(saved["completed_steps"], 6);
        for removed in [
            "best_epoch",
            "best_validation_loss",
            "initial_validation_loss",
            "improved_over_epoch_zero",
            "export_epoch",
            "exported_epoch",
            "last_epoch",
        ] {
            assert!(saved.get(removed).is_none(), "{removed}");
        }
    }
}

#[test]
fn quantization_failure_keeps_the_final_network_and_reports_completed_updates() {
    for buckets in [1, 2] {
        let fixture = Fixture::new();
        let args = export_args(&fixture, buckets);
        write_base(&args.base, 1.0);
        let mut session = Session::new(args.clone()).unwrap();
        session.train().unwrap();
        // 3駒の線形寄与を各1/16 cpに固定すると、整数推論は0 cpで、誤差は3/16 cpとなる。
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
        let expected = quantize(&session.checkpoint.model)
            .unwrap()
            .encode(&session.base.piece_values, &session.base.rule_set)
            .unwrap();
        let error = session.finish().unwrap_err();
        assert!(matches!(
            error.downcast_ref::<Error>(),
            Some(Error::QuantizationError)
        ));
        let summary: Value =
            serde_json::from_slice(&fs::read(args.output_dir.join("summary.json")).unwrap())
                .unwrap();
        assert_eq!(summary["status"], "quantization_error");
        assert_eq!(summary["completed_superbatches"], args.superbatches.get());
        assert_eq!(summary["completed_steps"], args.planned_steps().unwrap());
        assert_eq!(
            summary["quantization"]["mean_absolute_error_cp"],
            3.0 / 16.0
        );
        assert_eq!(summary["quantization"]["limit_cp"], 1.0 / 40.0);
        assert_eq!(summary["quantization"]["passed"], false);
        assert!(summary["removal_sign"].is_object());
        assert_eq!(
            fs::read(args.output_dir.join("nnue.bin")).unwrap(),
            expected
        );
    }
}
