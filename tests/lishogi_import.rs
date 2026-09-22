//! フェーズ5指示書の受理条件、再生境界、並列書き出しと開始局面の契約。
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use minase::eval::provenance::{Provenance, ResultOrigin, SearchCondition, StartOrigin};
use minase::eval::training_data::{Outcome, Reader};
use minase::notation::{sfen::parse_extended_sfen, usi};
use minase::{Color, Game, Rules};
use serde_json::{Value, json};

const CASES: &str = include_str!("fixtures/lishogi_import_cases.ndjson");
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "minase-phase5-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn json_file(path: &Path) -> Value {
    serde_json::from_reader(File::open(path).unwrap()).unwrap()
}
fn import(directory: &Directory, mode: &str, name: &str, concurrency: &str) -> Output {
    let input = directory.path("input.ndjson");
    let mut command = Command::new(env!("CARGO_BIN_EXE_lishogi_import"));
    command
        .args([mode, "--input"])
        .arg(input)
        .arg("--output")
        .arg(directory.path(name))
        .arg("--report")
        .arg(directory.path(&format!("{name}.report.json")))
        .args([
            "--nodes",
            "200",
            "--hash-mb",
            "1",
            "--concurrency",
            concurrency,
        ]);
    if mode == "games" {
        command.args(["--seed", "290501", "--allow-dirty"]);
    }
    command.output().unwrap()
}
fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn games_filter_in_specified_order_and_preserve_provenance_and_records() {
    let dir = Directory::new();
    fs::write(dir.path("input.ndjson"), CASES).unwrap();
    success(import(&dir, "games", "one.mnsd", "1"));
    success(import(&dir, "games", "two.mnsd", "2"));
    for suffix in ["", ".provenance.json", ".games.json", ".report.json"] {
        assert_eq!(
            fs::read(dir.path(&format!("one.mnsd{suffix}"))).unwrap(),
            fs::read(dir.path(&format!("two.mnsd{suffix}"))).unwrap()
        );
    }
    let report = json_file(&dir.path("one.mnsd.report.json"));
    assert_eq!(
        report["accepted_ids"],
        json!(["SNjoPiHz", "Lqyj1bLC", "gZ0HcfLK"])
    );
    assert_eq!(report["accepted"], 3);
    let expected = BTreeMap::from([
        ("variant", vec!["variant"]),
        ("initial_sfen", vec!["initial"]),
        ("bot", vec!["bot"]),
        ("unrated", vec!["unrated"]),
        ("missing_speed", vec!["missing-speed"]),
        ("rating", vec!["provisional", "missing-rating"]),
        ("status_outoftime", vec!["time", "opening-prefix-2u7dwJf9"]),
        ("status_draw", vec!["UUnYczs0"]),
        ("unknown_status", vec!["unknown"]),
        ("winner", vec!["missing-winner"]),
        ("duplicate", vec!["SNjoPiHz"]),
        ("illegal_default_rules", vec!["illegal"]),
        ("winner_mismatch", vec!["royals-not-lost"]),
        ("resigner_mismatch", vec!["wrong-winner"]),
    ]);
    for (reason, entry) in report["excluded"].as_object().unwrap() {
        let ids = expected.get(reason.as_str()).cloned().unwrap_or_default();
        assert_eq!(entry["count"], ids.len(), "{reason}");
        assert_eq!(entry["ids"], json!(ids), "{reason}");
    }
    let provenance =
        Provenance::read(File::open(dir.path("one.mnsd.provenance.json")).unwrap()).unwrap();
    assert_eq!(provenance.result_origin, ResultOrigin::Human);
    assert_eq!(provenance.start_origin, StartOrigin::Random);
    assert_eq!(
        provenance.teacher.search_condition,
        SearchCondition::Standalone
    );
    assert_eq!(provenance.lambda, 0.0);
    let mappings = provenance.games.unwrap();
    assert_eq!(
        mappings
            .iter()
            .map(|g| (g.game, g.id.as_str(), g.ply))
            .collect::<Vec<_>>(),
        vec![
            (1, "SNjoPiHz", None),
            (2, "Lqyj1bLC", None),
            (3, "gZ0HcfLK", None)
        ]
    );
    let mut reader = Reader::new(File::open(dir.path("one.mnsd")).unwrap()).unwrap();
    assert_eq!(reader.header().rule_set(), "L0,P0,R1,E0");
    assert_eq!(reader.header().teacher_nodes(), 200);
    let mut counts = [0; 3];
    let mut last_game = 1;
    while let Some(record) = reader.read_record().unwrap() {
        let game = record.game_number();
        assert!(game >= last_game);
        last_game = game;
        counts[game as usize - 1] += 1;
        assert!(record.ply() < [22, 81, 19][game as usize - 1]);
        let winning_color = if game == 2 {
            Color::Black
        } else {
            Color::White
        };
        assert_eq!(
            record.outcome(),
            if record.to_position().unwrap().side_to_move() == winning_color {
                Outcome::Win
            } else {
                Outcome::Loss
            }
        );
    }
    for (count, bound) in counts.into_iter().zip([22, 81, 19]) {
        assert!(count > 0 && count <= bound);
    }
    let details = json_file(&dir.path("one.mnsd.games.json"));
    assert_eq!(details["SNjoPiHz"]["sente_rating"], 1800);
    assert_eq!(details["SNjoPiHz"]["clock"]["byoyomi"], 30);
    for (index, id) in ["SNjoPiHz", "Lqyj1bLC", "gZ0HcfLK"].iter().enumerate() {
        assert_eq!(details[id]["recorded_positions"], counts[index]);
    }
}

#[test]
fn openings_keep_unrated_unfinished_games_and_stop_before_illegal_moves() {
    let dir = Directory::new();
    let mut game: Value = CASES
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|g| g["id"] == "opening-prefix-2u7dwJf9")
        .unwrap();
    game["rated"] = json!(false);
    game["winner"] = Value::Null;
    game["players"]["sente"]["provisional"] = json!(true);
    game["moves"] = json!(format!("{} 1あ", game["moves"].as_str().unwrap()));
    fs::write(dir.path("input.ndjson"), format!("{game}\n")).unwrap();
    success(import(&dir, "openings", "openings.txt", "2"));
    let report = json_file(&dir.path("openings.txt.report.json"));
    assert_eq!(report["accepted"], 1);
    assert_eq!(report["candidates"], 2);
    assert_eq!(report["retained"], 2);
    assert_eq!(report["truncated_illegal"]["opening-prefix-2u7dwJf9"], 61);
    assert_eq!(
        report["retained_plies"]["opening-prefix-2u7dwJf9"],
        json!([40, 60])
    );
    let mut replay = Game::new(Rules::ENGINE_DEFAULT);
    let mut expected = BTreeMap::new();
    for (index, text) in game["moves"]
        .as_str()
        .unwrap()
        .split_whitespace()
        .take(60)
        .enumerate()
    {
        replay
            .play(usi::parse(replay.position(), text).unwrap())
            .unwrap();
        if index + 1 == 40 || index + 1 == 60 {
            expected.insert(index + 1, replay.position().clone());
        }
    }
    for line in fs::read_to_string(dir.path("openings.txt"))
        .unwrap()
        .lines()
    {
        let mut fields = line.splitn(3, ' ');
        assert_eq!(fields.next(), Some("opening-prefix-2u7dwJf9"));
        let ply: usize = fields.next().unwrap().parse().unwrap();
        let setup =
            parse_extended_sfen(fields.next().unwrap(), Rules::ENGINE_DEFAULT.moves).unwrap();
        assert_eq!(setup.next_move_number() as usize, ply + 1);
        assert_eq!(setup.position(), &expected[&ply]);
    }
}

#[test]
fn invalid_json_reports_line_and_does_not_leave_an_output() {
    let dir = Directory::new();
    for invalid in ["{", "{}", "null", "{\"id\":42}"] {
        fs::write(
            dir.path("input.ndjson"),
            format!("{}\n{invalid}\n", CASES.lines().next().unwrap()),
        )
        .unwrap();
        let output = import(&dir, "games", "bad.mnsd", "1");
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("line 2"));
        assert!(!dir.path("bad.mnsd").exists());
    }
}

#[test]
fn opening_generation_cli_requires_zero_random_moves_and_one_game_per_line() {
    let dir = Directory::new();
    fs::write(dir.path("input.ndjson"), CASES).unwrap();
    success(import(&dir, "openings", "openings.txt", "1"));
    let command = |random_moves: &str, name: &str| {
        Command::new(env!("CARGO_BIN_EXE_selfplay_gen"))
            .args(["generate", "--openings"])
            .arg(dir.path("openings.txt"))
            .arg("--output")
            .arg(dir.path(name))
            .args([
                "--random-moves",
                random_moves,
                "--seed",
                "290502",
                "--nodes",
                "1",
                "--hash-mb",
                "1",
                "--max-ply",
                "65",
                "--allow-dirty",
            ])
            .output()
            .unwrap()
    };
    let rejected = command("1", "bad.mnsd");
    assert_eq!(rejected.status.code(), Some(1));
    assert!(!dir.path("bad.mnsd").exists());
    success(command("0", "selfplay.mnsd"));
    let provenance =
        Provenance::read(File::open(dir.path("selfplay.mnsd.provenance.json")).unwrap()).unwrap();
    assert_eq!(provenance.result_origin, ResultOrigin::Selfplay);
    assert_eq!(provenance.start_origin, StartOrigin::HumanGame);
    assert_eq!(provenance.lambda, 0.75);
    assert_eq!(
        provenance
            .games
            .unwrap()
            .iter()
            .map(|g| (g.game, g.id.as_str(), g.ply))
            .collect::<Vec<_>>(),
        vec![
            (1, "opening-prefix-2u7dwJf9", Some(40)),
            (2, "opening-prefix-2u7dwJf9", Some(60))
        ]
    );
}

#[test]
fn all_terminal_reasons_are_distinct_and_earlier_filters_win() {
    let dir = Directory::new();
    let base: Value = serde_json::from_str(CASES.lines().nth(3).unwrap()).unwrap();
    let statuses = [
        ("outoftime", "status_outoftime"),
        ("timeout", "status_timeout"),
        ("aborted", "status_aborted"),
        ("draw", "status_draw"),
        ("repetition", "status_repetition"),
        ("perpetualCheck", "status_perpetual_check"),
        ("bareKing", "status_bare_king"),
        ("created", "status_created"),
        ("started", "status_started"),
        ("mate", "status_mate"),
        ("stalemate", "status_stalemate"),
        ("cheat", "status_cheat"),
        ("noStart", "status_no_start"),
        ("unknownFinish", "status_unknown_finish"),
        ("future-status", "unknown_status"),
    ];
    let mut cases = Vec::new();
    for (status, _) in statuses {
        let mut case = base.clone();
        case["id"] = json!(status);
        case["status"] = json!(status);
        case["winner"] = Value::Null;
        cases.push(case);
    }
    for (id, field, value) in [
        ("no-user", "user", Value::Null),
        ("ai", "aiLevel", json!(1)),
    ] {
        let mut case = base.clone();
        case["id"] = json!(id);
        case["players"]["sente"][field] = value;
        cases.push(case);
    }
    let mut initial = base.clone();
    initial["id"] = json!("null-initial");
    initial["initialSfen"] = Value::Null;
    cases.push(initial);
    let mut duplicate = base.clone();
    duplicate["status"] = json!("draw");
    cases.extend([base, duplicate]);
    fs::write(
        dir.path("input.ndjson"),
        cases.iter().map(|c| format!("{c}\n")).collect::<String>(),
    )
    .unwrap();
    success(import(&dir, "games", "statuses.mnsd", "2"));
    let report = json_file(&dir.path("statuses.mnsd.report.json"));
    for (status, reason) in statuses {
        assert!(
            report["excluded"][reason]["ids"]
                .as_array()
                .unwrap()
                .contains(&json!(status))
        );
    }
    assert_eq!(report["accepted"], 1);
    assert_eq!(report["excluded"]["status_draw"]["count"], 2);
    assert_eq!(report["excluded"]["winner"]["count"], 0);
    assert_eq!(report["excluded"]["duplicate"]["count"], 0);
    assert_eq!(report["excluded"]["bot"]["ids"], json!(["no-user", "ai"]));
    assert_eq!(
        report["excluded"]["initial_sfen"]["ids"],
        json!(["null-initial"])
    );
}

#[test]
fn engine_terminal_winner_is_checked_and_later_moves_are_ignored() {
    let dir = Directory::new();
    let mut good: Value = serde_json::from_str(CASES.lines().next().unwrap()).unwrap();
    let mut bad = good.clone();
    bad["id"] = json!("contradictory");
    bad["winner"] = json!("sente");
    good["moves"] = json!(format!("{} 1あ", good["moves"].as_str().unwrap()));
    fs::write(dir.path("input.ndjson"), format!("{bad}\n{good}\n")).unwrap();
    success(import(&dir, "games", "end.mnsd", "1"));
    let report = json_file(&dir.path("end.mnsd.report.json"));
    assert_eq!(report["accepted_ids"], json!(["SNjoPiHz"]));
    assert_eq!(
        report["excluded"]["winner_mismatch"]["ids"],
        json!(["contradictory"])
    );
    assert_eq!(report["excluded"]["illegal_default_rules"]["count"], 0);
}

#[test]
fn invalid_utf8_has_line_number_and_existing_outputs_are_preserved() {
    let dir = Directory::new();
    let mut input = format!("{}\n", CASES.lines().next().unwrap()).into_bytes();
    input.extend([0xff, b'\n']);
    fs::write(dir.path("input.ndjson"), input).unwrap();
    let output = import(&dir, "games", "bad.mnsd", "1");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("line 2"));
    fs::write(dir.path("input.ndjson"), CASES).unwrap();
    fs::write(dir.path("existing.mnsd"), b"keep").unwrap();
    assert_eq!(
        import(&dir, "games", "existing.mnsd", "1").status.code(),
        Some(1)
    );
    assert_eq!(fs::read(dir.path("existing.mnsd")).unwrap(), b"keep");
}
