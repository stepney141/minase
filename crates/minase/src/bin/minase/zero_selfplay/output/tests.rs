//! 出力先の占有、生成条件の照合、および集計の範囲と順序。

use super::*;
use crate::zero_selfplay::game::CalibrationResult;
use crate::zero_selfplay::tests::{TemporaryDirectory, config};

fn chunk(chunk: u32, positions: u64) -> ChunkSummary {
    ChunkSummary {
        chunk,
        first_game: chunk * 10 + 1,
        counts: Counts {
            games: 10,
            positions,
            searches: positions,
            no_resign_games: 1,
            resign_calibration_games: vec![ResignCalibrationGame {
                game: chunk * 10 + 1,
                result: CalibrationResult::Draw,
                min_value_black: f64::from(0.01_f32),
                min_value_white: f64::from(0.23_f32),
            }],
            ..Counts::default()
        },
    }
}

#[test]
fn target_requires_a_contiguous_prefix_of_all_training_positions() {
    let mut completed = BTreeMap::new();
    for (number, positions, included) in [(2, 100, None), (0, 23, None), (1, 37, Some(2))] {
        completed.insert(number, chunk(number, positions));
        assert_eq!(included_chunks(&completed, 3, 60), included);
    }
    assert_eq!(included_chunks(&completed, 3, 23), Some(1));
    assert_eq!(included_chunks(&completed, 3, 61), Some(3));
    assert_eq!(included_chunks(&completed, 3, 1000), Some(3));
}

#[test]
fn calibration_and_training_files_are_sorted_across_chunks() {
    let summary = summarize(
        &[chunk(2, 100), chunk(0, 23), chunk(1, 37)],
        1000,
        StartPosition::Initial,
    );
    assert_eq!(
        summary
            .counts
            .resign_calibration_games
            .iter()
            .map(|g| g.game)
            .collect::<Vec<_>>(),
        [1, 11, 21]
    );
    assert_eq!(summary.counts.no_resign_games, 3);
    assert_eq!(summary.counts.searches, 160);
    assert_eq!(summary.training_positions, 160);
    assert_eq!(
        summary.training_files,
        ["chunk-00000.mnsd", "chunk-00001.mnsd", "chunk-00002.mnsd"]
    );
    let encoded = serde_json::to_vec(&summary).unwrap();
    let decoded: Summary = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, summary);
}

#[test]
fn resume_uses_only_the_target_prefix_without_inference_or_rewriting_files() {
    let directory = TemporaryDirectory::new();
    let config = Config {
        games: 40,
        chunk_games: 10,
        ..config()
    };
    drop(open_directory(&directory.0, &config).unwrap());
    for (number, positions) in [(2, 1000), (0, 23), (1, 37)] {
        fs::write(directory.0.join(games_filename(number)), b"retained bytes").unwrap();
        atomic_json(
            &directory.0,
            &format!("chunk-{number:05}.json"),
            &chunk(number, positions),
        )
        .unwrap();
    }
    let old_time = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000);
    let before: Vec<_> = fs::read_dir(&directory.0)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(old_time))
                .unwrap();
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    let one = NonZeroUsize::new(1).unwrap();
    let report = generate(&directory.0, &config, 60, &[23; 32], one, one, |_| {
        panic!("no inference for a completed prefix")
    })
    .unwrap();
    assert_eq!(report.generated_games, 0);
    assert_eq!(report.generated_positions, 0);
    assert_eq!(report.summary.chunks, 2);
    assert_eq!(report.summary.training_positions, 60);
    assert_eq!(
        report
            .summary
            .counts
            .resign_calibration_games
            .iter()
            .map(|g| g.game)
            .collect::<Vec<_>>(),
        [1, 11]
    );
    assert_eq!(
        report.summary.training_files,
        ["chunk-00000.mnsd", "chunk-00001.mnsd"]
    );
    let smaller = generate(&directory.0, &config, 23, &[23; 32], one, one, |_| {
        panic!("no inference for a smaller target")
    })
    .unwrap();
    assert_eq!(smaller.summary.chunks, 1);
    for (path, bytes) in before {
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), old_time);
    }
    assert!(!directory.0.join("chunk-00003.json").exists());
}

#[test]
fn mate_and_max_ply_draws_count_results_and_all_training_positions() {
    use minase::training::records::{Entry, Exclusion, GameRecord, Origin, Policy, Value};
    let mut counts = Counts::default();
    for (number, outcome) in [
        Outcome::MateInThree {
            winner: Color::Black,
        },
        Outcome::MateInThree {
            winner: Color::White,
        },
        Outcome::MaxPlyDraw,
    ]
    .into_iter()
    .enumerate()
    {
        let record = GameRecord::new(
            number as u32 + 2,
            Origin::RandomSelfplay,
            outcome,
            0,
            minase_core::Position::initial(),
            vec![Entry::new(None, Value::None, Exclusion::None, Policy::None, Some(0)).unwrap()],
        )
        .unwrap();
        counts.add_game(&CompletedGame {
            number: number as u32 + 2,
            record,
            calibration: None,
            searches: 2,
            plies: 2,
        });
    }
    assert_eq!(
        (counts.black_wins, counts.white_wins, counts.draws),
        (1, 1, 1)
    );
    assert_eq!(counts.mate_in_three_games, 2);
    assert_eq!(counts.draw_reason_max_ply, 1);
    assert_eq!((counts.positions, counts.searches), (6, 6));
}

#[test]
fn output_rejects_nonempty_directory_without_config_and_requires_existing_parent() {
    let directory = TemporaryDirectory::new();
    fs::write(directory.0.join("unrelated"), b"keep").unwrap();
    assert!(matches!(
        open_directory(&directory.0, &config()),
        Err(Error::NonemptyDirectory(_))
    ));
    assert_eq!(fs::read(directory.0.join("unrelated")).unwrap(), b"keep");
    assert!(!directory.0.join(".zero_selfplay.lock").exists());
    assert!(matches!(
        open_directory(&directory.0.join("absent/child"), &config()),
        Err(Error::Io(_))
    ));
}

#[test]
fn output_resumes_initialization_with_only_lock_and_temporary_config() {
    for temporary_config in [false, true] {
        let directory = TemporaryDirectory::new();
        fs::write(directory.0.join(".zero_selfplay.lock"), b"").unwrap();
        if temporary_config {
            fs::write(directory.0.join("config.json.tmp"), b"unfinished JSON").unwrap();
        }
        let config = config();
        let lock = open_directory(&directory.0, &config).unwrap();
        let saved: Config =
            serde_json::from_reader(File::open(directory.0.join("config.json")).unwrap()).unwrap();
        assert_eq!(saved, config);
        assert!(!directory.0.join("config.json.tmp").exists());
        assert!(matches!(
            open_directory(&directory.0, &config),
            Err(Error::Locked { .. })
        ));
        drop(lock);
        drop(open_directory(&directory.0, &config).unwrap());
    }
}

#[test]
fn resume_rejects_changed_nn_backend() {
    let directory = TemporaryDirectory::new();
    let config = config();
    drop(open_directory(&directory.0, &config).unwrap());
    for backend in ["cuda", "tensorrt"] {
        let changed = Config {
            nn_backend: backend.into(),
            ..config.clone()
        };
        let Err(Error::ConfigMismatch(fields)) = open_directory(&directory.0, &changed) else {
            panic!("a different inference provider must be rejected");
        };
        assert_eq!(fields, ["nn_backend"]);
    }
    let saved: Config =
        serde_json::from_reader(File::open(directory.0.join("config.json")).unwrap()).unwrap();
    assert_eq!(saved.nn_backend, "cpu");
    drop(open_directory(&directory.0, &config).unwrap());
}

#[test]
fn output_lock_is_exclusive_and_all_changed_condition_names_are_reported() {
    let directory = TemporaryDirectory::new();
    let config = config();
    let lock = open_directory(&directory.0, &config).unwrap();
    assert!(matches!(
        open_directory(&directory.0, &config),
        Err(Error::Locked { .. })
    ));
    drop(lock);
    let mut changed = config.clone();
    changed.rules.push_str(",E3");
    changed.generation_commit = "f".repeat(40);
    changed.network_sha256 = "f".repeat(64);
    changed.nn_backend = "tensorrt".into();
    changed.seed += 1;
    changed.games += 1;
    changed.chunk_games += 1;
    changed.simulations += 1;
    changed.resign_threshold = 0.5;
    changed.max_ply += 1;
    let Err(Error::ConfigMismatch(fields)) = open_directory(&directory.0, &changed) else {
        panic!("changed conditions must be rejected");
    };
    assert_eq!(
        fields,
        [
            "rules",
            "generation_commit",
            "network_sha256",
            "nn_backend",
            "seed",
            "games",
            "chunk_games",
            "simulations",
            "resign_threshold",
            "max_ply"
        ]
    );
    let saved: Config =
        serde_json::from_reader(File::open(directory.0.join("config.json")).unwrap()).unwrap();
    assert_eq!(saved, config);
    drop(open_directory(&directory.0, &config).unwrap());
}

#[test]
fn discarding_an_unfinished_chunk_removes_its_only_temporary_file() {
    let directory = TemporaryDirectory::new();
    let config = config();
    let header = Header::new(
        config.rules.clone(),
        config.generation_commit.clone(),
        [23; 32],
        TeacherKind::Mcts,
        u64::from(config.simulations),
        config.seed,
    )
    .unwrap();
    let writer = ChunkWriter::new(&directory.0, &config, &header, 0).unwrap();
    assert!(directory.0.join("chunk-00000.mnsd.tmp").exists());
    writer.discard(&directory.0).unwrap();
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
}
