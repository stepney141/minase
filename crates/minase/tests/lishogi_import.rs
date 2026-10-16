//! フェーズ5指示書の受理条件、再生境界、並列書き出しと開始局面の契約。
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use minase::training::records::{
    Entry, Exclusion, GameRecord, Header, Origin, Outcome, Policy, Reader, TeacherKind,
    Value as TeacherValue, Writer,
};
use minase_core::notation::{sfen::parse_extended_sfen, usi};
use minase_core::{Color, Game, GameResult, Rules, WinReason};
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
        fs::remove_dir_all(&self.0).expect("temporary directory must be removable");
    }
}
fn json_file(path: &Path) -> Value {
    serde_json::from_reader(File::open(path).unwrap()).unwrap()
}
fn import(directory: &Directory, mode: &str, name: &str, concurrency: &str) -> Output {
    let input = directory.path("input.ndjson");
    let mut command = Command::new(env!("CARGO_BIN_EXE_minase"));
    command.args(["data", "lishogi"]);
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
fn games_filter_in_specified_order_and_preserve_details_and_records() {
    let dir = Directory::new();
    let cases: Vec<Value> = CASES
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let input = ["SNjoPiHz", "Lqyj1bLC", "gZ0HcfLK", "variant", "illegal"]
        .into_iter()
        .map(|id| format!("{}\n", cases.iter().find(|game| game["id"] == id).unwrap()))
        .collect::<String>();
    fs::write(dir.path("input.ndjson"), input).unwrap();
    success(import(&dir, "games", "one.mnsd", "1"));
    success(import(&dir, "games", "two.mnsd", "2"));
    for suffix in ["", ".report.json"] {
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
    for (reason, ids) in [
        ("variant", vec!["variant"]),
        ("illegal_default_rules", vec!["illegal"]),
    ] {
        assert_eq!(report["excluded"][reason]["count"], ids.len(), "{reason}");
        assert_eq!(report["excluded"][reason]["ids"], json!(ids), "{reason}");
    }
    // 入力1、MNSD2、報告2の5ファイルだけを作る。
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 5);
    let mut reader = Reader::new(File::open(dir.path("one.mnsd")).unwrap()).unwrap();
    assert_eq!(reader.header().rule_set(), "L0,P0,R1,E0");
    assert_eq!(reader.header().teacher_search_budget(), 200);
    let mut counts = [0; 3];
    let mut last_game = 0;
    while let Some(decoded) = reader.read_game().unwrap() {
        let record = decoded.record();
        assert_eq!(record.origin(), Origin::HumanGame);
        assert_eq!(record.start_ply(), 0);
        assert_eq!(record.start_position(), &minase_core::Position::initial());
        let game = record.game_number();
        assert!(game > last_game);
        last_game = game;
        let winning_color = if game == 2 {
            Color::Black
        } else {
            Color::White
        };
        let id = ["SNjoPiHz", "Lqyj1bLC", "gZ0HcfLK"][game as usize - 1];
        let input: Value = CASES
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .find(|g| g["id"] == id)
            .unwrap();
        let mut replay = Game::new(Rules::ENGINE_DEFAULT);
        let mut moves = Vec::new();
        for text in input["moves"].as_str().unwrap().split_whitespace() {
            if replay.result().is_some() {
                break;
            }
            let mv = usi::parse(replay.position(), text).unwrap();
            replay.play(mv).unwrap();
            moves.push(mv);
        }
        assert_eq!(record.entries().len(), moves.len() + 1);
        assert_eq!(
            record.outcome(),
            Outcome::Game(match replay.result() {
                Some(result) => result,
                None => GameResult::Win {
                    winner: winning_color,
                    reason: WinReason::Resignation
                },
            })
        );
        let last = record.entries().last().unwrap();
        assert_eq!(last.played_move(), None);
        if replay.result().is_some() {
            if id == "SNjoPiHz" {
                assert_eq!(
                    record.outcome(),
                    Outcome::Game(GameResult::Win {
                        winner: Color::White,
                        reason: WinReason::RoyalCapture,
                    })
                );
            }
            assert_eq!(last.value(), TeacherValue::None);
            assert_eq!(last.policy(), &Policy::None);
            assert_eq!(last.exclusion(), Exclusion::None);
        } else {
            assert!(matches!(last.value(), TeacherValue::AlphaBeta(_)));
            assert!(
                matches!(last.policy(), Policy::Candidates(c) if c.len() == 1 && c[0].1 == 65_535)
            );
        }
        let mut exclusions = [0_u64; 4];
        for (k, entry) in record.entries().iter().enumerate() {
            assert_eq!(entry.played_move(), moves.get(k).copied());
            exclusions[entry.exclusion() as usize] += 1;
            match entry.policy() {
                Policy::PlayedMove => {
                    assert!(entry.played_move().is_some());
                }
                Policy::Candidates(c) => {
                    assert_eq!(c.len(), 1);
                    assert_eq!(c[0].1, 65_535);
                    assert_ne!(entry.played_move(), Some(c[0].0));
                }
                Policy::None => assert_eq!(k, moves.len()),
            }
            if entry.exclusion() == Exclusion::None && entry.value() != TeacherValue::None {
                counts[game as usize - 1] += 1;
            }
        }
        for (i, reason) in ["mate_band", "tactical", "repetition"].iter().enumerate() {
            assert_eq!(
                report["games"][id]["excluded_positions"][reason],
                exclusions[i + 1]
            );
        }
    }
    assert_eq!(last_game, 3);
    for (count, bound) in counts.into_iter().zip([22, 81, 19]) {
        assert!(count <= bound);
    }
    let details = &report["games"];
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
    assert!(report.get("games").is_none());
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
fn invalid_input_reports_line_and_does_not_leave_an_output() {
    let (name, invalid) = ("incomplete JSON", b"{".as_slice());
    let dir = Directory::new();
    let mut input = format!("{}\n", CASES.lines().next().unwrap()).into_bytes();
    input.extend_from_slice(invalid);
    input.push(b'\n');
    fs::write(dir.path("input.ndjson"), input).unwrap();
    let output = import(&dir, "games", "bad.mnsd", "1");
    assert_eq!(output.status.code(), Some(1), "{name}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("line 2"),
        "{name}"
    );
    assert!(!dir.path("bad.mnsd").exists(), "{name}");
    assert!(!dir.path("bad.mnsd.report.json").exists(), "{name}");
}

#[test]
fn opening_generation_cli_requires_zero_random_moves_and_one_game_per_line() {
    let dir = Directory::new();
    fs::write(dir.path("input.ndjson"), CASES).unwrap();
    success(import(&dir, "openings", "openings.txt", "1"));
    let command = |random_moves: &str, name: &str| {
        Command::new(env!("CARGO_BIN_EXE_minase"))
            .args(["data", "selfplay"])
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
                "400",
                "--hash-mb",
                "1",
                "--max-ply",
                "4000",
                "--allow-dirty",
            ])
            .output()
            .unwrap()
    };
    let rejected = command("1", "bad.mnsd");
    assert_eq!(rejected.status.code(), Some(1));
    assert!(!dir.path("bad.mnsd").exists());
    let output = command("0", "selfplay.mnsd");
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let summary: BTreeMap<_, _> = stdout
        .lines()
        .filter_map(|line| line.split_once(": "))
        .collect();
    let lines = fs::read_to_string(dir.path("openings.txt"))
        .unwrap()
        .lines()
        .count();
    assert_eq!(summary["games"].parse::<usize>().unwrap(), lines);
    success(output);
    let mut reader = Reader::new(File::open(dir.path("selfplay.mnsd")).unwrap()).unwrap();

    while let Some(decoded) = reader.read_game().unwrap() {
        assert_eq!(decoded.record().origin(), Origin::HumanOpeningSelfplay);
    }
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 4);
}

#[test]
fn existing_outputs_are_preserved() {
    let dir = Directory::new();
    fs::write(dir.path("input.ndjson"), CASES).unwrap();
    fs::write(dir.path("existing.mnsd"), b"keep").unwrap();
    assert_eq!(
        import(&dir, "games", "existing.mnsd", "1").status.code(),
        Some(1)
    );
    assert_eq!(fs::read(dir.path("existing.mnsd")).unwrap(), b"keep");
}

#[test]
fn inspect_accepts_and_displays_synthetic_records() {
    let dir = Directory::new();
    let path = dir.path("synthetic.mnsd");
    let header = Header::new(
        "L0,P0,R1,E0".into(),
        "0".repeat(40),
        [0; 32],
        TeacherKind::AlphaBeta,
        0,
        0,
    )
    .unwrap();
    let mut writer = Writer::new(File::create(&path).unwrap(), header).unwrap();
    writer
        .write_game(
            &GameRecord::new(
                0,
                Origin::Synthetic,
                Outcome::NotFromGame,
                0,
                minase_core::Position::initial(),
                vec![
                    Entry::new(
                        None,
                        TeacherValue::None,
                        Exclusion::None,
                        Policy::None,
                        None,
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        )
        .unwrap();
    writer.finish().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["data", "selfplay", "inspect"])
        .arg(path)
        .args(["--dump", "1"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let fields: BTreeMap<_, _> = text
        .lines()
        .filter_map(|line| line.split_once(": "))
        .collect();
    for (field, expected) in [
        ("origin", "Synthetic"),
        ("outcome", "none"),
        ("game_number", "0"),
        ("ply", "0"),
        ("played_move", "none"),
        ("policy", "none"),
    ] {
        assert_eq!(fields[field], expected, "{field}");
    }
}

/// 各欄の意味と、先頭N局というdumpの単位および全対局の集計を検査する。
#[test]
fn inspect_dump_reports_entry_fields_and_summarizes_all_games() {
    let dir = Directory::new();
    let path = dir.path("dump.mnsd");
    for kind in [TeacherKind::AlphaBeta, TeacherKind::Mcts] {
        let header = Header::new(
            "L0,P0,R1,E0".into(),
            "1".repeat(40),
            [0xab; 32],
            kind,
            42,
            7,
        )
        .unwrap();
        let mut writer = Writer::new(File::create(&path).unwrap(), header).unwrap();
        let mut game = Game::new(Rules::ENGINE_DEFAULT);
        let first = usi::parse(game.position(), "1i1h").unwrap();
        let initial = game.position().clone();
        game.play(first).unwrap();
        let last_position = game.position().clone();
        let best = game.legal_moves()[0];
        let (value, visits) = match kind {
            TeacherKind::AlphaBeta => (TeacherValue::AlphaBeta(123), None),
            TeacherKind::Mcts => (TeacherValue::Mcts(123), Some(456)),
        };
        let record = GameRecord::new(
            9,
            Origin::HumanGame,
            Outcome::Game(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::Resignation,
            }),
            4,
            initial.clone(),
            vec![
                Entry::new(
                    Some(first),
                    value,
                    Exclusion::Tactical,
                    Policy::PlayedMove,
                    visits,
                )
                .unwrap(),
                Entry::new(
                    None,
                    value,
                    Exclusion::None,
                    Policy::Candidates(vec![(best, 65_535)]),
                    visits,
                )
                .unwrap(),
            ],
        )
        .unwrap();
        writer.write_game(&record).unwrap();
        writer
            .write_game(
                &GameRecord::new(
                    12,
                    Origin::Synthetic,
                    Outcome::NotFromGame,
                    0,
                    initial.clone(),
                    vec![
                        Entry::new(
                            None,
                            TeacherValue::None,
                            Exclusion::None,
                            Policy::None,
                            visits.map(|_| 0),
                        )
                        .unwrap(),
                    ],
                )
                .unwrap(),
            )
            .unwrap();
        writer.finish().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_minase"))
            .args(["data", "selfplay", "inspect"])
            .arg(&path)
            .args(["--dump", "1"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        let (header, rest) = text.split_once("entry: 9 0\n").unwrap();
        let (first_entry, rest) = rest.split_once("entry: 9 1\n").unwrap();
        let (last_entry, summary) = rest.split_once("summary:\n").unwrap();
        let fields = |section: &str| -> BTreeMap<String, String> {
            section
                .lines()
                .filter_map(|line| line.split_once(": "))
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect()
        };
        let header = fields(header);
        for (key, expected) in [
            ("magic", "MNSD".to_owned()),
            ("format_version", "2".to_owned()),
            ("rule_set", "L0,P0,R1,E0".to_owned()),
            ("generation_commit", "1".repeat(40)),
            ("network_checksum", "ab".repeat(32)),
            ("teacher_kind", format!("{kind:?}")),
            ("teacher_search_budget", "42".to_owned()),
            ("seed", "7".to_owned()),
            ("game_count", "2".to_owned()),
            ("entry_count", "3".to_owned()),
        ] {
            assert_eq!(header[key], expected, "{kind:?}: header {key}");
        }
        for (section, position, side, exclusion, policy, played, outcome, ply) in [
            (
                first_entry,
                &initial,
                "Black",
                "tactical",
                "played".to_owned(),
                "1i1h",
                "Win",
                "4",
            ),
            (
                last_entry,
                &last_position,
                "White",
                "none",
                format!("1 {}:65535", usi::text_generated(&last_position, best)),
                "none",
                "Loss",
                "5",
            ),
        ] {
            let entry = fields(section);
            for (key, expected) in [
                ("sfen", minase_core::to_sfen(position)),
                ("side_to_move", side.to_owned()),
                ("lion_square", "none".to_owned()),
                ("lion_by_kirin_promotion", "false".to_owned()),
                ("value", "123".to_owned()),
                ("exclusion", exclusion.to_owned()),
                ("policy", policy),
                ("played_move", played.to_owned()),
                ("outcome", outcome.to_owned()),
                ("game_number", "9".to_owned()),
                ("ply", ply.to_owned()),
                ("origin", "HumanGame".to_owned()),
            ] {
                assert_eq!(entry[key], expected, "{kind:?}: ply={ply}, {key}");
            }
            assert_eq!(
                entry.get("root_visits").map(String::as_str),
                visits.map(|_| "456"),
                "{kind:?}: ply={ply}"
            );
            assert!(
                !entry.contains_key("entry"),
                "--dump 1 must omit the second game"
            );
        }
        let summary = fields(summary);
        for (key, expected) in [
            ("games", 2),
            ("entries", 3),
            ("games_origin_0", 0),
            ("games_origin_1", 0),
            ("games_origin_2", 1),
            ("games_origin_3", 1),
            ("exclusion_none", 2),
            ("exclusion_mate_band", 0),
            ("exclusion_tactical", 1),
            ("exclusion_repetition", 0),
            ("value_none", 1),
            ("policy_none", 1),
            ("policy_played", 1),
            ("policy_candidates", 1),
            ("policy_candidate_count_1", 1),
            ("eligible_positions", 1),
            ("origin_0", 0),
            ("origin_1", 0),
            ("origin_2", 1),
            ("origin_3", 0),
            ("outcome_loss", 1),
            ("outcome_draw", 0),
            ("outcome_win", 0),
            ("score_minimum", 123),
            ("score_maximum", 123),
            ("black_to_move", 0),
            ("white_to_move", 1),
        ] {
            assert_eq!(
                summary[key].parse::<u64>().unwrap(),
                expected,
                "{kind:?}: summary {key}"
            );
        }
        assert_eq!(summary["score_average"].parse::<f64>().unwrap(), 123.0);
        // 2局目だけを途中で切り、失敗位置と非0終了を検査する。
        let mut bytes = fs::read(&path).unwrap();
        bytes.pop();
        fs::write(&path, bytes).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_minase"))
            .args(["data", "selfplay", "inspect"])
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("game at file index 2:"));
    }
}

/// 保存対局だけの除外数と学習対象数はファイルから数え直して一致し、打ち切り対局は書かれない。
#[test]
fn selfplay_summary_counts_saved_entries_and_discards_capped_games() {
    let dir = Directory::new();
    for cap in [1200, 1] {
        let path = dir.path(&format!("selfplay-{cap}.mnsd"));
        let output = Command::new(env!("CARGO_BIN_EXE_minase"))
            .args(["data", "selfplay", "generate", "--output"])
            .arg(&path)
            .args([
                "--games",
                "1",
                "--seed",
                "7",
                "--nodes",
                "100",
                "--random-moves",
                "4",
                "--hash-mb",
                "1",
                "--concurrency",
                "1",
                "--max-ply",
                &cap.to_string(),
                "--allow-dirty",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        let summary: BTreeMap<_, _> = text.lines().filter_map(|l| l.split_once(": ")).collect();
        let mut reader = Reader::new(File::open(path).unwrap()).unwrap();
        let mut saved_games = 0;
        let mut eligible = 0;
        let mut exclusions = [0; 4];
        while let Some(decoded) = reader.read_game().unwrap() {
            saved_games += 1;
            for entry in decoded.record().entries() {
                exclusions[entry.exclusion() as usize] += 1;
                if entry.exclusion() == Exclusion::None && entry.value() != TeacherValue::None {
                    eligible += 1;
                }
            }
        }
        assert_eq!(
            summary["recorded_positions"].parse::<u64>().unwrap(),
            eligible
        );
        for (index, reason) in ["mate_band", "tactical", "repetition"].iter().enumerate() {
            assert_eq!(
                summary[format!("excluded_{reason}_in_saved_games").as_str()]
                    .parse::<u64>()
                    .unwrap(),
                exclusions[index + 1]
            );
        }
        if cap == 1 {
            assert_eq!(reader.header().game_count(), 0);
            assert_eq!(reader.header().entry_count(), 0);
            assert_eq!(summary["games_discarded_max_ply"], "1");
        }
        assert_eq!(reader.header().game_count(), saved_games);
        assert_eq!(
            reader.header().entry_count(),
            exclusions.iter().sum::<u64>()
        );
        assert_eq!(
            summary["games"].parse::<u64>().unwrap(),
            saved_games + summary["games_discarded_max_ply"].parse::<u64>().unwrap()
        );
    }
}
