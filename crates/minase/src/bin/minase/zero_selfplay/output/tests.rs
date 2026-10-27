//! 誤投了率5%の境界、出力先の占有、および生成条件の照合。

use super::*;
use crate::zero_selfplay::tests::{TemporaryDirectory, config};

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
                    ..Counts::default()
                },
            },
        ];
        let summary = summarize(&chunks);
        assert_eq!(summary.false_resignation_rate, expected_rate);
        assert_eq!(summary.resigned_games_excluded, excluded);
        assert_eq!(summary.counts.games, 20);
        assert_eq!(summary.training_positions, if excluded { 50 } else { 60 });
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
