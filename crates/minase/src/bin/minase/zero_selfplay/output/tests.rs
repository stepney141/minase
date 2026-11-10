//! 誤投了率5%の境界、出力先の占有、および生成条件の照合。

use super::*;
use crate::zero_selfplay::tests::{TemporaryDirectory, config};

#[test]
fn target_requires_a_contiguous_prefix_and_counts_resigned_positions() {
    let mut completed = BTreeMap::new();
    for (chunk, positions, resigned_positions, included) in
        [(2, 100, 0, None), (0, 20, 3, None), (1, 30, 7, Some(2))]
    {
        completed.insert(
            chunk,
            ChunkSummary {
                chunk,
                first_game: chunk * 10 + 1,
                counts: Counts {
                    games: 10,
                    positions,
                    resigned_positions,
                    ..Counts::default()
                },
            },
        );
        assert_eq!(included_chunks(&completed, 3, 60), included);
    }
    assert_eq!(included_chunks(&completed, 3, 23), Some(1));
    assert_eq!(included_chunks(&completed, 3, 61), Some(3));
    assert_eq!(included_chunks(&completed, 3, 1000), Some(3));
}

#[test]
fn resume_summarizes_only_the_target_prefix_without_inference_or_changing_committed_files() {
    for (false_count, later_false_count, expected_rate, excluded) in
        [(0, 20, 0.0, false), (2, 0, 0.1, true)]
    {
        let directory = TemporaryDirectory::new();
        let config = Config {
            games: 400,
            chunk_games: 100,
            ..config()
        };
        drop(open_directory(&directory.0, &config).unwrap());
        // 塊2が塊1より先に確定し、塊3は未確定のまま再開する。
        for (chunk, positions, resigned_positions, false_resignations, triggers) in [
            (2, 1000, 80, later_false_count, 100),
            (0, 20, 3, 0, 0),
            (1, 30, 7, false_count, 20),
        ] {
            let summary = ChunkSummary {
                chunk,
                first_game: chunk * 100 + 1,
                counts: Counts {
                    games: 100,
                    positions,
                    resigned_positions,
                    false_resignations,
                    resign_triggered_playthrough_games: triggers,
                    ..Counts::default()
                },
            };
            for resigned in [false, true] {
                // 再開時は集計だけを読み、MNSDを読み直さない。
                fs::write(
                    directory.0.join(games_filename(chunk, resigned)),
                    b"retained bytes",
                )
                .unwrap();
            }
            atomic_json(&directory.0, &format!("chunk-{chunk:05}.json"), &summary).unwrap();
        }
        let old_time = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut before = Vec::new();
        for entry in fs::read_dir(&directory.0).unwrap() {
            let path = entry.unwrap().path();
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(old_time))
                .unwrap();
            before.push((path.clone(), fs::read(&path).unwrap()));
        }
        let one = NonZeroUsize::new(1).unwrap();
        let report = generate(&directory.0, &config, 60, &[23; 32], one, one, |_| {
            panic!("a completed prefix must not construct an evaluator")
        })
        .unwrap();
        assert_eq!(report.generated_games, 0);
        assert_eq!(report.generated_positions, 0);
        let summary: Summary =
            serde_json::from_reader(File::open(directory.0.join("summary.json")).unwrap()).unwrap();
        assert_eq!(summary, report.summary);
        assert_eq!(summary.target_positions, 60);
        assert_eq!(summary.chunks, 2);
        assert_eq!(
            summary.counts,
            Counts {
                games: 200,
                positions: 50,
                resigned_positions: 10,
                false_resignations: false_count,
                resign_triggered_playthrough_games: 20,
                ..Counts::default()
            }
        );
        assert_eq!(summary.false_resignation_rate, Some(expected_rate));
        assert_eq!(summary.resigned_games_excluded, excluded);
        assert_eq!(summary.training_positions, if excluded { 50 } else { 60 });
        assert_eq!(
            summary.training_files,
            if excluded {
                vec!["chunk-00000.mnsd", "chunk-00001.mnsd"]
            } else {
                vec![
                    "chunk-00000.mnsd",
                    "chunk-00000.resigned.mnsd",
                    "chunk-00001.mnsd",
                    "chunk-00001.resigned.mnsd",
                ]
            }
        );
        // 目標だけを変えても同じconfig.jsonで再開できる。
        let smaller = generate(&directory.0, &config, 23, &[23; 32], one, one, |_| {
            panic!("a smaller target must not construct an evaluator")
        })
        .unwrap();
        assert_eq!(smaller.summary.target_positions, 23);
        assert_eq!(smaller.summary.chunks, 1);
        for (path, bytes) in before {
            assert_eq!(fs::read(&path).unwrap(), bytes, "{}", path.display());
            assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), old_time);
        }
        assert!(!directory.0.join("chunk-00003.json").exists());
    }
}

#[test]
fn training_files_use_strict_five_percent_limit_and_handle_no_triggers() {
    for (false_count, triggers, expected_rate, excluded) in [
        (1, 20, Some(0.05), false),
        (2, 20, Some(0.1), true),
        (0, 0, None, false),
    ] {
        let chunks = [
            ChunkSummary {
                chunk: 1,
                first_game: 11,
                counts: Counts {
                    games: 10,
                    positions: 30,
                    resigned_positions: 7,
                    full_searches: 40,
                    fast_searches: 120,
                    mate_in_three_games: 2,
                    false_resignations: false_count,
                    resign_triggered_playthrough_games: triggers,
                    ..Counts::default()
                },
            },
            ChunkSummary {
                chunk: 0,
                first_game: 1,
                counts: Counts {
                    games: 10,
                    positions: 20,
                    resigned_positions: 3,
                    full_searches: 25,
                    fast_searches: 75,
                    mate_in_three_games: 3,
                    ..Counts::default()
                },
            },
        ];
        let summary = summarize(&chunks, 60, StartPosition::RandomOpening);
        assert_eq!(summary.false_resignation_rate, expected_rate);
        assert_eq!(summary.resigned_games_excluded, excluded);
        assert_eq!(summary.counts.games, 20);
        assert_eq!(summary.training_positions, if excluded { 50 } else { 60 });
        assert_eq!(summary.counts.full_searches, 65);
        assert_eq!(summary.counts.fast_searches, 195);
        assert_eq!(summary.counts.mate_in_three_games, 5);
        let expected = if excluded {
            vec!["chunk-00000.mnsd", "chunk-00001.mnsd"]
        } else {
            vec![
                "chunk-00000.mnsd",
                "chunk-00000.resigned.mnsd",
                "chunk-00001.mnsd",
                "chunk-00001.resigned.mnsd",
            ]
        };
        assert_eq!(summary.training_files, expected);
    }
}

#[test]
fn mate_in_three_counts_winners_and_full_search_training_positions() {
    use minase::training::records::{Entry, Exclusion, GameRecord, Origin, Policy, Value};
    use minase_core::Position;

    let mut counts = Counts::default();
    for winner in Color::ALL {
        let result = Outcome::MateInThree { winner };
        let record = GameRecord::new(
            1,
            Origin::RandomSelfplay,
            result,
            0,
            Position::initial(),
            vec![Entry::new(None, Value::None, Exclusion::None, Policy::None, Some(0)).unwrap()],
        )
        .unwrap();
        counts.add_game(&CompletedGame {
            number: 1,
            record: Some(record),
            resigned: false,
            playthrough: true,
            first_trigger: Some(winner.opposite()),
            result: Some(result),
            full_searches: 2,
            fast_searches: 6,
            plies: 20,
        });
    }
    counts.add_game(&CompletedGame {
        number: 3,
        record: None,
        resigned: false,
        playthrough: false,
        first_trigger: None,
        result: None,
        full_searches: 3,
        fast_searches: 9,
        plies: 3_000,
    });
    assert_eq!(counts.games_file_games, 2);
    assert_eq!((counts.black_wins, counts.white_wins), (1, 1));
    assert_eq!(counts.mate_in_three_games, 2);
    assert_eq!(counts.discarded_max_ply, 1);
    assert_eq!((counts.full_searches, counts.fast_searches), (7, 21));
    assert_eq!(counts.positions, 4);
    assert_eq!(counts.resigned_positions, 0);
    assert_eq!(counts.resign_triggered_playthrough_games, 2);
    assert_eq!(counts.false_resignations, 0);
    assert_eq!(counts.win_reason_mate, 0);
    assert_eq!(counts.win_reason_resignation, 0);
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
    changed.playthrough_percent = 50;
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
            "playthrough_percent",
            "max_ply"
        ]
    );
    let saved: Config =
        serde_json::from_reader(File::open(directory.0.join("config.json")).unwrap()).unwrap();
    assert_eq!(saved, config);
    drop(open_directory(&directory.0, &config).unwrap());
}
