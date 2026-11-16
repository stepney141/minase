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
        start_position: cli::StartPosition::Initial,
        games: 6,
        chunk_games: 2,
        simulations: 4,
        resign_threshold: 0.01,
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

fn all_records(path: &Path, config: &Config) -> Vec<DecodedGame> {
    (0..config.games.div_ceil(config.chunk_games))
        .flat_map(|chunk| read_games(&path.join(format!("chunk-{chunk:05}.mnsd"))).1)
        .collect()
}

#[test]
fn all_searches_and_resignations_are_recorded_in_one_training_file_per_chunk() {
    for threshold in [0.0, 0.01] {
        let directory = TemporaryDirectory::new();
        let config = Config {
            games: 22,
            chunk_games: 7,
            resign_threshold: threshold,
            simulations: 1,
            max_ply: 12,
            ..config()
        };
        let report = generate(&directory.0, &config, 2, 3);
        let counts = &report.summary.counts;
        let games = all_records(&directory.0, &config);
        assert_eq!(games.len(), 22);
        assert_eq!(counts.no_resign_games, 3);
        assert_eq!(
            counts
                .resign_calibration_games
                .iter()
                .map(|g| g.game)
                .collect::<Vec<_>>(),
            [1, 11, 21]
        );
        let mut positions = 0;
        for decoded in games {
            let record = decoded.record();
            let no_resign = [1, 11, 21].contains(&record.game_number());
            let resigned = threshold > 0.0 && !no_resign;
            assert_eq!(record.start_ply(), 0);
            assert_eq!(record.start_position(), &minase_core::Position::initial());
            assert_eq!(record.origin(), records::Origin::RandomSelfplay);
            assert_eq!(
                record.outcome(),
                if resigned {
                    Outcome::Game(GameResult::Win {
                        winner: Color::White,
                        reason: WinReason::Resignation,
                    })
                } else {
                    Outcome::MaxPlyDraw
                }
            );
            let searched = if resigned { 11 } else { 12 };
            assert_eq!(record.entries().len(), searched + usize::from(!resigned));
            for (ply, entry) in record.entries()[..searched].iter().enumerate() {
                assert_eq!(entry.played_move().is_none(), resigned && ply == 10);
                assert_eq!(
                    entry.value(),
                    if ply % 2 == 0 {
                        Value::Mcts(66)
                    } else {
                        Value::Mcts(65469)
                    }
                );
                let Policy::Candidates(candidates) = entry.policy() else {
                    panic!("every search has a policy")
                };
                assert_eq!(
                    candidates.iter().map(|(_, p)| u32::from(*p)).sum::<u32>(),
                    65535
                );
                assert_eq!(entry.visits(), Some(1));
            }
            if !resigned {
                let last = record.entries().last().unwrap();
                assert_eq!(
                    (
                        last.played_move(),
                        last.value(),
                        last.policy(),
                        last.visits()
                    ),
                    (None, Value::None, &Policy::None, Some(0))
                );
            }
            positions += searched as u64;
        }
        assert_eq!(counts.positions, positions);
        assert_eq!(counts.searches, positions);
        assert_eq!(report.summary.training_positions, positions);
        assert_eq!(
            counts.win_reason_resignation,
            if threshold > 0.0 { 19 } else { 0 }
        );
        assert_eq!(
            counts.draw_reason_max_ply,
            if threshold > 0.0 { 3 } else { 22 }
        );
        for item in &counts.resign_calibration_games {
            assert_eq!(item.result, game::CalibrationResult::Draw);
            assert_eq!(item.min_value_black, f64::from(1.0_f32 - 0.999_f32));
            assert_eq!(item.min_value_white, f64::from(1.0_f32 - 0.001_f32));
        }
        for chunk in 0..4 {
            let (header, _) = read_games(&directory.0.join(format!("chunk-{chunk:05}.mnsd")));
            assert_eq!(header.teacher_kind(), TeacherKind::Mcts);
            assert_eq!(header.teacher_search_budget(), 1);
            assert_eq!(header.seed(), config.seed);
            assert_eq!(header.rule_set(), config.rules);
            assert_eq!(header.generation_commit(), config.generation_commit);
            assert_eq!(header.network_checksum(), &CHECKSUM);
            assert!(
                !directory
                    .0
                    .join(format!("chunk-{chunk:05}.resigned.mnsd"))
                    .exists()
            );
        }
        let saved: Summary =
            serde_json::from_slice(&fs::read(directory.0.join("summary.json")).unwrap()).unwrap();
        assert_eq!(saved, report.summary);
    }
}

#[test]
fn resume_regenerates_only_missing_chunk_deterministically_across_parallelism() {
    for start_position in [
        cli::StartPosition::Initial,
        cli::StartPosition::ShuffledSetup,
    ] {
        let directory = TemporaryDirectory::new();
        let config = Config {
            start_position,
            ..config()
        };
        let first = generate(&directory.0, &config, 2, 2);
        assert!(first.summary.counts.win_reason_resignation > 0);
        let repeated_directory = TemporaryDirectory::new();
        let repeated = generate(&repeated_directory.0, &config, 1, 3);
        assert_eq!(repeated.summary, first.summary);
        let mut before = Vec::new();
        let old_time = UNIX_EPOCH + Duration::from_secs(1_000_000);
        for chunk in 0..3 {
            for suffix in [".mnsd", ".json"] {
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
            directory.0.join("chunk-00001.mnsd.tmp"),
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
fn reaching_target_stops_generation_at_the_first_complete_chunk() {
    let directory = TemporaryDirectory::new();
    let config = Config {
        seed: 1,
        ..config()
    };
    let report = output::generate(&directory.0, &config, 1, &CHECKSUM, nz(1), nz(1), |_| {
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
    assert!(saved.counts.positions >= 1);
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
    assert_eq!(report.generated_games, committed_games);
    assert_eq!(committed_games, saved.counts.games);
    assert!(report.generated_games < u64::from(config.games));
    assert_eq!(saved.training_files, ["chunk-00000.mnsd"]);
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
fn inference_batches_contain_multiple_games_and_max_ply_draws_keep_records() {
    let directory = TemporaryDirectory::new();
    let config = Config {
        games: 4,
        resign_threshold: 0.0,
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
    assert_eq!(report.summary.counts.draw_reason_max_ply, 4);
    assert_eq!(report.summary.counts.plies, 4 * 18);
    assert_eq!(report.summary.counts.positions, 4 * 18);
    assert_eq!(report.summary.counts.win_reason_resignation, 0);
    assert_eq!(all_records(&directory.0, &config).len(), 4);
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
fn shuffled_records_start_at_seeded_setup_and_record_every_search() {
    let directory = TemporaryDirectory::new();
    let config = Config {
        start_position: cli::StartPosition::ShuffledSetup,
        games: 2,
        max_ply: 12,
        simulations: 1,
        ..config()
    };
    generate(&directory.0, &config, 2, 2);
    let mut starts = Vec::new();
    for decoded in all_records(&directory.0, &config) {
        let record = decoded.record();
        let seed = derive_seed(config.seed, u64::from(record.game_number()));
        let expected = minase_core::Position::shuffled_initial(&mut XorShift64::new(derive_seed(
            seed.get(),
            0,
        )));
        assert_eq!(record.start_ply(), 0);
        assert_eq!(record.start_position(), &expected);
        assert_eq!(record.origin(), records::Origin::RandomSelfplay);
        let first = &record.entries()[0];
        assert!(matches!(first.value(), Value::Mcts(_)));
        assert!(matches!(first.policy(), Policy::Candidates(_)));
        assert_eq!(first.visits(), Some(1));
        starts.push(expected);
    }
    assert_ne!(starts[0], starts[1]);
}

#[test]
fn start_position_is_saved_in_config_and_summary_with_cli_spelling() {
    for (start_position, spelling) in [
        (cli::StartPosition::ShuffledSetup, "shuffled-setup"),
        (cli::StartPosition::Initial, "initial"),
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
        "--resign-threshold",
        "0",
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
    assert!(
        Cli::try_parse_from(
            required
                .into_iter()
                .chain(["--start-position", "random-opening"])
        )
        .is_err()
    );
    let complete: Vec<_> = required
        .into_iter()
        .chain(["--start-position", "initial"])
        .collect();
    for index in (1..complete.len()).step_by(2) {
        let mut missing = complete.clone();
        missing.drain(index..index + 2);
        assert_eq!(
            Cli::try_parse_from(missing).err().unwrap().kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }
    for threshold in ["NaN", "inf", "-inf", "-0.1", "1.1"] {
        let mut args = required.to_vec();
        let index = args
            .iter()
            .position(|arg| *arg == "--resign-threshold")
            .unwrap();
        args[index + 1] = threshold;
        assert!(
            Cli::try_parse_from(args.into_iter().chain(["--start-position", "initial"])).is_err()
        );
    }
    for simulations in ["1", "3", "5"] {
        let mut args = required.to_vec();
        let index = args.iter().position(|arg| *arg == "--simulations").unwrap();
        args[index + 1] = simulations;
        assert!(
            Cli::try_parse_from(args.into_iter().chain(["--start-position", "initial"])).is_ok()
        );
    }
    for (spelling, kind) in [
        ("shuffled-setup", cli::StartPosition::ShuffledSetup),
        ("initial", cli::StartPosition::Initial),
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
        start_position: cli::StartPosition::Initial,
        games: 2,
        target_positions: 100,
        chunk_games: 2,
        simulations: 4,
        resign_threshold: 0.0,
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
        resign_threshold: 0.0,
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
