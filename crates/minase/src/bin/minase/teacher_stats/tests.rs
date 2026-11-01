use super::*;
use minase::training::records::{
    Entry, Exclusion, GameRecord, Header, Origin, Policy, TeacherKind, Value as TeacherValue,
    Writer, sorted_legal_moves,
};
use minase_core::{Color, Game, Position, Rules};
use std::error::Error as _;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "minase-teacher-stats-{}-{}-{}",
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

fn record(number: u32, start_ply: u16, plies: usize, outcome: Outcome) -> GameRecord {
    let mut game = Game::new(Rules::ENGINE_DEFAULT);
    let mut entries = Vec::new();
    for _ in 0..plies {
        let played = sorted_legal_moves(&game).unwrap()[0];
        entries.push(
            Entry::new(
                Some(played),
                TeacherValue::None,
                Exclusion::None,
                Policy::None,
                None,
            )
            .unwrap(),
        );
        game.play(played).unwrap();
    }
    entries.push(
        Entry::new(
            None,
            TeacherValue::None,
            Exclusion::None,
            Policy::None,
            None,
        )
        .unwrap(),
    );
    GameRecord::new(
        number,
        Origin::HumanGame,
        outcome,
        start_ply,
        Position::initial(),
        entries,
    )
    .unwrap()
}

fn write_games(path: &Path, records: &[GameRecord]) {
    let header = Header::new(
        "engine-default".into(),
        "0123456789abcdef0123456789abcdef01234567".into(),
        [0; 32],
        TeacherKind::AlphaBeta,
        1,
        1,
    )
    .unwrap();
    let mut writer = Writer::new(File::create(path).unwrap(), header).unwrap();
    for record in records {
        writer.write_game(record).unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn histogram_uses_nearest_rank_and_counts_duplicates() {
    let mut histogram = Histogram::default();
    for value in [1001, 0, 10, 10, 1000] {
        histogram.record(value);
    }
    assert_eq!(histogram.mean(), 404.2);
    assert_eq!(histogram.percentile(50), 10);
    assert_eq!(histogram.percentile(90), 1001);
    assert_eq!(histogram.percentile(95), 1001);
    assert_eq!(histogram.percentile(99), 1001);
    assert_eq!(
        histogram.bins.range(1001..).map(|(_, &n)| n).sum::<u64>(),
        1
    );
}

#[test]
fn summary_counts_games_positions_lengths_and_end_reasons_across_files() {
    let directory = TemporaryDirectory::new();
    let first = directory.0.join("first.mnsd");
    let second = directory.0.join("second.mnsd");
    let draw = Outcome::Game(GameResult::Draw {
        reason: DrawReason::Agreement,
    });
    let win = Outcome::Game(GameResult::Win {
        winner: Color::Black,
        reason: WinReason::Resignation,
    });
    write_games(&first, &[record(1, 0, 1, draw)]);
    write_games(&second, &[record(1, 10, 2, draw), record(2, 0, 0, win)]);

    let single = summarize(std::slice::from_ref(&first)).unwrap();
    assert_eq!(single["files"], 1);
    assert_eq!(single["games"], 1);
    assert_eq!(single["positions"], 2);
    assert_eq!(single["game_length_plies"]["max"], 1);

    // 手数は1、12、0で、終局面も含めた局面数は2 + 3 + 1 = 6となる。
    let combined = summarize(&[first, second]).unwrap();
    assert_eq!(combined["files"], 2);
    assert_eq!(combined["games"], 3);
    assert_eq!(combined["positions"], 6);
    assert_eq!(combined["game_length_plies"]["max"], 12);
    assert_eq!(
        combined["end_reasons"],
        json!({
            "not_from_game": 0,
            "win_royal_capture": 0,
            "win_repetition": 0,
            "win_piece_exhaustion": 0,
            "win_bare_king": 0,
            "win_no_legal_move": 0,
            "win_mate": 0,
            "win_resignation": 1,
            "win_mate_in_three": 0,
            "draw_repetition": 0,
            "draw_piece_exhaustion": 0,
            "draw_bare_king": 0,
            "draw_agreement": 2,
        })
    );
}

#[test]
fn missing_input_preserves_path_and_io_error() {
    let directory = TemporaryDirectory::new();
    let missing = directory.0.join("missing.mnsd");
    let error = summarize(std::slice::from_ref(&missing)).unwrap_err();
    assert!(error.to_string().contains(&missing.display().to_string()));
    assert_eq!(
        error
            .source()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::NotFound
    );
    assert!(matches!(error, Error::Open { path, .. } if path == missing));
}

#[test]
fn invalid_input_preserves_path_and_record_error() {
    let directory = TemporaryDirectory::new();
    let path = directory.0.join("invalid.mnsd");
    let mut header = [0; records::HEADER_LEN];
    header[..4].copy_from_slice(b"NOPE");
    fs::write(&path, header).unwrap();
    let error = summarize(std::slice::from_ref(&path)).unwrap_err();
    assert!(error.to_string().contains(&path.display().to_string()));
    assert!(matches!(
        error.source().unwrap().downcast_ref::<records::Error>(),
        Some(records::Error::InvalidMagic(magic)) if magic == b"NOPE"
    ));
    assert!(matches!(error, Error::Read { path: input, .. } if input == path));
}

#[test]
fn files_without_games_are_rejected() {
    let directory = TemporaryDirectory::new();
    let paths = [
        directory.0.join("empty1.mnsd"),
        directory.0.join("empty2.mnsd"),
    ];
    for path in &paths {
        write_games(path, &[]);
    }
    let error = summarize(&paths).unwrap_err();
    for path in &paths {
        assert!(error.to_string().contains(&path.display().to_string()));
    }
    assert!(matches!(error, Error::NoGames(inputs) if inputs == paths));
}
