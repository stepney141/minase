//! 生成した局面と対局のまとまり、および対局番号順の書き出し。

use std::collections::BTreeMap;
use std::io::{self, Seek, Write};

use crate::training::records::{Record, Writer};

use super::statistics::{RecordedStatistics, Statistics};
use super::{invalid_data, training_error};

/// 探索キーを伴う書き出し対象レコード。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CompletedRecord {
    /// 固定長形式へ書き出すレコード。
    pub record: Record,
    /// 対局間の局面重複を判定する探索キー。
    pub search_key: u64,
}

/// 1局分のレコードと統計。
#[derive(PartialEq, Eq, Debug)]
pub struct CompletedGame {
    /// 1から始まる対局番号。
    pub game_number: u32,
    /// 終局対局から採用したレコード。
    pub records: Vec<CompletedRecord>,
    /// 打ち切り対局を含む生成統計。
    pub stats: Statistics,
}

/// 任意の到着順の対局を対局番号順に書き出して集計する。
pub fn merge_completed_games<I, W, F>(
    messages: I,
    writer: &mut Writer<W>,
    games: u32,
    mut on_completed: F,
) -> io::Result<(Statistics, RecordedStatistics)>
where
    I: IntoIterator<Item = io::Result<CompletedGame>>,
    W: Write + Seek,
    F: FnMut(u64, &Statistics),
{
    let mut pending = BTreeMap::new();
    let mut next_to_write = 1_u64;
    let mut completed_count = 0_u64;
    let mut total = Statistics::default();
    let mut recorded = RecordedStatistics::default();
    for message in messages {
        let completed = message?;
        pending.insert(completed.game_number, completed);
        while let Some(completed) = pending.remove(&(next_to_write as u32)) {
            for record in &completed.records {
                writer
                    .write_record(&record.record)
                    .map_err(training_error)?;
                recorded.record(record);
            }
            total.merge(&completed.stats);
            completed_count += 1;
            on_completed(completed_count, &total);
            next_to_write += 1;
        }
    }
    if next_to_write != u64::from(games) + 1 {
        return Err(invalid_data(format!(
            "worker channel closed after {} of {} games",
            next_to_write - 1,
            games
        )));
    }
    Ok((total, recorded))
}
