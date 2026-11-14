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
            if !total.checked_merge(&completed.stats) {
                return Err(invalid_data("generation statistics count overflow"));
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::training::records::{
        Entry, Exclusion, Header, Origin, Outcome, Policy, TeacherKind, Value,
    };
    use minase_core::{DrawReason, Game, GameResult, Rules};
    use std::io::Cursor;

    fn completed_game(game_number: u32) -> CompletedGame {
        let game = Game::new(Rules::JCSA);
        let score = i16::try_from(game_number).unwrap() - 2;
        let search_key = if game_number >= 3 {
            10
        } else {
            u64::from(game_number) * 10
        };
        CompletedGame {
            game_number,
            record: Some(
                GameRecord::new(
                    game_number,
                    Origin::RandomSelfplay,
                    Outcome::Game(GameResult::Draw {
                        reason: DrawReason::Agreement,
                    }),
                    0,
                    game.position().clone(),
                    vec![
                        Entry::new(
                            None,
                            Value::AlphaBeta(score),
                            Exclusion::None,
                            Policy::None,
                            None,
                        )
                        .unwrap(),
                    ],
                )
                .unwrap(),
            ),
            records: vec![CompletedRecord { score, search_key }],
            stats: Statistics {
                searched_positions: u64::from(game_number),
                recordable_positions: 1,
                recorded_positions: 1,
                searched_nodes: u64::from(game_number) * 100,
                ..Statistics::default()
            },
        }
    }

    fn merge_in_order(order: &[u32]) -> (Vec<u8>, Statistics, Vec<u64>, u64) {
        let header = Header::new(
            "L0,P0,R1,E0".to_owned(),
            "0".repeat(40),
            [0; 32],
            TeacherKind::AlphaBeta,
            100,
            1,
        )
        .unwrap();
        let mut writer = Writer::new(Cursor::new(Vec::new()), header).unwrap();
        let messages = order.iter().map(|&number| Ok(completed_game(number)));
        let (statistics, recorded) =
            merge_completed_games(messages, &mut writer, 4, |_, _| {}).unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        (
            bytes,
            statistics,
            recorded.score_frequencies,
            recorded.duplicate_positions,
        )
    }

    #[test]
    fn completed_games_are_merged_independently_of_arrival_order() {
        let sequential = merge_in_order(&[1, 2, 3, 4]);
        let shuffled = merge_in_order(&[4, 2, 1, 3]);

        assert_eq!(sequential, shuffled);
        assert_eq!(sequential.1.recorded_positions, 4);
        assert_eq!(sequential.2.iter().sum::<u64>(), 4);
        assert_eq!(sequential.3, 2);
    }

    #[test]
    fn completed_game_merge_rejects_a_missing_game_number() {
        let header = Header::new(
            "L0,P0,R1,E0".to_owned(),
            "0".repeat(40),
            [0; 32],
            TeacherKind::AlphaBeta,
            100,
            1,
        )
        .unwrap();
        let mut writer = Writer::new(Cursor::new(Vec::new()), header).unwrap();
        let messages = [1, 3, 4].map(|number| Ok(completed_game(number)));

        let error = match merge_completed_games(messages, &mut writer, 4, |_, _| {}) {
            Ok(_) => panic!("game 2 is missing"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn completed_game_merge_rejects_statistics_overflow() {
        let header = Header::new(
            "L0,P0,R1,E0".to_owned(),
            "0".repeat(40),
            [0; 32],
            TeacherKind::AlphaBeta,
            100,
            1,
        )
        .unwrap();
        let mut writer = Writer::new(Cursor::new(Vec::new()), header).unwrap();
        let mut first = completed_game(1);
        first.stats.searched_positions = u64::MAX;
        let messages = [Ok(first), Ok(completed_game(2))];

        let error = match merge_completed_games(messages, &mut writer, 2, |_, _| {}) {
            Ok(_) => panic!("statistics overflow must be rejected"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
