//! 生成した局面と対局のまとまり、および対局番号順の書き出し。

use std::collections::BTreeMap;
use std::io::{self, Seek, Write};

use crate::training::records::{GameRecord, Writer};

use super::statistics::{RecordedStatistics, Statistics};
use super::{data_error, invalid_data};

/// 採用局面の探索値と、統計に使う探索キー。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CompletedRecord {
    /// 採用局面の探索値。
    pub score: i16,
    /// 対局間の局面重複を判定する探索キー。
    pub search_key: u64,
}

/// 1局分のレコードと統計。
#[derive(PartialEq, Eq, Debug)]
pub struct CompletedGame {
    /// 1から始まる対局番号。
    pub game_number: u32,
    /// 保存する対局。手数上限で破棄した場合はなし。
    pub record: Option<GameRecord>,
    /// 終局対局から採用した局面の統計用データ。
    pub records: Vec<CompletedRecord>,
    /// 打ち切り対局を含む生成統計。
    pub stats: Statistics,
}

/// 任意の到着順の対局を対局番号順に書き出して集計する。
/// 各対局の生成が決定的なら、生成コミット、シード、引数が同じ出力は並列度によらずバイト単位で一致する。
/// 生成側は手数上限とノード上限で1局の探索量を制限するため、ここでは書き出し待ち件数に上限を設けない。
/// ただし、この関数はワーカー数も各対局の所要時間も制御せず、待ち件数がワーカー数の定数倍に収まることは保証しない。
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
            if let Some(game) = &completed.record {
                writer.write_game(game).map_err(data_error)?;
                recorded.record_exclusions(game);
            }
            for record in &completed.records {
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
