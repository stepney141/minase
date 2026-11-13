//! 投了の有効・無効、MNSDの内容、手数上限、および欠けた塊だけの再生成を検査する。
//! 学習済みのネットを使わず、本番のEvaluator境界から決定的な値を与える。

use super::*;
use std::fs::{self, File, FileTimes};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use minase::nn::test_util::UniformEvaluator;
use minase::nn::{Evaluator, NnError, Query};
use minase::training::records::{
    self, DecodedGame, Header, Outcome, Policy, Reader, TeacherKind, Value,
};
use minase_core::{
    Color, GameResult, WinReason,
    rng::{XorShift64, derive_seed},
};
use output::{Config, RunReport, Summary};

pub(super) struct TemporaryDirectory(pub(super) PathBuf);

impl TemporaryDirectory {
    pub(super) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "minase-zero-selfplay-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove the test directory");
    }
}

const CHECKSUM: [u8; 32] = [23; 32];

pub(super) fn config() -> Config {
    Config {
        rules: Rules::JCSA.to_string(),
        generation_commit: "0123456789abcdef0123456789abcdef01234567".into(),
        network_sha256: minase::datagen::hex(&CHECKSUM),
        nn_backend: "cpu".into(),
        seed: 7,
        start_position: cli::StartPosition::RandomOpening,
        games: 6,
        chunk_games: 2,
        simulations: 4,
        playthrough_percent: 1,
        max_ply: 20,
    }
}

fn nz(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).unwrap()
}

struct BiasedEvaluator;

impl Evaluator for BiasedEvaluator {
    fn evaluate(
        &mut self,
        queries: &[Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        logits.clear();
        values.clear();
        for query in queries {
            logits.resize(logits.len() + query.moves.len(), 0.0);
            values.push(if query.position.side_to_move() == Color::Black {
                0.001
            } else {
                0.999
            });
        }
        Ok(())
    }
}

fn generate(path: &Path, config: &Config, threads: usize, batch: usize) -> RunReport {
    output::generate(
        path,
        config,
        u64::MAX,
        &CHECKSUM,
        nz(threads),
        nz(batch),
        |_| Ok(Box::new(BiasedEvaluator)),
    )
    .unwrap()
}

fn read_games(path: &Path) -> (Header, Vec<DecodedGame>) {
    let mut reader = Reader::new(File::open(path).unwrap()).unwrap();
    let header = reader.header().clone();
    let mut games = Vec::new();
    while let Some(game) = reader.read_game().unwrap() {
        games.push(game);
    }
    (header, games)
}

fn all_records(path: &Path, config: &Config, resigned: bool) -> Vec<DecodedGame> {
    let mut games = Vec::new();
    for chunk in 0..config.games.div_ceil(config.chunk_games) {
        let suffix = if resigned { ".resigned.mnsd" } else { ".mnsd" };
        games.extend(read_games(&path.join(format!("chunk-{chunk:05}{suffix}"))).1);
    }
    games
}

/// 基本シードと対局番号から、仕様で定める完全探索の手番列を求める。
fn full_searches(config: &Config, number: u32) -> impl Iterator<Item = bool> {
    let seed = derive_seed(config.seed, u64::from(number));
    let mut rng = XorShift64::new(derive_seed(seed.get(), 3));
    std::iter::from_fn(move || Some(rng.next() < (1_u64 << 62)))
}

fn first_full_black_search(config: &Config, number: u32) -> Option<usize> {
    let seed = derive_seed(config.seed, u64::from(number));
    let (game, _) = minase::datagen::opening::generate_opening(Rules::JCSA, seed).unwrap();
    (game.ply_count()..config.max_ply)
        .zip(full_searches(config, number))
        .position(|(ply, full)| ply.is_multiple_of(2) && full)
}

#[test]
fn playthrough_hundred_disables_resignation_and_counts_false_triggers_including_discards() {
    let directory = TemporaryDirectory::new();
    let config = Config {
        playthrough_percent: 100,
        ..config()
    };
    let report = generate(&directory.0, &config, 2, 3);
    let counts = &report.summary.counts;
    assert_eq!(counts.games, u64::from(config.games));
    assert_eq!(counts.playthrough_games, counts.games);
    assert_eq!(counts.resigned_games, 0);
    let triggers = (1..=config.games)
        .filter(|&number| first_full_black_search(&config, number).is_some())
        .count() as u64;
    assert_eq!(counts.resign_triggered_playthrough_games, triggers);
    assert!(all_records(&directory.0, &config, true).is_empty());
    assert!(all_records(&directory.0, &config, false).is_empty());
    assert_eq!(counts.false_resignations, triggers);
    assert_eq!(counts.discarded_max_ply, u64::from(config.games));
    assert_eq!(
        counts.plies,
        u64::from(config.games) * u64::from(config.max_ply)
    );
    assert_eq!(counts.positions, 0);
    assert_eq!(counts.resigned_positions, 0);
    assert_eq!(report.summary.training_positions, 0);
    assert_eq!(report.summary.target_positions, u64::MAX);
    assert_eq!(report.summary.chunks, 3);
    assert_eq!(report.summary.false_resignation_rate, Some(1.0));
    assert!(report.summary.resigned_games_excluded);
}

#[test]
fn enabled_resignation_records_first_full_black_search_with_mcts_headers_policy_and_visits() {
    let directory = TemporaryDirectory::new();
    let config = Config {
        games: 8,
        chunk_games: 3,
        playthrough_percent: 20,
        ..config()
    };
    let report = generate(&directory.0, &config, 2, 2);
    let playthrough = |number: u32| {
        let seed = derive_seed(config.seed, u64::from(number));
        derive_seed(seed.get(), 2).get() % 100 < u64::from(config.playthrough_percent)
    };
    let expected: Vec<_> = (1..=config.games)
        .filter(|&number| {
            !playthrough(number) && first_full_black_search(&config, number).is_some()
        })
        .collect();
    assert!(!expected.is_empty());
    assert!(expected.len() < config.games as usize);
    let resigned = all_records(&directory.0, &config, true);
    assert_eq!(
        resigned
            .iter()
            .map(|game| game.record().game_number())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(report.summary.counts.resigned_games, expected.len() as u64);
    assert_eq!(
        report.summary.counts.win_reason_resignation,
        expected.len() as u64
    );
    for chunk in 0..config.games.div_ceil(config.chunk_games) {
        for suffix in [".mnsd", ".resigned.mnsd"] {
            let (header, _) = read_games(&directory.0.join(format!("chunk-{chunk:05}{suffix}")));
            assert_eq!(header.teacher_kind(), TeacherKind::Mcts);
            assert_eq!(
                header.teacher_search_budget(),
                u64::from(config.simulations)
            );
            assert_eq!(header.seed(), config.seed);
            assert_eq!(header.rule_set(), config.rules);
            assert_eq!(header.generation_commit(), config.generation_commit);
            assert_eq!(header.network_checksum(), &CHECKSUM);
        }
    }
    let mut positions = 0;
    for decoded in resigned {
        let record = decoded.record();
        let seed = derive_seed(config.seed, u64::from(record.game_number()));
        let (_, opening) = minase::datagen::opening::generate_opening(Rules::JCSA, seed).unwrap();
        let opening_len = opening.len();
        assert!((8..=16).contains(&opening_len));
        let searched = first_full_black_search(&config, record.game_number()).unwrap() + 1;
        assert_eq!(record.entries().len(), opening_len + searched);
        assert_eq!(record.start_position(), &minase_core::Position::initial());
        assert_eq!(record.start_ply(), 0);
        assert_eq!(record.origin(), records::Origin::RandomSelfplay);
        assert_eq!(
            record.outcome(),
            Outcome::Game(GameResult::Win {
                winner: Color::White,
                reason: WinReason::Resignation
            })
        );
        let mut replay = minase_core::Game::new(Rules::JCSA);
        let mut full = full_searches(&config, record.game_number());
        for (index, entry) in record.entries().iter().enumerate() {
            assert_eq!(entry.exclusion(), records::Exclusion::None);
            if index < opening_len {
                assert_eq!(entry.played_move(), Some(opening[index]));
                assert_eq!(entry.value(), Value::None);
                assert_eq!(entry.visits(), Some(0));
                assert_eq!(entry.policy(), &Policy::None);
            } else {
                assert!(matches!(entry.value(), Value::Mcts(_)));
                if full.next().unwrap() {
                    positions += 1;
                    assert_eq!(entry.visits(), Some(config.simulations));
                    let Policy::Candidates(candidates) = entry.policy() else {
                        panic!("full search policy is required")
                    };
                    assert_eq!(
                        candidates
                            .iter()
                            .map(|(_, probability)| u32::from(*probability))
                            .sum::<u32>(),
                        65535
                    );
                    assert!(candidates.iter().all(|(_, probability)| *probability > 0));
                    let legal = records::sorted_legal_moves(&replay).unwrap();
                    let indices: Vec<_> = candidates
                        .iter()
                        .map(|(mv, _)| legal.iter().position(|legal| legal == mv).unwrap())
                        .collect();
                    assert!(indices.windows(2).all(|pair| pair[0] < pair[1]));
                } else {
                    assert_eq!(entry.visits(), Some(config.simulations / 4));
                    assert_eq!(entry.policy(), &Policy::None);
                    assert!(entry.played_move().is_some());
                }
            }
            if let Some(mv) = entry.played_move() {
                replay.play(mv).unwrap();
            }
        }
        let last = record.entries().last().unwrap();
        assert_eq!(last.played_move(), None);
        assert_eq!(last.value(), Value::Mcts(66));
        assert_eq!(
            decoded.positions().last().unwrap().side_to_move(),
            Color::Black
        );
    }
    assert_eq!(positions, report.summary.counts.resigned_positions);
    assert!(report.summary.counts.fast_searches > 0);
    let saved: Summary =
        serde_json::from_reader(File::open(directory.0.join("summary.json")).unwrap()).unwrap();
    assert_eq!(saved, report.summary);
    let saved_config: serde_json::Value =
        serde_json::from_reader(File::open(directory.0.join("config.json")).unwrap()).unwrap();
    assert_eq!(saved_config["nn_backend"], "cpu");
    assert_eq!(saved_config.as_object().unwrap().len(), 11);
    for absent in [
        "version",
        "threads",
        "devices",
        "games_per_thread",
        "target_positions",
    ] {
        assert!(saved_config.get(absent).is_none());
    }
}

#[test]
fn resume_regenerates_only_missing_chunk_deterministically_across_parallelism() {
    for start_position in [
        cli::StartPosition::RandomOpening,
        cli::StartPosition::ShuffledSetup,
    ] {
        let directory = TemporaryDirectory::new();
        let config = Config {
            start_position,
            ..config()
        };
        let first = generate(&directory.0, &config, 2, 2);
        assert!(first.summary.counts.resigned_games > 0);
        let repeated_directory = TemporaryDirectory::new();
        let repeated = generate(&repeated_directory.0, &config, 1, 3);
        assert_eq!(repeated.summary, first.summary);
        let mut before = Vec::new();
        let old_time = UNIX_EPOCH + Duration::from_secs(1_000_000);
        for chunk in 0..3 {
            for suffix in [".mnsd", ".resigned.mnsd", ".json"] {
                let path = directory.0.join(format!("chunk-{chunk:05}{suffix}"));
                assert_eq!(
                    fs::read(&path).unwrap(),
                    fs::read(repeated_directory.0.join(path.file_name().unwrap())).unwrap()
                );
                File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_times(FileTimes::new().set_modified(old_time))
                    .unwrap();
                before.push((
                    chunk,
                    path.clone(),
                    fs::read(&path).unwrap(),
                    fs::metadata(&path).unwrap().modified().unwrap(),
                ));
            }
        }
        fs::remove_file(directory.0.join("chunk-00001.json")).unwrap();
        fs::write(
            directory.0.join("chunk-00001.resigned.mnsd.tmp"),
            vec![0xff; 100_000],
        )
        .unwrap();
        fs::write(directory.0.join("summary.json"), b"stale summary").unwrap();
        let resumed = generate(&directory.0, &config, 1, 3);
        assert_eq!(resumed.generated_games, 2);
        assert_eq!(resumed.summary, first.summary);
        for (chunk, path, bytes, modified) in &before {
            assert_eq!(&fs::read(path).unwrap(), bytes, "{}", path.display());
            if *chunk != 1 {
                assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), *modified);
            } else {
                assert_ne!(fs::metadata(path).unwrap().modified().unwrap(), *modified);
            }
        }
        let complete = output::generate(
            &directory.0,
            &config,
            u64::MAX,
            &CHECKSUM,
            nz(3),
            nz(4),
            |_| panic!("completed cycles must not construct an evaluator"),
        )
        .unwrap();
        assert_eq!(complete.generated_games, 0);
        assert_eq!(complete.generated_positions, 0);
        assert_eq!(complete.summary, first.summary);
        let changed = Config {
            seed: config.seed + 1,
            ..config.clone()
        };
        let error = output::generate(
            &directory.0,
            &changed,
            u64::MAX,
            &CHECKSUM,
            nz(1),
            nz(1),
            |_| panic!("mismatched conditions must fail before inference"),
        )
        .err()
        .unwrap();
        assert!(matches!(error, Error::ConfigMismatch(fields) if fields == ["seed"]));
    }
}

#[test]
fn reaching_target_stops_generation_and_removes_unfinished_chunk_files() {
    let directory = TemporaryDirectory::new();
    let config = Config {
        seed: 1,
        ..config()
    };
    let report = output::generate(&directory.0, &config, 1, &CHECKSUM, nz(1), nz(6), |_| {
        Ok(Box::new(BiasedEvaluator))
    })
    .unwrap();
    let saved: Summary =
        serde_json::from_reader(File::open(directory.0.join("summary.json")).unwrap()).unwrap();
    let first: output::ChunkSummary =
        serde_json::from_reader(File::open(directory.0.join("chunk-00000.json")).unwrap()).unwrap();
    assert_eq!(saved, report.summary);
    assert_eq!(saved.target_positions, 1);
    assert_eq!(saved.chunks, 1);
    assert_eq!(saved.counts, first.counts);
    assert_eq!(saved.counts.games, 2);
    assert!(saved.counts.positions + saved.counts.resigned_positions >= 1);
    let mut committed_games = 0;
    for entry in fs::read_dir(&directory.0).unwrap() {
        let path = entry.unwrap().path();
        assert_ne!(
            path.extension().and_then(|suffix| suffix.to_str()),
            Some("tmp")
        );
        if path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("chunk-")
            && path.extension().and_then(|suffix| suffix.to_str()) == Some("json")
        {
            let chunk: output::ChunkSummary =
                serde_json::from_reader(File::open(path).unwrap()).unwrap();
            committed_games += chunk.counts.games;
        }
    }
    // 確定済みの局数との差により、書きかけの塊が実際にあったことも確かめる。
    assert!(report.generated_games > committed_games);
    assert!(committed_games > saved.counts.games);
    assert!(report.generated_games < u64::from(config.games));
    assert_eq!(
        saved.training_files,
        ["chunk-00000.mnsd", "chunk-00000.resigned.mnsd"]
    );
}

struct BatchRecorder(Arc<Mutex<Vec<usize>>>);

impl Evaluator for BatchRecorder {
    fn evaluate(
        &mut self,
        queries: &[Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        self.0.lock().unwrap().push(queries.len());
        UniformEvaluator.evaluate(queries, logits, values)
    }
}

#[test]
fn inference_batches_contain_multiple_games_and_max_ply_discards_have_no_records() {
    let directory = TemporaryDirectory::new();
    let config = Config {
        games: 4,
        playthrough_percent: 100,
        max_ply: 18,
        ..config()
    };
    let batches = Arc::new(Mutex::new(Vec::new()));
    let report = output::generate(
        &directory.0,
        &config,
        u64::MAX,
        &CHECKSUM,
        nz(1),
        nz(3),
        |_| Ok(Box::new(BatchRecorder(Arc::clone(&batches)))),
    )
    .unwrap();
    let batches = batches.lock().unwrap();
    assert!(batches.contains(&3));
    assert!(batches.iter().all(|&count| (1..=3).contains(&count)));
    assert_eq!(report.summary.counts.discarded_max_ply, 4);
    assert_eq!(report.summary.counts.plies, 4 * 18);
    assert_eq!(report.summary.counts.games_file_games, 0);
    assert_eq!(report.summary.counts.resigned_games, 0);
    assert_eq!(report.summary.counts.resign_triggered_playthrough_games, 0);
    assert_eq!(report.summary.false_resignation_rate, None);
    assert!(all_records(&directory.0, &config, false).is_empty());
    assert!(all_records(&directory.0, &config, true).is_empty());
}

#[derive(Clone, Copy)]
enum Failure {
    Values,
    Logits,
    Nan,
    Inference,
    Panic,
}

struct FailingEvaluator {
    failure: Failure,
    dropped: Arc<AtomicUsize>,
}

impl Drop for FailingEvaluator {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

impl Evaluator for FailingEvaluator {
    fn evaluate(
        &mut self,
        queries: &[Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        UniformEvaluator.evaluate(queries, logits, values)?;
        match self.failure {
            Failure::Values => {
                values.pop();
            }
            Failure::Logits => {
                logits.pop();
            }
            Failure::Nan => values[0] = f32::NAN,
            Failure::Inference => return Err(NnError::MissingOutput { name: "value" }),
            Failure::Panic => panic!("inference panic fixture"),
        }
        Ok(())
    }
}

#[test]
fn inference_failures_keep_typed_causes_join_all_workers_and_allow_resume() {
    for failure in [
        Failure::Values,
        Failure::Logits,
        Failure::Nan,
        Failure::Inference,
        Failure::Panic,
    ] {
        let directory = TemporaryDirectory::new();
        let config = config();
        let dropped = Arc::new(AtomicUsize::new(0));
        let error = output::generate(
            &directory.0,
            &config,
            u64::MAX,
            &CHECKSUM,
            nz(2),
            nz(2),
            |_| {
                Ok(Box::new(FailingEvaluator {
                    failure,
                    dropped: Arc::clone(&dropped),
                }))
            },
        )
        .err()
        .unwrap();
        assert_eq!(dropped.load(Ordering::Relaxed), 2);
        match failure {
            Failure::Values => assert!(matches!(
                error,
                Error::Inference(NnError::OutputShape { name: "values", .. })
            )),
            Failure::Logits => assert!(matches!(
                error,
                Error::Inference(NnError::OutputShape { name: "logits", .. })
            )),
            Failure::Nan => assert!(matches!(
                error,
                Error::Mcts(minase::search::mcts::Error::Tree(
                    minase::search::mcts::tree::TreeError::InvalidEvaluation
                ))
            )),
            Failure::Inference => {
                use std::error::Error as _;
                assert!(error.source().unwrap().is::<NnError>());
                assert!(matches!(
                    error,
                    Error::Inference(NnError::MissingOutput { name: "value" })
                ));
            }
            Failure::Panic => assert!(
                matches!(error, Error::Panic(payload) if payload.downcast_ref::<&str>() == Some(&"inference panic fixture"))
            ),
        }
        assert!(!directory.0.join("summary.json").exists());
        let resumed = generate(&directory.0, &config, 1, 2);
        assert_eq!(resumed.generated_games, u64::from(config.games));
    }
}

#[test]
fn shuffled_records_start_at_seeded_setup_without_random_opening_entries() {
    let mut starts = Vec::new();
    for resigned in [false, true] {
        let directory = TemporaryDirectory::new();
        let config = Config {
            start_position: cli::StartPosition::ShuffledSetup,
            games: if resigned { 6 } else { 2 },
            max_ply: MAX_PLY,
            playthrough_percent: if resigned { 1 } else { 100 },
            ..config()
        };
        output::generate(
            &directory.0,
            &config,
            u64::MAX,
            &CHECKSUM,
            nz(2),
            nz(2),
            |_| {
                if resigned {
                    Ok(Box::new(BiasedEvaluator))
                } else {
                    Ok(Box::new(UniformEvaluator))
                }
            },
        )
        .unwrap();
        assert!(
            !all_records(&directory.0, &config, resigned).is_empty(),
            "resigned={resigned}"
        );
        for file_resigned in [false, true] {
            for decoded in all_records(&directory.0, &config, file_resigned) {
                let record = decoded.record();
                let seed = derive_seed(config.seed, u64::from(record.game_number()));
                let expected = minase_core::Position::shuffled_initial(&mut XorShift64::new(
                    derive_seed(seed.get(), 0),
                ));
                assert_eq!(record.start_ply(), 0);
                assert_eq!(record.start_position(), &expected);
                assert_eq!(record.origin(), records::Origin::RandomSelfplay);
                let first = &record.entries()[0];
                assert!(matches!(first.value(), Value::Mcts(_)));
                let full = full_searches(&config, record.game_number()).next().unwrap();
                assert_eq!(matches!(first.policy(), Policy::Candidates(_)), full);
                assert_eq!(
                    first.visits(),
                    Some(if full {
                        config.simulations
                    } else {
                        config.simulations / 4
                    })
                );
                starts.push(expected);
            }
        }
    }
    assert!(starts.iter().any(|start| start != &starts[0]));
}

#[test]
fn start_position_is_saved_in_config_and_summary_with_cli_spelling() {
    for (start_position, spelling) in [
        (cli::StartPosition::ShuffledSetup, "shuffled-setup"),
        (cli::StartPosition::RandomOpening, "random-opening"),
    ] {
        let directory = TemporaryDirectory::new();
        let config = Config {
            start_position,
            ..config()
        };
        generate(&directory.0, &config, 1, 2);
        for name in ["config.json", "summary.json"] {
            let saved: serde_json::Value =
                serde_json::from_slice(&fs::read(directory.0.join(name)).unwrap()).unwrap();
            assert_eq!(saved["start_position"], spelling);
        }
        assert_eq!(start_position.to_string(), spelling);
    }
}

#[test]
fn changed_or_missing_start_position_rejects_resume_without_rewriting_files() {
    let directory = TemporaryDirectory::new();
    let original = config();
    generate(&directory.0, &original, 1, 2);
    let changed = Config {
        start_position: cli::StartPosition::ShuffledSetup,
        ..original.clone()
    };
    for missing in [false, true] {
        if missing {
            let path = directory.0.join("config.json");
            let mut saved: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            saved.as_object_mut().unwrap().remove("start_position");
            fs::write(path, serde_json::to_vec(&saved).unwrap()).unwrap();
        }
        let before: Vec<_> = fs::read_dir(&directory.0)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let bytes = fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        let error = output::generate(
            &directory.0,
            &changed,
            u64::MAX,
            &CHECKSUM,
            nz(1),
            nz(1),
            |_| panic!("invalid config must fail before inference"),
        )
        .err()
        .unwrap();
        if missing {
            assert!(matches!(&error, Error::Json(_)));
            assert!(error.to_string().contains("start_position"));
        } else {
            assert!(matches!(error, Error::ConfigMismatch(fields) if fields == ["start_position"]));
        }
        for (path, bytes) in before {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }
}

#[test]
fn start_position_cli_is_required_and_accepts_only_the_two_kinds() {
    use clap::Parser;
    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        arguments: Arguments,
    }
    let required = [
        "minase",
        "--output-dir",
        "cycle",
        "--nn-model",
        "net.onnx",
        "--nn-backend",
        "cpu",
        "--generation-commit",
        "0123456789abcdef0123456789abcdef01234567",
        "--seed",
        "1",
        "--games",
        "2",
        "--target-positions",
        "100",
        "--chunk-games",
        "2",
        "--simulations",
        "4",
        "--playthrough-percent",
        "100",
        "--threads",
        "1",
        "--devices",
        "1",
        "--games-per-thread",
        "2",
    ];
    assert_eq!(
        Cli::try_parse_from(required).err().unwrap().kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
    assert!(
        Cli::try_parse_from(required.into_iter().chain(["--start-position", "unknown"])).is_err()
    );
    for (spelling, kind) in [
        ("shuffled-setup", cli::StartPosition::ShuffledSetup),
        ("random-opening", cli::StartPosition::RandomOpening),
    ] {
        let parsed =
            Cli::try_parse_from(required.into_iter().chain(["--start-position", spelling]))
                .unwrap();
        assert_eq!(parsed.arguments.start_position, kind);
    }
}

fn execution_arguments(
    directory: &Path,
    backend: cli::Backend,
    threads: usize,
    devices: usize,
) -> Arguments {
    Arguments {
        output_dir: directory.join("output"),
        nn_model: directory.join("missing.onnx"),
        nn_backend: backend,
        generation_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        seed: 1,
        start_position: cli::StartPosition::RandomOpening,
        games: 2,
        target_positions: 100,
        chunk_games: 2,
        simulations: 4,
        playthrough_percent: 100,
        threads: nz(threads),
        devices: nz(devices),
        games_per_thread: nz(1),
    }
}

// 複数GPU化の設計: スレッド数がGPU数の倍数でなければ、外部資源に触れる前に拒む。
#[test]
fn startup_rejects_threads_not_divisible_by_devices_before_reading_network() {
    let directory = TemporaryDirectory::new();
    for backend in [
        cli::Backend::TensorRt,
        cli::Backend::Cuda,
        cli::Backend::Cpu,
    ] {
        let arguments = execution_arguments(&directory.0, backend, 3, 2);
        assert!(matches!(
            run(arguments),
            Err(Error::ThreadsNotMultipleOfDevices { threads, devices })
                if threads.get() == 3 && devices.get() == 2
        ));
    }
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
}

// フェーズ1の指示書: CPUではGPUを照会せず、devices=1だけを認める。
#[test]
fn cpu_requires_one_device_before_reading_network() {
    let directory = TemporaryDirectory::new();
    assert!(matches!(
        run(execution_arguments(&directory.0, cli::Backend::Cpu, 4, 2)),
        Err(Error::CpuDeviceCount { devices }) if devices.get() == 2
    ));
    assert!(matches!(
        run(execution_arguments(&directory.0, cli::Backend::Cpu, 4, 1)),
        Err(Error::Inference(NnError::ReadNetwork { path, .. }))
            if path == directory.0.join("missing.onnx")
    ));
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
}

// 複数GPU化の設計: GPUごとに同数のスレッドを割り当てられる指定を認める。
#[test]
fn execution_settings_accept_thread_multiples() {
    for backend in [cli::Backend::TensorRt, cli::Backend::Cuda] {
        for (threads, devices) in [(1, 1), (2, 2), (8, 4)] {
            execution_arguments(Path::new("unused"), backend, threads, devices)
                .validate_execution()
                .unwrap();
        }
    }
}

// 複数GPU化の設計: 使用するGPUの枚数は、照会した枚数と等しくなければならない。
#[test]
fn requested_gpu_count_must_match_inventory() {
    validate_gpu_count(nz(4), nz(4)).unwrap();
    assert!(matches!(
        validate_gpu_count(nz(2), nz(4)),
        Err(Error::DeviceCountMismatch { requested, available })
            if requested.get() == 2 && available.get() == 4
    ));
}

// 複数GPU化の設計: 8スレッドを4枚へ0,1,2,3の順で割り当て、1枚なら全スレッドを機器0へ置く。
#[test]
fn threads_cycle_through_cuda_devices() {
    let device_ids: Vec<_> = (0..8)
        .map(|thread| device_for_thread(thread, nz(4)))
        .collect();
    assert_eq!(device_ids, [0, 1, 2, 3, 0, 1, 2, 3]);
    for device_id in device_ids {
        assert!(matches!(
            cli::Backend::Cuda.provider(device_id).unwrap(),
            minase::nn::ExecutionProvider::Cuda { device_id: actual } if actual == device_id
        ));
    }
    for thread in 0..8 {
        assert_eq!(device_for_thread(thread, nz(1)), 0);
    }
}

// [`output::generate`]の契約: 推論器の生成関数にスレッド番号を0から順に1回ずつ渡す。
#[test]
fn evaluator_factory_receives_each_thread_index_in_order() {
    let directory = TemporaryDirectory::new();
    let config = Config {
        games: 2,
        playthrough_percent: 100,
        max_ply: 18,
        ..config()
    };
    let mut created = Vec::new();
    let report = output::generate(
        &directory.0,
        &config,
        u64::MAX,
        &CHECKSUM,
        nz(8),
        nz(1),
        |thread| {
            created.push(thread);
            Ok(Box::new(UniformEvaluator))
        },
    )
    .unwrap();
    assert_eq!(created, [0, 1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(report.generated_games, 2);
}
