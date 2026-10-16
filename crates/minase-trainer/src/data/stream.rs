//! MNSDを1局ずつ再生し、適格な局面を受け手へ入力順に渡す。
//! 先読みは対局内で計算し、読みかけの1局だけを保持する。
//! B=1の[`super::Dataset`]は、受け手が全局面をメモリへ集めて作る。

use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

use minase::training::records::{
    Exclusion, Header, Origin, Outcome as GameOutcome, Reader, TeacherKind, Value,
};

use super::{Error, Lookahead, Outcome, Result, Sample, compute_lookahead, is_validation};

/// 1ファイルから受け手へ渡した適格な局面の件数と入力情報。
pub(crate) struct FileSummary {
    /// 入力に指定されたパス。
    pub path: PathBuf,
    /// 復号済みのヘッダ。
    pub header: Header,
    /// 対局の由来。対局がないファイルではNone。
    pub origin: Option<Origin>,
    /// 受け手へ渡した適格な局面数。
    pub count: u64,
}

/// 受け手へ渡す局面、先読み後の探索値、および検証集合への所属。
pub(crate) struct Emitted {
    /// 元の探索値と対局情報を含む標本。
    pub sample: Sample,
    /// 先読み後の探索値をf32へ丸めた値。先読みなしでは元の探索値。
    pub teacher_score: f32,
    /// ヘッダのシードと対局番号で決まる検証集合への所属。
    pub validation: bool,
}

/// 1局ずつ検査・再生し、先読みを適用した適格な局面を入力順に渡す。
/// 受け手がエラーを返した時点で読み込みを止め、そのエラーを返す。
pub(crate) fn read_file(
    path: &Path,
    lookahead: Option<Lookahead>,
    sink: &mut dyn FnMut(Emitted) -> Result<()>,
) -> Result<FileSummary> {
    let mut reader = Reader::new(BufReader::new(File::open(path)?))?;
    if reader.header().teacher_kind() == TeacherKind::Mcts {
        return Err(Error::MctsTeacher {
            path: path.to_owned(),
        });
    }
    let mut summary = FileSummary {
        path: path.to_owned(),
        header: reader.header().clone(),
        origin: None,
        count: 0,
    };
    while let Some(game) = reader.read_game()? {
        let record = game.record();
        if record.origin() == Origin::Synthetic {
            return Err(Error::SyntheticOrigin {
                path: path.to_owned(),
            });
        }
        if summary
            .origin
            .is_some_and(|origin| origin != record.origin())
        {
            return Err(Error::MixedOrigins {
                path: path.to_owned(),
            });
        }
        summary.origin = Some(record.origin());
        let GameOutcome::Game(result) = record.outcome() else {
            return Err(Error::SyntheticOrigin {
                path: path.to_owned(),
            });
        };
        let mut samples = Vec::new();
        for (index, (entry, position)) in record.entries().iter().zip(game.positions()).enumerate()
        {
            if entry.exclusion() != Exclusion::None {
                continue;
            }
            let Value::AlphaBeta(score) = entry.value() else {
                continue;
            };
            let ply = u16::try_from(index)
                .ok()
                .and_then(|i| record.start_ply().checked_add(i))
                .ok_or_else(|| Error::Dataset {
                    path: path.to_owned(),
                    reason: "sample ply exceeds u16",
                })?;
            let sample = Sample::from_position(
                position,
                score,
                Outcome::from_game_result(result, position.side_to_move()),
                record.game_number(),
                ply,
                record.origin(),
            )?;
            samples.push(sample);
        }
        let raw: Vec<_> = samples
            .iter()
            .map(|sample| f64::from(sample.score()))
            .collect();
        let scores = match lookahead {
            Some(options) => {
                let games: Vec<_> = samples.iter().map(Sample::game_number).collect();
                let plies: Vec<_> = samples.iter().map(Sample::ply).collect();
                compute_lookahead(&games, &plies, &raw, options)?
            }
            None => raw,
        };
        let validation = is_validation(summary.header.seed(), record.game_number());
        for (sample, score) in samples.into_iter().zip(scores) {
            sink(Emitted {
                sample,
                teacher_score: score as f32,
                validation,
            })?;
            summary.count += 1;
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase::training::records::{Entry, GameRecord, Policy, Writer, sorted_legal_moves};
    use minase_core::{Color, Game, GameResult, Position, WinReason};
    use std::{
        fs,
        io::Cursor,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "minase-trainer-stream-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn write(&self, games: &[GameRecord]) -> PathBuf {
            let header = Header::new(
                "L0,P0,R1,E0".into(),
                "1".repeat(40),
                [1; 32],
                TeacherKind::AlphaBeta,
                1000,
                11,
            )
            .unwrap();
            let mut writer = Writer::new(Cursor::new(Vec::new()), header).unwrap();
            for game in games {
                writer.write_game(game).unwrap();
            }
            let path = self.0.join("games.mnsd");
            fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn played_game(number: u32, start_ply: u16, values: &[(Value, Exclusion)]) -> GameRecord {
        let mut game = Game::with_default_rules();
        let mut entries = Vec::new();
        for (index, &(value, exclusion)) in values.iter().enumerate() {
            let mv = if index + 1 == values.len() {
                None
            } else {
                Some(sorted_legal_moves(&game).unwrap()[index])
            };
            entries.push(Entry::new(mv, value, exclusion, Policy::None, None).unwrap());
            if let Some(mv) = mv {
                game.play(mv).unwrap();
            }
        }
        GameRecord::new(
            number,
            Origin::RandomSelfplay,
            GameOutcome::Game(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::Resignation,
            }),
            start_ply,
            Position::initial(),
            entries,
        )
        .unwrap()
    }

    #[test]
    fn emitted_samples_preserve_order_split_and_whole_file_lookahead_bits() {
        // 指示書の契約: 対局ごとの先読みはファイル全体の先読みとf32のビットが一致する。
        // 適格な項目の間の除外・値なしを手数から詰めず、対局の境界を越えて先読みしない。
        let fixture = Fixture::new();
        let values = [
            (Value::AlphaBeta(7), Exclusion::None),
            (Value::AlphaBeta(2), Exclusion::None),
            (Value::AlphaBeta(999), Exclusion::Tactical),
            (Value::AlphaBeta(1), Exclusion::None),
            (Value::None, Exclusion::None),
            (Value::AlphaBeta(-32768), Exclusion::None),
        ];
        let path = fixture.write(&[
            played_game(1, 10, &values),
            played_game(2, 0, &[(Value::None, Exclusion::None)]),
            played_game(4, 20, &values),
            played_game(5, u16::MAX, &[(Value::AlphaBeta(123), Exclusion::None)]),
        ]);
        let games = [1, 1, 1, 1, 4, 4, 4, 4, 5];
        let plies = [10, 11, 13, 15, 20, 21, 23, 25, u16::MAX];
        let raw = [7.0, 2.0, 1.0, -32768.0, 7.0, 2.0, 1.0, -32768.0, 123.0];
        for lookahead in [None, Some(Lookahead::new(0.9, 3).unwrap())] {
            let mut emitted = Vec::new();
            let summary = read_file(&path, lookahead, &mut |item| {
                emitted.push(item);
                Ok(())
            })
            .unwrap();
            assert_eq!(summary.path, path);
            assert_eq!(summary.header.seed(), 11);
            assert_eq!(summary.origin, Some(Origin::RandomSelfplay));
            assert_eq!(summary.count, 9);
            assert_eq!(emitted.len(), 9);
            let expected_scores = match lookahead {
                Some(options) => compute_lookahead(&games, &plies, &raw, options).unwrap(),
                None => raw.to_vec(),
            };
            let mut replay = Game::with_default_rules();
            let mut positions = vec![replay.position().clone()];
            for index in 0..5 {
                let mv = sorted_legal_moves(&replay).unwrap()[index];
                replay.play(mv).unwrap();
                positions.push(replay.position().clone());
            }
            for (index, item) in emitted.iter().enumerate() {
                assert_eq!(item.sample.game_number(), games[index]);
                assert_eq!(item.sample.ply(), plies[index]);
                assert_eq!(f64::from(item.sample.score()), raw[index]);
                assert_eq!(item.sample.origin(), Origin::RandomSelfplay);
                assert_eq!(item.validation, is_validation(11, games[index]));
                assert_eq!(item.validation, games[index] == 4);
                assert_eq!(
                    item.teacher_score.to_bits(),
                    (expected_scores[index] as f32).to_bits()
                );
                let local = if index == 8 {
                    0
                } else {
                    [0, 1, 3, 5][index % 4]
                };
                let position = &positions[local];
                assert_eq!(item.sample.to_position().unwrap(), *position);
                assert_eq!(
                    item.sample.outcome(),
                    if local % 2 == 0 {
                        Outcome::Win
                    } else {
                        Outcome::Loss
                    }
                );
            }
        }
    }

    #[test]
    fn sink_error_stops_before_later_samples_and_invalid_game() {
        let fixture = Fixture::new();
        let values = [(Value::AlphaBeta(7), Exclusion::None); 2];
        let path = fixture.write(&[played_game(1, 0, &values), played_game(4, 0, &values)]);
        // 後続対局の復号より先に、受け手が返した固有のエラーを伝える。
        let length = fs::metadata(&path).unwrap().len();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(length - 1)
            .unwrap();
        let mut calls = 0;
        let result = read_file(&path, Some(Lookahead::new(0.9, 3).unwrap()), &mut |_| {
            calls += 1;
            Err(Error::Io(std::io::Error::from_raw_os_error(28)))
        });
        assert!(matches!(result, Err(Error::Io(error)) if error.raw_os_error() == Some(28)));
        assert_eq!(calls, 1);
        assert!(matches!(
            read_file(&path, None, &mut |_| Ok(())),
            Err(Error::Mnsd(_))
        ));
    }

    #[test]
    fn empty_file_has_no_origin_or_emissions() {
        let fixture = Fixture::new();
        let path = fixture.write(&[]);
        let summary =
            read_file(&path, None, &mut |_| panic!("empty file emitted a sample")).unwrap();
        assert_eq!(summary.count, 0);
        assert_eq!(summary.origin, None);
    }
}
