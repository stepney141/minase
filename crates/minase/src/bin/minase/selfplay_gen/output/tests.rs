//! 塊の確定、再開、組み直し、および出力先の拒否条件を検査する。

use super::*;
use crate::selfplay_gen::{
    opening::read_openings,
    play::{PlaySettings, play_game_from_opening},
    test_support::{TemporaryDirectory, epoch_zero_nnue, test_table},
};
use fs2::FileExt;
use minase::training::records::{Entry, GameRecord, Origin, Outcome, Policy, TeacherKind};
use minase_core::{Position, to_sfen};
use sha2::{Digest, Sha256};
use std::{
    fs::{FileTimes, OpenOptions},
    io::Cursor,
    time::{Duration, UNIX_EPOCH},
};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn nz_u16(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value).unwrap()
}

fn nz_u32(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

fn nz_usize(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).unwrap()
}

fn conditions(
    nnue: &Nnue,
    games: u32,
    chunk_games: u32,
    max_ply: u16,
    hash_mb: usize,
    openings_sha256: Option<String>,
    dirty: bool,
) -> GenerationConditions {
    GenerationConditions::new(
        Rules::JCSA,
        COMMIT.to_owned(),
        *nnue.checksum(),
        7,
        nz_u32(games),
        nz_u32(chunk_games),
        nz_u32(100),
        0,
        nz_u16(max_ply),
        nz_usize(hash_mb),
        openings_sha256,
        dirty,
    )
    .unwrap()
}

fn opening_fixture(directory: &Path, name: &str, plies: &[u32]) -> (Vec<Opening>, String) {
    use minase_core::{
        Color::{Black, White},
        PieceKind::{King, Rook},
        test_util::{position, sq},
    };
    let start = position(
        Black,
        &[
            (sq(5, 8), White, King),
            (sq(5, 5), Black, Rook),
            (sq(0, 1), Black, Rook),
            (sq(0, 0), Black, King),
        ],
    );
    let text = plies
        .iter()
        .enumerate()
        .map(|(index, ply)| format!("{name}-{index} {ply} {} - {}\n", to_sfen(&start), ply + 1))
        .collect::<String>();
    let path = directory.join(format!("{name}.txt"));
    fs::write(&path, text.as_bytes()).unwrap();
    let checksum = hex(&Sha256::digest(text.as_bytes()));
    (read_openings(text.as_bytes()).unwrap(), checksum)
}

fn run(
    directory: &Path,
    conditions: &GenerationConditions,
    nnue: &Nnue,
    openings: Option<&[Opening]>,
    concurrency: usize,
) -> RunReport {
    generate_to_directory(directory, conditions, nnue, openings, nz_usize(concurrency)).unwrap()
}

fn direct_reference(
    conditions: &GenerationConditions,
    nnue: &Nnue,
    openings: &[Opening],
) -> Vec<u8> {
    let mut writer = Writer::new(Cursor::new(Vec::new()), conditions.header.clone()).unwrap();
    let settings = PlaySettings {
        base_seed: conditions.config.seed,
        nodes: conditions.config.nodes.get(),
        random_moves: conditions.config.random_moves,
        max_ply: conditions.config.max_ply.get(),
    };
    let mut table = test_table();
    for (index, opening) in openings.iter().enumerate() {
        let game_number = u32::try_from(index + 1).unwrap();
        let game = play_game_from_opening(
            nnue,
            conditions.rules,
            game_number,
            settings,
            &mut table,
            Some(opening),
        )
        .unwrap();
        if let Some(record) = &game.record {
            writer.write_game(record).unwrap();
        }
    }
    writer.finish().unwrap().into_inner()
}

fn write_config(directory: &Path, conditions: &GenerationConditions) {
    fs::create_dir(directory).unwrap();
    atomic_json(directory, "config.json", &conditions.config).unwrap();
}

fn synthetic_record(number: u32, value: Value, exclusion: Exclusion) -> GameRecord {
    GameRecord::new(
        number,
        Origin::Synthetic,
        Outcome::NotFromGame,
        0,
        Position::initial(),
        vec![Entry::new(None, value, exclusion, Policy::None, None).unwrap()],
    )
    .unwrap()
}

fn write_chunk_with_header(
    directory: &Path,
    conditions: &GenerationConditions,
    header: Header,
    chunk: u32,
    records: &[GameRecord],
    statistics: Statistics,
) {
    let path = directory.join(chunk_mnsd_name(chunk));
    let mut writer = Writer::new(File::create(path).unwrap(), header).unwrap();
    for record in records {
        writer.write_game(record).unwrap();
    }
    writer.finish().unwrap().sync_all().unwrap();
    let first_game = u32::try_from(conditions.config.game_range(chunk).start).unwrap();
    let summary = ChunkSummary {
        chunk,
        first_game,
        statistics,
    };
    atomic_json(directory, &chunk_summary_name(chunk), &summary).unwrap();
}

fn write_chunk(
    directory: &Path,
    conditions: &GenerationConditions,
    chunk: u32,
    records: &[GameRecord],
    statistics: Statistics,
) {
    write_chunk_with_header(
        directory,
        conditions,
        conditions.header.clone(),
        chunk,
        records,
        statistics,
    );
}

fn output_error(result: io::Result<RunReport>) -> io::Error {
    match result {
        Ok(_) => panic!("the output must be rejected"),
        Err(error) => error,
    }
}

fn assert_report_data_eq(first: &RunReport, second: &RunReport) {
    assert_eq!(first.statistics, second.statistics);
    assert_eq!(first.recorded, second.recorded);
}

/// 塊を連結した本は、同じ対局を直接書いた参照出力と一致する。
#[test]
fn reassembled_teacher_matches_direct_writer_bytes() {
    let temporary = TemporaryDirectory::new();
    let nnue = epoch_zero_nnue();
    let (openings, checksum) = opening_fixture(&temporary.0, "reference", &[60; 5]);
    let conditions = conditions(&nnue, 5, 2, 64, 1, Some(checksum), false);
    let output = temporary.0.join("output");

    let report = run(&output, &conditions, &nnue, Some(&openings), 3);

    assert_eq!(report.generated_games, 5);
    assert_eq!(report.committed_games, 5);
    assert_eq!(report.chunks_reused, 0);
    let saved: serde_json::Value =
        serde_json::from_reader(File::open(output.join("config.json")).unwrap()).unwrap();
    assert_eq!(saved.as_object().unwrap().len(), 12);
    assert_eq!(
        saved["openings_sha256"].as_str(),
        conditions.config.openings_sha256.as_deref()
    );
    assert_eq!(saved["dirty"], false);
    for absent in ["concurrency", "output_dir", "openings"] {
        assert!(saved.get(absent).is_none());
    }
    assert_eq!(
        fs::read(output.join("teacher.mnsd")).unwrap(),
        direct_reference(&conditions, &nnue, &openings)
    );
}

/// 復号で求める分布、除外数、重複数は既知の入力と一致する。
#[test]
fn reassembly_statistics_match_hand_counted_records() {
    let temporary = TemporaryDirectory::new();
    let nnue = epoch_zero_nnue();
    let conditions = conditions(&nnue, 5, 3, 64, 1, None, false);
    let output = temporary.0.join("output");
    write_config(&output, &conditions);
    let first = [
        synthetic_record(1, Value::AlphaBeta(-2), Exclusion::None),
        synthetic_record(2, Value::AlphaBeta(0), Exclusion::Tactical),
        synthetic_record(3, Value::None, Exclusion::Repetition),
    ];
    write_chunk(
        &output,
        &conditions,
        0,
        &first,
        Statistics {
            black_wins: 3,
            searched_positions: 3,
            recordable_positions: 3,
            recorded_positions: 1,
            excluded_tactical: 1,
            excluded_repetition: 1,
            total_plies: 3,
            searched_plies: 3,
            searched_nodes: 300,
            ..Statistics::default()
        },
    );
    let second = [synthetic_record(4, Value::AlphaBeta(2), Exclusion::None)];
    write_chunk(
        &output,
        &conditions,
        1,
        &second,
        Statistics {
            discarded_games: 1,
            white_wins: 1,
            searched_positions: 1,
            recordable_positions: 1,
            recorded_positions: 1,
            total_plies: 1,
            searched_plies: 1,
            searched_nodes: 100,
            ..Statistics::default()
        },
    );

    let report = run(&output, &conditions, &nnue, None, 1);

    assert_eq!(report.generated_games, 0);
    assert_eq!(report.chunks_reused, 2);
    assert_eq!(report.statistics.black_wins, 3);
    assert_eq!(report.statistics.white_wins, 1);
    assert_eq!(report.statistics.discarded_games, 1);
    assert_eq!(report.recorded.exclusions, [0, 0, 1, 1]);
    assert_eq!(report.recorded.score_frequencies[score_index(-2)], 1);
    assert_eq!(report.recorded.score_frequencies[score_index(2)], 1);
    assert_eq!(report.recorded.score_frequencies.iter().sum::<u64>(), 2);
    assert_eq!(report.recorded.duplicate_positions, 1);
    assert_eq!(
        score_mean_and_std(&report.recorded.score_frequencies),
        Some((0.0, 2.0))
    );
    assert_eq!(
        score_percentile(&report.recorded.score_frequencies, 50),
        Some(-2)
    );
    let mut reader = Reader::new(File::open(output.join("teacher.mnsd")).unwrap()).unwrap();
    assert_eq!(reader.header().game_count(), 4);
    assert_eq!(reader.header().entry_count(), 4);
    let mut numbers = Vec::new();
    while let Some(game) = reader.read_game().unwrap() {
        numbers.push(game.record().game_number());
    }
    assert_eq!(numbers, [1, 2, 3, 4]);
}

/// 欠けた塊だけを作り直し、残した塊を変更しない。
#[test]
fn resume_regenerates_only_the_missing_chunk() {
    let temporary = TemporaryDirectory::new();
    let nnue = epoch_zero_nnue();
    let (openings, checksum) = opening_fixture(&temporary.0, "resume", &[60; 5]);
    let conditions = conditions(&nnue, 5, 2, 64, 1, Some(checksum), false);
    let output = temporary.0.join("output");
    run(&output, &conditions, &nnue, Some(&openings), 3);
    let teacher = fs::read(output.join("teacher.mnsd")).unwrap();
    let old_time = UNIX_EPOCH + Duration::from_secs(1_000_000);
    let mut original = BTreeMap::new();
    for chunk in 0..3 {
        for name in [chunk_mnsd_name(chunk), chunk_summary_name(chunk)] {
            let path = output.join(&name);
            let file = OpenOptions::new().write(true).open(&path).unwrap();
            file.set_times(FileTimes::new().set_modified(old_time))
                .unwrap();
            original.insert(name, fs::read(path).unwrap());
        }
    }
    fs::remove_file(output.join(chunk_mnsd_name(1))).unwrap();
    fs::remove_file(output.join(chunk_summary_name(1))).unwrap();
    fs::write(
        output.join(format!("{}.tmp", chunk_mnsd_name(1))),
        b"incomplete",
    )
    .unwrap();

    let report = run(&output, &conditions, &nnue, Some(&openings), 2);

    assert_eq!(report.generated_games, 2);
    assert_eq!(report.committed_games, 2);
    assert_eq!(report.chunks_reused, 2);
    assert_eq!(fs::read(output.join("teacher.mnsd")).unwrap(), teacher);
    for chunk in 0..3 {
        for name in [chunk_mnsd_name(chunk), chunk_summary_name(chunk)] {
            let path = output.join(&name);
            assert_eq!(fs::read(&path).unwrap(), original[&name]);
            if chunk != 1 {
                assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), old_time);
            }
        }
    }
}

/// 同時対局数は教師と本全体の集計を変えない。
#[test]
fn parallelism_changes_only_execution_fields() {
    let temporary = TemporaryDirectory::new();
    let nnue = epoch_zero_nnue();
    let (openings, checksum) = opening_fixture(&temporary.0, "parallel", &[60; 6]);
    let conditions = conditions(&nnue, 6, 2, 64, 1, Some(checksum), false);
    let first_path = temporary.0.join("first");
    let second_path = temporary.0.join("second");

    let first = run(&first_path, &conditions, &nnue, Some(&openings), 1);
    let second = run(&second_path, &conditions, &nnue, Some(&openings), 4);

    assert_report_data_eq(&first, &second);
    assert_eq!(
        fs::read(first_path.join("teacher.mnsd")).unwrap(),
        fs::read(second_path.join("teacher.mnsd")).unwrap()
    );
    for chunk in 0..3 {
        for name in [chunk_mnsd_name(chunk), chunk_summary_name(chunk)] {
            assert_eq!(
                fs::read(first_path.join(&name)).unwrap(),
                fs::read(second_path.join(name)).unwrap()
            );
        }
    }
}

/// 全塊が確定済みなら、置換表を作らず組み直す。
#[test]
fn completed_chunks_skip_transposition_table_construction() {
    let temporary = TemporaryDirectory::new();
    let nnue = epoch_zero_nnue();
    let conditions = conditions(&nnue, 1, 1, 1, usize::MAX, None, false);
    let output = temporary.0.join("output");
    write_config(&output, &conditions);
    write_chunk(
        &output,
        &conditions,
        0,
        &[],
        Statistics {
            discarded_games: 1,
            ..Statistics::default()
        },
    );

    let report = run(&output, &conditions, &nnue, None, 8);

    assert_eq!(report.generated_games, 0);
    assert_eq!(report.committed_games, 0);
    assert_eq!(report.chunks_reused, 1);
    let reader = Reader::new(File::open(output.join("teacher.mnsd")).unwrap()).unwrap();
    assert_eq!(reader.header().game_count(), 0);
}

/// 条件と一部の塊を写した出力先から残りを生成できる。
#[test]
fn copied_committed_chunks_resume_in_a_new_directory() {
    let temporary = TemporaryDirectory::new();
    let nnue = epoch_zero_nnue();
    let (openings, checksum) = opening_fixture(&temporary.0, "copy", &[60; 5]);
    let conditions = conditions(&nnue, 5, 2, 64, 1, Some(checksum), false);
    let original = temporary.0.join("original");
    let copied = temporary.0.join("copied");
    run(&original, &conditions, &nnue, Some(&openings), 3);
    fs::create_dir(&copied).unwrap();
    for name in [
        "config.json".to_owned(),
        chunk_mnsd_name(0),
        chunk_summary_name(0),
    ] {
        fs::copy(original.join(&name), copied.join(name)).unwrap();
    }

    let report = run(&copied, &conditions, &nnue, Some(&openings), 2);

    assert_eq!(report.generated_games, 3);
    assert_eq!(report.chunks_reused, 1);
    assert_eq!(
        fs::read(original.join("teacher.mnsd")).unwrap(),
        fs::read(copied.join("teacher.mnsd")).unwrap()
    );
    for chunk in 0..3 {
        for name in [chunk_mnsd_name(chunk), chunk_summary_name(chunk)] {
            assert_eq!(
                fs::read(original.join(&name)).unwrap(),
                fs::read(copied.join(name)).unwrap()
            );
        }
    }
}

/// 最後の短い塊と保存0局の塊も、開始局面一覧から再開できる。
#[test]
fn short_and_empty_chunks_resume_across_parallelism() {
    let temporary = TemporaryDirectory::new();
    let nnue = epoch_zero_nnue();
    let (openings, checksum) = opening_fixture(&temporary.0, "mixed", &[60, 61, 61, 61, 61]);
    let conditions = conditions(&nnue, 5, 2, 61, 1, Some(checksum), false);
    let output = temporary.0.join("output");
    let first = run(&output, &conditions, &nnue, Some(&openings), 1);
    let teacher = fs::read(output.join("teacher.mnsd")).unwrap();
    let empty = Reader::new(File::open(output.join(chunk_mnsd_name(1))).unwrap()).unwrap();
    assert_eq!(empty.header().game_count(), 0);
    let last: ChunkSummary =
        serde_json::from_reader(File::open(output.join(chunk_summary_name(2))).unwrap()).unwrap();
    assert_eq!(last.first_game, 5);
    assert_eq!(last.statistics.discarded_games, 1);
    fs::remove_file(output.join(chunk_mnsd_name(1))).unwrap();
    fs::remove_file(output.join(chunk_summary_name(1))).unwrap();
    fs::write(output.join(format!("{}.tmp", chunk_mnsd_name(1))), b"stale").unwrap();

    let resumed = run(&output, &conditions, &nnue, Some(&openings), 3);
    let other_path = temporary.0.join("other");
    let other = run(&other_path, &conditions, &nnue, Some(&openings), 4);

    assert_eq!(resumed.generated_games, 2);
    assert_eq!(resumed.chunks_reused, 2);
    assert_report_data_eq(&first, &resumed);
    assert_report_data_eq(&first, &other);
    assert_eq!(fs::read(output.join("teacher.mnsd")).unwrap(), teacher);
    assert_eq!(fs::read(other_path.join("teacher.mnsd")).unwrap(), teacher);
}

/// 生成条件と開始局面一覧の相違は欄名を持つエラーになる。
#[test]
fn config_mismatch_names_every_changed_field() {
    let temporary = TemporaryDirectory::new();
    let nnue = epoch_zero_nnue();
    let (openings, checksum) = opening_fixture(&temporary.0, "conditions-a", &[60; 2]);
    let original = conditions(&nnue, 2, 1, 64, 1, Some(checksum), false);
    let output = temporary.0.join("output");
    run(&output, &original, &nnue, Some(&openings), 1);

    macro_rules! mismatch {
        ($field:ident, $value:expr, $name:literal) => {{
            let mut changed = original.clone();
            changed.config.$field = $value;
            let error = output_error(generate_to_directory(
                &output,
                &changed,
                &nnue,
                None,
                nz_usize(1),
            ));
            let cause = error
                .get_ref()
                .unwrap()
                .downcast_ref::<OutputError>()
                .unwrap();
            assert!(
                matches!(cause, OutputError::ConfigMismatch(fields) if fields == &[$name]),
                "{cause}"
            );
        }};
    }
    mismatch!(rules, "different".to_owned(), "rules");
    mismatch!(generation_commit, "f".repeat(40), "generation_commit");
    mismatch!(network_checksum, "f".repeat(64), "network_checksum");
    mismatch!(seed, 8, "seed");
    mismatch!(games, nz_u32(3), "games");
    mismatch!(chunk_games, nz_u32(2), "chunk_games");
    mismatch!(nodes, nz_u32(101), "nodes");
    mismatch!(random_moves, 1, "random_moves");
    mismatch!(max_ply, nz_u16(65), "max_ply");
    mismatch!(hash_mb, nz_usize(2), "hash_mb");

    let (changed_openings, changed_checksum) =
        opening_fixture(&temporary.0, "conditions-b", &[60; 2]);
    let changed = conditions(&nnue, 2, 1, 64, 1, Some(changed_checksum), false);
    let error = output_error(generate_to_directory(
        &output,
        &changed,
        &nnue,
        Some(&changed_openings),
        nz_usize(1),
    ));
    assert!(matches!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<OutputError>()
            .unwrap(),
        OutputError::ConfigMismatch(fields) if fields == &["openings_sha256"]
    ));
}

/// 未コミットの変更を含む生成は塊をまたいで再開しない。
#[test]
fn dirty_state_rejects_committed_resume_and_rewrites_without_chunks() {
    let temporary = TemporaryDirectory::new();
    let nnue = epoch_zero_nnue();
    let (openings, checksum) = opening_fixture(&temporary.0, "dirty", &[60; 2]);
    for (saved_dirty, current_dirty) in [(true, false), (false, true)] {
        let saved = conditions(&nnue, 2, 1, 64, 1, Some(checksum.clone()), saved_dirty);
        let current = conditions(&nnue, 2, 1, 64, 1, Some(checksum.clone()), current_dirty);
        let output = temporary
            .0
            .join(format!("committed-{saved_dirty}-{current_dirty}"));
        run(&output, &saved, &nnue, Some(&openings), 1);
        let error = output_error(generate_to_directory(
            &output,
            &current,
            &nnue,
            Some(&openings),
            nz_usize(1),
        ));
        assert!(matches!(
            error
                .get_ref()
                .unwrap()
                .downcast_ref::<OutputError>()
                .unwrap(),
            OutputError::DirtyResume
        ));
    }
    for (saved_dirty, current_dirty) in [(true, false), (false, true)] {
        let saved = conditions(&nnue, 2, 1, 64, 1, Some(checksum.clone()), saved_dirty);
        let current = conditions(&nnue, 2, 1, 64, 1, Some(checksum.clone()), current_dirty);
        let output = temporary
            .0
            .join(format!("empty-{saved_dirty}-{current_dirty}"));
        write_config(&output, &saved);
        run(&output, &current, &nnue, Some(&openings), 1);
        let rewritten: Config =
            serde_json::from_reader(File::open(output.join("config.json")).unwrap()).unwrap();
        assert_eq!(rewritten.dirty, current_dirty);
    }
}

/// 集計JSONに対応するMNSDがなければ探索前に拒否する。
#[test]
fn missing_committed_mnsd_fails_before_table_construction() {
    let temporary = TemporaryDirectory::new();
    let nnue = epoch_zero_nnue();
    let conditions = conditions(&nnue, 2, 2, 1, usize::MAX, None, false);
    let output = temporary.0.join("output");
    write_config(&output, &conditions);
    atomic_json(
        &output,
        &chunk_summary_name(0),
        &ChunkSummary {
            chunk: 0,
            first_game: 1,
            statistics: Statistics {
                discarded_games: 2,
                ..Statistics::default()
            },
        },
    )
    .unwrap();

    let error = output_error(generate_to_directory(
        &output,
        &conditions,
        &nnue,
        None,
        nz_usize(1),
    ));

    assert!(matches!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<OutputError>()
            .unwrap(),
        OutputError::MissingChunkFile { chunk: 0, .. }
    ));
    assert!(!output.join("teacher.mnsd").exists());
}

/// 集計JSONの塊番号がファイル名と異なれば拒否する。
#[test]
fn chunk_summary_number_must_match_its_file_name() {
    let temporary = TemporaryDirectory::new();
    let nnue = epoch_zero_nnue();
    let conditions = conditions(&nnue, 2, 2, 1, 1, None, false);
    let output = temporary.0.join("output");
    write_config(&output, &conditions);
    fs::write(output.join(chunk_mnsd_name(0)), b"").unwrap();
    atomic_json(
        &output,
        &chunk_summary_name(0),
        &ChunkSummary {
            chunk: 1,
            first_game: 1,
            statistics: Statistics::default(),
        },
    )
    .unwrap();

    let error = output_error(generate_to_directory(
        &output,
        &conditions,
        &nnue,
        None,
        nz_usize(1),
    ));

    assert!(matches!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<OutputError>()
            .unwrap(),
        OutputError::ChunkSummaryNumberMismatch {
            file_chunk: 0,
            saved_chunk: 1,
        }
    ));
}

/// 集計JSONの先頭対局番号が担当範囲と異なれば拒否する。
#[test]
fn chunk_summary_first_game_must_match_its_range() {
    let temporary = TemporaryDirectory::new();
    let nnue = epoch_zero_nnue();
    let conditions = conditions(&nnue, 4, 2, 1, 1, None, false);
    let output = temporary.0.join("output");
    write_config(&output, &conditions);
    fs::write(output.join(chunk_mnsd_name(1)), b"").unwrap();
    atomic_json(
        &output,
        &chunk_summary_name(1),
        &ChunkSummary {
            chunk: 1,
            first_game: 2,
            statistics: Statistics::default(),
        },
    )
    .unwrap();

    let error = output_error(generate_to_directory(
        &output,
        &conditions,
        &nnue,
        None,
        nz_usize(1),
    ));

    assert!(matches!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<OutputError>()
            .unwrap(),
        OutputError::ChunkSummaryFirstGameMismatch {
            chunk: 1,
            saved: 2,
            expected: 3,
        }
    ));
}

/// ヘッダ、担当範囲、および塊の局数を組み直し前に検査する。
#[test]
fn reassembly_rejects_header_range_and_count_errors() {
    let temporary = TemporaryDirectory::new();
    let nnue = epoch_zero_nnue();
    let conditions = conditions(&nnue, 2, 2, 64, 1, None, false);

    let header_path = temporary.0.join("header");
    write_config(&header_path, &conditions);
    let wrong_header = Header::new(
        Rules::JCSA.to_string(),
        COMMIT.to_owned(),
        *nnue.checksum(),
        TeacherKind::AlphaBeta,
        100,
        8,
    )
    .unwrap();
    write_chunk_with_header(
        &header_path,
        &conditions,
        wrong_header,
        0,
        &[],
        Statistics {
            discarded_games: 2,
            ..Statistics::default()
        },
    );
    let error = output_error(generate_to_directory(
        &header_path,
        &conditions,
        &nnue,
        None,
        nz_usize(1),
    ));
    assert!(matches!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<OutputError>()
            .unwrap(),
        OutputError::HeaderMismatch { chunk: 0 }
    ));

    let range_path = temporary.0.join("range");
    write_config(&range_path, &conditions);
    write_chunk(
        &range_path,
        &conditions,
        0,
        &[synthetic_record(3, Value::AlphaBeta(0), Exclusion::None)],
        Statistics {
            discarded_games: 1,
            recorded_positions: 1,
            ..Statistics::default()
        },
    );
    let error = output_error(generate_to_directory(
        &range_path,
        &conditions,
        &nnue,
        None,
        nz_usize(1),
    ));
    assert!(matches!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<OutputError>()
            .unwrap(),
        OutputError::GameOutOfRange {
            chunk: 0,
            game: 3,
            ..
        }
    ));

    let count_path = temporary.0.join("count");
    write_config(&count_path, &conditions);
    write_chunk(
        &count_path,
        &conditions,
        0,
        &[synthetic_record(1, Value::AlphaBeta(0), Exclusion::None)],
        Statistics {
            recorded_positions: 1,
            ..Statistics::default()
        },
    );
    let error = output_error(generate_to_directory(
        &count_path,
        &conditions,
        &nnue,
        None,
        nz_usize(1),
    ));
    assert!(matches!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<OutputError>()
            .unwrap(),
        OutputError::ChunkGameCountMismatch {
            chunk: 0,
            saved: 1,
            discarded: 0,
            expected: 2,
        }
    ));
}

/// 条件のない非空出力先と、別記述子が保持するロックを拒否する。
#[test]
fn nonempty_and_locked_directories_are_rejected() {
    let temporary = TemporaryDirectory::new();
    let nnue = epoch_zero_nnue();
    let conditions = conditions(&nnue, 1, 1, 1, 1, None, false);
    let nonempty = temporary.0.join("nonempty");
    fs::create_dir(&nonempty).unwrap();
    fs::write(nonempty.join("unrelated"), b"keep").unwrap();
    let error = output_error(generate_to_directory(
        &nonempty,
        &conditions,
        &nnue,
        None,
        nz_usize(1),
    ));
    assert!(matches!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<OutputError>()
            .unwrap(),
        OutputError::NonemptyDirectory(path) if path == &nonempty
    ));

    let locked = temporary.0.join("locked");
    fs::create_dir(&locked).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(locked.join(LOCK_NAME))
        .unwrap();
    lock.try_lock_exclusive().unwrap();
    let error = output_error(generate_to_directory(
        &locked,
        &conditions,
        &nnue,
        None,
        nz_usize(1),
    ));
    assert!(matches!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<OutputError>()
            .unwrap(),
        OutputError::Locked { path, .. } if path == &locked
    ));
    FileExt::unlock(&lock).unwrap();
}

/// 壊れたJSONとMNSDは、パニックせず下位の型を保ったエラーとして拒否する。
#[test]
fn corrupt_external_files_return_typed_errors() {
    let temporary = TemporaryDirectory::new();
    let nnue = epoch_zero_nnue();
    let conditions = conditions(&nnue, 1, 1, 1, 1, None, false);

    let config_path = temporary.0.join("bad-config");
    fs::create_dir(&config_path).unwrap();
    fs::write(config_path.join("config.json"), b"{").unwrap();
    let error = output_error(generate_to_directory(
        &config_path,
        &conditions,
        &nnue,
        None,
        nz_usize(1),
    ));
    assert!(matches!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<OutputError>()
            .unwrap(),
        OutputError::Json(_)
    ));

    let summary_path = temporary.0.join("bad-summary");
    write_config(&summary_path, &conditions);
    fs::write(summary_path.join(chunk_summary_name(0)), b"{").unwrap();
    fs::write(summary_path.join(chunk_mnsd_name(0)), b"").unwrap();
    let error = output_error(generate_to_directory(
        &summary_path,
        &conditions,
        &nnue,
        None,
        nz_usize(1),
    ));
    assert!(matches!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<OutputError>()
            .unwrap(),
        OutputError::Json(_)
    ));

    let mnsd_path = temporary.0.join("bad-mnsd");
    write_config(&mnsd_path, &conditions);
    atomic_json(
        &mnsd_path,
        &chunk_summary_name(0),
        &ChunkSummary {
            chunk: 0,
            first_game: 1,
            statistics: Statistics {
                discarded_games: 1,
                ..Statistics::default()
            },
        },
    )
    .unwrap();
    fs::write(mnsd_path.join(chunk_mnsd_name(0)), b"broken").unwrap();
    let error = output_error(generate_to_directory(
        &mnsd_path,
        &conditions,
        &nnue,
        None,
        nz_usize(1),
    ));
    assert!(matches!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<OutputError>()
            .unwrap(),
        OutputError::Records(records::Error::Truncated(records::Section::Header))
    ));
}
