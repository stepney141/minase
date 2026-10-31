//! 選択したMNSD対局の全着手を再生し、王手限定の3手以内の勝ちの判定時間を測る。
//! `--data <MNSD> --selection <external-evalのJSON> --out <出力JSON>`をすべて指定する。
//! 時間は判定の呼び出しだけをマイクロ秒で測り、読み込みと再生は含めない。
//! 分位点はnearest-rank、手数帯は1〜100手、101〜200手のように両端を含む。
//! 標本のない結果の平均と最大は、測定値がないことを示すJSONのnullとする。

#[path = "../../../../../../crates/minase/src/bin/minase/zero_selfplay/mate.rs"]
mod mate;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use minase::training::records::Reader;
use minase_core::{Game, GameStatus};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

// 自己対局生成器と同じアロケータで対局履歴を複製する。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
type Selection = BTreeSet<(u64, u32)>;

fn selection(value: &Value) -> Result<Selection> {
    let games = value
        .get("selected_games")
        .and_then(Value::as_array)
        .ok_or("selected_games must be an array")?;
    if games.is_empty() {
        return Err("selected_games must not be empty".into());
    }
    let mut selected = BTreeSet::new();
    for game in games {
        let seed = game
            .get("seed")
            .and_then(Value::as_u64)
            .ok_or("selected_games.seed must be a u64")?;
        let number = game
            .get("game")
            .and_then(Value::as_u64)
            .ok_or("selected_games.game must be a u32")?;
        let number = u32::try_from(number)?;
        if !selected.insert((seed, number)) {
            return Err(format!("duplicate selected game: seed={seed}, game={number}").into());
        }
    }
    Ok(selected)
}

fn file_info(path: &Path) -> Result<Value> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 65_536];
    let mut size = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok(json!({
        "path": path,
        "size": size,
        "sha256": minase::datagen::hex(&hasher.finalize()),
    }))
}

#[derive(Default)]
struct Timing {
    count: u64,
    total: f64,
    max: Option<f64>,
}

impl Timing {
    fn record(&mut self, micros: f64) {
        self.count += 1;
        self.total += micros;
        self.max = Some(match self.max {
            Some(previous) => previous.max(micros),
            None => micros,
        });
    }

    fn summary(&self) -> Value {
        json!({
            "count": self.count,
            "mean_us": (self.count != 0).then(|| self.total / self.count as f64),
            "max_us": self.max,
        })
    }
}

fn measure<R: Read>(reader: &mut Reader<R>, selected: &Selection) -> Result<Value> {
    let seed = reader.header().seed();
    let rules = reader.header().rules();
    if let Some(&(other, number)) = selected.iter().find(|(other, _)| *other != seed) {
        return Err(format!("selected game is not in data: seed={other}, game={number}").into());
    }
    let mut found = BTreeSet::new();
    let mut times = Vec::new();
    let mut totals = Timing::default();
    let mut wins = Timing::default();
    let mut no_wins = Timing::default();
    let mut skipped = Timing::default();
    let mut ply_bins: BTreeMap<u32, Timing> = BTreeMap::new();
    while let Some(decoded) = reader.read_game()? {
        let record = decoded.record();
        let key = (seed, record.game_number());
        if !selected.contains(&key) {
            continue;
        }
        if !found.insert(key) {
            return Err(format!("duplicate game in data: seed={}, game={}", key.0, key.1).into());
        }
        let mut game = Game::from_position(rules, record.start_position().clone());
        for entry in record.entries() {
            let Some(mv) = entry.played_move() else {
                continue;
            };
            if matches!(game.play(mv)?, GameStatus::Finished(_)) {
                continue;
            }
            let started = Instant::now();
            let verdict = mate::mate_in_three(&game);
            let micros = started.elapsed().as_secs_f64() * 1e6;
            match verdict? {
                mate::Verdict::Win => wins.record(micros),
                mate::Verdict::NoWin => no_wins.record(micros),
                mate::Verdict::TwoRoyals => skipped.record(micros),
            }
            times.push(micros);
            totals.record(micros);
            let ply = u32::from(record.start_ply()) + game.ply_count();
            let first = (ply - 1) / 100 * 100 + 1;
            ply_bins.entry(first).or_default().record(micros);
        }
        if found.len().is_multiple_of(25) || found.len() == selected.len() {
            eprintln!(
                "measured {}/{} games, {} calls, {:.3} s in the detector",
                found.len(),
                selected.len(),
                totals.count,
                totals.total / 1e6
            );
        }
    }
    if let Some(&(missing_seed, number)) = selected.difference(&found).next() {
        return Err(
            format!("selected game is not in data: seed={missing_seed}, game={number}").into(),
        );
    }
    times.sort_by(f64::total_cmp);
    // pは千分率。整数の順位で丸めを固定し、空なら分位点を定義しない。
    let percentile = |p: usize| {
        if times.is_empty() {
            None
        } else {
            Some(times[(p * times.len()).div_ceil(1000) - 1])
        }
    };
    let bins: Vec<_> = ply_bins
        .into_iter()
        .map(|(first, timing)| {
            json!({
                "first_ply": first,
                "last_ply": first + 99,
                "count": timing.count,
                "mean_us": timing.total / timing.count as f64,
            })
        })
        .collect();
    Ok(json!({
        "selected_game_count": selected.len(),
        "measured_game_count": found.len(),
        "call_count": totals.count,
        "latency_us": {
            "mean": (totals.count != 0).then(|| totals.total / totals.count as f64),
            "median": percentile(500),
            "p90": percentile(900),
            "p99": percentile(990),
            "p99_9": percentile(999),
            "max": totals.max,
            "total": totals.total,
        },
        "outcomes": {
            "win": wins.summary(),
            "no_win": no_wins.summary(),
            "skipped_two_royals": skipped.summary(),
        },
        "ply_bins": bins,
    }))
}

fn arguments() -> Result<(PathBuf, PathBuf, PathBuf)> {
    let mut args = std::env::args_os().skip(1);
    let mut options = BTreeMap::new();
    while let Some(key) = args.next() {
        let key = key.to_str().ok_or("option names must be UTF-8")?;
        if !["--data", "--selection", "--out"].contains(&key) {
            return Err(format!("unknown option: {key}").into());
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {key}"))?;
        if options
            .insert(key.to_owned(), PathBuf::from(value))
            .is_some()
        {
            return Err(format!("duplicate option: {key}").into());
        }
    }
    Ok((
        options.remove("--data").ok_or("--data required")?,
        options
            .remove("--selection")
            .ok_or("--selection required")?,
        options.remove("--out").ok_or("--out required")?,
    ))
}

fn run(data: &Path, selection_path: &Path, out: &Path) -> Result<()> {
    let selected = selection(&serde_json::from_reader(BufReader::new(File::open(
        selection_path,
    )?))?)?;
    let data_info = file_info(data)?;
    let selection_info = file_info(selection_path)?;
    let mut reader = Reader::new(BufReader::new(File::open(data)?))?;
    let mut report = measure(&mut reader, &selected)?;
    report["data"] = data_info;
    report["selection"] = selection_info;
    let mut writer = BufWriter::new(File::create(out)?);
    serde_json::to_writer_pretty(&mut writer, &report)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn main() -> Result<()> {
    let (data, selection, out) = arguments()?;
    run(&data, &selection, &out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase::training::records::{
        Entry, Exclusion, GameRecord, Header, Origin, Outcome, Policy, TeacherKind,
        Value as TeacherValue, Writer,
    };
    use minase_core::{Color, Move, PieceCode, PieceKind, Position, PositionBuilder, Square};
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "minase-mate-cost-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn step(from: (u8, u8), to: (u8, u8)) -> Move {
        Move {
            from: Square::new(from.0, from.1).unwrap(),
            mid: None,
            to: Square::new(to.0, to.1).unwrap(),
            promote: false,
        }
    }

    fn position(two_royals: bool) -> Position {
        use Color::{Black, White};
        use PieceKind::{King, Pawn, Rook};
        let mut builder = PositionBuilder::new(White);
        for (file, rank, color, kind) in [
            (11, 11, Black, King),
            (0, 5, Black, Rook),
            (0, 0, White, King),
            (11, 5, White, Pawn),
        ] {
            builder
                .put(
                    Square::new(file, rank).unwrap(),
                    PieceCode::new(color, kind).unwrap(),
                )
                .unwrap();
        }
        if two_royals {
            builder
                .put(
                    Square::new(1, 0).unwrap(),
                    PieceCode::new_promoted(White, PieceKind::CrownPrince).unwrap(),
                )
                .unwrap();
        }
        builder.finish().unwrap()
    }

    fn record(number: u32, ply: u16, start: Position, moves: &[Move]) -> GameRecord {
        let entries = moves
            .iter()
            .copied()
            .map(Some)
            .chain([None])
            .map(|mv| {
                Entry::new(mv, TeacherValue::None, Exclusion::None, Policy::None, None).unwrap()
            })
            .collect();
        GameRecord::new(
            number,
            Origin::Synthetic,
            Outcome::NotFromGame,
            ply,
            start,
            entries,
        )
        .unwrap()
    }

    fn write_data(path: &Path) {
        let header = Header::new(
            "engine-default".into(),
            "a".repeat(40),
            [0; 32],
            TeacherKind::AlphaBeta,
            1,
            42,
        )
        .unwrap();
        let mut writer = Writer::new(File::create(path).unwrap(), header).unwrap();
        let records = [
            // 最初の2手も計測し、序盤の除外をしない。
            record(
                1,
                0,
                Position::initial(),
                &[step((0, 3), (0, 4)), step((0, 8), (0, 7))],
            ),
            // 白歩の手の後は黒の1手勝ち。続く王捕獲の直後は終局済みなので数えない。
            record(
                2,
                0,
                position(false),
                &[step((11, 5), (11, 4)), step((0, 5), (0, 0))],
            ),
            record(3, 199, position(true), &[step((11, 5), (11, 4))]),
            // 選択しない対局は計数しない。
            record(4, 0, position(false), &[step((11, 5), (11, 4))]),
        ];
        for record in records {
            writer.write_game(&record).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn synthetic_mnsd_counts_all_recorded_moves_and_each_verdict() {
        let dir = TestDir::new();
        let data = dir.0.join("data.bin");
        let selected = dir.0.join("selection.json");
        let out = dir.0.join("report.json");
        write_data(&data);
        std::fs::write(
            &selected,
            serde_json::to_vec(&json!({"selected_games": [
                {"seed": 42, "game": 1}, {"seed": 42, "game": 2}, {"seed": 42, "game": 3}
            ]}))
            .unwrap(),
        )
        .unwrap();
        run(&data, &selected, &out).unwrap();
        let report: Value = serde_json::from_reader(File::open(out).unwrap()).unwrap();
        assert_eq!(report["selected_game_count"], 3);
        assert_eq!(report["measured_game_count"], 3);
        assert_eq!(report["call_count"], 4);
        assert_eq!(report["outcomes"]["win"]["count"], 1);
        assert_eq!(report["outcomes"]["no_win"]["count"], 2);
        assert_eq!(report["outcomes"]["skipped_two_royals"]["count"], 1);
        assert_eq!(report["ply_bins"][0]["first_ply"], 1);
        assert_eq!(report["ply_bins"][0]["count"], 3);
        assert_eq!(report["ply_bins"][1]["first_ply"], 101);
        assert_eq!(report["ply_bins"][1]["count"], 1);
    }

    #[test]
    fn malformed_and_duplicate_selections_are_errors() {
        for value in [
            json!({}),
            json!({"selected_games": []}),
            json!({"selected_games": {}}),
            json!({"selected_games": [{"seed": 42}]}),
            json!({"selected_games": [{"seed": -1, "game": 1}]}),
            json!({"selected_games": [{"seed": 42, "game": 4_294_967_296_u64}]}),
            json!({"selected_games": [{"seed": 42, "game": 1}, {"seed": 42, "game": 1}]}),
        ] {
            assert!(selection(&value).is_err(), "{value}");
        }
    }

    #[test]
    fn selected_games_must_exist_in_the_data() {
        let dir = TestDir::new();
        let path = dir.0.join("data.bin");
        write_data(&path);
        for key in [(42, 5), (43, 1)] {
            let mut reader = Reader::new(BufReader::new(File::open(&path).unwrap())).unwrap();
            assert!(measure(&mut reader, &BTreeSet::from([key])).is_err());
        }
    }
}
