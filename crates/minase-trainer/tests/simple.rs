//! HalfKP_vmの入力とCLIの公開契約。
mod common;
use minase::eval::nnue::input::{FeatureSet, HalfKpVm, Indexer, for_each_active_row};
use minase_core::{
    Color::{Black, White},
    PieceKind::{CrownPrince, GoldGeneral, King, Pawn},
};
use minase_trainer::nnue::{
    data::{BatchSlot, Error, PackedOccurrence, SENTINEL, build_batches},
    simple::Features,
};
#[test]
fn hand_numbered_features_virtual_rows_and_reverse_index_match_engine() {
    let p = common::position(
        &[
            (3, 0, Black, King),
            (4, 1, Black, CrownPrince),
            (2, 3, Black, Pawn),
            (9, 10, White, King),
            (7, 9, White, GoldGeneral),
        ],
        Black,
        Some((5, 5)),
    );
    let batch =
        BatchSlot::from_evaluation_positions(FeatureSet::HalfKpVm, std::slice::from_ref(&p), Black)
            .unwrap();
    let features = Features::from_batch(&batch).unwrap();
    for (v, side, expected, virtuals) in [
        (
            0,
            Black,
            vec![43075, 44253, 53104, 53490],
            vec![2899, 4077, 12928, 13314],
        ),
        (
            1,
            White,
            vec![113359, 116764, 117890, 120461],
            vec![6223, 9628, 10754, 13325],
        ),
    ] {
        let mut rows = features[0].views()[v].clone();
        rows.sort_unstable();
        assert_eq!(rows, expected);
        assert_eq!(rows.iter().map(|r| r % 13392).collect::<Vec<_>>(), virtuals);
        let mut engine = Vec::new();
        for_each_active_row::<HalfKpVm>(HalfKpVm::context(&p, side).unwrap(), side, &p, |r| {
            engine.push(r)
        });
        engine.sort_unstable();
        assert_eq!(rows, engine);
    }
    let inverse = batch.first_layer_inverse();
    assert_eq!(inverse.starts().len(), 964225);
    for (row, range) in inverse.starts().windows(2).enumerate() {
        for &word in &inverse.entries()[range[0] as usize..range[1] as usize] {
            let entry = PackedOccurrence::from_word(word).unwrap();
            assert!(!entry.rotated());
            let offset = (entry.position() * 2 + entry.perspective() as usize) * 145
                + entry.column() as usize;
            assert_eq!(batch.features()[offset], row as u32);
        }
    }
    assert_eq!(
        inverse.entries().len(),
        batch.features().iter().filter(|&&r| r != SENTINEL).count()
    );
}
#[test]
fn valued_teacher_with_ambiguous_anchor_reports_file_game_and_ply() {
    let p = common::position(
        &[
            (0, 0, Black, King),
            (1, 0, Black, King),
            (11, 11, White, King),
        ],
        Black,
        None,
    );
    let fixture = common::Fixture::new(&[p]);
    let error = build_batches(&fixture.dataset, FeatureSet::HalfKpVm, &[&[0]]).unwrap_err();
    match error {
        Error::TeacherAnchor {
            path,
            game,
            ply,
            error,
        } => {
            assert_eq!(
                path,
                fixture.root.join("records.bin").canonicalize().unwrap()
            );
            assert_eq!((game, ply), (1, 0));
            assert_eq!(
                error,
                minase::eval::nnue::input::AnchorError::MultipleKings { color: Black }
            );
        }
        other => panic!("{other}"),
    }
}
#[test]
fn simple_cli_exposes_arguments_and_explicit_cuda_errors() {
    let bin = env!("CARGO_BIN_EXE_minase-trainer");
    for command in ["train", "check-equivalence", "bench-step"] {
        let help = std::process::Command::new(bin)
            .args(["nnue", command, "simple", "--help"])
            .output()
            .unwrap();
        assert!(help.status.success());
        assert!(
            String::from_utf8(help.stdout)
                .unwrap()
                .contains("--teacher-cp-per-logit")
        );
        let mut args = vec![
            "nnue",
            command,
            "simple",
            "--teacher-cp-per-logit",
            "1072.65",
            "--lambda",
            "0.75",
        ];
        if command == "train" {
            args.extend([
                "--data",
                "missing.mnsd",
                "--output-dir",
                "missing",
                "--rules",
                "JCSA",
                "--superbatches",
                "1",
                "--validation-superbatches",
                "1",
                "--lr-schedule",
                "constant:1",
                "--device",
                "cuda",
            ]);
        } else {
            args.extend(["--batch", "16", "--seed", "1"]);
            if command == "bench-step" {
                args.extend(["--data", "missing.mnsd", "--device", "cuda"]);
            }
        }
        let result = std::process::Command::new(bin).args(args).output().unwrap();
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr)
                .contains("Simple CUDA backend is not implemented")
        );
    }
}

#[test]
fn memory_and_spill_preserve_anchor_error_origin_and_skip_valueless_entries() {
    use minase::training::records::{
        Entry, Exclusion, GameRecord, Header, Origin, Outcome, Policy, TeacherKind, Value, Writer,
    };
    use minase_trainer::data::{Dataset, SpillStore};
    let invalid = common::position(
        &[
            (0, 0, Black, King),
            (1, 0, Black, King),
            (11, 11, White, King),
        ],
        Black,
        None,
    );
    let fixture = common::Fixture::new(&[minase_core::Position::initial()]);
    let path = fixture.root.join("source.mnsd");
    let write = |with_value| {
        let header = Header::new(
            "JCSA".into(),
            "1".repeat(40),
            [2; 32],
            TeacherKind::AlphaBeta,
            200,
            11,
        )
        .unwrap();
        let mut writer = Writer::new(std::fs::File::create(&path).unwrap(), header).unwrap();
        for (game, position, value) in [
            (1, minase_core::Position::initial(), Value::AlphaBeta(120)),
            (4, minase_core::Position::initial(), Value::AlphaBeta(120)),
            (
                17,
                invalid.clone(),
                if with_value {
                    Value::AlphaBeta(100)
                } else {
                    Value::None
                },
            ),
        ] {
            writer
                .write_game(
                    &GameRecord::new(
                        game,
                        Origin::RandomSelfplay,
                        Outcome::MaxPlyDraw,
                        29,
                        position,
                        vec![Entry::new(None, value, Exclusion::None, Policy::None, None).unwrap()],
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        writer.finish().unwrap();
    };
    for with_value in [false, true] {
        write(with_value);
        let dataset = Dataset::open(std::slice::from_ref(&path), None, 1.try_into().unwrap())
            .unwrap()
            .with_lambda(0.75)
            .unwrap();
        assert_eq!(dataset.record_count(), if with_value { 3 } else { 2 });
        let indices: Vec<_> = (0..dataset.record_count()).collect();
        let memory = build_batches(&dataset, FeatureSet::HalfKpVm, &[&indices]);
        let spill = SpillStore::create(
            std::slice::from_ref(&path),
            None,
            1.try_into().unwrap(),
            &fixture.root,
            2,
            100,
        )
        .unwrap()
        .with_lambda(0.75)
        .unwrap();
        let mut records: Vec<_> = spill
            .training_records()
            .collect::<minase_trainer::data::Result<_>>()
            .unwrap();
        // game 17 belongs to the training split with seed 11.
        assert!(records.iter().any(|r| r.sample.game_number() == 17) == with_value);
        records.sort_by_key(|r| r.index);
        let disk = BatchSlot::from_spill_records(FeatureSet::HalfKpVm, &records);
        if with_value {
            for error in [memory.unwrap_err(), disk.unwrap_err()] {
                match error {
                    Error::TeacherAnchor {
                        path: actual,
                        game,
                        ply,
                        ..
                    } => {
                        assert_eq!(actual, path.canonicalize().unwrap());
                        assert_eq!((game, ply), (17, 29));
                    }
                    other => panic!("{other}"),
                }
            }
        } else {
            assert!(memory.is_ok());
            assert!(disk.is_ok());
        }
    }
}
