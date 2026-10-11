//! サイクルの生成を塊に分けて進め、確定済みの塊を再開時に残し、全体のサマリを表示する。

use super::chunk::{
    ChunkPlan, ChunkSummary, Context, Counts, EvaluatorFactory, generate_chunk, ratio,
};
use super::cli::{Arguments, EvaluatorKind};
use super::play::GameSettings;
use crate::selfplay_gen::opening::read_openings;
use minase::datagen::git::{git_output, validate_commit_hash};
use minase::datagen::provenance::hex;
use minase::datagen::{data_error, invalid_data};
use minase::nn::mnzt;
use std::{
    fs::{self, File},
    io::{self, BufReader},
};

/// 生成の結果。
#[derive(Debug)]
pub(super) struct Report {
    /// 塊ごとのサマリ。再開時に残した塊を含む。
    pub(super) chunks: Vec<ChunkSummary>,
    /// この実行で生成した塊の数。
    pub(super) generated: usize,
}

/// 引数の条件で全塊を生成し、全体のサマリを標準出力へ表示する。
///
/// `evaluator`はスレッドごとに推論器を作り、`network_checksum`はそのネットの検査和である。
pub(super) fn generate(
    arguments: &Arguments,
    evaluator: &EvaluatorFactory<'_>,
    network_checksum: [u8; 32],
) -> io::Result<Report> {
    if arguments.nn_model.is_some() && arguments.evaluator == EvaluatorKind::Uniform {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--nn-model cannot be used with --evaluator uniform",
        ));
    }
    if arguments.threads > arguments.games_in_flight {
        return Err(invalid_data(format!(
            "--threads {} exceeds --games-in-flight {}",
            arguments.threads, arguments.games_in_flight
        )));
    }
    let openings = arguments
        .openings
        .as_ref()
        .map(|path| read_openings(path))
        .transpose()?;
    let generation_commit = git_output(&["rev-parse", "HEAD"])?;
    validate_commit_hash(&generation_commit)?;
    if !arguments.allow_dirty && !git_output(&["status", "--porcelain"])?.is_empty() {
        return Err(invalid_data(
            "the working tree is dirty; commit changes or pass --allow-dirty",
        ));
    }
    let rule_set = arguments.rules.to_string();
    let plans = chunk_plans(arguments);
    fs::create_dir_all(&arguments.out_dir)?;
    if !arguments.resume
        && let Some(plan) = plans.iter().find(|plan| {
            plan.path(&arguments.out_dir, "mnzt").exists()
                || plan.path(&arguments.out_dir, "json").exists()
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "chunk {} already exists in {}; pass --resume to keep the finished chunks",
                plan.index,
                arguments.out_dir.display()
            ),
        ));
    }
    let context = Context {
        settings: GameSettings {
            rules: arguments.rules,
            simulations: arguments.simulations,
            max_ply: arguments.max_ply,
            resign_threshold: arguments.resign_threshold,
            resign_playthrough: arguments.resign_playthrough,
        },
        openings: openings.as_deref(),
        games_in_flight: arguments.games_in_flight,
        threads: arguments.threads,
        nn_batch_size: arguments.nn_batch_size,
        out_dir: &arguments.out_dir,
        rule_set: &rule_set,
        generation_commit: &generation_commit,
        network_path: arguments.nn_model.as_deref(),
        network_checksum,
        evaluator,
    };

    let mut report = Report {
        chunks: Vec::with_capacity(plans.len()),
        generated: 0,
    };
    for plan in &plans {
        let summary = if arguments.resume && plan.path(&arguments.out_dir, "mnzt").exists() {
            read_finished_chunk(&context, plan)?
        } else {
            report.generated += 1;
            generate_chunk(&context, plan)?
        };
        eprintln!(
            "progress: chunk={}/{} games={} positions={} elapsed_seconds={:.3}",
            plan.index + 1,
            plans.len(),
            summary.counts.games,
            summary.counts.positions,
            summary.elapsed_seconds
        );
        report.chunks.push(summary);
    }
    print_summary(
        arguments,
        &rule_set,
        &generation_commit,
        &network_checksum,
        &report,
    );
    Ok(report)
}

/// 塊の番号、シード、および対局数を並べる。
///
/// 塊のシードは基本シードに塊の番号×塊の対局数×2を足した値とし、局のシードの系列が
/// 塊の間で重ならないようにする（docs/lessons/derive-seed-adjacent-collision.md）。
fn chunk_plans(arguments: &Arguments) -> Vec<ChunkPlan> {
    let games = arguments.games.get();
    let chunk_games = arguments.chunk_games.get();
    (0..games.div_ceil(chunk_games))
        .map(|index| {
            let first_game = u64::from(index) * u64::from(chunk_games);
            ChunkPlan {
                index,
                seed: arguments.seed.wrapping_add(first_game.wrapping_mul(2)),
                games: chunk_games.min(games - index * chunk_games),
                first_game,
            }
        })
        .collect()
}

/// 確定済みの塊のサマリを読み、MNZTのヘッダとともに現在の条件と一致することを確かめる。
fn read_finished_chunk(context: &Context<'_>, plan: &ChunkPlan) -> io::Result<ChunkSummary> {
    let summary_path = plan.path(context.out_dir, "json");
    let summary: ChunkSummary = serde_json::from_reader(BufReader::new(File::open(&summary_path)?))
        .map_err(|error| invalid_data(format!("{}: {error}", summary_path.display())))?;
    if summary.chunk != plan.index
        || summary.seed != plan.seed
        || summary.counts.games != u64::from(plan.games)
        || summary.network_checksum != hex(&context.network_checksum)
    {
        return Err(invalid_data(format!(
            "{} was generated for chunk {} with seed {}, {} games, and network {}, \
             but the current arguments give chunk {} with seed {}, {} games, and network {}",
            summary_path.display(),
            summary.chunk,
            summary.seed,
            summary.counts.games,
            summary.network_checksum,
            plan.index,
            plan.seed,
            plan.games,
            hex(&context.network_checksum)
        )));
    }
    let mnzt_path = plan.path(context.out_dir, "mnzt");
    let reader = mnzt::Reader::new(File::open(&mnzt_path)?).map_err(data_error)?;
    let actual = reader.header();
    let expected = context.header(plan.seed)?;
    if actual.rule_set() != expected.rule_set()
        || actual.generation_commit() != expected.generation_commit()
        || actual.network_checksum() != expected.network_checksum()
        || actual.teacher() != expected.teacher()
        || actual.seed() != expected.seed()
        || actual.record_count() != summary.counts.positions
    {
        return Err(invalid_data(format!(
            "{} does not match the current generation: rules {}, commit {}, teacher {:?}, \
             seed {}, records {} (expected rules {}, commit {}, teacher {:?}, seed {}, \
             records {} from the chunk summary, and the same network checksum)",
            mnzt_path.display(),
            actual.rule_set(),
            actual.generation_commit(),
            actual.teacher(),
            actual.seed(),
            actual.record_count(),
            expected.rule_set(),
            expected.generation_commit(),
            expected.teacher(),
            expected.seed(),
            summary.counts.positions
        )));
    }
    Ok(summary)
}

/// 全塊の合計を設計書へ転記できる1項目1行の形式で表示する。
fn print_summary(
    arguments: &Arguments,
    rule_set: &str,
    generation_commit: &str,
    network_checksum: &[u8; 32],
    report: &Report,
) {
    let mut counts = Counts::new();
    let mut elapsed_seconds = 0.0;
    for chunk in &report.chunks {
        counts.merge(&chunk.counts);
        elapsed_seconds += chunk.elapsed_seconds;
    }
    println!("summary:");
    println!("seed: {}", arguments.seed);
    println!("games: {}", arguments.games);
    println!("chunk_games: {}", arguments.chunk_games);
    println!("chunks: {}", report.chunks.len());
    println!("chunks_generated: {}", report.generated);
    println!("chunks_reused: {}", report.chunks.len() - report.generated);
    println!("games_in_flight: {}", arguments.games_in_flight);
    println!("threads: {}", arguments.threads);
    println!("simulations: {}", arguments.simulations);
    println!("max_ply: {}", arguments.max_ply);
    println!("resign_threshold: {}", arguments.resign_threshold);
    println!("resign_playthrough: {}", arguments.resign_playthrough);
    println!("evaluator: {}", arguments.evaluator.name());
    match &arguments.nn_model {
        Some(path) => println!("nn_model: {}", path.display()),
        None => println!("nn_model: none"),
    }
    println!("network_checksum: {}", hex(network_checksum));
    println!("rules: {rule_set}");
    println!("commit: {generation_commit}");
    println!("games_completed: {}", counts.games);
    println!("positions: {}", counts.positions);
    for (name, count) in &counts.endings {
        println!("ending_{name}: {count}");
    }
    println!("games_discarded_max_ply: {}", counts.max_ply_games);
    println!("games_resigned: {}", counts.resigned_games);
    println!("games_playthrough: {}", counts.playthrough_games);
    println!(
        "false_resign_candidates: {}",
        counts.false_resign_candidates
    );
    println!("false_resignations: {}", counts.false_resignations);
    if counts.false_resign_candidates == 0 {
        println!("false_resign_rate_percent: n/a");
    } else {
        println!(
            "false_resign_rate_percent: {:.6}",
            100.0 * ratio(counts.false_resignations, counts.false_resign_candidates)
        );
    }
    println!("average_plies: {:.6}", counts.average_plies());
    println!("elapsed_seconds: {elapsed_seconds:.6}");
    println!(
        "positions_per_second: {:.6}",
        if elapsed_seconds > 0.0 {
            counts.positions as f64 / elapsed_seconds
        } else {
            0.0
        }
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zero_selfplay::cli::EvaluatorKind;
    use minase::nn::POLICY_SIZE;
    use minase::nn::evaluator::{Evaluator, EvaluatorError, UniformEvaluator};
    use minase::nn::ort_evaluator::NnBackend;
    use minase::nn::policy;
    use minase::training::records::Outcome;
    use minase_core::{Game, Rules};
    use sha2::{Digest, Sha256};
    use std::{
        collections::{BTreeMap, BTreeSet},
        num::{NonZeroU16, NonZeroU32, NonZeroUsize},
        path::PathBuf,
        process,
        sync::atomic::{AtomicU64, Ordering},
    };

    /// 試験ごとに分けた一時ディレクトリ。
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "minase-zero-selfplay-{}-{}",
                process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// 指示書の小さな条件の引数を作る。
    fn arguments(out_dir: PathBuf) -> Arguments {
        Arguments {
            out_dir,
            games: NonZeroU32::new(6).unwrap(),
            chunk_games: NonZeroU32::new(3).unwrap(),
            seed: 11,
            rules: Rules::ENGINE_DEFAULT,
            games_in_flight: NonZeroUsize::new(3).unwrap(),
            threads: NonZeroUsize::new(1).unwrap(),
            simulations: NonZeroU32::new(8).unwrap(),
            max_ply: NonZeroU16::new(200).unwrap(),
            resign_threshold: 0.01,
            resign_playthrough: 0.2,
            openings: None,
            evaluator: EvaluatorKind::Uniform,
            nn_backend: NnBackend::Cpu,
            nn_model: None,
            nn_cache_dir: None,
            nn_batch_size: NonZeroUsize::new(256).unwrap(),
            resume: false,
            allow_dirty: true,
        }
    }

    /// 一様評価器で生成する。
    fn generate_uniform(arguments: &Arguments) -> io::Result<Report> {
        generate(
            arguments,
            &|| Ok(Box::new(UniformEvaluator) as Box<dyn Evaluator>),
            [0; 32],
        )
    }

    /// 塊のMNZTの全レコードを読む。
    fn read_records(path: &std::path::Path) -> (mnzt::Header, Vec<mnzt::Record>) {
        let reader = mnzt::Reader::new(File::open(path).unwrap()).unwrap();
        let header = reader.header().clone();
        (header, reader.collect::<Result<Vec<_>, _>>().unwrap())
    }

    /// 塊のMNZTとサマリの経路を返す。
    fn chunk_path(directory: &std::path::Path, index: u32, extension: &str) -> PathBuf {
        directory.join(format!("chunk-{index:05}.{extension}"))
    }

    /// 塊のサマリを読む。
    fn read_summary(directory: &std::path::Path, index: u32) -> ChunkSummary {
        serde_json::from_reader(File::open(chunk_path(directory, index, "json")).unwrap()).unwrap()
    }

    /// 同じ順に並んだ対局番号ごとのレコードを返す。
    fn records_by_game(records: &[mnzt::Record]) -> BTreeMap<u32, Vec<&mnzt::Record>> {
        let mut games = BTreeMap::<u32, Vec<&mnzt::Record>>::new();
        for record in records {
            games.entry(record.game_number()).or_default().push(record);
        }
        games
    }

    /// 指示書「試験」: 2塊が書かれ、各レコードの方策の確率の和が1に近く、ラベルが合法手の集合に含まれ、
    /// 価値が0〜1である。塊のサマリの対局数と記録数がMNZTと一致する。
    #[test]
    fn writes_chunks_with_complete_policy_targets() {
        let directory = TempDir::new();
        let arguments = arguments(directory.0.clone());
        let report = generate_uniform(&arguments).unwrap();
        assert_eq!(report.chunks.len(), 2);
        assert_eq!(report.generated, 2);
        let mut total_records = 0;
        for index in 0..2 {
            let (header, records) = read_records(&chunk_path(&directory.0, index, "mnzt"));
            let summary = read_summary(&directory.0, index);
            assert_eq!(summary, report.chunks[index as usize]);
            assert_eq!(header.rule_set(), "L0,P0,R1,E0");
            assert_eq!(header.network_checksum(), &[0; 32]);
            assert_eq!(summary.nn_model, None);
            assert_eq!(summary.network_checksum, "0".repeat(64));
            assert_eq!(header.teacher(), mnzt::Teacher::MctsSimulations(8));
            assert_eq!(header.seed(), 11 + u64::from(index) * 3 * 2);
            assert_eq!(summary.seed, header.seed());
            assert_eq!(summary.counts.games, 3);
            assert_eq!(summary.counts.positions, header.record_count());
            assert_eq!(
                summary.counts.games,
                summary.counts.max_ply_games + summary.counts.endings.values().sum::<u64>()
            );
            for (number, game) in records_by_game(&records) {
                assert!((1..=3).contains(&number));
                assert!(game.windows(2).all(|pair| pair[0].ply() < pair[1].ply()));
            }
            for record in &records {
                let sum: f32 = record
                    .policy()
                    .iter()
                    .map(|entry| entry.probability())
                    .sum();
                assert!((sum - 1.0).abs() < 1e-4, "{sum}");
                assert!((0.0..=1.0).contains(&record.value()));
                let position = record.to_position().unwrap();
                let side = position.side_to_move();
                let legal: BTreeSet<u16> = Game::from_position(Rules::ENGINE_DEFAULT, position)
                    .legal_moves()
                    .into_iter()
                    .map(|mv| policy::label(mv, side).unwrap())
                    .collect();
                assert!(
                    record
                        .policy()
                        .iter()
                        .all(|entry| legal.contains(&entry.label()))
                );
            }
            total_records += records.len();
            for extension in ["mnzt.tmp", "mnzt.policy.tmp", "json.tmp"] {
                assert!(!chunk_path(&directory.0, index, extension).exists());
            }
        }
        assert!(total_records > 0);
    }

    /// 指示書「試験」: 塊1を確定した後に止まった状態から再開すると、確定済みの塊を生成し直さず、
    /// 欠けた塊だけを作る。再開を指定しなければ既存の塊を上書きしない。
    #[test]
    fn resume_keeps_finished_chunks_and_regenerates_missing_ones() {
        let directory = TempDir::new();
        let mut arguments = arguments(directory.0.clone());
        generate_uniform(&arguments).unwrap();
        let first = chunk_path(&directory.0, 0, "mnzt");
        let second = chunk_path(&directory.0, 1, "mnzt");
        let first_sha = Sha256::digest(fs::read(&first).unwrap());
        let second_bytes = fs::read(&second).unwrap();
        let first_modified = fs::metadata(&first).unwrap().modified().unwrap();

        // 塊2の書き込み中に止まった状態を作る。
        fs::remove_file(&second).unwrap();
        fs::remove_file(chunk_path(&directory.0, 1, "json")).unwrap();
        fs::write(chunk_path(&directory.0, 1, "mnzt.tmp"), b"partial").unwrap();

        let error = generate_uniform(&arguments).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);

        arguments.resume = true;
        let report = generate_uniform(&arguments).unwrap();
        assert_eq!(report.generated, 1);
        assert_eq!(report.chunks.len(), 2);
        assert_eq!(Sha256::digest(fs::read(&first).unwrap()), first_sha);
        assert_eq!(
            fs::metadata(&first).unwrap().modified().unwrap(),
            first_modified
        );
        assert_eq!(fs::read(&second).unwrap(), second_bytes);
        assert!(!chunk_path(&directory.0, 1, "mnzt.tmp").exists());

        // 条件の異なる再開は、確定済みの塊と混ぜずに拒否する。
        arguments.seed = 12;
        assert!(generate_uniform(&arguments).is_err());
    }

    /// 指示書N「試験」: `--nn-model`と`--evaluator uniform`の併用は、塊を書く前に引数の誤りとして拒否する。
    #[test]
    fn nn_model_is_rejected_with_the_uniform_evaluator() {
        let directory = TempDir::new();
        let mut arguments = arguments(directory.0.join("cycle"));
        arguments.nn_model = Some(directory.0.join("net.onnx"));
        let error = generate_uniform(&arguments).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!arguments.out_dir.exists());
    }

    /// 指示書N「試験」: 塊のサマリには、推論に使ったネットのファイルと検査和を記録し、検査和はMNZTの
    /// ヘッダと一致する。推論器は差し替えるので、ネットのファイルは読まない。
    #[test]
    fn chunk_summaries_record_the_model_path_and_checksum() {
        let directory = TempDir::new();
        let mut arguments = arguments(directory.0.join("cycle"));
        arguments.evaluator = EvaluatorKind::Ort;
        arguments.nn_model = Some(PathBuf::from("nets/cycle-10.onnx"));
        arguments.games = NonZeroU32::new(3).unwrap();
        generate(
            &arguments,
            &|| Ok(Box::new(UniformEvaluator) as Box<dyn Evaluator>),
            [7; 32],
        )
        .unwrap();
        let json: serde_json::Value =
            serde_json::from_reader(File::open(chunk_path(&arguments.out_dir, 0, "json")).unwrap())
                .unwrap();
        assert_eq!(json["nn_model"], "nets/cycle-10.onnx");
        assert_eq!(json["network_checksum"], "07".repeat(32));
        let (header, _) = read_records(&chunk_path(&arguments.out_dir, 0, "mnzt"));
        assert_eq!(header.network_checksum(), &[7; 32]);

        // 検査和の異なるネットでの再開は、確定済みの塊と混ぜずに拒否する。
        arguments.resume = true;
        assert!(
            generate(
                &arguments,
                &|| Ok(Box::new(UniformEvaluator) as Box<dyn Evaluator>),
                [8; 32],
            )
            .is_err()
        );
    }

    /// 指示書「試験」: 手数の上限20では全対局が上限に達し、記録は0件で、上限到達の件数が対局数と一致する。
    #[test]
    fn games_reaching_the_ply_limit_are_not_recorded() {
        let directory = TempDir::new();
        let mut arguments = arguments(directory.0.clone());
        arguments.max_ply = NonZeroU16::new(20).unwrap();
        let report = generate_uniform(&arguments).unwrap();
        for (index, summary) in report.chunks.iter().enumerate() {
            let (header, _) = read_records(&chunk_path(&directory.0, index as u32, "mnzt"));
            assert_eq!(header.record_count(), 0);
            assert_eq!(summary.counts.positions, 0);
            assert_eq!(summary.counts.max_ply_games, summary.counts.games);
            assert_eq!(summary.counts.games, 3);
            assert_eq!(summary.average_plies, 20.0);
        }
    }

    /// すべての方策logitを0、勝率を一定値とする評価器。
    struct ConstantValueEvaluator(f32);

    impl Evaluator for ConstantValueEvaluator {
        fn evaluate(
            &mut self,
            _: &[f32],
            batch: usize,
            out_policy: &mut [f32],
            out_value: &mut [f32],
        ) -> Result<(), EvaluatorError> {
            assert_eq!(out_policy.len(), batch * POLICY_SIZE);
            out_policy.fill(0.0);
            out_value.fill(self.0);
            Ok(())
        }
    }

    /// 指示書「試験」: 根の勝率が閾値を下回った側が投了の負けとして記録され、最後まで指させる局は投了せず、
    /// 誤った投了の件数が定義どおりに数えられる。
    ///
    /// 葉の勝率を0.995とすると、根の各辺のQは1−0.995＝0.005となり、閾値0.01を下回る。
    /// したがって投了を許した局は最初に探索した局面で手番側が投了し、最後まで指させる局は最初の手番側で
    /// 投了の条件が成立する。
    #[test]
    fn the_side_below_the_threshold_resigns_except_in_playthrough_games() {
        let directory = TempDir::new();
        let mut arguments = arguments(directory.0.clone());
        arguments.games = NonZeroU32::new(10).unwrap();
        arguments.chunk_games = NonZeroU32::new(10).unwrap();
        arguments.games_in_flight = NonZeroUsize::new(4).unwrap();
        arguments.resign_playthrough = 0.5;
        arguments.max_ply = NonZeroU16::new(120).unwrap();
        let report = generate(
            &arguments,
            &|| Ok(Box::new(ConstantValueEvaluator(0.995)) as Box<dyn Evaluator>),
            [0; 32],
        )
        .unwrap();
        let counts = &report.chunks[0].counts;
        assert_eq!(counts.games, 10);
        assert!(counts.playthrough_games > 0 && counts.playthrough_games < 10);
        assert_eq!(counts.resigned_games, 10 - counts.playthrough_games);
        assert_eq!(counts.endings["win_resignation"], counts.resigned_games);
        assert_eq!(counts.false_resign_candidates, counts.playthrough_games);
        // 最後まで指させた局で投了の条件が成立した側が負けた局は、その側の負けで終局した局だけである。
        let (_, records) = read_records(&chunk_path(&directory.0, 0, "mnzt"));
        let games = records_by_game(&records);
        let mut resigned = 0;
        let mut trigger_lost = 0;
        for game in games.values() {
            if game.len() == 1 && game[0].value() < 0.01 {
                // 投了した局は最初に探索した局面だけを持ち、その手番側の負けである。
                assert_eq!(game[0].outcome(), Outcome::Loss);
                resigned += 1;
            } else if game[0].outcome() == Outcome::Loss {
                // 最後まで指させた局の最初の手番側が、投了の条件が成立した側である。
                trigger_lost += 1;
            }
        }
        assert_eq!(resigned, counts.resigned_games);
        assert_eq!(
            counts.false_resignations,
            counts.false_resign_candidates - trigger_lost
        );
    }

    /// 実戦開始の局面一覧を指定すると、対局の通し番号の順に一覧の局面を循環して使い、
    /// 記録の手数は元の棋譜の手数から数える。
    #[test]
    fn openings_are_used_in_turn_and_keep_their_original_ply() {
        use minase_core::notation::sfen::{SetupPosition, to_extended_sfen};

        let directory = TempDir::new();
        let mut lines = String::new();
        let mut game = Game::new(Rules::ENGINE_DEFAULT);
        for (index, plies) in [(1, 30_u32), (2, 41)] {
            while game.ply_count() < plies {
                let mv = game.legal_moves()[0];
                game.play(mv).unwrap();
            }
            let setup = SetupPosition::new(
                game.position().clone(),
                game.position().lion_capture_square(),
                plies + 1,
            )
            .unwrap();
            lines.push_str(&format!(
                "line{index} {plies} {}\n",
                to_extended_sfen(&setup)
            ));
        }
        let path = directory.0.join("openings.txt");
        fs::write(&path, lines).unwrap();
        let mut arguments = arguments(directory.0.join("cycle"));
        arguments.openings = Some(path);
        arguments.games = NonZeroU32::new(3).unwrap();
        arguments.chunk_games = NonZeroU32::new(3).unwrap();
        arguments.max_ply = NonZeroU16::new(400).unwrap();
        generate_uniform(&arguments).unwrap();
        let (_, records) = read_records(&chunk_path(&arguments.out_dir, 0, "mnzt"));
        let first_plies: BTreeMap<u32, u16> = records_by_game(&records)
            .into_iter()
            .map(|(number, game)| (number, game[0].ply()))
            .collect();
        for (number, ply) in first_plies {
            let expected = if number == 2 { 41 } else { 30 };
            assert_eq!(ply, expected, "game {number}");
        }
    }

    /// 指示書「試験」: 同じシードの2回の生成は、`--threads 1`でMNZTのバイト列が一致する。
    #[test]
    fn generation_is_deterministic_with_one_thread() {
        let bytes = || {
            let directory = TempDir::new();
            generate_uniform(&arguments(directory.0.clone())).unwrap();
            (0..2)
                .map(|index| fs::read(chunk_path(&directory.0, index, "mnzt")).unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(bytes(), bytes());
    }

    /// 指示書「試験」: 投了も手数の上限もない条件では、一様評価器の対局が審判層の規則による終局で終わる。
    #[test]
    fn uniform_games_end_by_the_rules_without_resignation_or_ply_limit() {
        let directory = TempDir::new();
        let mut arguments = arguments(directory.0.clone());
        arguments.games = NonZeroU32::new(2).unwrap();
        arguments.chunk_games = NonZeroU32::new(2).unwrap();
        arguments.games_in_flight = NonZeroUsize::new(2).unwrap();
        arguments.resign_threshold = 0.0;
        arguments.max_ply = NonZeroU16::MAX;
        let report = generate_uniform(&arguments).unwrap();
        let counts = &report.chunks[0].counts;
        assert_eq!(counts.games, 2);
        assert_eq!(counts.max_ply_games, 0);
        assert_eq!(counts.resigned_games, 0);
        assert_eq!(counts.endings.values().sum::<u64>(), 2);
        assert!(counts.positions > 0);
    }
}
