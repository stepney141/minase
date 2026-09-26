//! 保存済み局面の探索値の付け直しと再開。

use super::{cli::RescoreArguments, engine_default_rules};
use minase::datagen::{
    data_error,
    git::{git_output, validate_commit_hash},
    invalid_data,
};
use minase::eval::Pst;
use minase::search::{DEFAULT_THREADS, SearchLimits, SearchSnapshot, TranspositionTable, search};
use minase::training::records::{Reader, Record, best_move_is_tactical};
use minase::training::rescore::{self, RescoreEntry, RescoreHeader, RescoreReader, RescoreStatus};
use minase::{Game, MoveGenerator, Rules};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufReader, Seek, SeekFrom, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    sync::mpsc,
    thread,
    time::Instant,
};

/// 付け直しコマンドの進行に関する失敗。
#[derive(Debug)]
pub(super) enum RescoreCommandError {
    /// ワーカーが停止した。
    WorkerStopped,
    /// 探索ワーカーがパニックした。
    WorkerPanicked,
    /// 総ノード数の桁あふれ。
    NodeCountOverflow,
    /// 来歴コマンドが扱わない棋譜由来の指定。
    UnsupportedOrigin,
}

impl std::fmt::Display for RescoreCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::WorkerStopped => "rescore worker stopped before returning a result",
            Self::WorkerPanicked => "rescore worker panicked",
            Self::NodeCountOverflow => "rescore node count overflow",
            Self::UnsupportedOrigin => "provenance requires --result-origin selfplay and --start-origin random; game mappings belong to the game converter",
        })
    }
}

impl std::error::Error for RescoreCommandError {}

/// 付け直し全体の集計。再開前の記録も合計に含める。
#[derive(Default)]
struct RescoreSummary {
    rescored: u64,
    incomplete: u64,
    nodes: u64,
}

impl RescoreSummary {
    fn add(&mut self, entry: RescoreEntry) -> io::Result<()> {
        match entry.status() {
            RescoreStatus::NotTarget => (),
            RescoreStatus::Rescored => self.rescored += 1,
            RescoreStatus::Incomplete => self.incomplete += 1,
        }
        self.nodes = self
            .nodes
            .checked_add(entry.nodes())
            .ok_or_else(|| data_error(RescoreCommandError::NodeCountOverflow))?;
        Ok(())
    }
}

/// 既存MNRSは全条件と書き込み済み記録を検査し、新規ならヘッダを書く。
fn open_rescore_output(
    path: &Path,
    header: &RescoreHeader,
    targets: &[u64],
    summary: &mut RescoreSummary,
) -> io::Result<(File, u64)> {
    let encoded = header.encode().map_err(data_error)?;
    match OpenOptions::new().read(true).append(true).open(path) {
        Ok(file) => {
            let mut reader = RescoreReader::new(file).map_err(data_error)?;
            reader
                .header()
                .validate_resume(header)
                .map_err(data_error)?;
            let written = reader.written_count();
            for index in 0..written {
                let entry = reader
                    .read_entry()
                    .map_err(data_error)?
                    .ok_or_else(|| data_error(rescore::Error::InvalidLength))?;
                if (entry.status() != RescoreStatus::NotTarget)
                    != targets.binary_search(&index).is_ok()
                {
                    return Err(data_error(rescore::Error::TargetStatusMismatch(index)));
                }
                summary.add(entry)?;
            }
            Ok((reader.into_inner(), written))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&encoded)?;
            file.flush()?;
            Ok((file, 0))
        }
        Err(error) => Err(error),
    }
}

/// 履歴を復元せず、置換表を消去して1局面を探索する。
fn rescore_record(
    record: &Record,
    pst: &Pst,
    rules: Rules,
    limits: &SearchLimits,
    table: &mut TranspositionTable,
) -> io::Result<RescoreEntry> {
    let position = record.to_position().map_err(data_error)?;
    let game = Game::from_position(rules, position);
    let snapshot = SearchSnapshot::from_game(&game).map_err(data_error)?;
    table.clear();
    let result = search(pst, &snapshot, limits, DEFAULT_THREADS, table).map_err(data_error)?;
    if result.depth == 0 {
        return Ok(RescoreEntry::incomplete(result.nodes));
    }
    let tactical = best_move_is_tactical(
        game.position(),
        &MoveGenerator::new(rules.moves),
        result.best_move,
    )
    .map_err(data_error)?;
    let score = i16::try_from(result.score).map_err(data_error)?;
    RescoreEntry::rescored(tactical, score, result.depth, result.nodes).map_err(data_error)
}

/// 局面単位で並列化し、最大ワーカー数件の記録を番号順に書く。
///
/// 段階9「付け直しの探索の条件」。ワーカーは各自の表を使い回す。
/// 入力、未出力の結果ともに全件をメモリへ載せない。
fn write_rescore_records(
    reader: &mut Reader<BufReader<File>>,
    output: &mut File,
    arguments: &RescoreArguments,
    targets: &[u64],
    written: u64,
    pst: &Pst,
    summary: &mut RescoreSummary,
) -> io::Result<()> {
    let rules = engine_default_rules()?;
    let limits = SearchLimits::new(None, Some(u64::from(arguments.nodes)), None, None)
        .map_err(data_error)?;
    for _ in 0..written {
        reader.read_record().map_err(data_error)?;
    }
    let remaining = reader.header().record_count() - written;
    let workers =
        usize::try_from(remaining.min(arguments.concurrency.get() as u64)).map_err(data_error)?;
    thread::scope(|scope| -> io::Result<()> {
        let (results_tx, results_rx) = mpsc::channel();
        let mut jobs = Vec::new();
        for _ in 0..workers {
            let mut table = TranspositionTable::new(arguments.hash_mb.get()).map_err(data_error)?;
            let (job_tx, job_rx) = mpsc::sync_channel::<(usize, Record)>(1);
            jobs.push(job_tx);
            let results_tx = results_tx.clone();
            let limits = &limits;
            thread::Builder::new().spawn_scoped(scope, move || {
                while let Ok((slot, record)) = job_rx.recv() {
                    let result = match catch_unwind(AssertUnwindSafe(|| {
                        rescore_record(&record, pst, rules, limits, &mut table)
                    })) {
                        Ok(result) => result,
                        Err(_) => Err(data_error(RescoreCommandError::WorkerPanicked)),
                    };
                    let failed = result.is_err();
                    if results_tx.send((slot, result)).is_err() || failed {
                        break;
                    }
                }
            })?;
        }
        drop(results_tx);
        let mut index = written;
        let progress_interval = reader.header().record_count().div_ceil(20).max(1);
        while index < reader.header().record_count() {
            let mut entries = Vec::with_capacity(workers);
            let mut searching = 0;
            for job in &jobs {
                let Some(record) = reader.read_record().map_err(data_error)? else {
                    break;
                };
                let slot = entries.len();
                entries.push(RescoreEntry::not_target());
                if targets.binary_search(&(index + slot as u64)).is_ok() {
                    job.send((slot, record))
                        .map_err(|_| data_error(RescoreCommandError::WorkerStopped))?;
                    searching += 1;
                }
            }
            for _ in 0..searching {
                let (slot, entry) = results_rx
                    .recv()
                    .map_err(|_| data_error(RescoreCommandError::WorkerStopped))?;
                entries[slot] = entry?;
            }
            for entry in entries {
                // 1記録を1回のwrite_allで追記し、その記録を直ちにflushする。
                output.write_all(&entry.encode())?;
                output.flush()?;
                summary.add(entry)?;
                index += 1;
                if index.is_multiple_of(progress_interval)
                    || index == reader.header().record_count()
                {
                    eprintln!(
                        "progress: records={index}/{}",
                        reader.header().record_count()
                    );
                }
            }
        }
        Ok(())
    })
}

/// 実行条件を固定し、保存済みMNSDを読み取り専用で付け直す。
pub(super) fn rescore(arguments: &RescoreArguments) -> io::Result<()> {
    let start = Instant::now();
    let generation_commit = git_output(&["rev-parse", "HEAD"])?;
    validate_commit_hash(&generation_commit)?;
    if !git_output(&["status", "--porcelain"])?.is_empty() && !arguments.allow_dirty {
        return Err(invalid_data(
            "the working tree is dirty; commit changes or pass --allow-dirty",
        ));
    }
    let pst = match &arguments.pst {
        Some(path) => std::sync::Arc::new(Pst::decode(&fs::read(path)?).map_err(data_error)?),
        None => minase::eval::weights().map_err(data_error)?,
    };
    let mut input = File::open(&arguments.input)?;
    let mnsd_sha256 = rescore::sha256(&mut input)?;
    input.seek(SeekFrom::Start(0))?;
    let mut reader = Reader::new(BufReader::new(input)).map_err(data_error)?;
    let targets = rescore::read_targets(
        BufReader::new(File::open(&arguments.targets)?),
        reader.header().record_count(),
    )
    .map_err(data_error)?;
    let header = RescoreHeader {
        mnsd_sha256,
        record_count: reader.header().record_count(),
        targets_sha256: rescore::targets_checksum(&targets),
        target_count: targets.len() as u64,
        network_checksum: *pst.checksum(),
        nodes: arguments.nodes,
        // MNSDのヘッダと同じ表記（規則コードの列挙）で書く。
        rule_set: engine_default_rules()?.to_string(),
        hash_mb: u32::try_from(arguments.hash_mb.get()).map_err(data_error)?,
        generation_commit,
        binary_sha256: rescore::sha256(File::open(std::env::current_exe()?)?)?,
    };
    let mut summary = RescoreSummary::default();
    let (mut output, written) =
        open_rescore_output(&arguments.output, &header, &targets, &mut summary)?;
    let previous = summary.rescored + summary.incomplete;
    if written == header.record_count {
        println!("already_complete: true");
    } else {
        write_rescore_records(
            &mut reader,
            &mut output,
            arguments,
            &targets,
            written,
            &pst,
            &mut summary,
        )?;
    }
    let elapsed = start.elapsed().as_secs_f64();
    let searched = summary.rescored + summary.incomplete - previous;
    println!("targets: {}", header.target_count);
    println!("rescored: {}", summary.rescored);
    println!("incomplete_depth_one: {}", summary.incomplete);
    println!("searched_nodes: {}", summary.nodes);
    println!("elapsed_seconds: {elapsed:.3}");
    println!(
        "mean_seconds_per_position: {:.6}",
        if searched == 0 {
            0.0
        } else {
            elapsed / searched as f64
        }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{RescoreFixture, test_table};
    use minase::{Color, Position, training::records::Outcome};
    use std::{io::Cursor, num::NonZeroUsize};

    // 指示書D: 同じ条件、局面ごとの独立性、並列数によらない記録順、入力の不変性。
    #[test]
    fn rescore_is_reproducible_parallel_and_preserves_source() {
        let mut fixture = RescoreFixture::new();
        // 置換表の再利用が結果へ影響し得る深さまで探索する。
        fixture.arguments.nodes = 3_000;
        let before = rescore::sha256(File::open(&fixture.arguments.input).unwrap()).unwrap();
        rescore(&fixture.arguments).unwrap();
        let first = fs::read(&fixture.arguments.output).unwrap();
        fixture.arguments.output = fixture.directory.join("parallel.mnrs");
        fixture.arguments.concurrency = NonZeroUsize::new(3).unwrap();
        rescore(&fixture.arguments).unwrap();
        let second = fs::read(&fixture.arguments.output).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            rescore::sha256(File::open(&fixture.arguments.input).unwrap()).unwrap(),
            before
        );
        let mut reader = RescoreReader::new(Cursor::new(first)).unwrap();
        assert!(reader.is_complete());
        assert_eq!(reader.header().mnsd_sha256, before);
        assert_eq!(reader.header().target_count, 3);
        assert_eq!(
            reader.header().targets_sha256,
            rescore::targets_checksum(&[0, 2, 4])
        );
        assert_eq!(
            reader.header().network_checksum,
            *minase::eval::weights().unwrap().checksum()
        );
        assert_eq!(reader.header().rule_set, "L0,P0,R1,E0");
        let entries: Vec<_> = (0..5)
            .map(|_| reader.read_entry().unwrap().unwrap())
            .collect();
        assert_eq!(entries[0], entries[2]);
        assert_eq!(entries[2], entries[4]);
        assert_eq!(entries[0].status(), RescoreStatus::Rescored);
        assert!(entries[0].depth() >= 1);
        assert!(entries[0].nodes() > 0);
        assert_eq!(entries[1].encode(), [0; 16]);
        assert_eq!(entries[3].encode(), [0; 16]);
    }

    // 指示書D: 記録境界で中断した場合だけ再開し、無中断の出力に一致する。
    #[test]
    fn rescore_resumes_exactly_and_rejects_condition_changes_and_torn_entries() {
        let mut fixture = RescoreFixture::new();
        rescore(&fixture.arguments).unwrap();
        let complete = fs::read(&fixture.arguments.output).unwrap();
        for count in [0, 1, 3, 5] {
            fs::write(&fixture.arguments.output, &complete[..240 + count * 16]).unwrap();
            rescore(&fixture.arguments).unwrap();
            assert_eq!(fs::read(&fixture.arguments.output).unwrap(), complete);
        }
        let interrupted = &complete[..240 + 16];
        fs::write(&fixture.arguments.output, interrupted).unwrap();
        fixture.arguments.nodes = 201;
        let error = rescore(&fixture.arguments).unwrap_err();
        assert!(matches!(
            error.get_ref().unwrap().downcast_ref::<rescore::Error>(),
            Some(rescore::Error::HeaderMismatch("nodes"))
        ));
        assert_eq!(fs::read(&fixture.arguments.output).unwrap(), interrupted);
        fixture.arguments.nodes = 200;
        for (offset, expected) in [
            (8, "mnsd_sha256"),
            (40, "record_count"),
            (204, "binary_sha256"),
        ] {
            let mut changed = interrupted.to_vec();
            changed[offset] ^= 1;
            fs::write(&fixture.arguments.output, &changed).unwrap();
            let error = rescore(&fixture.arguments).unwrap_err();
            assert!(
                matches!(error.get_ref().unwrap().downcast_ref::<rescore::Error>(), Some(rescore::Error::HeaderMismatch(field)) if *field == expected)
            );
            assert_eq!(fs::read(&fixture.arguments.output).unwrap(), changed);
        }
        fs::write(&fixture.arguments.output, &complete[..240 + 17]).unwrap();
        let error = rescore(&fixture.arguments).unwrap_err();
        assert!(matches!(
            error.get_ref().unwrap().downcast_ref::<rescore::Error>(),
            Some(rescore::Error::InvalidLength)
        ));
    }

    #[test]
    fn rescore_marks_unfinished_depth_and_rejects_positions_without_legal_moves() {
        let mut fixture = RescoreFixture::new();
        fixture.arguments.nodes = 1;
        rescore(&fixture.arguments).unwrap();
        let mut reader =
            RescoreReader::new(File::open(&fixture.arguments.output).unwrap()).unwrap();
        let entry = reader.read_entry().unwrap().unwrap();
        assert_eq!(entry.status(), RescoreStatus::Incomplete);
        assert_eq!(entry.score(), 0);
        assert_eq!(entry.depth(), 0);
        assert!(!entry.tactical());
        assert!(entry.nodes() > 0);
        let empty = Record::from_position(&Position::empty(Color::Black), 0, Outcome::Draw, 1, 0);
        let limits = SearchLimits::new(None, Some(200), None, None).unwrap();
        let error = rescore_record(
            &empty,
            &minase::eval::weights().unwrap(),
            engine_default_rules().unwrap(),
            &limits,
            &mut test_table(),
        )
        .unwrap_err();
        assert!(matches!(
            error
                .get_ref()
                .unwrap()
                .downcast_ref::<minase::search::SearchError>(),
            Some(minase::search::SearchError::NoLegalMoves)
        ));
    }
}
