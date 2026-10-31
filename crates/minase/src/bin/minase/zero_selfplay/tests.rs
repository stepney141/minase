//! 投了の有効・無効、MNSDの内容、手数上限、および欠けた塊だけの再生成を検査する。
//! 学習済みのネットを使わず、本番のEvaluator境界から決定的な値を与える。

use super::*;
use std::fs::{self, File, FileTimes};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use minase::nn::{Evaluator, NnError, Query, UniformEvaluator};
use minase::training::records::{DecodedGame, Header, Outcome, Policy, Reader, TeacherKind, Value};
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
        rules: engine_default_rules().unwrap().to_string(),
        generation_commit: "0123456789abcdef0123456789abcdef01234567".into(),
        network_sha256: minase::datagen::hex(&CHECKSUM),
        nn_backend: "cpu".into(),
        seed: 7,
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
    output::generate(path, config, &CHECKSUM, nz(threads), nz(batch), || {
        Ok(Box::new(BiasedEvaluator))
    })
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
    let (game, _) =
        minase::datagen::opening::generate_opening(engine_default_rules().unwrap(), seed).unwrap();
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
        let (_, opening) =
            minase::datagen::opening::generate_opening(engine_default_rules().unwrap(), seed)
                .unwrap();
        let opening_len = opening.len();
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
        let mut replay = minase_core::Game::new(engine_default_rules().unwrap());
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
    assert_eq!(saved_config.as_object().unwrap().len(), 10);
    for absent in ["version", "threads", "games_per_thread"] {
        assert!(saved_config.get(absent).is_none());
    }
}

#[test]
fn resume_regenerates_only_missing_chunk_deterministically_across_parallelism() {
    let directory = TemporaryDirectory::new();
    let config = config();
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
    let complete = output::generate(&directory.0, &config, &CHECKSUM, nz(3), nz(4), || {
        panic!("completed cycles must not construct an evaluator")
    })
    .unwrap();
    assert_eq!(complete.generated_games, 0);
    assert_eq!(complete.generated_positions, 0);
    assert_eq!(complete.summary, first.summary);
    let changed = Config {
        seed: config.seed + 1,
        ..config.clone()
    };
    let error = output::generate(&directory.0, &changed, &CHECKSUM, nz(1), nz(1), || {
        panic!("mismatched conditions must fail before inference")
    })
    .err()
    .unwrap();
    assert!(matches!(error, Error::ConfigMismatch(fields) if fields == ["seed"]));
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
    let report = output::generate(&directory.0, &config, &CHECKSUM, nz(1), nz(3), || {
        Ok(Box::new(BatchRecorder(Arc::clone(&batches))))
    })
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
            Failure::Inference => return Err(NnError::PromotionDeferred),
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
        let error = output::generate(&directory.0, &config, &CHECKSUM, nz(2), nz(2), || {
            Ok(Box::new(FailingEvaluator {
                failure,
                dropped: Arc::clone(&dropped),
            }))
        })
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
                    Error::Inference(NnError::PromotionDeferred)
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
