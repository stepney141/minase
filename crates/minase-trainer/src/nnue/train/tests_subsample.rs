use super::*;
use crate::data::TrainSubsample;
use minase::training::records::{
    Entry, Exclusion, GameRecord, Origin, Outcome as GameOutcome, Policy, TeacherKind,
    Value as TeacherValue, sorted_legal_moves,
};
use minase_core::{Game, GameResult, Position, WinReason};

/// 仕様の64ビット演算をu128の剰余演算で独立に評価する。
fn spec_mix(value: u64) -> u64 {
    let modulus = 1_u128 << 64;
    let mut z = u128::from(value);
    z = ((z ^ (z >> 30)) * 0xBF58_476D_1CE4_E5B9_u128) % modulus;
    z = ((z ^ (z >> 27)) * 0x94D0_49BB_1331_11EB_u128) % modulus;
    (z ^ (z >> 31)) as u64
}

fn spec_keeps(mode: &str, k: u64, r: u64, file: u64, game: u32, ply: u16) -> bool {
    let h_game = spec_mix((file << 32) | u64::from(game));
    let hash = if mode == "games" {
        h_game
    } else {
        let offset = (u128::from(ply) * 0x9E37_79B9_7F4A_7C15_u128) % (1_u128 << 64);
        spec_mix(h_game ^ offset as u64)
    };
    hash % k == r
}

struct InputPosition {
    index: u64,
    file: u64,
    game: u32,
    ply: u16,
    validation: bool,
}

/// 同じ対局番号を持つ2ファイルと、除外局面を挟む複数手の対局を作る。
fn played_inputs(fixture: &Fixture, args: &mut Args) -> Vec<InputPosition> {
    let mut positions = Vec::new();
    args.data.clear();
    for (file, seed) in [11_u64, 22].into_iter().enumerate() {
        let header = Header::new(
            "L0,P0,R1,E0".into(),
            "1".repeat(40),
            [2; 32],
            TeacherKind::AlphaBeta,
            200,
            seed,
        )
        .unwrap();
        let mut games = Vec::new();
        for number in 0..24_u32 {
            let mut game = Game::with_default_rules();
            let mut entries = Vec::new();
            let start = 7 + number as u16;
            let length = 5 + number as usize % 3;
            let split_offset = (u128::from(number) * 0x9E37_79B9_7F4A_7C15_u128) % (1_u128 << 64);
            let validation = spec_mix(seed ^ split_offset as u64).is_multiple_of(20);
            for i in 0..length {
                let mv = if i + 1 == length {
                    None
                } else {
                    Some(sorted_legal_moves(&game).unwrap()[i])
                };
                let exclusion = if i == 1 {
                    Exclusion::Tactical
                } else {
                    Exclusion::None
                };
                entries.push(
                    Entry::new(
                        mv,
                        TeacherValue::AlphaBeta(
                            (100 + number as i16 * 13 + i as i16 * 31)
                                * if i % 2 == 0 { 1 } else { -1 },
                        ),
                        exclusion,
                        Policy::None,
                        None,
                    )
                    .unwrap(),
                );
                if exclusion == Exclusion::None {
                    positions.push(InputPosition {
                        index: positions.len() as u64,
                        file: file as u64,
                        game: number,
                        ply: start + i as u16,
                        validation,
                    });
                }
                if let Some(mv) = mv {
                    game.play(mv).unwrap();
                }
            }
            games.push(
                GameRecord::new(
                    number,
                    Origin::RandomSelfplay,
                    GameOutcome::Game(GameResult::Win {
                        winner: if number % 2 == 0 {
                            Color::Black
                        } else {
                            Color::White
                        },
                        reason: WinReason::Resignation,
                    }),
                    start,
                    Position::initial(),
                    entries,
                )
                .unwrap(),
            );
        }
        // パス名の辞書順と--dataの入力順を逆にする。
        let path = fixture.0.join(format!("played-{}.mnsd", 1 - file));
        fs::write(&path, mnsd::bytes(header, &games)).unwrap();
        args.data.push(path);
    }
    positions
}

fn cpu_buckets(args: &mut Args, fixture: &Fixture, buckets: u32) {
    args.device = crate::nnue::options::DeviceKind::Cpu;
    args.threads = Some(1);
    args.buckets = buckets;
    if buckets > 1 {
        args.bucket_positions = Some(1000);
        args.scratch_dir = Some(fixture.0.clone());
    }
}

#[test]
fn subsample_arguments_reject_invalid_specs_before_reading_inputs() {
    assert!(parse_training_args(&[]).unwrap().train_subsample.is_none());
    for valid in [
        "games:2:0",
        "positions:4:3",
        "games:18446744073709551615:18446744073709551614",
    ] {
        let args = parse_training_args(&["--train-subsample", valid]).unwrap();
        assert_eq!(args.train_subsample.unwrap().to_string(), valid);
    }
    for invalid in [
        "",
        "games",
        "games:2",
        "games:2:0:1",
        "game:2:0",
        "positions:0:0",
        "games:1:0",
        "games:2:2",
        "positions:2:3",
        "games:2:-1",
        "games:-2:0",
        "games:+2:0",
        "positions:2:+0",
        "games:2.0:0",
        "games: 2:0",
        "games:2:0 ",
        "games::0",
        "games:2:",
        "games:18446744073709551616:0",
        "games:2:18446744073709551616",
    ] {
        let error = parse_training_args(&["--train-subsample", invalid]).unwrap_err();
        assert!(error.to_string().contains("--train-subsample"));
    }
}

#[test]
fn subsample_keys_match_spec_in_memory_and_spill_without_changing_preprocessing() {
    let fixture = Fixture::new();
    let mut args = fixture.training_args();
    let positions = played_inputs(&fixture, &mut args);
    cpu_buckets(&mut args, &fixture, 1);
    let baseline = Session::new(args.clone()).unwrap();
    let original: Value =
        serde_json::from_slice(&fs::read(args.output_dir.join("config.json")).unwrap()).unwrap();
    let all = memory_dataset(&baseline.dataset);
    let training: Vec<_> = positions.iter().filter(|p| !p.validation).collect();
    assert_eq!(
        all.training_indices(),
        training.iter().map(|p| p.index).collect::<Vec<_>>()
    );
    assert_eq!(training.len(), 216);
    let validation: Vec<_> = positions
        .iter()
        .filter(|p| p.validation)
        .map(|p| p.index)
        .collect();
    assert_eq!(all.validation_indices(), validation);
    for mode in ["games", "positions"] {
        for buckets in [1, 3] {
            let spec = format!("{mode}:4:1");
            let expected: Vec<_> = training
                .iter()
                .enumerate()
                .filter(|(_, p)| spec_keeps(mode, 4, 1, p.file, p.game, p.ply))
                .collect();
            // 仕様をPythonの任意精度整数でも評価した件数。
            assert_eq!(expected.len(), if mode == "games" { 87 } else { 54 });
            let expected_indices: Vec<_> = expected.iter().map(|(_, p)| p.index).collect();
            let expected_ordinals: Vec<u8> = expected
                .iter()
                .flat_map(|(i, _)| (*i as u64).to_le_bytes())
                .collect();
            let expected_games: std::collections::BTreeSet<_> =
                expected.iter().map(|(_, p)| (p.file, p.game)).collect();
            let mut selected = args.clone();
            cpu_buckets(&mut selected, &fixture, buckets);
            selected.train_subsample = Some(spec.parse().unwrap());
            selected.output_dir = fixture.0.join(format!("{mode}-{buckets}"));
            let session = Session::new(selected.clone()).unwrap();
            assert_eq!(session.selection.positions, expected.len() as u64);
            for parameter in Parameter::ALL {
                assert_eq!(
                    session.checkpoint.model.parameters()[parameter],
                    baseline.checkpoint.model.parameters()[parameter]
                );
            }
            assert_eq!(session.teacher_cp_per_logit, baseline.teacher_cp_per_logit);
            assert_eq!(session.dataset.validation_count(), validation.len() as u64);
            let config: Value =
                serde_json::from_slice(&fs::read(selected.output_dir.join("config.json")).unwrap())
                    .unwrap();
            assert_eq!(config["settings"]["train_subsample"], spec);
            assert_eq!(config["arguments"]["train_subsample"], spec);
            let summary = &config["preprocessing"]["train_subsample"];
            assert_eq!(summary["training_positions_before"], training.len());
            assert_eq!(summary["training_positions_after"], expected.len());
            assert_eq!(summary["training_games_after"], expected_games.len());
            assert_eq!(
                summary["training_indices_sha256"],
                crate::nn::hex(&sha256(&expected_ordinals))
            );
            let mut preprocessing = config["preprocessing"].clone();
            preprocessing
                .as_object_mut()
                .unwrap()
                .remove("train_subsample");
            assert_eq!(preprocessing, original["preprocessing"]);
            for epoch in [1, 2] {
                let mut actual = match &session.dataset {
                    Store::Memory(dataset) => {
                        epoch_permutation(session.selection.indices(dataset), 31, epoch)
                    }
                    Store::Spill(store) => {
                        let mut reader = store
                            .epoch(31, epoch, Instant::now() + Duration::from_secs(60))
                            .unwrap()
                            .unwrap();
                        let mut indices = Vec::new();
                        while let Some(chunk) = reader.next_chunk(17).unwrap() {
                            for record in chunk {
                                assert_eq!(
                                    record.teacher_score.to_bits(),
                                    all.teacher_score(record.index).unwrap().to_bits()
                                );
                                indices.push(record.index);
                            }
                        }
                        let actual_validation: Vec<_> = store
                            .validation_chunks(17)
                            .flat_map(|chunk| chunk.unwrap().into_iter().map(|record| record.index))
                            .collect();
                        assert_eq!(actual_validation, validation);
                        indices
                    }
                };
                actual.sort_unstable();
                assert_eq!(
                    actual, expected_indices,
                    "{mode}, buckets={buckets}, epoch={epoch}"
                );
            }
        }
    }
}

#[test]
fn subsample_batches_schedule_and_validation_use_selected_positions() {
    for mode in ["games", "positions"] {
        for buckets in [1, 3] {
            let fixture = Fixture::new();
            let mut args = fixture.training_args();
            cpu_buckets(&mut args, &fixture, buckets);
            args.train_subsample = Some(format!("{mode}:4:1").parse().unwrap());
            args.lr_schedule = "exponential:1:0.1".parse().unwrap();
            args.batch = 13;
            args.validation_interval = 1;
            let full = Dataset::open(
                &args.data,
                args.lookahead().unwrap(),
                std::num::NonZeroUsize::new(1).unwrap(),
            )
            .unwrap();
            let selected = full
                .training_indices()
                .iter()
                .filter(|&&i| {
                    let p = full.record(i).unwrap();
                    spec_keeps(mode, 4, 1, 0, p.game_number(), p.ply())
                })
                .count() as u64;
            let steps = selected.div_ceil(args.batch as u64);
            assert!(steps > 1 && steps < full.training_indices().len().div_ceil(args.batch) as u64);
            let mut session = Session::new(args.clone()).unwrap();
            assert!(session.advance().unwrap());
            assert!(session.advance().unwrap());
            let records = logs(&args.output_dir.join("training.jsonl"));
            for epoch in 1..=2_u64 {
                let row = &records[epoch as usize];
                assert_eq!(row["steps"], steps);
                assert_eq!(row["total_steps"], epoch * steps);
                let first = (-((epoch - 1) as f64)).exp2();
                let last = (-((epoch * steps - 1) as f64 / steps as f64)).exp2();
                assert_eq!(
                    row["lr_multiplier_first"].as_f64().unwrap().to_bits(),
                    first.to_bits()
                );
                assert_eq!(
                    row["lr_multiplier_last"].as_f64().unwrap().to_bits(),
                    last.to_bits()
                );
            }
            let intervals = logs(&args.output_dir.join("validation-interval.jsonl"));
            assert_eq!(intervals.len() as u64, 2 * (steps - 1));
            for (row, step) in intervals
                .iter()
                .zip((1..2 * steps).filter(|s| s % steps != 0))
            {
                assert_eq!(row["step"], step);
                assert_eq!(row["epoch"], step / steps + 1);
                assert_eq!(
                    row["epoch_fraction"].as_f64().unwrap(),
                    (step % steps) as f64 / steps as f64
                );
            }
        }
    }
}

#[test]
fn empty_subsample_stops_before_creating_training_output() {
    for mode in ["games", "positions"] {
        for buckets in [1, 3] {
            let fixture = Fixture::new();
            let mut args = fixture.training_args();
            cpu_buckets(&mut args, &fixture, buckets);
            let full = Dataset::open(
                &args.data,
                args.lookahead().unwrap(),
                std::num::NonZeroUsize::new(1).unwrap(),
            )
            .unwrap();
            // 296局面に対する1024剰余類には空の類が必ずある。
            let r = (0..1024)
                .find(|&r| {
                    full.training_indices().iter().all(|&i| {
                        let p = full.record(i).unwrap();
                        !spec_keeps(mode, 1024, r, 0, p.game_number(), p.ply())
                    })
                })
                .unwrap();
            args.train_subsample = Some(format!("{mode}:1024:{r}").parse().unwrap());
            // CUDAが使えなくても、デバイスを開く前に抽出0件を報告する。
            args.device = crate::nnue::options::DeviceKind::Cuda;
            let error = match Session::new(args.clone()) {
                Ok(_) => panic!("empty subsample accepted"),
                Err(error) => error,
            };
            assert!(matches!(
                error.downcast_ref::<Error>(),
                Some(Error::Invalid(
                    "--train-subsample selected 0 training positions"
                ))
            ));
            assert!(!args.output_dir.exists());
        }
    }
}

#[test]
fn cpu_subsample_resume_matches_continuous_training_and_rejects_changed_spec() {
    for mode in ["games", "positions"] {
        for buckets in [1, 3] {
            let fixture = Fixture::new();
            let mut args = fixture.training_args();
            cpu_buckets(&mut args, &fixture, buckets);
            args.train_subsample = Some(format!("{mode}:4:1").parse().unwrap());
            args.validation_interval = 1;
            run(args.clone()).unwrap();
            let mut resumed = args.clone();
            resumed.output_dir = fixture.0.join("resumed");
            let mut session = Session::new(resumed.clone()).unwrap();
            assert!(session.advance().unwrap());
            drop(session);
            resumed.resume = Some(resumed.output_dir.clone());
            for spec in [None, Some("games:4:2"), Some("positions:4:2")] {
                let mut changed = resumed.clone();
                changed.train_subsample = spec.map(|s| s.parse::<TrainSubsample>().unwrap());
                let error = match Session::new(changed) {
                    Ok(_) => panic!("changed subsample accepted"),
                    Err(error) => error,
                };
                assert!(matches!(
                    error.downcast_ref::<crate::nn::Error>(),
                    Some(crate::nn::Error::CheckpointMismatch(
                        MismatchField::Settings
                    ))
                ));
            }
            resumed.threads = Some(2);
            run(resumed.clone()).unwrap();
            for file in ["latest.npz", "best.npz"] {
                compare_checkpoint(&args.output_dir.join(file), &resumed.output_dir.join(file));
            }
            assert_eq!(
                fs::read(args.output_dir.join("nnue.bin")).unwrap(),
                fs::read(resumed.output_dir.join("nnue.bin")).unwrap()
            );
            assert_eq!(
                deterministic_logs(&args.output_dir),
                deterministic_logs(&resumed.output_dir)
            );
            assert_eq!(
                logs(&args.output_dir.join("validation-interval.jsonl")),
                logs(&resumed.output_dir.join("validation-interval.jsonl"))
            );
        }
    }
}

fn deterministic_logs(directory: &Path) -> Vec<Value> {
    let mut records = logs(&directory.join("training.jsonl"));
    for row in &mut records {
        for key in [
            "training_seconds",
            "validation_seconds",
            "interval_validation_seconds",
            "training_positions_per_second",
            "update_seconds",
            "elapsed_seconds",
        ] {
            row.as_object_mut().unwrap().remove(key);
        }
    }
    records
}

#[test]
fn subsample_key_arithmetic_matches_spec_at_integer_boundaries() {
    for mode in ["games", "positions"] {
        for k in [2, 7, u64::MAX] {
            for r in [0, k - 1] {
                let spec: TrainSubsample = format!("{mode}:{k}:{r}").parse().unwrap();
                for file in [0, 1, u64::from(u32::MAX) + 1, u64::MAX] {
                    for game in [0, 1, u32::MAX] {
                        for ply in [0, 1, u16::MAX] {
                            assert_eq!(
                                spec.keeps(file, game, ply),
                                spec_keeps(mode, k, r, file, game, ply)
                            );
                        }
                    }
                }
            }
        }
    }
}
