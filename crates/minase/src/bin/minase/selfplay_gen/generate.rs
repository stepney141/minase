//! 自己対局データの並列生成と要約の出力。

use super::{
    cli::GenerateArguments,
    engine_default_rules,
    opening::{Opening, OpeningError, read_openings},
    play::{PlaySettings, PlayedGame, play_game},
    statistics::{format_rate_percent, score_mean_and_std, score_percentile},
};
use minase::datagen::game::merge_completed_games;
use minase::datagen::git::{git_output, validate_commit_hash};
use minase::datagen::provenance::{ProvenanceArguments, write_mapped_provenance};
use minase::datagen::statistics::{RecordedStatistics, Statistics};
use minase::datagen::{data_error, invalid_data, training_error};
use minase::nn::mnzt;
use minase::search::TranspositionTable;
use minase::training::provenance::{ResultOrigin, SearchCondition, StartOrigin};
use minase::training::records::{Header, Writer};
use minase_core::Rules;
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    fs::{self, File, OpenOptions},
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Instant,
};

/// 来歴を確定して出力ファイルを作り、生成が失敗すれば出力を削除する。
pub(super) fn generate(arguments: &GenerateArguments) -> io::Result<()> {
    let openings = arguments
        .openings
        .as_ref()
        .map(|path| read_openings(path))
        .transpose()?;
    if openings.is_some() && arguments.random_moves != 0 {
        return Err(data_error(OpeningError::RandomMoves));
    }
    let games = match &openings {
        Some(openings) => u32::try_from(openings.len()).map_err(data_error)?,
        None => arguments
            .games
            .ok_or_else(|| data_error(OpeningError::MissingGames))?,
    };
    let rules = engine_default_rules()?;
    let rule_set = rules.to_string();
    let generation_commit = git_output(&["rev-parse", "HEAD"])?;
    validate_commit_hash(&generation_commit)?;
    let status = git_output(&["status", "--porcelain"])?;
    if !status.is_empty() && !arguments.allow_dirty {
        return Err(invalid_data(
            "the working tree is dirty; commit changes or pass --allow-dirty",
        ));
    }

    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&arguments.output)?;
    let mut created = vec![arguments.output.clone()];
    let result = create_policy_files(arguments, &mut created).and_then(|policy_files| {
        write_training_data(
            file,
            policy_files,
            arguments,
            rules,
            &rule_set,
            &generation_commit,
            openings.as_deref(),
            games,
        )
    });
    // 失敗した出力を残すと、Readerが受理する空ファイルや破損ファイルが最終パスに
    // 残り、同じコマンドの再実行もcreate_newで拒否されるため、失敗時は削除する。
    // 方策の列の一時置き場は、成否にかかわらず削除する。
    let scratch = arguments.mnzt_output.as_deref().map(policy_scratch_path);
    for path in created {
        if (result.is_err() || scratch.as_ref() == Some(&path))
            && let Err(remove_error) = fs::remove_file(&path)
        {
            eprintln!(
                "error: cannot remove incomplete output {}: {remove_error}",
                path.display()
            );
        }
    }
    result?;
    let mut provenance_path = arguments.output.as_os_str().to_owned();
    provenance_path.push(".provenance.json");
    write_mapped_provenance(
        &ProvenanceArguments {
            input: arguments.output.clone(),
            output: provenance_path.into(),
            result_origin: ResultOrigin::Selfplay,
            start_origin: if openings.is_some() {
                StartOrigin::HumanGame
            } else {
                StartOrigin::Random
            },
            lambda: 0.75,
            search_condition: SearchCondition::InGame,
        },
        openings.map(|openings| openings.into_iter().map(|opening| opening.origin).collect()),
    )
}

/// MNZTの出力先と、方策の列を溜める一時置き場。
struct PolicyFiles {
    /// MNZTの出力先。
    output: File,
    /// 方策の列の一時置き場。
    scratch: File,
}

/// MNZTの出力先の隣に置く、方策の列の一時置き場の経路を返す。
fn policy_scratch_path(output: &Path) -> PathBuf {
    let mut path = output.as_os_str().to_owned();
    path.push(".policy.tmp");
    path.into()
}

/// `--mnzt-output`の指定時に、MNZTの出力先と一時置き場を新規作成する。
///
/// 作成したファイルの経路を`created`へ加え、失敗時の削除を呼び出し側に任せる。
fn create_policy_files(
    arguments: &GenerateArguments,
    created: &mut Vec<PathBuf>,
) -> io::Result<Option<PolicyFiles>> {
    let Some(path) = &arguments.mnzt_output else {
        return Ok(None);
    };
    let output = OpenOptions::new().write(true).create_new(true).open(path)?;
    created.push(path.clone());
    let scratch_path = policy_scratch_path(path);
    let scratch = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&scratch_path)?;
    created.push(scratch_path);
    Ok(Some(PolicyFiles { output, scratch }))
}

/// 自己対局を並列実行し、対局番号順にレコードを書き出して要約を表示する。
#[allow(clippy::too_many_arguments)]
fn write_training_data(
    file: File,
    policy_files: Option<PolicyFiles>,
    arguments: &GenerateArguments,
    rules: Rules,
    rule_set: &str,
    generation_commit: &str,
    openings: Option<&[Opening]>,
    games: u32,
) -> io::Result<()> {
    // 探索は埋め込み学習PSTで着手するので、その重み本体の検査和を生成元として残す。
    let pst = minase::eval::weights().map_err(|error| invalid_data(error.to_string()))?;
    let network_checksum = *pst.checksum();
    let header = Header::new(
        rule_set.to_owned(),
        generation_commit.to_owned(),
        network_checksum,
        arguments.nodes,
        arguments.seed,
        0,
    )
    .map_err(training_error)?;
    let mut writer = Writer::new(file, header).map_err(training_error)?;
    // MNZTはネットを使わずにαβ探索で生成するので、ネットの検査和は全零とする。
    let mut policy_writer = policy_files
        .map(|files| {
            let header = mnzt::Header::new(
                rule_set.to_owned(),
                generation_commit.to_owned(),
                [0; 32],
                mnzt::Teacher::AlphaBetaNodes(u64::from(arguments.nodes)),
                arguments.seed,
            )?;
            mnzt::Writer::new(files.output, files.scratch, header)
        })
        .transpose()
        .map_err(data_error)?;
    let next_game = AtomicU64::new(1);
    let (sender, receiver) = mpsc::channel::<io::Result<PlayedGame>>();
    let start = Instant::now();
    let progress_interval = u64::from(games).div_ceil(20).max(1);
    let play_settings = PlaySettings {
        base_seed: arguments.seed,
        nodes: arguments.nodes,
        random_moves: arguments.random_moves,
        max_ply: arguments.max_ply,
    };

    let mut policy_total = 0_u64;
    let total = thread::scope(|scope| -> io::Result<(Statistics, RecordedStatistics)> {
        for _ in 0..arguments.concurrency.get() {
            let sender = sender.clone();
            let next_game = &next_game;
            let pst = pst.as_ref();
            scope.spawn(move || {
                let mut table = match TranspositionTable::new(arguments.hash_mb.get()) {
                    Ok(table) => table,
                    Err(error) => {
                        let _ = sender.send(Err(invalid_data(error.to_string())));
                        return;
                    }
                };
                loop {
                    let game_number = next_game.fetch_add(1, Ordering::Relaxed);
                    if game_number > u64::from(games) {
                        break;
                    }
                    let game_number = match u32::try_from(game_number) {
                        Ok(number) => number,
                        Err(error) => {
                            let _ = sender.send(Err(invalid_data(error.to_string())));
                            break;
                        }
                    };
                    let completed = match catch_unwind(AssertUnwindSafe(|| {
                        play_game(
                            pst,
                            rules,
                            game_number,
                            play_settings,
                            &mut table,
                            openings.map(|items| &items[game_number as usize - 1]),
                        )
                    })) {
                        Ok(completed) => completed,
                        Err(_) => Err(invalid_data(format!(
                            "self-play worker panicked in game {game_number}"
                        ))),
                    };
                    let failed = completed.is_err();
                    if sender.send(completed).is_err() || failed {
                        break;
                    }
                }
            });
        }
        drop(sender);

        // MNZTはMNSDと同じく対局番号順に書く。MNSDの統合へ渡す前に、到着した対局の
        // MNZTの記録を番号順に書き出し、各対局を書き終えた時点の累計を進捗表示へ渡す。
        let policy_totals = RefCell::new(VecDeque::new());
        let mut pending_policy = BTreeMap::new();
        let mut next_policy_game = 1_u32;
        let messages = receiver.into_iter().map(|message| {
            let played = message?;
            let Some(policy_writer) = policy_writer.as_mut() else {
                return Ok(played.completed);
            };
            pending_policy.insert(played.completed.game_number, played.policy_records);
            while let Some(records) = pending_policy.remove(&next_policy_game) {
                for record in &records {
                    policy_writer.write_record(record).map_err(data_error)?;
                }
                policy_total += records.len() as u64;
                policy_totals.borrow_mut().push_back(policy_total);
                next_policy_game += 1;
            }
            Ok(played.completed)
        });
        merge_completed_games(messages, &mut writer, games, |completed_count, total| {
            let policy_records = policy_totals.borrow_mut().pop_front();
            if completed_count.is_multiple_of(progress_interval)
                || completed_count == u64::from(games)
            {
                let policy_field = policy_records
                    .map(|count| format!(" mnzt_records={count}"))
                    .unwrap_or_default();
                eprintln!(
                    "progress: games={completed_count}/{} records={}{policy_field} elapsed_seconds={:.3}",
                    games,
                    total.recorded_positions,
                    start.elapsed().as_secs_f64()
                );
            }
        })
    })?;

    let (total, recorded) = total;

    let file = writer.finish().map_err(training_error)?;
    file.sync_all()?;
    let policy_records = match policy_writer {
        Some(policy_writer) => {
            policy_writer.finish().map_err(data_error)?.sync_all()?;
            Some(policy_total)
        }
        None => None,
    };
    print_generation_summary(
        arguments,
        rule_set,
        generation_commit,
        &total,
        &recorded,
        policy_records,
        start.elapsed().as_secs_f64(),
    );
    Ok(())
}

/// 生成統計を設計書へ転記できる1項目1行の形式で表示する。
fn print_generation_summary(
    arguments: &GenerateArguments,
    rule_set: &str,
    generation_commit: &str,
    stats: &Statistics,
    recorded: &RecordedStatistics,
    policy_records: Option<u64>,
    elapsed_seconds: f64,
) {
    let game_count = stats.black_wins + stats.white_wins + stats.draws + stats.discarded_games;
    let games = game_count as f64;
    let per_second = |count: u64| {
        if elapsed_seconds == 0.0 {
            0.0
        } else {
            count as f64 / elapsed_seconds
        }
    };

    println!("summary:");
    println!("seed: {}", arguments.seed);
    println!("games: {game_count}");
    println!("nodes: {}", arguments.nodes);
    println!("concurrency: {}", arguments.concurrency);
    println!("max_ply: {}", arguments.max_ply);
    println!("hash_mb: {}", arguments.hash_mb);
    println!("random_moves_max: {}", arguments.random_moves);
    println!("rules: {rule_set}");
    println!("commit: {generation_commit}");
    println!("games_completed: {game_count}");
    println!("games_discarded_max_ply: {}", stats.discarded_games);
    println!("win_reason_royal_capture: {}", stats.royal_capture_wins);
    println!("win_reason_mate: {}", stats.mate_wins);
    println!("win_reason_stalemate: {}", stats.stalemate_wins);
    println!("win_reason_repetition: {}", stats.repetition_wins);
    println!(
        "win_reason_piece_exhaustion: {}",
        stats.piece_exhaustion_wins
    );
    println!("win_reason_bare_king: {}", stats.bare_king_wins);
    println!("win_reason_resignation: {}", stats.resignation_wins);
    println!("draw_reason_repetition: {}", stats.repetition_draws);
    println!(
        "draw_reason_piece_exhaustion: {}",
        stats.piece_exhaustion_draws
    );
    println!("draw_reason_bare_king: {}", stats.bare_king_draws);
    println!("draw_reason_agreement: {}", stats.agreement_draws);
    println!("black_wins: {}", stats.black_wins);
    println!("white_wins: {}", stats.white_wins);
    println!("draws: {}", stats.draws);
    println!("injections_planned: {}", stats.planned_injections);
    println!("injections_performed: {}", stats.performed_injections);
    for (index, count) in stats.injection_offset_histogram.iter().enumerate() {
        let start = index * 10;
        println!("injection_offset_histogram_{start}_{}: {count}", start + 9);
    }
    println!("searched_positions: {}", stats.searched_positions);
    println!("recordable_positions: {}", stats.recordable_positions);
    println!("recorded_positions: {}", stats.recorded_positions);
    if let Some(policy_records) = policy_records {
        println!("mnzt_recorded_positions: {policy_records}");
    }
    println!("excluded_mate_band: {}", stats.excluded_mate_band);
    println!(
        "excluded_mate_band_rate_percent: {}",
        format_rate_percent(stats.excluded_mate_band, stats.recordable_positions)
    );
    println!("excluded_tactical: {}", stats.excluded_tactical);
    println!(
        "excluded_tactical_rate_percent: {}",
        format_rate_percent(stats.excluded_tactical, stats.recordable_positions)
    );
    println!("excluded_repetition: {}", stats.excluded_repetition);
    println!(
        "excluded_repetition_rate_percent: {}",
        format_rate_percent(stats.excluded_repetition, stats.recordable_positions)
    );
    match score_mean_and_std(&recorded.score_frequencies) {
        Some((mean, standard_deviation)) => {
            println!("score_mean: {mean:.6}");
            println!("score_std: {standard_deviation:.6}");
            for percentile in [1, 5, 25, 50, 75, 95, 99] {
                let score = score_percentile(&recorded.score_frequencies, percentile)
                    .expect("a non-empty score table has every requested percentile");
                println!("score_p{percentile:02}: {score}");
            }
        }
        None => {
            println!("score_mean: n/a");
            println!("score_std: n/a");
            for percentile in [1, 5, 25, 50, 75, 95, 99] {
                println!("score_p{percentile:02}: n/a");
            }
        }
    }
    println!("duplicate_positions: {}", recorded.duplicate_positions);
    if stats.recorded_positions == 0 {
        println!("duplicate_rate_percent: n/a");
    } else {
        println!(
            "duplicate_rate_percent: {:.6}",
            recorded.duplicate_positions as f64 * 100.0 / stats.recorded_positions as f64
        );
    }
    println!("average_total_ply: {:.6}", stats.total_plies as f64 / games);
    println!(
        "average_searched_ply: {:.6}",
        stats.searched_plies as f64 / games
    );
    println!("elapsed_seconds: {elapsed_seconds:.6}");
    println!(
        "searched_positions_per_second: {:.6}",
        per_second(stats.searched_positions)
    );
    println!(
        "recorded_positions_per_second: {:.6}",
        per_second(stats.recorded_positions)
    );
    println!("searched_nodes_total: {}", stats.searched_nodes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selfplay_gen::test_support::RescoreFixture;
    use minase::datagen::{
        game::{CompletedGame, CompletedRecord},
        provenance::hex,
    };
    use minase::nn::policy;
    use minase::training::{
        provenance::Provenance,
        records::{Outcome, Reader, Record},
        rescore,
    };
    use minase_core::Game;
    use std::{collections::BTreeSet, io::Cursor, num::NonZeroUsize};

    #[test]
    fn generate_emits_provenance_after_completing_mnsd() {
        let fixture = RescoreFixture::new();
        let arguments = GenerateArguments {
            output: fixture.directory.join("generated.mnsd"),
            mnzt_output: None,
            games: Some(1),
            openings: None,
            seed: 42,
            nodes: 1,
            random_moves: 0,
            concurrency: NonZeroUsize::new(1).unwrap(),
            max_ply: 1,
            hash_mb: NonZeroUsize::new(1).unwrap(),
            allow_dirty: true,
        };
        generate(&arguments).unwrap();
        let path = fixture.directory.join("generated.mnsd.provenance.json");
        let provenance = Provenance::read(File::open(path).unwrap()).unwrap();
        let reader = Reader::new(File::open(&arguments.output).unwrap()).unwrap();
        assert_eq!(
            provenance.teacher.generation_commit,
            reader.header().generation_commit()
        );
        assert_eq!(
            provenance.teacher.network_checksum,
            hex(reader.header().network_checksum())
        );
        assert_eq!(
            provenance.mnsd_sha256,
            hex(&rescore::sha256(File::open(&arguments.output).unwrap()).unwrap())
        );
        assert_eq!(provenance.teacher.search_condition, SearchCondition::InGame);
        assert_eq!(provenance.result_origin, ResultOrigin::Selfplay);
        assert_eq!(provenance.start_origin, StartOrigin::Random);
    }

    /// MNZTには、MNSDの局面選別を通らない局面も含めて探索したすべての局面が、
    /// 探索の最善手1個を確率1の方策として書かれる。
    #[test]
    fn generate_writes_every_searched_position_to_mnzt() {
        let fixture = RescoreFixture::new();
        let mnzt_path = fixture.directory.join("generated.mnzt");
        // 2局とも手数上限の前に終局する値を、埋め込みの重みで確かめて選んだ。
        let arguments = GenerateArguments {
            output: fixture.directory.join("generated.mnsd"),
            mnzt_output: Some(mnzt_path.clone()),
            games: Some(2),
            openings: None,
            seed: 7,
            nodes: 200,
            random_moves: 4,
            concurrency: NonZeroUsize::new(2).unwrap(),
            max_ply: 1200,
            hash_mb: NonZeroUsize::new(1).unwrap(),
            allow_dirty: true,
        };
        generate(&arguments).unwrap();

        let mnsd_count = Reader::new(File::open(&arguments.output).unwrap())
            .unwrap()
            .header()
            .record_count();
        let reader = mnzt::Reader::new(File::open(&mnzt_path).unwrap()).unwrap();
        let header = reader.header().clone();
        assert_eq!(header.rule_set(), "L0,P0,R1,E0");
        assert_eq!(header.network_checksum(), &[0; 32]);
        assert_eq!(header.teacher(), mnzt::Teacher::AlphaBetaNodes(200));
        assert_eq!(header.seed(), 7);
        let records = reader.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(u64::try_from(records.len()).unwrap(), header.record_count());
        assert_eq!(header.policy_count(), header.record_count());
        // 局面選別と乱数手の前の局面を含むので、同じ生成のMNSDより記録が多い。
        assert!(header.record_count() > mnsd_count);
        assert_eq!(
            records
                .iter()
                .map(mnzt::Record::game_number)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([1, 2])
        );
        assert!(
            records
                .windows(2)
                .all(|pair| pair[0].game_number() <= pair[1].game_number())
        );
        let rules = engine_default_rules().unwrap();
        for record in &records {
            let [entry] = record.policy() else {
                panic!("an alpha-beta record has exactly one policy entry");
            };
            assert_eq!(entry.probability(), 1.0);
            let position = record.to_position().unwrap();
            let side_to_move = position.side_to_move();
            let legal_labels = Game::from_position(rules, position)
                .legal_moves()
                .into_iter()
                .map(|mv| policy::label(mv, side_to_move).unwrap())
                .collect::<BTreeSet<_>>();
            assert!(legal_labels.contains(&entry.label()));
            assert!((0.0..=1.0).contains(&record.value()));
        }
        assert!(!policy_scratch_path(&mnzt_path).exists());
    }

    /// 既存のMNZTの出力先は上書きせず、生成を始める前に失敗してMNSDも残さない。
    #[test]
    fn generate_refuses_an_existing_mnzt_output() {
        let fixture = RescoreFixture::new();
        let mnzt_path = fixture.directory.join("existing.mnzt");
        fs::write(&mnzt_path, b"keep").unwrap();
        let arguments = GenerateArguments {
            output: fixture.directory.join("generated.mnsd"),
            mnzt_output: Some(mnzt_path.clone()),
            games: Some(1),
            openings: None,
            seed: 42,
            nodes: 1,
            random_moves: 0,
            concurrency: NonZeroUsize::new(1).unwrap(),
            max_ply: 1,
            hash_mb: NonZeroUsize::new(1).unwrap(),
            allow_dirty: true,
        };

        let error = generate(&arguments).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&mnzt_path).unwrap(), b"keep");
        assert!(!arguments.output.exists());
        assert!(!policy_scratch_path(&mnzt_path).exists());
    }

    /// テスト用の完了対局を1レコード付きで作る。
    fn completed_game(game_number: u32) -> CompletedGame {
        let game = Game::new(engine_default_rules().expect("engine-default rules are valid"));
        let score = i16::try_from(game_number).unwrap() - 2;
        let search_key = if game_number >= 3 {
            10
        } else {
            u64::from(game_number) * 10
        };
        CompletedGame {
            game_number,
            records: vec![CompletedRecord {
                record: Record::from_position(
                    game.position(),
                    score,
                    Outcome::Draw,
                    game_number,
                    game.ply_count().try_into().unwrap(),
                ),
                search_key,
            }],
            stats: Statistics {
                searched_positions: u64::from(game_number),
                recordable_positions: 1,
                recorded_positions: 1,
                searched_nodes: u64::from(game_number) * 100,
                ..Statistics::default()
            },
        }
    }

    /// 到着順を変えて統合し、完成バイト列と集計を返す。
    fn merge_in_order(order: &[u32]) -> (Vec<u8>, Statistics, Vec<u64>, u64) {
        let header = Header::new("L0,P0,R1,E0".to_owned(), "0".repeat(40), [0; 32], 100, 1, 0)
            .expect("the test header is valid");
        let mut writer =
            Writer::new(Cursor::new(Vec::new()), header).expect("the in-memory writer is valid");
        let messages = order.iter().map(|&number| Ok(completed_game(number)));
        let (statistics, recorded) = merge_completed_games(messages, &mut writer, 4, |_, _| {})
            .expect("all four games are present");
        let bytes = writer
            .finish()
            .expect("the in-memory writer finishes")
            .into_inner();
        (
            bytes,
            statistics,
            recorded.score_frequencies,
            recorded.duplicate_positions,
        )
    }

    /// 同じ対局集合は到着順によらず対局番号順のデータと集計になる。
    #[test]
    fn completed_games_are_merged_independently_of_arrival_order() {
        let sequential = merge_in_order(&[1, 2, 3, 4]);
        let shuffled = merge_in_order(&[4, 2, 1, 3]);

        assert_eq!(sequential, shuffled);
        assert_eq!(sequential.1.recorded_positions, 4);
        assert_eq!(sequential.2.iter().sum::<u64>(), 4);
        assert_eq!(sequential.3, 2);
    }

    /// 対局番号が欠けたまま入力が終われば統合を拒否する。
    #[test]
    fn completed_game_merge_rejects_a_missing_game_number() {
        let header = Header::new("L0,P0,R1,E0".to_owned(), "0".repeat(40), [0; 32], 100, 1, 0)
            .expect("the test header is valid");
        let mut writer =
            Writer::new(Cursor::new(Vec::new()), header).expect("the in-memory writer is valid");
        let messages = [1, 3, 4].map(|number| Ok(completed_game(number)));

        let error = match merge_completed_games(messages, &mut writer, 4, |_, _| {}) {
            Ok(_) => panic!("game 2 is missing"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
